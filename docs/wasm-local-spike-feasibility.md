# PACT WASM 本地 Spike 可行性报告与实施计划

- 日期：2026-10-21
- 状态：**已转正式、受支持的本地分发形态（2026-10-21，D33：去除“实验性”标记）**；W-Q1/W-Q2/W-Q3/W-Q5/W-Q6 已于 2026-10-21 冻结；W-P0 编译探针、W-P1 纯类型层拆分、W-P2/W-S0 WASM 门面、W-P3/W-S1 内存语义与对账、W-P4/W-S2 Node 持久化已完成（2026-10-21）；W-P6 决策门评估已产出（§11）并裁决 A；立项已登记于 `requirements.md` FR21 / `design.md` D31+D33 / `architecture.md` §1 及其变更文档；**W-Q7 CI、金标扩面、`pact-engine-kernel` 抽取与宿主边界安全审视均已完成（2026-10-21，D32）**；**W-P5 浏览器兼容性评估已完成（2026-10-21，结论：技术可行、未实现，触发条件见 §W-P5/§11.2）**；**DSH 插件 observe/govern 与 `pact-*` preset 治理型 loop 接管已于 2026-10-22 落地（FR25/D37，阶段 11；2026-10-23 迁至独立仓库 `onlookor/pact-dsh`），并在 `@deepseek-ai/dsh@0.2.1-rc.2` 完成真实装配；WASM 后端已于 2026-10-23 live**
- 范围：PACT 语义核心的 WASM 本地嵌入，优先 Node/DSH 宿主，其次浏览器
- 结论：**核心语义层高度可行；本地引擎中等偏高；浏览器完整本地运行时中等；整个 `pactd` 编译进 WASM 不可行**

## 1. 摘要

本报告评估“把 PACT 做成 WASM 并在 harness 内嵌运行”的技术可行性，并综合此前关于 DSH agent loop 替换、WASM 分发形态、NFR6 v2 联网演进的三轮讨论，给出实施计划。

核心判断：

1. **应做“语义核心 WASM”，不应做“整个 daemon WASM”。** `pact-core`、`pact-parser`、`pact-analysis`、`pact-compose`、`pact-cpn` 均为纯计算层，没有文件、网络、线程或异步运行时依赖，适合 `wasm32-unknown-unknown`。
2. **事件模型和读视图需要先拆成纯类型层。** `pact-store` 把事件/快照与 `std::fs`、turso 绑在一起；`pact-engine` 把纯视图与 tokio actor 绑在一起。先拆 `pact-store-model` / `pact-views`，WASM 才能与原生共用同一份事件格式。
3. **持久化由 JS 宿主提供，WASM 只做 `prepare/commit`。** 不要在 WASM 内假装拥有 fsync、flock、stdio 或 socket；这些能力在浏览器不存在，在 Node 应由宿主实现。
4. **本地 WASM 与 NFR6 v2 无硬依赖。** 单用户本地模式可独立落地；只有“浏览器加入共享团队、跨设备、多用户 ACL/审计”才条件依赖 v2 的 TLS/身份/Streamable HTTP。
5. **DSH loop 替换应与 WASM 分开推进。** WASM 解决分发与本地运行；DSH 插件解决 loop 治理。二者可组合，但不应互相阻塞。

## 2. 背景与目标

### 2.1 背景

- PACT 当前以 Rust workspace 交付 `pactd`、`pact`、`pact-mcp`、`pact-runtime` 等原生二进制；跨平台分发需要为 macOS/Windows/Linux 分别构建。
- 浏览器 harness、Electron/Tauri 应用、DSH 插件等宿主更希望“一个包在所有平台运行”，且浏览器内无法启动 `pactd` 或 `pact-mcp` 的 stdio 进程。
- PACT 的语义真相源是 JSONL 事件日志，turso 只是派生索引；这为“去掉原生存储、保留语义核心”提供了基础。

### 2.2 目标

- 证明 PACT 核心语义可以在 WASM 中运行，并与原生引擎行为一致。
- 给出可复用的 WASM API、存储适配边界和单写者方案。
- 给出与 DSH 插件、NFR6 v2 的耦合关系与排期边界。
- 产出一个可在本地 Node 环境运行的 spike，再决定是否进入浏览器与 DSH 集成。

### 2.3 非目标

- 不把 `pactd` 完整编译为 WASI/浏览器 WASM。
- 不在本轮修改论文母本、需求、设计、架构或变更文档。
- 不在 WASM 内实现 turso 索引、MCP/HTTP 服务、TLS、ACL、读审计。
- 不承诺浏览器端达到原生 fsync/flock 的持久化与互斥保证。
- 不把本地 WASM 当作多用户安全边界。

## 3. 技术可行性

### 3.1 依赖审计

| crate | 运行时依赖 | 是否适合 `wasm32-unknown-unknown` | 判断 |
|---|---|---|---|
| `pact-core` | `serde` | 是 | 纯函数、无 IO、无 unsafe，Gate/Rule-P/R/S/E/T 可直接复用 |
| `pact-parser` | `logos`、`pact-core` | 是 | 词法/语法/良构性均为纯计算 |
| `pact-analysis` | `petgraph`、`serde`、`pact-core` | 是 | A.1–A.6 纯静态分析，无 IO |
| `pact-compose` | `pact-core` | 是 | 组合运算纯函数 |
| `pact-cpn` | `pact-core`、`serde` | 是 | CPN 投影纯计算 |
| `pact-store` | `std::fs`、`tokio`、`turso`、`async-trait` | 否 | 日志、快照、索引绑定原生环境 |
| `pact-engine` | `tokio` actor、`mpsc`/`broadcast`、`ulid` | 否 | 命令处理可抽纯内核，但当前驱动不可直接编译 |
| `pact-http` | `axum`/`hyper`/`tokio`、`fs4` | 否（浏览器） | 浏览器无监听 socket；数据目录 flock（`pact-http/src/daemon.rs`）也在本层 |
| `pact-mcp` | stdio、REST 客户端 | 否（浏览器） | 浏览器无 stdio；可改远端 HTTP，但不是本地 WASM 目标 |
| `pact-runtime` | `tokio`、进程/客户端 | 否（浏览器） | 可作为远端驱动，不进入本地 WASM |

关键事实：

