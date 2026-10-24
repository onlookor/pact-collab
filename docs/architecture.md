# PACT 实现架构

对应需求：`docs/requirements.md`；详细设计：`docs/design.md`。本文件只保留当前架构，变更历史记入 `docs/architecture-change.md`。

## 1. 架构总览

单一权威进程 daemon（`pact daemon start` 或 `pactd`，同一实现）持有 Counter；AI agent、应用程序、人类从四种接入形态（MCP stdio / MCP Streamable HTTP / REST / CLI）接入同一 daemon。daemon 默认监听 unix socket，可选 TCP + TLS/mTLS（NFR6 v2，D34–D36）。

```
 ┌─────────────────────────────── 接口层 ────────────────────────────────┐
 │                                                                       │
 │  AI agent 进程 ──MCP: stdio──► pact-mcp ───────┐                      │
 │  AI agent 进程 ──MCP: HTTP───► /mcp ──────────┤                      │
 │                                                │                      │
 │  普通应用程序 ─────────────────────────────────► ├── REST ──► pact-http │
 │                                                │  (unix / TCP+TLS;    │  daemon
 │  人类 (终端) ──► pact-cli（在线命令）───────────┘   Bearer/签名/OIDC)  │ (REST/JSON
 │     （离线命令 analyze/compose/replay：本地执行，不经 daemon）           │  + SSE)
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

图例：`──►` 为调用/请求方向（同步请求）；`──MCP: stdio──►` 为子进程 stdin/stdout 上的双向 JSON-RPC 流，箭头仅示发起方；`──MCP: HTTP──►` 为 Streamable HTTP（`POST/GET/DELETE /mcp`，由 daemon 进程内处理）。REST 通道是通往引擎的唯一路径（unix socket 默认 / TCP+TLS 可选）；`/mcp` 工具层与 stdio 共用实现，最终仍经进程内 REST 路由进入引擎。分层自上而下：接口层 → 引擎层 → （纯函数语义层 / 存储层）。

核心约束：

- **依赖单向**：接口层 → 引擎 → （core/store）；core 不依赖 IO 与 async。
- **单一状态变更通道**：pact-http daemon 的 REST API 是通往引擎的唯一路径——pact-mcp（stdio）、内建 `/mcp` 与 pact-cli（在线命令）都是其客户端（不内嵌引擎），接口能力等价由构造保证；pact-cli 的离线命令（analyze/compose/replay/cpn）只依赖纯函数层，不触状态。
- **身份止于接口层**：Bearer、Ed25519 签名与 MCP OIDC 均在 pact-http 中间件解析为参与者名，之后与信任模式共用同一 ACL/参与者字段；核心、引擎与存储不感知认证方式。OIDC 仅作用于 `/mcp`，REST 不接受 OIDC 令牌。
- **语义集中在纯函数层**：pact-core 的规则函数是范式的唯一权威实现；引擎只做编排（查域、调用、应用效果、持久化、通知），不含任何语义判断。
- **零隐式动作的编译期保障**：规则函数签名以 `&CounterState` 为入参、效果为返回值——接口层无法绕过规则直接改状态（Rust 所有权强制，见 design.md D1）。
- **宿主集成层（非 Cargo，阶段 11）**：独立仓库 `pact-dsh`（包名 `dsh-plugin-pact`）是 DSH 侧的控制流适配器，只经 `PactBackend` 契约访问 PACT（WASM 或假实现），不嵌入引擎、不写事件日志、不持有语义状态；不反向进入任何 Rust crate。
- **纯模型可移植**：`pact-store-model`（事件/快照/实例键）、`pact-views`（查询/分析视图）与 `pact-engine-kernel`（实例会话语义）只依赖纯 crate（`pact-core`/`pact-parser`/`pact-analysis`/`serde`），可编译到 `wasm32-unknown-unknown`；`pact-store`/`pact-engine` 通过 re-export 保持既有 API（D29）。原生 `pact-engine` 与 `pact-wasm` 的实例会话均通过 `Deref` 嵌入同一个 `pact_engine_kernel::Session`，仅在持久化、通知、审计与错误映射等外围不同（D32）。`pact-wasm` 在其上提供 **WASM 本地分发形态（受支持，D31/D33）**：内存实例会话 + 两阶段提交 + 日志/快照导出恢复；Node 宿主适配层（`crates/pact-wasm/js/`）负责 JSONL+fsync、快照 tmp/rename 与 lockfile 单写者，磁盘布局与原生共用。**权威仍是原生 daemon**——WASM 为进程内单用户嵌入形态，不提供 MCP/HTTP 服务、不承担 ACL/审计与多用户边界，不进入 `dist/` 产物集。稳定化验收（Q7 CI、金标扩面、kernel 抽取、宿主边界安全审视）已全部完成（2026-10-21），故按 D33 去除“实验性”标记；浏览器持久化（OPFS/SharedWorker）与 DSH/v2 集成仍未接入（FR21）。

## 2. 关键数据流

### 2.1 一次 Rule-P（produce）请求

```
agent ──MCP pact_produce──► pact-mcp / 内建 /mcp ──REST POST /tokens──► pact-http ──Command::Produce──► engine actor
  pact-http 中间件：Bearer / Ed25519 签名（`/mcp` 另可 OIDC）→ 参与者名
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

