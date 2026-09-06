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
                │  │ 域会话表     │  │  - 检查+应用+持久化原子完成
                │  │  domain → {  │  │  - broadcast 通知总线（Rule-W 唤醒）
                │  │   Domain,    │  │  - Rule-T 终止检测
                │  │   CounterState,│ │  - Clock / IdGen 注入
                │  │   订阅者 }   │  │
                │  └─────────────┘  │
                └───┬───────────┬───┘
                    │           │
        纯函数调用   │           │  持久化
                    ▼           ▼
        ┌────────────────┐  ┌─────────────────────────────┐
        │ pact-core      │  │ pact-store                  │
        │ pact-parser    │  │  <domain>.events.jsonl 真相源│
        │ pact-analysis  │  │  <domain>.snapshot.json     │
        │ pact-compose   │  │  index.tursodb 派生索引     │
        └────────────────┘  │  (turso, IndexStore 隔离)    │
                            └─────────────────────────────┘
```

图例：`──►` 为调用/请求方向（同步请求）；`──MCP: stdio──►` 为子进程 stdin/stdout 上的双向 JSON-RPC 流，箭头仅示发起方；三条接入路径汇聚到同一 REST 通道（unix socket 默认 / 127.0.0.1 可选），该通道是通往引擎的唯一路径。分层自上而下：接口层 → 引擎层 → （纯函数语义层 / 存储层）。

核心约束：

- **依赖单向**：接口层 → 引擎 → （core/store）；core 不依赖 IO 与 async。
- **单一状态变更通道**：pact-http daemon 的 REST API 是通往引擎的唯一路径——pact-mcp 与 pact-cli（在线命令）都是其客户端（不内嵌引擎），三接口能力等价由构造保证；pact-cli 的离线命令（analyze/compose/replay）只依赖纯函数层，不触状态。
- **语义集中在纯函数层**：pact-core 的规则函数是范式的唯一权威实现；引擎只做编排（查域、调用、应用效果、持久化、通知），不含任何语义判断。
- **零隐式动作的编译期保障**：规则函数签名以 `&CounterState` 为入参、效果为返回值——接口层无法绕过规则直接改状态（Rust 所有权强制，见 design.md D1）。

## 2. 关键数据流

### 2.1 一次 Rule-P（produce）请求

```
agent ──MCP pact_produce──► pact-mcp ──REST POST /tokens──► pact-http ──Command::Produce──► engine actor
  engine: 查域会话 → rule_p(&domain, &state, ...)     [纯函数检查]
        → Ok(token) → 追加 JSONL(fsync) → 更新派生索引(turso) → state.counter.insert
        → broadcast Produced 通知（唤醒等待者）
        → 每次变更后惰性求值 Rule-T
  ←── Ok(Token) / Err(RuleError 结构化) ──
```

检查与应用在 actor 临界区内原子完成；任何一步失败不产生部分效果。

### 2.2 Rule-W（等待与唤醒）

- 主动：客户端订阅（MCP `pact_subscribe` / REST SSE），状态变更事件即时推送；
- 兜底：轮询 `pact_enabled` / `GET .../enabled`，获得当前可满足的 Rule-P 组合与见证前驱集合；
- 引擎自身不等待、不重试、不驱动参与者（被动引擎，design.md D5）。

### 2.3 启动恢复

```
daemon 启动（pact daemon start / pactd）→ 读 data_dir：
  逐域：截断尾部半行（崩溃残留）→ 载入最近快照（seq ≥ 最后一次 DomainLoaded 才有效）
       → 重放 events.jsonl 尾部（seq > snapshot.seq）
       → 校验/重建 index.tursodb → 重建内存态 → 恢复订阅总线
中部损坏行（完整行非法 JSON）→ fail-fast，命令以恢复失败统一应答（需人工介入）。
```

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
| 审计完整性 | JSONL 真相源即完整审计记录；DomainLoaded 事件存档声明原文 |
| 可恢复性 | 快照 + 重放；索引（turso，经 IndexStore trait 隔离，D13）永远可重建 |
| 可测试性 | Clock/IdGen 注入；core 纯环境属性测试；四层测试组织（单元/属性/集成/端到端+回归） |
| 安全边界 | 单机默认 unix socket；参与者令牌可选；核心不感知认证 |