- `pact-core` 的模块文档明确“无 IO，不依赖 async crate”，见 `../crates/pact-core/src/lib.rs`。
- `pact-core`、`pact-parser`、`pact-analysis`、`pact-compose`、`pact-cpn` 中未检索到 `std::fs`、`std::net`、`std::process`、`std::thread`、`tokio`、`SystemTime`、`Instant`、`rayon` 等调用。
- `pact-store/src/event.rs` 的 `Event`、`WireEvent`、`apply_event` 只依赖 `pact-core` + `serde`；阻塞点集中在 `log.rs`、`store.rs`、`index.rs`、`error.rs` 的 IO/索引部分。
- `pact-engine/src/views.rs` 与 `notify.rs` 的视图/通知类型是纯 serde 数据，阻塞点在 `engine.rs` 的 actor 驱动；可先把纯类型移出。
- `pact-store/src/snapshot.rs` 的 `Snapshot` 数据结构是纯 serde；`write_snapshot`/`load_snapshot` 是 IO，应留在原生层。
- turso 是派生索引，设计上可 rebuild；WASM 模式可以完全不带 turso。

### 3.2 可复用的语义面

WASM 核心可以直接复用：

- `load_domain` / `emit_domain`
- `validate_domain` 与 `SeedProduceOverlap`
- `gates_for` / `gate_allows` / `production_options` / `termination_holds`
- `CounterState::apply` 与 `Effect`（Seeded/Produced/Consumed）
- `pact-analysis` 的 A.1–A.6 报告
- `pact-compose` 的组合与冲突
- `pact-cpn` 的投影

WASM 需要自行重实现或抽取的部分：

- 命令分发与实例会话（`InstanceSession`）
- 事件序号分配、幂等表 `consumed_requests`
- Rule-T 检测后的 `Terminated` 事件生成
- 快照导出/恢复
- 存储、索引、通知、压缩、审计（均由 JS 宿主或原生层承担）

### 3.3 阻塞点与规避

| 阻塞点 | 影响 | 规避方案 |
|---|---|---|
| `pact-store` 的 `std::fs` / `tokio` / `turso` | 不能直接编译 | 拆出 `pact-store-model`，原生 store 保留 |
| `pact-engine` 的 tokio actor | 不能直接编译 | spike 先实现薄内核；长期抽 `pact-engine-kernel` |
| ULID `getrandom` | wasm 目标可能缺少随机源 | 由 JS 传入 token id / ts，核心保持纯函数 |
| `StoreError` 含 `PathBuf` / `io::Error` | 不能跨 wasm 边界 | 拆 `ModelError`（JSON/版本/序号）与 `StoreError`（IO/索引） |
| `AnalysisReport` 位于 `pact-engine` | 不能直接依赖 | 移入 `pact-views` 或由 `pact-wasm` 复用纯类型 |
| 浏览器无 fsync/flock/stdio | 持久化/互斥/进程能力缺失 | JS 存储适配 + Web Locks/SharedWorker；不承诺原生等价 |
| wasm32 target | 已安装并通过 5 个纯 crate 的 check/build（2026-10-21） | 后续按 W-P2 接入 `wasm-bindgen` 与 cdylib |

### 3.4 编译验证状态

- 工具链：`rustc 1.98.1`；`wasm32-unknown-unknown` target 已于 2026-10-21 安装。
- 已执行：
  - `cargo check --target wasm32-unknown-unknown -p pact-core -p pact-parser -p pact-analysis -p pact-compose -p pact-cpn` → 通过（3.56s）；
  - `cargo build --target wasm32-unknown-unknown`（同上五个 crate）→ 通过（1.86s），产出 wasm rlib。
- 结论：五个纯 crate 的**类型检查与代码生成**均通过，`serde`/`logos`/`petgraph` 依赖在 wasm 目标可用。
- 已由 W-P2 验证：`wasm-bindgen 0.2.127` 门面导出 6 个业务方法 + `abi_version`；Node 加载 `.wasm` 跑通 `analyze`/`load_domain`/`compose`；bindgen 处理后的 release 产物 **273,394 B（267 KiB）**，gzip 后 **89,054 B（87 KiB）**；模块加载 1.71 ms、构造 0.35 ms、首次 `analyze` 3.24 ms（Node 22，单次冷启动）。
- 已由 W-P3 验证：`prepare/commit/abort`、事件写入（内存）、快照导出/恢复、JSONL 双向互通、错误码与原生逐项对账（见 §W-P3 结果）。
- 已由 W-P4 验证：Node 文件存储（JSONL + fsync、快照 tmp/rename）、崩溃后「日志优先」恢复、末尾半行截断、单写者 lockfile、快照自足恢复、原生/WASM 交替打开同一日志。
- 尚未验证：浏览器 OPFS/SharedWorker（W-S3，W-Q6 暂不做）。WASM CI job 已落地（W-Q7，2026-10-21，见 §9.2）。

## 4. 建议架构

### 4.1 crate 布局

建议目标布局（名称可在立项时定）：

```text
crates/
  pact-core                纯语义，不变
  pact-parser              纯解析，不变
  pact-analysis            纯静态分析，不变
  pact-compose             纯组合，不变
  pact-cpn                 纯投影，不变
  pact-store-model         新：Event/WireEvent/Snapshot/InstanceKey/apply_event/ModelError
  pact-views               新：CounterView/ArchiveView/StatusView/GateClause/DomainSummary/AnalysisReport
  pact-store               原生：JSONL + 快照 + turso + flock
  pact-engine              原生：tokio actor + 存储 + 通知
  pact-wasm                新：wasm-bindgen 门面 + 内存实例会话 + JS 存储适配
```

`pact-store-model` 和 `pact-views` 只允许依赖 `pact-core`/`pact-analysis`/`serde`。原生 crate 通过 re-export 保持现有 API，避免一次性大改调用方。

### 4.2 WASM API（v0）

v0 建议全部使用 JSON 字符串作为 ABI，降低 `wasm-bindgen` 类型绑定复杂度；后续可升级为 `serde-wasm-bindgen` 对象。

