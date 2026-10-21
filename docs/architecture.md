# PACT 实现架构

对应需求：`docs/requirements.md`；详细设计：`docs/design.md`。本文件只保留当前架构，变更历史记入 `docs/architecture-change.md`。

## 1. 架构总览

单机单进程 daemon（`pact daemon start` 或 `pactd`，同一实现）为 Counter 的唯一权威；AI agent、应用程序、人类从三种接口接入同一 daemon。

```
 ┌─────────────────────────────── 接口层 ────────────────────────────────┐
 │                                                                       │
 │  AI agent 进程 ──MCP: stdio──► pact-mcp ───────┐                      │
 │                                                │                      │
 │  普通应用程序 ─────────────────────────────────► ├── REST ──► pact-http │
 │                                                │  (unix socket /      │  daemon
 │  人类 (终端) ──► pact-cli（在线命令）───────────┘   127.0.0.1)         │ (REST/JSON
 │     （离线命令 analyze/compose/replay：本地执行，不经 daemon）           │  + SSE)   │
 └───────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼（engine 仅嵌于 pact-http daemon 进程）
                ┌───────────────────┐
                │   pact-engine     │  引擎层：单权威 actor
                │  ┌─────────────┐  │  - 命令串行化（mpsc）
                │  │ 声明注册表   │  │  - 检查+应用+持久化原子完成
                │  │ domain→声明  │  │  - broadcast 通知总线（Rule-W 唤醒）
                │  ├─────────────┤  │  - Rule-T 终止检测（按实例）
                │  │ 实例会话表   │  │  - Clock / IdGen 注入
                │  │(domain,inst)│  │
                │  │ → {声明,状态,│  │
                │  │   订阅者 }   │  │
                │  └─────────────┘  │
                └───┬───────────┬───┘
                    │           │
        纯函数调用   │           │  持久化
                    ▼           ▼
        ┌────────────────┐  ┌─────────────────────────────┐
        │ pact-core      │  │ pact-store                  │
        │ pact-parser    │  │  <domain>/<instance>        │
        │ pact-analysis  │  │   .events.jsonl 真相源       │
        │ pact-compose   │  │   .snapshot.json（自足锚）    │
        └────────────────┘  │   .logmeta.json（压缩水位）   │
                            │  declaration.pact           │
                            │  index.tursodb              │
                            │   (domain,instance) 双列     │
                            │   (turso, IndexStore 隔离)   │
                            └─────────────────────────────┘
```

图例：`──►` 为调用/请求方向（同步请求）；`──MCP: stdio──►` 为子进程 stdin/stdout 上的双向 JSON-RPC 流，箭头仅示发起方；三条接入路径汇聚到同一 REST 通道（unix socket 默认 / 127.0.0.1 可选），该通道是通往引擎的唯一路径。分层自上而下：接口层 → 引擎层 → （纯函数语义层 / 存储层）。

核心约束：

- **依赖单向**：接口层 → 引擎 → （core/store）；core 不依赖 IO 与 async。
- **单一状态变更通道**：pact-http daemon 的 REST API 是通往引擎的唯一路径——pact-mcp 与 pact-cli（在线命令）都是其客户端（不内嵌引擎），三接口能力等价由构造保证；pact-cli 的离线命令（analyze/compose/replay/cpn）只依赖纯函数层，不触状态。
- **语义集中在纯函数层**：pact-core 的规则函数是范式的唯一权威实现；引擎只做编排（查域、调用、应用效果、持久化、通知），不含任何语义判断。
- **零隐式动作的编译期保障**：规则函数签名以 `&CounterState` 为入参、效果为返回值——接口层无法绕过规则直接改状态（Rust 所有权强制，见 design.md D1）。
- **纯模型可移植**：`pact-store-model`（事件/快照/实例键）、`pact-views`（查询/分析视图）与 `pact-engine-kernel`（实例会话语义）只依赖纯 crate（`pact-core`/`pact-parser`/`pact-analysis`/`serde`），可编译到 `wasm32-unknown-unknown`；`pact-store`/`pact-engine` 通过 re-export 保持既有 API（D29）。原生 `pact-engine` 与 `pact-wasm` 的实例会话均通过 `Deref` 嵌入同一个 `pact_engine_kernel::Session`，仅在持久化、通知、审计与错误映射等外围不同（D32）。`pact-wasm` 在其上提供 **WASM 本地分发形态（受支持，D31/D33）**：内存实例会话 + 两阶段提交 + 日志/快照导出恢复；Node 宿主适配层（`crates/pact-wasm/js/`）负责 JSONL+fsync、快照 tmp/rename 与 lockfile 单写者，磁盘布局与原生共用。**权威仍是原生 daemon**——WASM 为进程内单用户嵌入形态，不提供 MCP/HTTP 服务、不承担 ACL/审计与多用户边界，不进入 `dist/` 产物集。稳定化验收（Q7 CI、金标扩面、kernel 抽取、宿主边界安全审视）已全部完成（2026-10-21），故按 D33 去除“实验性”标记；浏览器持久化（OPFS/SharedWorker）与 DSH/v2 集成仍未接入（FR21）。