REST 入口（除 /health|/ready|/metrics|/mcp|RFC 9728 元数据外）
   → 验签（若启用）→ Bearer / 签名身份 → principal
   → ACL 空表：短路（既有信任声明行为）
   → ACL 非空：匹配 domain/instance 或 domain/*
               无身份 401；越权 403；GET /domains 按授权过滤
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

### 2.7 MCP Streamable HTTP 与 OIDC（D35/D36）

```
外部 MCP 客户端 ──POST /mcp（Authorization: Bearer <JWT>）──► pact-http
   require_mcp_auth：OIDC 校验（iss/aud/exp/nbf/alg/JWKS，仅 /mcp）
                    └─ 失败：401（WWW-Authenticate + resource_metadata）
                    └─ IdP/JWKS 不可达：503（不影响 REST）
   成功 → 参与者名（claim 经 principal_map）
        → StreamableHttpService<PactMcp, LocalSessionManager>
        → PactMcp 工具处理（每次调用注入 x-pact-verified-principal）
        → 进程内 tower oneshot 调自身 REST Router（带 x-pact-internal-auth）
        → engine actor（与普通 REST 请求同一命令通道）
```

- 工具实现只有一份（`pact-http::mcp`）；stdio `pact-mcp` 直接复用同一 `PactMcp`，差别仅在传输与连接参数。
- `/mcp` 鉴权与 REST 验签分离：OIDC 令牌不进入 REST 中间件，也不被缓存/转发；REST 仍只接受静态 Bearer 或 Ed25519 签名。
- `GET /.well-known/oauth-protected-resource` 只读取静态配置（`public_url`/`issuer`），不访问 IdP。
- 信任模式（`[mcp].enabled`、无 OIDC、无令牌）仅用于单机/回环，启动告警；`/mcp` 有 OIDC 或令牌时必须认证。

## 3. 进程与部署视图

```
单机 / 受控联网（v2）
┌──────────────────────────────── 主机 ────────────────────────────────┐
│  pactd (daemon)                                                      │
│    ├─ REST + SSE        @ unix socket（默认）/ TCP + 可选 TLS/mTLS     │
│    ├─ 内建 /mcp         @ 同一监听端口（Streamable HTTP，可选）        │
│    ├─ 身份中间件          Bearer / Ed25519 签名（REST）                │
│    │                     OIDC（仅 /mcp；外部 IdP + JWKS）              │
│  └─ data_dir（JSONL + snapshot + index.tursodb）                     │
│  pact-mcp（stdio 子进程，由 agent 宿主拉起）──REST/TLS──► daemon       │
│  外部 AI agent 进程（自驱，经 stdio 或 /mcp 接入）                      │
│  pact-cli（人类）──REST/TLS──► daemon                                 │
└──────────────────────────────────────────────────────────────────────┘
        │ 仅在启用 OIDC 时
        ▼
  外部 IdP（发现文档 + JWKS；daemon 只验证，不签发）
```

- 单一权威进程；`pact`（daemon + CLI）与 `pactd`（裸 daemon）为同一实现，`pact-mcp`/`pact-runtime` 是客户端/编排进程。
- TLS 终止在 pactd（可选）；若由反向代理终结 TLS，则 pactd 只监听回环并依赖静态令牌/签名 + ACL。
- OIDC 是可选外部依赖：只有 `/mcp` 触发发现/JWKS 请求；IdP 故障不影响 REST、CLI 与 stdio MCP。
- 无人值守闭环：外部脚本/systemd 启动 daemon 与各 agent 进程 → agent 加载域、注入种子（或种子由脚本注入）→ 参与者经 Counter 自组织推进 → Terminated 事件触发各 agent 退出（订阅可得）。

## 4. 联网拓扑（v2，已实现）

v2 不在本仓库引入新进程或新内部组件，只把接口层的传输与身份补全；唯一新增的外部依赖是可选 IdP（仅 OIDC）。Counter 单权威仍是 client/server，不存在多副本一致性问题。

### 4.1 部署拓扑

```
                    ┌──────────────────────────────────────────────┐
                    │                 pactd（单权威）                │
  MCP 客户端 ──TLS──►│  /mcp ──► require_mcp_auth ──┐               │
  (OIDC JWT)        │            │ OIDC 验证        │               │
                    │            ▼                  │               │
                    │        PactMcp ──进程内 REST──►│               │
                    │                               ├─► 中间件       │
  REST 客户端 ──TLS─►│  /domains... ─────────────────┤   验签/令牌    │
  (Bearer / 签名)    │                               │   + ACL       │
                    │                               ▼               │
                    │                          engine actor         │
                    │                               │               │
                    │                               ▼               │
                    │                     data_dir（JSONL/快照/索引）│
                    └───────────────┬──────────────────────────────┘
                                    │ 仅 OIDC 启用时
                                    ▼
                    ┌──────────────────────────────┐
                    │ IdP：discovery + JWKS（外部） │
                    └──────────────────────────────┘

  pact-mcp（stdio）──REST/TLS──► pactd（保留；供本地宿主/运行时）
  pact-runtime ──REST/TLS──► pactd；MCP 模式 spawn pact-mcp
```

拓扑要点：

- **TLS 终止位置**：pactd 进程内（rustls，TLS 1.2/1.3）；`client_ca` 存在时要求客户端证书（mTLS）。unix socket 明文且仅本机。若采用外部反向代理终止 TLS，pactd 只监听回环，身份由静态令牌/签名 + ACL 承担。
- **两条 MCP 路径**：外部远程客户端走 `/mcp`（Streamable HTTP，OIDC 或静态 Bearer）；本地宿主走 `pact-mcp` stdio（经 REST）。两者共用 `pact-http::mcp` 工具实现，行为一致。
- **签名身份**：REST 客户端可持 Ed25519 种子对请求签名；启用签名后非豁免 REST 请求必须签名（Bearer 不能替代），服务端验签后注入内部可信参与者，再走 ACL。
- **OIDC 边界**：仅 `/mcp` 使用；daemon 是资源服务器，不签发令牌、不做授权码流程。令牌不缓存、不写入日志、不转发给 REST。
- **内部调用**：`/mcp` 工具经进程内 tower Router 调自身 REST，使用随机能力令牌 `x-pact-internal-auth`，不发起 TCP 自连接，也不复用外部 JWT。

### 4.2 请求身份判定顺序

| 入口 | 判定顺序 | 失败语义 |
|------|----------|----------|
| REST（除豁免路径） | 内部能力令牌 → 启用签名时必须验签 → 否则静态 Bearer；均无则信任模式 | 缺/错身份 401、时间戳越界 401、nonce 重放 429、body > 8 MiB 413、participant 不匹配 401 |
| REST 域端点（ACL 非空） | 上述身份 → `(domain, instance)` / `domain/*` 授权 | 无身份 401、越权 403 |
| `/mcp` | OIDC JWT（配置时优先）→ `allow_static_tokens` 时静态 Bearer；均无配置则信任模式 | 令牌非法/claim 不符 401、IdP/JWKS 不可达 503、无 Authorization 的信任模式放行 |
| `/health` `/ready` `/metrics` / 元数据 | 无身份校验 | — |

### 4.3 与 v1 的差异

| 维度 | v1 | v2（已实现） |
|------|----|--------------|
| 传输 | unix socket / localhost / stdio | + TLS/mTLS、远程 MCP Streamable HTTP |
| 身份 | 信任声明 + 可选静态 Bearer | + Ed25519 签名 + MCP OIDC 资源服务器 |
| MCP | 仅 stdio | stdio（薄封装）+ daemon 内建 `/mcp` |
| 外部依赖 | 无 | 仅 OIDC 启用时依赖 IdP/JWKS |
| 权威 | 单进程引擎 | 同一引擎（不变） |
| 语义/存储 | pact-core / pact-store | **零改动** |

架构上联网仍只有接口层变化：新增 `pact-http::tls`、`sign`、`oidc`、`mcp` 模块与对应配置；`pact-engine`、`pact-store`、`pact-core` 未改。

## 5. 质量属性落点

| 属性 | 架构手段 |
|------|----------|
| 语义保真 | 语义只存在于纯函数 core（含良构校验、Rule-W/Rule-T 判定，D14）；黄金轨迹测试锚定论文全部示例 |
| 审计完整性 | JSONL 真相源即完整审计记录；DomainLoaded 事件存档声明原文；每条事件自足（Consumed 携 token 全量快照），单行即可审计与重放（重放时目标须仍在 Counter，缺失即 `ConsumeTargetNotOnCounter` fail-closed）。**日志压缩后**（D25 子项 1）审计视图 = 自足快照（含声明/幂等表）+ 保留的尾部事件，被丢弃前缀不再可查 |
| 可恢复性 | 快照 + 重放；索引（turso，经 IndexStore trait 隔离，D13）永远可重建、滞后由启动一致性校验触发重建自愈。压缩后日志无 `DomainLoaded` 时以**快照为锚**恢复（`snapshot_is_anchor`），快照缺失/不覆盖水位则 fail-fast；事件 schema 版本缺省按 v1、更高版本 fail-fast |
| 可测试性 | Clock/IdGen 注入；core 纯环境属性测试；四层测试组织（单元/属性/集成/端到端+回归） |
| 安全边界 | 默认 unix socket 本地单用户；可选 daemon 内 TLS/mTLS（`[tls]`，D34）与静态 Bearer / Ed25519 签名（`[auth.keys]`，D34）。**启用 `[auth.acl]` 后**（D25 子项 3）全部域作用域端点按 `(domain, instance)` 授权（支持 `domain/*` 通配），无身份 `401`、越权 `403`，`GET /domains` 按授权过滤；ACL 非空而令牌表与密钥表均为空 ⇒ 拒绝启动。未配置 ACL 时保持既有信任声明行为。`/health` `/ready` `/metrics` 免身份，由部署网络边界兜底；`/validate` 在签名启用时仍需签名。核心不感知认证 |
| 联网身份与传输（NFR6 v2，D34–D36） | TLS 终止在 pactd（可选 mTLS；内置客户端只验证服务端，不出示客户端证书）；REST 支持 Ed25519 签名（±300s、nonce 防重放、8 MiB；启用后非豁免请求必须签名，Bearer 不能替代）+ 静态 Bearer；`/mcp` 支持 OIDC 资源服务器（alg 白名单、iss/aud/exp/nbf、JWKS 有界缓存、claim 映射）或静态 Bearer，信任模式仅限无身份配置的单机/回环；OIDC 令牌不缓存、不转发 REST，IdP 故障只影响 `/mcp`（503） |
| 可运维性（NFR7，D25，**子项 1+2+3 已实现**） | **子项 2（2026-10-20）**：daemon 暴露**免身份** `/health`（存活）、`/ready`（就绪：引擎可应答）、`/metrics`（Prometheus 文本格式），可选 `pid_file`（启动写入/退出删除/陈旧检测），SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界 10s 等待在途 → `engine.shutdown()` 终态快照）。**子项 1（2026-10-20）**：事件日志前缀压缩（REST `/compact`、CLI `pact compact`、MCP `pact_compact`、可选启动自动压缩）+ schema 版本化（`WireEvent.version`，更高版本 fail-fast），详见 §2.3/§2.4。**子项 3（2026-10-20）**：域级 ACL 阻止任意客户端装卸域/触发压缩（`(domain, instance)` 授权，见 §2.5）；可选读审计流（`read_audit = true`，append+fsync，fail-closed）与只读视图 `GET .../read-audit` |
| 多实例承载（FR19，D27，**已实现**） | 同一域声明可并发承载多个 **Counter 实例**（实例键 `instance`）；实例为接口层一等键，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / 幂等表。**存储布局（已定案 + 已实现）**：域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、审计流 `‹instance›.read-audit.jsonl`、声明原文 `declaration.pact` 同目录）；索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键）。顶层遗留单层文件（`‹domain›.events.jsonl`）**不识别、fail-fast**（`LegacyLayoutDetected`）。同一 `data_dir` 上多实例以 flock（`‹data_dir›/pactd.lock`）保证单写者，禁止交错写同一日志；非空实例再加载默认拒绝（`DomainBusy`，须 `force`）。实例键以路径段承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` ≡ `default`），键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越 |
| 表达力边界（B1–B25） | 母本 §1.3/§1.4/§1.4.4/§2.3 声明范式不能直接表达什么；实现侧在 `docs/expressiveness-boundaries.md` 集中登记可编码模式、仅可近似项、范式外项与工具层边界，并以 `B<编号>` 在文档与代码注释中引用。摘要落点见 `docs/design.md` §18 |
| Token 消费时机（TC1–TC13） | 母本 §1.6/§1.7.3 提供规范依据（消费时机是域策略、Read 非 reservation、ack 屏障）；多下游共享 token 的消费所有者、顺序与保留策略集中登记于 `docs/token-consumption-timing.md`：判定准则 D1–D6、模式 P1–P9、反模式 A1–A9，并以 `TC<编号>` 引用。静态预警由 A.3（无 consumer）与 A.6（搁浅风险）承接；与表达力边界 B8/B17/B21/B22 交叉引用，不新增范式原语 |
| 层边界（D24–D26） | 范式机制限于 `crates/`；领域内容在独立仓库；生产驱动逻辑收于 `pact-runtime` crate（已实现，经 REST 接入 daemon，不内嵌引擎）；分发产物在同仓库 `dist/`（**阶段 9 已交付**：容器镜像 / systemd unit / musl 静态打包，详见 §6） |

## 6. 生产运行层与分发（D24–D26，2026-10-19 裁决；L3 运行层已实现）

在既有全量架构之上补一层 **L3 生产运行层**，与 L1 引擎、L2 领域层并列；引擎的被动性（D5）不变。

```
受控联网生产形态（v2）
┌──────────────────────── 主机 ────────────────────────┐
│  pact-runtime（L3，已交付 lib + bin）                  │
│    ├─ 参与者驱动循环（订阅/轮询 → my_gates/enabled）    │
│    ├─ 多 agent 编排（N 个 MCP/HTTP 客户端进程）         │
│    └─ 外部副作用执行器（消费 Consumed，saga 补偿）      │
│  pact-server (daemon, L1)  ← MCP stdio / /mcp / REST+SSE │
│    └─ data_dir（JSONL + snapshot + index.tursodb）       │
│  dist/（阶段 9 已交付）: Dockerfile · pactd.service · musl 静态打包  │
└──────────────────────────────────────────────────────┘
```

- **依赖方向**：`pact-runtime → pact-http / pact-engine / pact-core`（lib 层；REST 经 `pact_http::client`，线上视图取自 `pact-engine`，标识符·token 类型取自 `pact-core`）；`pact-mcp` 仅作可执行文件由编排 spawn、**不**作 lib 依赖；不反向；引擎、存储、核心语义零改动。
- **落地形态（阶段 7 已完成，2026-10-19）**：`crates/pact-runtime/` 交付 `ParticipantClient` 传输抽象（`RestClient` / `McpClient`）、驱动循环 `drive()`（P1/P2/P3 + 订阅推送/轮询兜底 + `pristine` 守卫与终止宽限）、编排 `Orchestrator`（每 agent `tokio::spawn`）、副作用执行器 `SideEffectExecutor`（archive 游标增量 + Rule-S 补偿事实）、TOML 配置驱动的 bin `pact-runtime`。
- **进程拓扑**：`pact-runtime` 作为 systemd 托管的常驻编排进程（或独立 bin），拉起 daemon 与各 agent 子进程；MCP server 由 agent 宿主按配置以 stdio 子进程拉起（保留），或由远程客户端直连 daemon `/mcp`（v2，D35）。runtime 的 REST 客户端支持 `--https/--tls-ca` 与 `--sign-key/--sign-as`；其 stdio MCP 子进程当前只透传 TLS 与 Bearer。
- **归属边界**：L1 生产化增强（日志压缩/schema 版本、health/ready/metrics/PID/优雅停机、域级 ACL 与可选审计读视图）仍在 `crates/`（阶段 8，已交付）；L2 领域内容仍在独立仓库；L3 收于 `pact-runtime`；分发产物入本仓库 `dist/`（阶段 9，已交付；不接入 CI 语义测试，D26）。
- **分发形态（阶段 9 已完成，2026-10-20）**：`dist/` 提供三条部署路径，均以**同一批静态二进制**为载荷（`pact` / `pactd` / `pact-mcp` / `pact-runtime`）——① 容器：`dist/Dockerfile` 多阶段构建（`rust:1.98-alpine` → `alpine:3.22`），非 root uid 10001、数据卷 `/var/lib/pact`、`EXPOSE 8787`、`HEALTHCHECK` 探 `/health`，默认配置 `dist/pact.container.toml`；② systemd：`dist/pactd.service` 以 `pact` 用户运行 `pactd --config /etc/pact/pact.toml`，`StateDirectory`/`RuntimeDirectory` 提供 `/var/lib/pact` 与 `/run/pact`，`TimeoutStopSec=20s` 覆盖 10s 优雅停机；③ 静态包：`dist/build-static.sh` 交叉构建 `x86_64-unknown-linux-musl` 并产出含 `SHA256SUMS` 的 tar.gz。三条路径均不改变进程拓扑、依赖方向与数据流；容器与 systemd 默认仅绑回环/受信网络，需要跨主机时启用 v2 TLS/mTLS 或由外部 TLS 终结并配合 `[auth.tokens]`/`[auth.keys]` + `[auth.acl]`。
- **MCP 提供形态**：权威恒为 daemon；`pact-mcp` 是传输适配器，随本仓库交付、由 agent 宿主拉起；daemon 内建 Streamable HTTP `/mcp` 已落地（v2，D35），与 stdio 共用工具实现，不另立项目。

## 7. DSH 宿主集成（D37，阶段 11）

在既有 L1/L2/L3 之外增加一个**宿主侧控制层**，只存在于 DSH 进程内，不改变 daemon、引擎与存储拓扑。

```
DSH 宿主进程
┌──────────────────────────────────────────────────────────┐
│  agent-loop（原行 disabled，pact-agent-loop 接管）          │
│    └─ PactAgentLoop extends AgentLoop                    │
│         ├─ agentPreset 以 pact- 开头 → 安装 agent 级钩子   │
│         │     ├─ agent/pre-step        → 门控（reject）    │
│         │     ├─ tools/pre-execute     → 门控（deny）      │
│         │     ├─ tools/post-execute    → 审计（不改结果）   │
│         │     └─ agent/turn-stopping   → 可选 steer 收尾    │
│         └─ 其余 preset → super（官方 ReactLoopAgent，不变） │
│                                                          │
│  观察行 dsh-pact-observe                                  │
│    └─ session/event → 有界缓冲 + 可选 JSONL（0700/0600）    │
│                                                          │
│  PactBackend（注入）                                      │
│    ├─ FakePactBackend（单测/演示）                         │
│    └─ WasmPactBackend（惰性 openPactHost；本地单用户）      │
└──────────────────────────────────────────────────────────┘
```

- **依赖方向**：`pact-dsh → DSH SDK（agent-loop/agent/tools/session-projection，DSH 安装内解析）+ PactBackend`；不依赖任何 Rust crate，不被 `crates/` 依赖，不进入 `Cargo.toml`。
- **权威边界**：插件不复制 Counter 状态机；门判定委托后端（WASM 内核或原生 daemon）。插件只决定“是否允许开始/执行”，不执行语义规则。
- **失败方向**：`govern` 后端不可用 → 拒绝（fail-closed）；`observe` → 放行并留痕；`turn-stopping` 读失败 → 不阻断收尾。
- **进程与端口**：不新增监听端口、不拉起子进程；审计文件仅由操作者配置的 `mirrorDir` 写入。
- **归属**：`pact-dsh` 已迁出为独立仓库（2026-10-23），与 `crates/` 解耦，不进入 `dist/`；阶段 12 的 token 级工具映射与真实 DSH 冒烟另行评估。
- **装配与门控验证（2026-10-22/23）**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 上以 web profile 完成安装与启动；`--dump-config` 显示原 `agent-loop` 禁用、`pact-agent-loop` 插入；运行时 `ctx.agentLoop` 为 `PactAgentLoop`，DSH 按 peer 解析 `@deepseek-ai/dsh-agent-loop`。另在 headless profile 中以 `pact-demo` preset 和 `deepseek-official/deepseek-flash` 驱动真实 agent turn，验证允许、工具拒绝与预检拒绝三条路径；拓扑与依赖方向不变。
- **WASM 后端接缝（2026-10-23）**：组装入口 `crates/pact-wasm/js/open-host.js`（CommonJS）把 wasm-bindgen 产物、`FileStore` 与声明组装为 `PactHost`；插件通过配置里的文件路径动态 import，**不形成 ESM 包依赖**，`crates/` 仍不被 `plugins/` 依赖。已在同一 headless profile 中以 `backend=wasm` 复跑三条路径：允许（bash 实际执行）、`denyTools` 拒绝、空 Counter 实例的 `pre-step` 拒绝；实例日志与锁由 WASM 宿主在 DSH 进程内创建，拓扑仍为“宿主内控制层 + 本地文件存储”。