```rust
#[wasm_bindgen]
pub struct PactWasm { /* domains + instances + committed state */ }

#[wasm_bindgen]
impl PactWasm {
    pub fn new() -> PactWasm;

    // 域与实例
    pub fn load_domain(&mut self, name: &str, declaration: &str, instance: &str, force: bool) -> Result<String, JsValue>;
    pub fn instantiate(&mut self, domain: &str, instance: &str) -> Result<String, JsValue>;
    pub fn validate(&mut self, declaration: &str) -> Result<String, JsValue>;

    // 两阶段提交：prepare 不改变已提交状态，commit 才应用
    pub fn prepare_seed(&mut self, key: &str, participant: &str, ty: &str, thing_ref: &str, token_id: &str, ts: &str) -> Result<String, JsValue>;
    pub fn prepare_produce(&mut self, key: &str, participant: &str, ty: &str, predecessors: &str, thing_ref: &str, token_id: &str, ts: &str) -> Result<String, JsValue>;
    pub fn prepare_consume(&mut self, key: &str, participant: &str, token_id: &str, request_id: Option<String>, ts: &str) -> Result<String, JsValue>;
    pub fn commit(&mut self, tx_id: &str) -> Result<String, JsValue>;
    pub fn abort(&mut self, tx_id: &str);

    // 只读视图
    pub fn read(&mut self, key: &str, participant: &str, token_id: &str, ts: &str) -> Result<String, JsValue>;
    pub fn counter(&self, key: &str, filter: Option<String>) -> Result<String, JsValue>;
    pub fn archive(&self, key: &str, filter: Option<String>) -> Result<String, JsValue>;
    pub fn gates(&self, key: &str, participant: &str) -> Result<String, JsValue>;
    pub fn enabled(&self, key: &str, participant: &str) -> Result<String, JsValue>;
    pub fn status(&self, key: &str) -> Result<String, JsValue>;

    // 分析/组合
    pub fn analyze(&self, declaration: &str) -> Result<String, JsValue>;
    pub fn compose(&self, fragments: &str, instance: &str) -> Result<String, JsValue>;

    // 恢复
    pub fn export_snapshot(&self, key: &str) -> Result<String, JsValue>;
    pub fn restore(&mut self, snapshot_json: &str, events_json: &str) -> Result<String, JsValue>;
}
```

说明：

- `key` 使用 `domain/instance` 字符串，避免跨边界传结构体。
- `predecessors` 传 JSON 数组字符串。
- `prepare_*` 返回 `{tx_id, event, result}`；`commit` 之后 `event` 才可以被宿主持久化确认。
- `read` 是 Rule-R，不改变状态，但宿主应追加读审计（本地 spike 可省略）。
- `analyze` 返回与原生 `AnalysisReport` 同构的字段，确保三接口一致。

### 4.3 prepare/commit 协议

原生引擎的顺序：

1. 校验命令；
2. 生成事件；
3. 追加 JSONL 并 fsync；
4. 应用到内存态；
5. 更新索引/通知。

WASM 本地模式：

1. `prepare_*` 校验并生成事件，分配 `seq`，但不改变已提交状态；
2. JS 存储层 `append(event)` + `flush/fsync`；
3. `commit(tx_id)` 应用事件并更新 `consumed_requests`、`terminated`；
4. 若持久化失败，`abort(tx_id)`，内存态不变；
5. 若进程在 2/3 之间崩溃，重启后按“日志优先”重放：日志有事件则提交，日志无事件则丢弃。

要求：

- 同一实例的命令必须由 JS 单线程串行化；`prepare` 到 `commit` 之间不允许其他写命令。
- `seq` 在 prepare 时分配；若 abort，该 seq 不复用，避免日志出现空洞；或者允许空洞但恢复时按 `last_seq+1` 继续。v0 建议不复用。
- `Terminated` 事件由 `commit` 后根据 `termination_holds` 生成，作为第二个待持久化事件返回；宿主同样 append/commit。

### 4.4 ID 与时间注入

- `token_id` 由 JS 生成（ULID 或测试用确定性序列），避免 WASM 依赖 `getrandom`。
- `ts` 由 JS 传入（ISO-8601），核心不读系统时钟。
- 测试中固定 ID/ts，使 WASM 与原生事件序列可以逐字节对账。
- 原生引擎的 `IdGen`/`Clock` 仍保持不变；WASM 通过参数注入达到同样的可测试性。

### 4.5 存储适配

JS 宿主定义统一接口：

```ts
interface PactStore {
  append(key: string, wireEvent: string): Promise<void>; // 内含 flush/fsync
  readAll(key: string): Promise<string[]>;
  readSnapshot(key: string): Promise<string | null>;
  writeSnapshot(key: string, json: string): Promise<void>;
  listInstances(domain: string): Promise<string[]>;
}
```

实现：

| 宿主 | 实现 | 持久化保证 |
|---|---|---|
| Node / Electron / DSH 宿主 | `fs` 追加 JSONL + `fsync`；快照 tmp+rename | 与原生接近 |
| 浏览器 | OPFS `createSyncAccessHandle` + `flush` | best-effort，弱于 fsync |
| 测试 | 内存数组 | 确定性 |

WASM 不直接调用存储接口；由 JS 在 `prepare` 与 `commit` 之间调用，保持“日志先于内存确认”的顺序。

### 4.6 单写者

- **Node/Electron**：一个进程持有一个数据目录；可用文件锁保证跨进程单写者（原生侧对应 `pact-http/src/daemon.rs` 对 `<data_dir>/pactd.lock` 的 flock）。
- **浏览器同源多 tab**：一个 `SharedWorker` 持有唯一 `PactWasm` 实例，各 tab 通过 `MessagePort` 调用；不可用时用 `navigator.locks` 按 `(domain, instance)` 串行化。
- **跨设备/多用户**：本地 WASM 不承担协调，必须连接远端 `pactd`。

PACT 的 Counter 单权威前提不能被多个 WASM 实例直接写同一份日志绕过。

## 5. 对账与验收

### 5.1 金标对账

同一组命令序列分别驱动：

- 原生 `pact-engine`（tokio + 原生 store）；
- `pact-wasm` 内存内核（固定 ID/ts）。

比较：

- 事件类型、seq、token 内容、ArchiveEntry；
- Counter/Archive/Status 视图；
- `gates`/`enabled` 结果；
- 错误码（非法 gate、缺失前驱、重复消费、版本过高）；
- `analyze` 报告；
- `Terminated` 判定。

语料：

- `examples/domains/react_loop.pact`
- `examples/domains/cross_harness.pact`
- `examples/domains/se_lifecycle.pact`
- `examples/domains/re_pipeline.pact`
- 良构性失败域、非法 gate、缺失前驱、重复 request_id、快照恢复

### 5.2 日志互通

必须测试：