## 2. 关键数据流

### 2.1 一次 Rule-P（produce）请求

```
agent ──MCP pact_produce──► pact-mcp ──REST POST /tokens──► pact-http ──Command::Produce──► engine actor
  engine: 查域会话 → rule_p(&domain, &state, ...)     [纯函数检查]
        → Ok(token) → 追加 JSONL(fsync，**提交点**) → 派生索引投影(turso，尽力而为) → state.apply
        → broadcast Produced 通知（唤醒等待者）
        → 每次变更后惰性求值 Rule-T
  ←── Ok(Token) / Err(RuleError 结构化) ──
```

检查与应用在 actor 临界区内原子完成；**提交点（JSONL fsync）之前**的失败不产生部分效果；提交点之后的派生步骤（索引投影 / 快照 / Terminated / 通知）尽力而为、不回滚（design.md D23）。Rule-E 额外支持可选 `request_id` 幂等（重复键返回首次归档结果，幂等表由日志重建）。

### 2.2 Rule-W（等待与唤醒）

- 主动：客户端订阅（MCP `pact_subscribe` / REST SSE），状态变更事件即时推送；
- 兜底：轮询 `pact_enabled` / `GET .../enabled`，获得当前可满足的 Rule-P 组合与见证前驱集合；
- 引擎自身不等待、不重试、不驱动参与者（被动引擎，design.md D5）。

### 2.3 启动恢复

```
daemon 启动（pact daemon start / pactd）→ 先取 data_dir 独占 flock（<data_dir>/pactd.lock，第二实例拒绝启动）
  顶层遗留单层文件（<domain>.events.jsonl / .snapshot.json）→ fail-fast（LegacyLayoutDetected）须人工迁移
  逐域实例（<domain>/<instance>.events.jsonl）：读 <instance>.logmeta.json 水位 base_seq → 截断尾部半行（崩溃残留）
       → 载入最近快照（seq ≥ 最后一次 DomainLoaded 才有效）→ 分两支：
           日志含 DomainLoaded ⇒ 重放 events.jsonl 尾部（seq > snapshot.seq）
           日志无 DomainLoaded（已压缩）⇒ **快照即锚**：以快照的 declaration/dl_seq/state/consumed_requests 为基补尾部事件；
               无快照或不覆盖 max(base_seq, 首行seq-1) ⇒ fail-fast（SnapshotRequiredForCompactedLog）
       → 校验/重建 index.tursodb（含 base_seq > 0 时补 prune_events 自愈）→ 重建实例内存态
       → 恢复声明注册表（读 <domain>/declaration.pact）→ 恢复订阅总线
       → compact_on_start 为真时逐实例自动压缩（单个失败仅告警，不阻断启动）
中部损坏行（完整行非法 JSON）→ fail-fast，命令以恢复失败统一应答（需人工介入）。
事件 schema 版本：缺 version 视为 v1；version > EVENT_SCHEMA_VERSION ⇒ fail-fast（UnsupportedSchemaVersion）。
```

### 2.4 日志压缩（D25 子项 1）