1. WASM 生成的 JSONL 能被原生 `pactd` 启动加载；
2. 原生生成的 JSONL 能被 WASM `restore` 加载；
3. 快照 + 事件重放结果一致；
4. `WireEvent.version` 高于支持上限时 fail-fast；
5. Consumed 事件携带全量 token，WASM 与原生一致。

这是防止“WASM 版另起炉灶”的核心验收项。

### 5.3 验收矩阵

| 能力 | W-S0 编译探针 | W-S1 内存语义 | W-S2 Node 持久化 | W-S3 浏览器 |
|---|---|---|---|---|
| load/validate/analyze/compose | 必须 | 必须 | 必须 | 必须 |
| seed/produce/read/consume | 可选 | 必须 | 必须 | 必须 |
| prepare/commit | 否 | 必须 | 必须 | 必须 |
| 事件 JSONL 互通 | 否 | 必须 | 必须 | 必须 |
| 快照导出/恢复 | 否 | 必须 | 必须 | 必须 |
| 重启重放 | 否 | 可选 | 必须 | 必须 |
| Node fsync | 否 | 否 | 必须 | 否 |
| OPFS/SharedWorker | 否 | 否 | 否 | 必须 |
| 多 tab 单写者 | 否 | 否 | 否 | 必须 |

## 6. 与 DSH、NFR6 v2 的关系

### 6.1 DSH agent loop

- DSH 的 loop 是 `AgentFactory` 插件，可替换；但 WASM 只解决“PACT 在哪里运行”，不解决“DSH loop 是否由 PACT 驱动”。
- 建议顺序：
  1. 本地 WASM core + 存储适配；
  2. DSH 插件以 `observe` 模式镜像 session 事件；
  3. 需要治理时再进入 `govern` 模式（`agent/pre-step`、`tools/pre-execute`）；
  4. 真正的 loop 替换只对 `pact-*` preset 开放，不全局替换。
- WASM 不要求 DSH 先完成 loop 替换；DSH 插件也不要求 PACT 先有原生 daemon。
- **2026-10-22 状态更新（阶段 11，FR25/D37）**：用户明确要求推进 loop 替换，已在 `plugins/dsh/` 落地（2026-10-23 迁至独立仓库 `onlookor/pact-dsh`） observe/govern 两档与 `pact-*` preset 的选择性工厂接管；`PactBackend` 可注入，`FakePactBackend` 与 `WasmPactBackend` 均已完成，Node 测试 **47 项**通过；已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上完成真实装配（profile 安装、`--dump-config`、运行时 `ctx.agentLoop` 为 `PactAgentLoop`），并用 `deepseek-official/deepseek-flash` 验证 `pact-*` 允许、工具拒绝与预检拒绝三条路径。
- **2026-10-23 状态更新（WASM 后端 live）**：以 `wasm-bindgen 0.2.127` 生成 Node 产物，新增 `crates/pact-wasm/js/open-host.js` 作为组装入口（`PactWasm` 工厂 + `FileStore` + 声明 → `PactHost`）；DSH 插件以 `backend=wasm`、`wasmModule` 指向该入口复跑同一三条路径：允许（bash 实际执行，`isError:false`）、`denyTools` 拒绝、空 Counter 实例的 `agent/pre-step` 拒绝（仅 `turn/start → turn/end`）。实例事件日志由插件运行时经 WASM 宿主创建，证明“本地 WASM 内核 + 宿主内控制层”链路可用；浏览器共享/多 tab 仍按 §5 的 OPFS + Dedicated Worker 方案另评。

### 6.2 NFR6 v2 联网

- NFR6 v2 只改接口层（TLS、签名身份、MCP Streamable HTTP），引擎/存储/语义零改动；见 `requirements.md` NFR5–NFR7、`architecture.md` §4、`design.md` §12。
- WASM 本地模式不依赖 v2：单用户、同源多 tab 可在本地完成。
- 条件依赖：
  - 浏览器加入共享团队 Counter → 需要远端 daemon + v2 传输/身份；
  - 跨设备同步 → 需要远端 daemon；
  - 多用户 ACL/审计 → 需要远端 daemon；
  - 边缘/serverless → 需要单写者存储，v2 本身不解决。

### 6.3 共享接缝

| 接缝 | 关系 | 约束 |
|---|---|---|
| 事件 schema / 快照 | 强约束 | 必须共用 `pact-store-model`，不得为 WASM 新增字段 |
| 存储格式 | 强约束 | `<domain>/<instance>.events.jsonl` 与快照格式保持互通 |
| 传输抽象 | 软耦合 | `WasmClient` 与 REST/MCP 是同一接缝的不同适配器 |
| 单写者 | 软耦合 | WASM 用 SharedWorker/Web Locks，原生用 flock |
| ACL/审计 | 条件依赖 | 本地 WASM 不做安全边界；多用户走 v2 daemon |
| turso 索引 | 可分离 | WASM 可省略；迁移到 daemon 时重建 |

## 7. 实施计划

> 以下为阶段与退出条件，不给时间估算。每个阶段必须通过退出条件才进入下一阶段。

### W-P0：立项与编译探针（已完成，2026-10-21）

- 产出：
  - spike 范围按 §9.1 冻结；
  - `wasm32-unknown-unknown` target 已安装；
  - 对账语料与验收矩阵已确定（§5）。
- 退出条件（已满足）：
  - `cargo check --target wasm32-unknown-unknown -p pact-core -p pact-parser -p pact-analysis -p pact-compose -p pact-cpn` 通过；
  - `cargo build --target wasm32-unknown-unknown`（同五个 crate）通过。

### W-P1：纯类型层拆分（已完成，2026-10-21）

- 内容：
  - 新 `pact-store-model`：`Event`、`WireEvent`、`Snapshot`、`InstanceKey`、`apply_event`、`ModelError`、`EVENT_SCHEMA_VERSION`；
  - 新 `pact-views`：`CounterView`、`ArchiveView`、`StatusView`、`GateClause`、`DomainSummary`、`DomainListing`、`CompactionOutcome`、`AnalysisReport`；
  - 原生 `pact-store`/`pact-engine` re-export，保持外部 API 不变。
- 退出条件（已满足）：
  - 全仓测试通过（166 项；含 `pact-store` 既有恢复/压缩/审计测试）；
  - `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings` 干净；
  - 磁盘格式零变化（serde 属性逐字迁移，测试覆盖 JSONL/快照读写）；
  - 原生 daemon 读取既有日志与快照的路径由 `pact-store` 恢复测试覆盖；
  - `cargo check --target wasm32-unknown-unknown -p pact-store-model -p pact-views` 通过。

### W-P2：WASM 编译探针（W-S0，已完成，2026-10-21）

- 内容：
  - 新 `pact-wasm` crate（`crate-type = ["cdylib", "rlib"]`），原生测试与 wasm 共用 `PactHost`；
  - `wasm-bindgen` 导出 `PactWasm::{new, load_domain, validate, analyze, compose, list_domains}`，全部 JSON 字符串 ABI；
  - Node 脚本加载 `.wasm`，跑 `react_loop.pact`、`cross_harness.pact` 的静态分析；
  - 测量体积、加载与首次分析耗时。
- 实测：
  - 构建：`cargo build --target wasm32-unknown-unknown -p pact-wasm --profile wasm-release` 通过；
  - 产物：bindgen 处理后 raw 273,394 B（267 KiB），gzip 89,054 B（87 KiB）；
  - 导出：`pactwasm_new`、`pactwasm_load_domain`、`pactwasm_validate`、`pactwasm_analyze`、`pactwasm_compose`、`pactwasm_list_domains`、`pactwasm_abi_version`；
  - 对账：两个示例的 `analyze` JSON 与原生 `pact --json analyze` **深度相等**；
  - 时延（Node 22）：模块加载 1.71 ms、构造 0.35 ms、首次 `analyze` 3.24 ms。
- 退出条件（已满足）：`.wasm` 构建成功；分析输出与原生一致；体积与启动时间已记录为预算基线。
- 未纳入：浏览器绑定（`--target web`）、`wasm-pack` 打包、独立 CI job（W-Q7 留到 W-P2 之后另行决定）。

### W-P3：内存语义与对账（W-S1）

- 内容：
  - `PactWasm` 持有多个 domain/instance；
  - 实现 seed/produce/read/consume、prepare/commit、快照导出/恢复；
  - 固定 ID/ts 的黄金语料；
  - 与原生 engine 的事件序列、视图、错误码逐项对账；
  - JSONL 双向互通测试。
- 退出条件：
  - 语料零差异；
  - WASM 日志可被原生 daemon 加载；
  - 原生日志可被 WASM `restore` 加载；
  - 重复 `request_id` 幂等行为一致。
- 结果（2026-10-21，已完成，W-S1）：
  - **内存宿主**：`PactHost` 持有声明注册表 + 多 `(domain, instance)` 会话；每会话独立声明快照、事件序列、`last_seq`/`next_seq`、Rule-E 幂等表与 `terminated`。`load_domain` 对齐原生语义（空实例可重置、非空实例 `DomainBusy`、`load_domain_with(force)` 显式替换），`instantiate` 从已登记声明创建额外实例。
  - **两阶段提交**：`prepare_seed/produce/consume` 校验并分配 seq，返回 `{tx_id, seq, event, result}`（`event` 为完整 JSONL 行）；宿主 append/fsync 后 `commit(tx_id, terminated_ts)` 应用状态并按原生顺序返回 Rule-T `Terminated` 事件；`abort` 丢弃事务并回收 seq，保证日志连续、可被原生 `Store` 加载。
  - **只读与查询**：`read`（Rule-R）、`counter`/`archive`/`gates`/`enabled`/`status`，字段与原生 `pact-views` 同构。
  - **导出/恢复**：`export_events` 输出逐行含 `version` 的 JSONL；`export_snapshot` 输出与原生 `Snapshot` 同构的自足快照；`restore` 复刻原生 `recover_instance`（DomainLoaded/快照锚、事件重放、幂等表重建）。
  - **金标对账**（`crates/pact-wasm/tests/parity.rs`）：固定 `T0–T8` 与 `seq-0..2` 走完 requirement→design→implementation 生命周期，**8 条事件逐行与原生日志相等**（含 `Terminated`）；`status`/`counter`/`archive`/`gates`/`enabled` 逐项相等；`SeedAuthorityDenied`/`PredecessorNotOnCounter`/`TokenNotOnCounter`/`DomainBusy`/`IdempotencyConflict` 与原生错误码一致；同一 `request_id` 重放不产生事件且返回首次归档结果。
  - **JSONL 双向互通**：原生日志 → WASM `restore`（纯日志路径与「日志 + 快照」快速路径）状态逐字段一致；WASM 导出的日志与快照写入 `pipeline/default.events.jsonl` / `default.snapshot.json` 后，原生 daemon 打开即恢复一致。
  - **Node 运行时**：`abi_version = pact-wasm/0.2`；Node 22 实测 `load_domain_at` → `prepare_seed` → `commit` → `prepare_produce` → `read` → `export_events`/`export_snapshot` → 新实例 `restore` 全链路通过，未授权种子按 `SeedAuthorityDenied` 抛出。
  - **体积**（`wasm-release`，wasm-bindgen 0.2.127 Node 目标）：原始 514,656 B（503 KiB），bindgen 后 450,190 B（440 KiB），gzip 140,457 B（137 KiB）；相对 W-S0 的 267 KiB 增加 173 KiB（新增会话/提交/恢复逻辑），仍属可接受预算，记录为新基线。

### W-P4：Node 持久化与最小宿主（W-S2）

- 内容：
  - JS `PactStore` 的 Node 实现：JSONL + fsync + 快照 tmp/rename；
  - 崩溃/重启测试；
  - 最小 CLI 或 Node demo；
  - DSH 插件不在本轮范围（W-Q5），留待 W-P3 对账后另立项目。
- 退出条件：
  - 重启后状态一致；
  - 原生 daemon 与 WASM 宿主可交替打开同一日志；
  - 单写者约束有测试。