```
运维入口（REST POST .../compact | CLI pact compact | MCP pact_compact | 启动自动）
   engine: 取当前实例内存态 → Snapshot{version,seq,dl_seq,declaration,state,consumed_requests}
         → store.compact：① 写全量自足快照（**提交点**）
                          ② 写 <instance>.logmeta.json 水位 base_seq = max(seq-keep_last, prev_base)（单调不回退）
                          ③ 原子重写 events.jsonl（保留 seq > base_seq 行，tmp+fsync+rename）
                          ④ 裁剪索引 events 行（seq <= base_seq）
                          ⑤ 重置追加句柄续接点 last_seq = snapshot.seq
  ←── CompactionOutcome{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events} ──
```

压缩谓词 = 「已入快照的前缀」；被丢弃前缀不再可查（审计视图 = 快照 + 保留尾部事件），`tokens` 投影行保留。水位先于日志重写落盘，故崩溃于任一中间点均落到两个一致态之一：未截断超集态（重放全量）或已截断 + 水位 + 快照（快照锚恢复）。

### 2.5 读审计与域级授权（D25 子项 3）

```
Rule-R 成功 → engine actor（单写者）
   ├─ read_audit 关（默认）：无持久化副作用
   └─ read_audit 开：append <domain>/<instance>.read-audit.jsonl（单行 JSON + fsync）
                      │ 失败 ⇒ 本次读取 500（fail-closed，绝不「读成功但无审计」）
                      └─ 独立审计流：不产生事件 / 不推进 seq / 不参与重放·Rule-T·快照·压缩

REST 入口（除 /health|/ready|/metrics|/validate 外）
   → ACL 空表：短路（既有信任声明行为）
   → ACL 非空：Bearer 令牌 → principal → 匹配 domain/instance 或 domain/*
               无令牌 401；越权 403；GET /domains 按授权过滤
               启用的 ACL 覆盖生命周期 / 四类动作 / 查询 / compact / read-audit 视图
```

ACL 与审计均位于接口层与引擎 actor 边界，`pact-core` 不感知；`/compact` 在启用 ACL 后与其它实例端点同等受 `(domain, instance)` 约束。

### 2.6 CPN 编码生成（阶段 6，离线）

```
.pact 文件 ──► pact-parser（解析 + A.5 良构校验）──► Domain
                                                    │
                                                    ▼
                                   pact-cpn::generate(domain, CpnOptions)
                                                    │  （纯函数，零 IO）
                                                    ▼
                                            CPN Tools XML
                                                    │
                                       pact-cli `pact cpn --out`
                                                    ▼
                                              model.cpn
```

- 只依赖 `pact-core`；与 daemon、store、engine、MCP 完全解耦，不改变任何线上格式。
- 单实例作用域：每次生成只编码一个 `Counter instance`，库所/迁移以 `i_<instance>__` 前缀命名，跨实例不共享。
- 负条件（`not`/`count =`）需调用方提供 `--bound T=N` 的并发上界；`not via` 显式拒绝而非生成错误模型。
- 输出为 CPN Tools XML format 6，经 CPN DTD 校验；GUI 加载与状态空间实验属人工验收（D28）。

## 3. 进程与部署视图

```
单机（v1）
┌─────────────────────────────── 主机 ───────────────────────────────┐
│  pact-server (daemon)                                             │
│    ├─ MCP stdio（由 agent 启动时拉起的子进程形态，或连接 daemon）     │
│    ├─ MCP Streamable HTTP / REST + SSE  @ unix socket / 127.0.0.1  │
│  └─ data_dir（JSONL + snapshot + index.tursodb）                  │
│  外部 AI agent 进程（自驱，经 MCP 接入）                             │
│  pact-cli（人类）                                                   │
└───────────────────────────────────────────────────────────────────┘
```

- 单一静态二进制（`pact`：daemon 与 CLI 合一，子命令区分）；无运行时依赖。
- 无人值守闭环：外部脚本/systemd 启动 daemon 与各 agent 进程 → agent 加载域、注入种子（或种子由脚本注入）→ 参与者经 Counter 自组织推进 → Terminated 事件触发各 agent 退出（订阅可得）。

## 4. 联网演进（v2 预留，v1 不实现）