- 结果（2026-10-21，已完成，W-S2）：
  - **存储适配器**（`crates/pact-wasm/js/pact-store.js`）：布局与原生逐字一致（`<root>/<domain>/<instance>.events.jsonl`、`.snapshot.json`、`.lock`）；`append` 逐行写 + `fsync`；快照走 tmp 写入 + `fsync` + `rename` + 目录 `fsync`；`readEvents` 按字节截断末尾半行（崩溃残留）并 `fsync` 修复。
  - **单写者**：`.lock` 以 `O_EXCL` 创建并写入 pid，关闭时删除；同实例二次打开抛 `InstanceLocked`，持有者进程已消失时回收陈旧锁。
  - **最小宿主**（`pact-host.js`）：`open` 优先以日志/快照恢复（空日志 + 无快照才新建）；写路径严格执行 `prepare → append+fsync → commit`，追加失败即 `abort`；Rule-T `Terminated` 事件提交后追加。
  - **demo**（`js/demo.js`）：建实例 → 种子 → 生产 → 快照 → 关闭 → 重开，输出状态/计数器/归档/事件行数。
  - **测试**（`js/test/persistence.test.js`，`node --test` 7 项全绿）：重启状态一致、append 后未 commit 的「崩溃」按日志优先恢复、末尾半行截断、单写者拒绝/释放后重开、快照自足（清空日志仍可恢复）、幂等键跨重启、Rule-T 终止态落盘。
  - **交替打开**（`parity.rs::native_and_wasm_alternate_on_shared_log`）：原生 A 写入 → WASM B 接力消费并覆写 → 原生 C 打开一致并追加 → WASM D 再次打开一致。
  - **边界**：Node 版以 lockfile 近似单写者，不提供 `flock` 的内核级互斥；浏览器 OPFS/SharedWorker 仍不做（W-Q6）。

### W-P5：浏览器适配（W-S3，可选）

- 内容：
  - OPFS / IndexedDB 存储适配；
  - SharedWorker 单例 + Web Locks；
  - 多 tab 单写者测试；
  - 明确浏览器持久化弱保证。
- 退出条件：
  - 多 tab 不产生双写；
  - 页面刷新后可恢复；
  - 文档明确“浏览器模式不提供原生 fsync 等价保证”。

**W-P5 兼容性评估（2026-10-21，仅评估，未实现）**

- **结论**：**技术可行、无 Rust/WASM 层阻塞；工程量为中等，主要工作集中在 JS 存储适配与 Worker/锁编排；持久化保证弱于 Node/原生，不建议在没有真实浏览器宿主需求时启动实现**。与 W-P6 裁决 A 一致：浏览器形态仍不进入 FR21 范围。
- **已具备（无需改动内核）**：
  - `pact-wasm` 已成功构建 `--target web` 与 `--target no-modules`；产物 wasm 455,929 B（gzip 142,196 B），JS 胶水 43–47 KB，体积可接受。
  - 内核不读系统时钟、不依赖 `getrandom`、不使用线程/共享内存/SIMD，因此**不需要** COOP/COEP、`SharedArrayBuffer` 或随机数 polyfill。
  - 全部 ABI 为 JSON 字符串，`abi_version` 可探测；浏览器与 Node 可共用同一份 `.wasm`。
  - `Session` 无 IO，事件 schema/快照结构与原生一致，浏览器适配不需要改 Rust 语义层。
- **阻塞项与必须改造的部分**：
  - **Node 存储适配层不可复用**：`crates/pact-wasm/js/pact-store.js` 依赖 `node:fs`/`node:path`、`fsync`、`rename`、`O_EXCL`，浏览器无对应 API；需新增 OPFS 适配器（独立文件，不复用 Node 模块）。
  - **主线程不能做同步写**：`FileSystemSyncAccessHandle` 仅在 Dedicated Worker 可用，且需安全上下文（HTTPS）。浏览器宿主应把「WASM 实例 + 存储」放进同一个 Dedicated Worker，主线程只通过消息调用。
  - **没有 rename，也没有目录 fsync**：Node 版快照的 `tmp + rename + 目录 fsync` 无法照搬。建议改为**原位写快照 + 完整性校验**（magic/长度/校验和），加载时校验失败即忽略并回退日志；日志始终是真相源，快照只是加速恢复，因此不需要原子替换。
  - **没有 `O_EXCL` 锁文件**：改用 **Web Locks**（`navigator.locks`，Worker 内可用）做实例级单写者；`ifAvailable: true` 快速失败，或排队等待。`SyncAccessHandle` 自身对单文件的独占只能覆盖单文件，不能覆盖「日志 + 快照」整套实例。
  - **无原生 `fsync` 等价保证**：`SyncAccessHandle.flush()` 只承诺把该文件内容刷出，目录项/元数据与浏览器进程崩溃语义不受控；需配合 `navigator.storage.persist()` 降低淘汰概率，并在文档中明确弱保证。
  - **存储可被淘汰**：OPFS 仍受配额与浏览器清理策略约束（尤其 iOS/Safari 长期未使用场景）；必须提供导出/备份路径，不能把浏览器存储当唯一副本。
  - **长会话内存**：WASM 宿主当前保留实例完整 `events` 向量，浏览器长生命周期页面会累积；W-P5 需增加「重载自快照/日志」或暴露压缩能力，避免内存无界增长。
- **浏览器能力基线（桌面）**：

  | 能力 | 要求 | 支持基线 |
  |---|---|---|
  | WebAssembly + wasm-bindgen | 必需 | 现代桌面/移动浏览器均支持 |
  | OPFS（`navigator.storage.getDirectory`） | 必需 | Chrome/Edge 86+、Firefox 111+、Safari 15.2+（跨浏览器可用自 2023-03） |
  | `FileSystemSyncAccessHandle` | 必需，仅 Dedicated Worker | Chrome/Edge 102+、Firefox 111+、Safari 15.2+ |
  | Web Locks（`navigator.locks`） | 必需，跨 tab 单写者 | Chrome 69+、Firefox 96+、Safari 15.4+ |
  | SharedWorker | 可选（非必需） | 桌面支持，但 iOS Safari 不可用；**不列入基线** |
  | `navigator.storage.persist()` | 建议 | 支持，但授予与否由浏览器/用户决定 |

- **建议的浏览器形态（若立项）**：
  1. 每页面一个 **Dedicated Worker**：内部加载 `.wasm` + OPFS 适配器，所有写路径仍为 `prepare → flush → commit`。
  2. 实例级 **Web Locks**：打开时以 `ifAvailable` 获取独占锁，关闭/页面卸载自动释放；第二个 tab 得到明确 `InstanceLocked` 错误而非双写。
  3. **日志优先恢复**：末尾半行用 `SyncAccessHandle.truncate()` 截断；快照原位写 + 校验，损坏即忽略。
  4. **不依赖 SharedWorker**：避免 iOS 与生命周期不确定性；多 tab 协调只依赖 Web Locks。
  5. 差异仅落在 JS 适配层；Rust 内核与 ABI 不变，Node 形态继续使用现有 `FileStore`。
- **测试与 CI 影响**：Node 的 `node:test` 无法覆盖 OPFS。需要 Playwright/Chromium 无头测试（Worker + OPFS + 刷新恢复 + 双 tab 锁）；Firefox/WebKit 可做人工或独立矩阵，不建议在 spike 阶段全量接入。现有 `wasm` CI job 无需改动，W-P5 另立 job。
- **风险判断**：语义漂移风险低（内核共用）；主要风险是**持久化弱保证、浏览器存储淘汰、跨浏览器锁/Worker 差异**与测试成本。适合作为「浏览器本地单用户、可接受导出备份」的实验形态，不适合作为唯一权威存储。
- **状态**：**未实现**。触发条件仍按 §11.2：出现明确的浏览器宿主/离线单用户需求，且接受弱持久化边界。

### W-P6：决策门

- 评估：
  - 是否抽取 `pact-engine-kernel`，让原生与 WASM 共用命令层（**已完成，2026-10-21，D32**）；
  - 是否把 DSH 插件推进到 `govern` 模式（**不推进**）；
  - 是否需要远端 v2 模式（**不需要**）；
  - 是否进入正式立项（**A. 条件立项 → 同日按 D33 去除“实验性”标记，转正式**）。
- 退出条件：
  - 形成立项/不立项结论；
  - 若立项，另行更新需求/设计/架构变更文档，不在本报告内直接改决策文档。

## 8. 风险登记

| 编号 | 风险 | 影响 | 缓解 |
|---|---|---|---|
| W-R1 | WASM 与原生引擎语义漂移 | 高 | 金标对账；必要时抽 `pact-engine-kernel` |
| W-R2 | 事件模型拆分引入回归 | 中 | 纯重构、零格式变化、全量测试 |
| W-R3 | wasm 工具链/CI 缺失 | 中 | W-P0 安装 target；增加 wasm CI job |
| W-R4 | `wasm-bindgen` ABI 复杂 | 中 | v0 用 JSON 字符串 ABI |
| W-R5 | token id / 时间不可复现 | 中 | JS 注入；测试固定序列 |
| W-R6 | 浏览器持久化弱于原生 | 中 | Node 先做 fsync；浏览器文档化；关键场景用 daemon |
| W-R7 | 多 tab/多写者 | 高 | SharedWorker/Web Locks；单写者测试 |
| W-R8 | wasm 体积/启动超预算 | 低中 | 实测；按需裁剪 analysis；wasm-opt |
| W-R9 | 客户端无 ACL/审计 | 高（多用户） | 本地模式限定单用户；多用户走 v2 daemon |
| W-R10 | DSH 预发布 API 漂移 | 中 | 插件适配层；版本锁定；不把 DSH 作为 W-P0/W-P1 前置 |

## 9. 决策与待决问题

### 9.1 已冻结（2026-10-21）

| 编号 | 问题 | 决定 | 理由 |
|---|---|---|---|
| W-Q1 | 纯类型层命名 | 拆为 `pact-store-model` + `pact-views`，不合并 `pact-model` | 事件模型只依赖 `pact-core`+`serde`；views 依赖 `pact-analysis`。合并会让 WASM 无谓引入分析依赖 |
| W-Q2 | v0 ABI | JSON 字符串 | 跨语言稳定、易与原生 CLI 对账；后续可在 JS 包装层升级为对象，不改 Rust 内部 |
| W-Q3 | 写协议 | `prepare/commit` 作为唯一写路径；W-S1 用即时提交的内存适配器演练同一 API | 与原生“日志先于内存确认”一致；避免 `execute+rollback` 形成第二条语义路径 |
| W-Q5 | DSH 插件范围 | 本轮不做；W-P3 对账通过后另立项目 | DSH 为 developer preview，API 漂移风险 W-R10；WASM core 应独立可交付 |
| W-Q6 | 浏览器 OPFS | 本轮不做；W-S3 仅保留为 W-P4 后的可选门 | 浏览器持久化弱保证、SharedWorker 复杂度高；浏览器共享团队本身条件依赖 NFR6 v2 |

### 9.2 延后决策门

| 编号 | 问题 | 决策点 | 当前处置 |
|---|---|---|---|
| W-Q7 | 独立 WASM CI job | W-P2 产出稳定 `.wasm` 后 | **已完成（2026-10-21）**：`ci.yml` 新增 `wasm` job——wasm32 构建 + clippy + ABI/语义/体积门禁（`js/ci-smoke.js`）+ Node 持久化测试；设计见 `docs/design-change.md` 第 34 轮 |
| W-Q4 | 抽取 `pact-engine-kernel` | W-P6 决策门 | W-P3 明确不抽；先量化 WASM 内核与原生命令层的对账差异，再决定是否共用 |

## 10. 结论与建议

- **技术可行性：高（核心语义）/ 中高（本地引擎）/ 中（浏览器）/ 低（完整 daemon WASI）。**
- **建议立项形态：** W-P0–W-P4 已完成“语义核心 + 内存引擎 + 金标对账 + Node 持久化”，全程不碰浏览器、不碰 DSH、不碰 NFR6 v2；W-P6 评估见 §11，用户裁决 **A（条件立项）**，随后按 D33 去除“实验性”标记、转为受支持的本地分发形态。
- **最重要的架构约束：** 事件 schema、快照格式、存储布局必须与原生共用；WASM 不能成为第二套语义或第二套日志格式。
- **与 NFR6 的关系：** 无硬依赖；本地 WASM 可独立推进；只有远程团队/多用户模式才条件依赖 v2。
- **下一步：** W-P4/W-S2 已完成；W-P6 评估见 §11——用户裁决 **A（条件立项）**，并按 D33 去除“实验性”标记：WASM 作为受支持的本地分发形态维护（FR21/D31/D33），权威仍是原生引擎。W-Q7 CI、金标扩面、`pact-engine-kernel` 抽取与宿主边界安全审视均已完成（2026-10-21）；W-P5 浏览器兼容性评估已完成但未实现；DSH `govern`、v2、浏览器均不推进。立项与转正登记已写入需求/设计/架构及其变更文档。

## 11. W-P6 决策门评估（2026-10-21）

### 11.1 证据基线