| 维度 | v1 | v2（联网） |
|------|----|-----------|
| 传输 | unix socket / localhost / stdio | TLS + 远程 MCP Streamable HTTP |
| 身份 | 信任声明 + 可选 bearer token | 参与者签名密钥 |
| 权威 | 单进程引擎（不变） | 同一引擎作为服务器（不变） |
| 语义/存储 | pact-core / pact-store | **零改动** |

架构上联网不引入新组件：Counter 单权威意味着联网形态就是 client/server，不存在多副本一致性问题。需要变更的全部集中在接口层的认证与传输。

## 5. 质量属性落点

| 属性 | 架构手段 |
|------|----------|
| 语义保真 | 语义只存在于纯函数 core（含良构校验、Rule-W/Rule-T 判定，D14）；黄金轨迹测试锚定论文全部示例 |
| 审计完整性 | JSONL 真相源即完整审计记录；DomainLoaded 事件存档声明原文；每条事件自足（Consumed 携 token 全量快照），单行即可审计与重放。**日志压缩后**（D25 子项 1）审计视图 = 自足快照（含声明/幂等表）+ 保留的尾部事件，被丢弃前缀不再可查 |
| 可恢复性 | 快照 + 重放；索引（turso，经 IndexStore trait 隔离，D13）永远可重建、滞后由启动一致性校验触发重建自愈。压缩后日志无 `DomainLoaded` 时以**快照为锚**恢复（`snapshot_is_anchor`），快照缺失/不覆盖水位则 fail-fast；事件 schema 版本缺省按 v1、更高版本 fail-fast |
| 可测试性 | Clock/IdGen 注入；core 纯环境属性测试；四层测试组织（单元/属性/集成/端到端+回归） |
| 安全边界 | 单机默认 unix socket；参与者令牌可选；**启用 `[auth.acl]` 后**（D25 子项 3）全部域作用域端点按 `(domain, instance)` 授权（支持 `domain/*` 通配），无令牌 `401`、越权 `403`，`GET /domains` 按授权过滤；ACL 非空而令牌表为空 ⇒ 拒绝启动。未配置 ACL 时保持既有信任声明行为（生命周期/查询不绑定身份）。`/health` `/ready` `/metrics` `/validate` 免令牌，由部署网络边界兜底；核心不感知认证 |
| 可运维性（NFR7，D25，**子项 1+2+3 已实现**） | **子项 2（2026-10-20）**：daemon 暴露**免令牌** `/health`（存活）、`/ready`（就绪：引擎可应答）、`/metrics`（Prometheus 文本格式），可选 `pid_file`（启动写入/退出删除/陈旧检测），SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界 10s 等待在途 → `engine.shutdown()` 终态快照）。**子项 1（2026-10-20）**：事件日志前缀压缩（REST `/compact`、CLI `pact compact`、MCP `pact_compact`、可选启动自动压缩）+ schema 版本化（`WireEvent.version`，更高版本 fail-fast），详见 §2.3/§2.4。**子项 3（2026-10-20）**：域级 ACL 阻止任意客户端装卸域/触发压缩（`(domain, instance)` 授权，见 §2.5）；可选读审计流（`read_audit = true`，append+fsync，fail-closed）与只读视图 `GET .../read-audit` |
| 多实例承载（FR19，D27，**已实现**） | 同一域声明可并发承载多个 **Counter 实例**（实例键 `instance`）；实例为接口层一等键，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / 幂等表。**存储布局（已定案 + 已实现）**：域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、审计流 `‹instance›.read-audit.jsonl`、声明原文 `declaration.pact` 同目录）；索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键）。顶层遗留单层文件（`‹domain›.events.jsonl`）**不识别、fail-fast**（`LegacyLayoutDetected`）。同一 `data_dir` 上多实例以 flock（`‹data_dir›/pactd.lock`）保证单写者，禁止交错写同一日志；非空实例再加载默认拒绝（`DomainBusy`，须 `force`）。实例键以路径段承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` ≡ `default`），键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越 |
| 层边界（D24–D26） | 范式机制限于 `crates/`；领域内容在独立仓库；生产驱动逻辑收于 `pact-runtime` crate（已实现，经 REST 接入 daemon，不内嵌引擎）；分发产物在同仓库 `dist/`（**阶段 9 已交付**：容器镜像 / systemd unit / musl 静态打包，详见 §6） |

## 6. 生产运行层与分发（D24–D26，2026-10-19 裁决；L3 运行层已实现）

在既有单机架构之上补一层 **L3 生产运行层**，与 L1 引擎、L2 领域层并列；引擎的被动性（D5）不变。

```
单机生产形态
┌──────────────────────── 主机 ────────────────────────┐
│  pact-runtime（L3，已交付 lib + bin）                  │
│    ├─ 参与者驱动循环（订阅/轮询 → my_gates/enabled）    │
│    ├─ 多 agent 编排（N 个 MCP/HTTP 客户端进程）         │
│    └─ 外部副作用执行器（消费 Consumed，saga 补偿）      │
│  pact-server (daemon, L1)  ← MCP stdio 或 REST+SSE      │
│    └─ data_dir（JSONL + snapshot + index.tursodb）       │
│  dist/（阶段 9 已交付）: Dockerfile · pactd.service · musl 静态打包  │
└──────────────────────────────────────────────────────┘
```

- **依赖方向**：`pact-runtime → pact-http / pact-engine / pact-core`（lib 层；REST 经 `pact_http::client`，线上视图取自 `pact-engine`，标识符·token 类型取自 `pact-core`）；`pact-mcp` 仅作可执行文件由编排 spawn、**不**作 lib 依赖；不反向；引擎、存储、核心语义零改动。
- **落地形态（阶段 7 已完成，2026-10-19）**：`crates/pact-runtime/` 交付 `ParticipantClient` 传输抽象（`RestClient` / `McpClient`）、驱动循环 `drive()`（P1/P2/P3 + 订阅推送/轮询兜底 + `pristine` 守卫与终止宽限）、编排 `Orchestrator`（每 agent `tokio::spawn`）、副作用执行器 `SideEffectExecutor`（archive 游标增量 + Rule-S 补偿事实）、TOML 配置驱动的 bin `pact-runtime`。
- **进程拓扑**：`pact-runtime` 作为 systemd 托管的常驻编排进程（或独立 bin），拉起 daemon 与各 agent 子进程；MCP server 仍由 agent 宿主按配置以 stdio 子进程拉起（v1，D20）。
- **归属边界**：L1 生产化增强（日志压缩/schema 版本、health/ready/metrics/PID/优雅停机、域级 ACL 与可选审计读视图）仍在 `crates/`（阶段 8，已交付）；L2 领域内容仍在独立仓库；L3 收于 `pact-runtime`；分发产物入本仓库 `dist/`（阶段 9，已交付；不接入 CI 语义测试，D26）。
- **分发形态（阶段 9 已完成，2026-10-20）**：`dist/` 提供三条部署路径，均以**同一批静态二进制**为载荷（`pact` / `pactd` / `pact-mcp` / `pact-runtime`）——① 容器：`dist/Dockerfile` 多阶段构建（`rust:1.98-alpine` → `alpine:3.22`），非 root uid 10001、数据卷 `/var/lib/pact`、`EXPOSE 8787`、`HEALTHCHECK` 探 `/health`，默认配置 `dist/pact.container.toml`；② systemd：`dist/pactd.service` 以 `pact` 用户运行 `pactd --config /etc/pact/pact.toml`，`StateDirectory`/`RuntimeDirectory` 提供 `/var/lib/pact` 与 `/run/pact`，`TimeoutStopSec=20s` 覆盖 10s 优雅停机；③ 静态包：`dist/build-static.sh` 交叉构建 `x86_64-unknown-linux-musl` 并产出含 `SHA256SUMS` 的 tar.gz。三条路径均不改变进程拓扑、依赖方向与数据流；容器与 systemd 都要求仅绑回环/受信网络（v1 无 TLS）。
- **MCP 提供形态**：权威恒为 daemon；`pact-mcp` 是传输适配器，随本仓库交付、由 agent 宿主拉起；服务端形态（daemon 内建 Streamable HTTP）随 v2 落地，不另立项目。