| 维度 | 结果 |
|---|---|
| 语义一致性 | 代表性生命周期 8 条事件与原生逐行相等；5 类错误码一致；幂等重放一致（W-P3） |
| 互操作 | 原生日志 ↔ WASM `restore` 双向通过；原生与 WASM 交替打开同一日志通过（W-P3/W-P4） |
| 持久化 | Node JSONL+fsync、快照 tmp/rename、崩溃后日志优先、半行截断、单写者均有测试（W-P4） |
| 分发体积 | bindgen 后 440 KiB、gzip 137 KiB（W-P3 基线） |
| 代码规模 | `pact-engine/src/engine.rs` 1635 行（含 actor/store/通知/压缩）；`pact-wasm/src/host.rs` 1107 行（含注释与 8 项单测）；真正重叠的命令编排约 300 行量级 |
| 覆盖缺口 | 金标语料只覆盖单域单实例单生命周期；未覆盖多实例并发、组合域、GateExpr 条件分支、压缩后恢复 |
| 生产消费者 | 尚无真实宿主接入（无 Electron/Node 应用、无 DSH 插件），浏览器未验证 |

### 11.2 四个问题的结论

| 问题 | 结论 | 理由与触发条件 |
|---|---|---|
| W-Q4 抽取 `pact-engine-kernel` | **已抽取（2026-10-21，D32）**；立项后列为第一优先工程项并完成 | W-P3 已量化双实现在 seq 分配、幂等表、Rule-T 终止、恢复锚点四处重复（W-R1）。金标扩面完成后，`pact-engine-kernel` 以纯 `Session` 统一实例会话语义，原生与 WASM 仅保留持久化/通知/审计/错误映射等外围差异；约 300 行双实现漂移消除，全量 184 项测试与 parity 7 项通过 |
| W-Q5 DSH 推进到 `govern` | **已完成（2026-10-22/23，阶段 11，用户裁决；FR25/D37）** | 用户明确要求推进 loop 替换，接受 DSH developer preview 的 API 漂移风险（W-R10）。落地为独立仓库 `onlookor/pact-dsh`（当时位于本仓库 `plugins/dsh/`）：observe 镜像 + govern 门控 + `pact-*` preset 选择性工厂接管（`PactAgentLoop extends AgentLoop`），其余 agent 旁路；后端可注入，`FakePactBackend` 与 `WasmPactBackend` 均已接线。真实 DSH 装配已在 `@deepseek-ai/dsh@0.2.1-rc.2` 验证；`backend=wasm` 以 `wasm-bindgen 0.2.127` 产物在 headless profile 中实跑允许/工具拒绝/预检拒绝三条路径并通过 |
| NFR6 v2 远端模式 | **不需要** | 本地 WASM 单用户模式无 v2 依赖；v2 只服务跨设备/共享团队/多用户 ACL 与审计。触发条件：出现浏览器共享团队或跨设备协作需求 |
| 是否正式立项 | **A. 条件立项**，2026-10-21 用户裁决；同日按 D33 去除“实验性”标记、转为受支持形态 | 按 11.4 边界登记为 FR21/D31/D33；W-Q7 CI、金标扩面、kernel 抽取与安全审视均已完成 |

### 11.3 立项选项

| 选项 | 内容 | 代价 | 适用判断 |
|---|---|---|---|
| A. 条件立项（后按 D33 转正式） | 把 WASM 本地运行登记为本地分发形态（Node/桌面宿主）；不替代 `pactd`，不含浏览器与多用户。稳定化验收完成后去除“实验性”标记 | 需补 W-Q7 CI、ABI 版本策略、安全审视；稳定化前完成 kernel 抽取与覆盖扩展 | 仓库已投入 W-P0–W-P4 且质量达标，若继续保留代码，CI 是防止腐烂的最低成本；推荐 |
| B. 保持 spike 不立项 | 代码留仓库、文档标注 spike；不接 CI | 零新增承诺 | 适合“暂无宿主需求、也不愿承担 CI 维护”的情形；但 wasm target/绑定版本会随时间腐烂 |
| C. 正式立项（含浏览器/v2） | 含 OPFS/SharedWorker、多用户与远端 v2 | 高：浏览器持久化弱保证、SharedWorker 复杂度、v2 全套安全能力 | 当前无需求证据，不建议 |

### 11.4 W-P6 建议

**推荐 A（条件立项）——已于 2026-10-21 由用户裁决采纳，并于同日按 D33 去除“实验性”标记、转为受支持形态**，边界与前置条件：

1. **定位**：WASM 是 `pactd` 之外的**受支持本地分发形态**，权威仍是原生引擎；不承诺浏览器与多用户能力。
2. **准入条件（均已满足）**：W-Q7 CI（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）；ABI 采用 `pact-wasm/0.x` 并记录破坏性变更；推送/发布前完成宿主边界安全审视。
3. **工程项顺序**：W-Q7 CI（**已完成**）→ 扩大金标（多实例、组合域、条件分支、压缩恢复，**已完成**）→ 抽取 `pact-engine-kernel` 消除双实现（**已完成，D32**）→ 浏览器兼容性评估（**已完成，未实现**）。
4. **不纳入**：DSH 团队级原生运行时、NFR6 v2、浏览器 OPFS；触发条件见 11.2。DSH 的 observe/govern 与 `pact-*` preset 治理型 loop 接管已由阶段 11 单独落地（FR25/D37），不使用 WASM 内核作为先行条件。
5. **文档落点**：需求 FR21（`requirements-change.md` 第 18–19 轮）、设计 D31 + §16（`design-change.md` 第 33 轮）、D32（第 36 轮）与 D33（第 37 轮）、架构 §1 组件边界（`architecture-change.md` 第 20–22 轮）；DSH 插件见 FR25/D37（requirements 第 21 轮、design 第 40 轮、architecture 第 24 轮）。

## 12. 参考

- `requirements.md` NFR5–NFR7
- `architecture.md` §4 联网演进、§6 生产运行层与分发
- `design.md` §7.1–§7.3 事件类型/存储布局/IndexStore、§12 联网演进路径、§13 生产化边界
- `../crates/pact-core/src/lib.rs`
- `../crates/pact-store/src/event.rs`
- `../crates/pact-store/src/snapshot.rs`
- `../crates/pact-engine/src/engine.rs`
- `../crates/pact-engine/src/views.rs`
- `../crates/pact-analysis/src/lib.rs`
- `../crates/pact-parser/src/convert.rs`
- `../crates/pact-compose/src/lib.rs`
- `../crates/pact-cpn/src/lib.rs`
- `../examples/domains/react_loop.pact`
- `../examples/domains/cross_harness.pact`
