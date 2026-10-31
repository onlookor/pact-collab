# PACT 实现需求

来源：`paper/pactv1_rev3.md`（范式论文最新版，规范母本）+ 用户指示。本文件只保留当前有效需求，变更历史记入 `docs/requirements-change.md`。

## 1. 项目目标

实现论文提出的 PACT（Precondition-governed Autonomous Counter-based Token）范式的完整工程实现：一个单权威的协调引擎，忠实实现论文的全部形式语义（六原语、SOS 规则、GateHolds、GateExpr、静态分析、片段组合）；v1 默认单机，v2 在接口层补足受控联网（TLS/mTLS、签名身份、MCP Streamable HTTP、MCP OIDC）。

## 2. 总体约束

- **语言**：Rust（决策依据见 `docs/pact工程落地语言选择.md` 及 `docs/design.md` 决策 D1，无反对意见）。
- **部署范围**：默认单机（unix socket / 回环）；NFR6 v2 已支持受控联网（daemon 内 TLS/mTLS、Ed25519 签名、MCP Streamable HTTP、MCP OIDC），Counter 仍为单权威 client/server，无分布式共识。
- **开发环境**：所有开发工作在 Trae 沙箱环境中进行。
- **无人值守**：整个协调工作从启动到完成（含 Rule-T 终止）可全程无人工介入。

## 3. 接口需求

按优先级排序：AI agent（主要）≈ 普通应用程序（主要）> 人类。

### FR1 AI Agent 接口（MCP 协议）
- 提供 MCP server，agent 以参与者身份接入，执行全部领域操作；形态为 stdio（`pact-mcp` 薄封装，经 REST）与 daemon 内建 Streamable HTTP `/mcp`（FR23），二者工具面一致。
- 工具集覆盖：域加载/校验、Rule-S/P/R/E 四类动作、Counter/Archive 查询、静态分析、片段组合、参与者能力查询（`my_gates`）、Rule-P 可满足性查询（`enabled`，服务 Rule-W）、Counter 变更订阅（Rule-W 唤醒）、终止状态查询。

### FR2 应用程序接口（REST/JSON）
- 提供 REST API，与 MCP 共享同一引擎门面，能力对等（除订阅用 SSE 外）。
- 身份为静态 Bearer 或 Ed25519 签名（FR22）；REST 不接受 OIDC 令牌。

### FR3 人类接口（CLI）
- **注入点**：通过 `.pact` 域声明文件（论文 §3.1 文法）注入 TypeSchema / PreconditionGraph / GateMatrix / SeedAuthority 四个注入点；种子注入（Rule-S）。
- **领域操作**：produce / read / consume / 查询 / 静态分析 / 片段组合的命令行入口。

## 4. 功能需求

### FR4 范式核心语义（pact-core）
- 六原语：Token、Participant、Precondition、Gate、Counter、Exhaustion。
- SOS 规则：Rule-P（生产，前驱不被消费；前提含 `Precond(T) ≠ ∅`——源类型不可生产，rev2 §1.4；前驱实例互异只按 TokenId 判定——Rule-P Note）、Rule-R（读取，非破坏性）、Rule-E（消费，显式、无级联、入 Archive）、Rule-S（种子注入，豁免前置条件，受 SeedAuthority 约束）、Rule-W（等待）、Rule-T（终止，rev2 Rule-P-enabled 三条件定义）。
- GateHolds 统一准入谓词（§1.4.0）：静态 gate + 条件子句合取；仅作用于 read/consume。
- `via` 条件的溯源敏感见证检查：见证必须同时在 Counter 上且 id 出现在目标 token 的 `depends_on` 中（实例级，非类型级）。
- 零隐式动作原则：Counter 变更当且仅当显式的 Rule-P/E/S。

### FR5 GateExpr 条件语言（§1.7.3）
- 原子：`via T`、`exists T`、`count T >= n`、`count T = n`；组合：`and` / `or` / `not`。
- count 字面量受 PosNat 约束（§1.7.3/§3.1）：n 必须匹配 `[1-9][0-9]*`——0 与前导零均为文法违例（`count T = 0` ≡ `not exists T`、`count T >= 0` ≡ true，按 Normalization rule 须拒绝；缺席语义用 `not exists T` 表达）。
- 条件为纯函数，搜索空间限于当前 Counter（+ 目标 token 的 depends_on），禁止引用 Archive 与外部状态。

### FR6 域声明文法（§3.1）
- 解析 `types` / `preconditions` / `gates` / `seed_authority` 四段式声明。
- 良构校验为 §3.1 六项约束（rev2）：(1) RHS Uniqueness——同一 rhs 至多一条前置规则；(2) LHS Uniqueness——单一规则内前驱类型名互异（`{A + A} → B` 非法，自环 `A → A` 合法）；(3) Non-Empty LHS——`{ } → T` 非法，源类型仅由规则缺席定义；(4) Type Closure——引用类型必须声明；(5) Positive-Nat Literal——count 字面量非 0；(6) Source-Production Constraint——源类型不得声明 produce gate（源类型唯一入口为 SeedAuthority）。解析期语法拒绝（约束 2/3/5 的违例形式）+ 声明期 `validate_domain` 防御（覆盖程序化构造/组合出的域）。
- `via` 糖形式与 `when` 形式等价（§1.7.4）。

### FR7 静态分析（附录 A，全部六项）
- 可达性（O(|V|+|E|)）、Gate 覆盖、消费可达性、结构死锁（Tarjan SCC + 种子入口检查）、前置与源生产良构性（rev2 A.5——§3.1 六项约束的违规清单 + Seed/Produce 重叠咨询性警告，三元输出 Violations/Warnings/WF(D)，且为其余分析的前提，附录 D 顺序置于报告首位）、搁浅风险（无环精确 / 有环保守过近似）。

### FR8 片段组合（§1.8）
- 并行组合 = 四元组按集合并；四条件一致性检查（C1 前置唯一性保持、C2 共享类型前置一致、C3 gate 条件语法同一性、C4 源生产封闭——rev2）。

### FR9 持久化与审计
- 事件溯源：Rule-S/P/E 均为不可变事件；append-only JSONL 事件日志为唯一真相源。
- SQLite 作为派生索引（查询加速），可从日志重建。
- Archive 语义：消费记录含 token 全量数据、exhausted_by、exhausted_at。
- 事件自足：Consumed 事件携带 token 全量快照（含可选幂等键 request_id），单行日志即可独立审计与重放，不依赖同一域实例其他行。
- 提交契约：事件日志 fsync 为**提交点**；其后步骤（索引投影、快照、Terminated、通知）为派生动作，失败不回滚、不失败本次调用（索引滞后由启动一致性校验重建自愈）。
- 崩溃恢复：快照 + 日志尾部重放。

### FR10 引擎服务
- 单权威：所有状态变更经串行化执行点，GateHolds 检查与状态变更原子完成。
- 多域会话：一个引擎实例可承载多个命名域，各自独立 Counter/Archive；**域内可并发承载多个 Counter 实例**（D27，见 FR19）。
- Rule-W 唤醒：订阅推送（状态变更事件主动通知订阅者）+ 轮询查询兜底。
- Rule-T 检测：Counter 为空且无任何参与者的 Rule-P 可满足时发出终止事件（信息性，不锁定域实例）。
- 外部副作用边界：引擎不执行外部副作用，ThingRef 对引擎不透明；消费成功与外部动作成功为**独立承诺**（最终一致，见 FR15）。

### FR11 无人值守运行
- 引擎被动：不编排、不拉起 agent 进程；agent 由外部机制启动后通过 MCP 自驱接入，靠 Counter 状态自组织（决策 D5）。
- 提供从域加载 → 种子注入 → 协作推进 → 终止检测的完整无介入闭环所需全部协议能力。

### FR12 领域数据格式
- `.pact` 域声明文件为四个注入点的唯一载体，格式遵循论文 §3.1 文法。
- 定义线上数据格式（JSON）：Token、ArchiveEntry、规则动作请求/响应、结构化错误（错误码 + 载荷）、Counter 变更通知事件。
- 规则动作请求：Rule-E（consume）可选携带 `request_id` 幂等键；同一域实例内重复键返回首次归档结果，同键异 token 返回错误码 `IdempotencyConflict`（HTTP 409）。
- 全部接口（MCP / REST / CLI 输出）共享同一套 JSON 线上格式，保证三接口行为可对照。
- 时间戳统一 ISO-8601 UTC；字段名 snake_case。

### FR13 测试组织
- 随变更组织四类测试：单元测试（各 crate 纯函数）、集成测试（store/engine 命令流、Rule-E 幂等去重与跨重启）、端到端测试（MCP/REST/CLI 完整生命周期）、回归测试（论文 §4.2 区分性场景 State A/B 及全部黄金轨迹）。

### FR14 通用示例与用户手册
- 交付一套**与具体方法论技能无关**的需求工程/软件工程通用域示例（`examples/`），用于向本系统用户说明使用场景与方法：
  - `examples/domains/`：五份通用域声明（软件工程生命周期、需求工程流水线、ReAct/agent 自驱循环、跨治理框架复用、外部物化确认/witness-token）；`examples/fragments/`：两份片段组合演示（成功合并与 C2 冲突各一）。
  - `examples/offline/run.sh` 与 `examples/online/run.sh`：可重复执行的两条上手路径（离线三件套 / 在线 daemon 全生命周期至 Rule-T），各自录制真实轨迹输出。
  - `examples/README.md`：场景导航与逐例讲解，自足可读，指向手册对应章节。
- 交付权威用户手册 `docs/manual.md`：适用判据（论文 §2.1 五条判据、§2.3 九条边界）、三接口完整命令面、六项静态分析、组合冲突、审计与事件重放、错误码全集、故障排查、已知偏差与限制。
- 二者分工：`examples/` 是**可运行的教学路径**（读得懂、跑得起），`docs/manual.md` 是**参考权威**（查得到全貌）；两者交叉引用，不重复维护同一事实。
- **定位表述**：所有面向用户的 README 与手册开篇必须说明 PACT 的定位（声明式协同层，不是 agent 框架或工作流执行器）、归类、与常见工具的异同及不适用边界；对比只陈述可核对的能力差异，不贬低他者；不得把某个宿主的装配细节（如 DSH 插件“不重写 loop”）当作产品定位。

### FR15 消耗原子性、幂等与物化边界
- 提交契约：Rule-E 的提交点 = 事件日志 fsync；其后步骤（索引投影、快照、Terminated、通知）失败不回滚、不失败本次调用（决策 D23）。
- 原子域：GateHolds 检查（含 via/exists/count/not 读集）与「Counter 移除 + Archive 追加」在同一临界区完成，无 TOCTOU、无并发双消费。
- 幂等：consume 可选 `request_id`；同一域实例内重复请求返回首次归档结果，不重复消费；跨重启（由事件日志重建幂等表）仍生效；同键异 token → `IdempotencyConflict`。
- 边界：引擎不执行外部副作用，`consume` 成功 ≠ 外部动作成功；"产物未就位"应前移为 gate 准入条件（witness-token 模式）或后置为显式补偿（saga 式），**不得**表现为消费回滚。
- 文档化：上述契约与边界须写入 `docs/design.md`、`docs/manual.md` 并在 `examples/` 提供对应示例。

### FR16 生产运行层（agent 驱动运行时 + 外部副作用执行器）

引擎保持被动（FR11、决策 D5）：不编排、不拉起 agent。生产系统须在引擎之外补齐：

- **可复用的参与者驱动运行时**：给定参与者名与其门（`my_gates` / `enabled` / `subscribe`），实现「等待 → 决策 → 行动」自驱循环，并负责 N 个 agent 进程的启动与编排。当前该逻辑仅存在于 e2e 测试（`pact-mcp/tests/unattended_e2e.rs` 的生产优先 / 汇消费 / 等待三档策略），无可复用交付物。
- **外部副作用执行器**：订阅 `Consumed` 事件（或经 outbox）真正落地外部动作；失败按 FR15 边界以追加补偿事实（saga）承担，不表现为消费回滚。
- **归属**：本仓库新增 crate（决策 D24）。引擎、存储、核心语义零改动。

### FR17 引擎生产化

- **事件日志压缩与 schema 版本化**：`pact-store` 提供快照之后的事件日志截断/轮转与保留策略；事件与线上格式补 `version` 字段与兼容契约（旧日志重放语义明确）。
  - **截断机制**：**前缀删除 + 旁路水位清单**——快照为提交点，`<instance>.logmeta.json` 记 `base_seq` 水位（先于日志重写），日志原子重写（tmp+fsync+rename）仅保留 `seq > base_seq` 行；水位单调不回退。
  - **快照自足与幂等存活**：压缩后日志可能不含 `DomainLoaded`，故快照须承载 `version` / `seq` / `dl_seq` / `declaration` / `state` / `consumed_requests`（**Rule-E 幂等表随快照持久化**，否则前缀内 `Consumed` 行被丢弃会破坏跨重启幂等）；恢复以快照为锚补尾部事件。
  - **数据安全优先**：日志已截断而快照缺失或不覆盖水位 ⇒ **fail-fast**（`SnapshotRequiredForCompactedLog`），不静默丢数据。
  - **版本兼容契约**：普通事件 `EVENT_SCHEMA_VERSION = 1`，状态交接事件 `Imported` 写 `IMPORTED_EVENT_SCHEMA_VERSION = 2`，读取上限 `SUPPORTED_EVENT_SCHEMA_VERSION = 2`（见 FR26）；缺 `version` 的旧日志/旧快照视同 v1（向后兼容）；`version > 上限` ⇒ **fail-fast `UnsupportedSchemaVersion`**（不静默按 v1 重放）。
  - **触发方式**：运维显式接口（REST `POST .../compact`、CLI `pact compact`、MCP `pact_compact`）+ 可选启动自动压缩（`pact.toml` 的 `compact_on_start` / `compact_keep_last`；缺省关闭）。
  - **固有取舍（须文档化）**：压缩后被丢弃前缀不再可查——审计视图退化为「快照 + 保留尾部事件」，`events` 索引行被裁剪（`tokens` 投影行保留）；NFR2 的「日志即完整审计」在压缩后由「快照 + 保留尾部」共同承担。v1 不做自动周期性轮转。
- **运维接口**：daemon 提供 `GET /health`（存活）、`GET /ready`（就绪：引擎可应答）、`GET /metrics`（Prometheus 文本格式运行指标，含 uptime 与域实例计数）；可选 `pid_file`（缺省不写，含陈旧检测）；信号化优雅停机（SIGINT/SIGTERM → 停止接受新连接 → 有界等待在途 → 终态快照 → 清理）。运维端点**不校验令牌**（仅供本地/内网探活与抓取）。
- **域级授权与审计读视图**：接口层按域 ACL（阻止任意客户端 `load` / `unload` 任意域）；读审计（Rule-R 与视图记录）为**可选开启**扩展，默认关闭（修订决策 D9 的默认值表述，见 D25/D43）。
  - **ACL 粒度**：条目为 `domain/instance`，支持 `domain/*` 通配（实例键，FR19）；配置形如 `[auth.acl] principal = ["proj/*", "other/default"]`。
  - **ACL 范围**：启用后覆盖全部域作用域端点——生命周期（load / instantiate / unload / compact / `:analyze` / `:compose`）、四类动作（seed / produce / read / consume）、查询（counter / archive / gates / enabled / status / events）与读审计视图；`GET /domains` 按授权过滤（空域剔除）。无令牌 → `401`，越权 → `403`。**未配置 ACL 时保持既有行为**（生命周期与查询不绑定身份，向后兼容）。
  - **配置合法性**：ACL 非空而令牌表为空 ⇒ daemon 启动失败（身份不可判定）；每条目须为合法 `domain/instance` 或 `domain/*`。
  - **读审计**：开启 `read_audit = true` 后，每次成功的 Rule-R 读取在 `<data_dir>/<domain>/<instance>.read-audit.jsonl` 追加一条自足记录（`version/kind/ts/domain/instance/participant/token_id`），每次成功的 Counter/Archive 视图读取追加一条 `kind=view` 记录（`view`、`type_filter`、按类型聚合的 `types[{type,tokens,refs_visible,refs_filtered}]`）。视图记录只记暴露计数，**不记录 `thing_ref` 值**。**fail-closed**：审计写失败则本次读取或视图以 `500` 拒绝，不允许「读成功但无审计」。审计流**不进入**事件日志 / `seq` / 重放 / Rule-T，不参与快照与压缩。
  - **只读视图**：REST `GET /domains/{d}[/instances/{i}]/read-audit?limit=N`（缺省 100，上限 1000，时间正序）返回 `{records:[...]}`；旧记录无 `kind` 字段时按 `read` 解析。

### FR18 部署与分发产物

- 本仓库 `dist/` 目录交付：容器镜像（Dockerfile）、systemd unit、静态二进制（musl）打包脚本。
- 与 FR14/决策 D22 同精神：发行产物属交付资产，不接入 CI 的语义测试（`examples/**` 仍不被 cargo test 遍历）。

### FR19 域双键实例化（D27）

- **定义键 / 实例键分离**：同一域声明（定义键 `domain`）可并发承载多个 **Counter 实例**（实例键 `instance`），各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表。
- **实例为接口层一等键**：REST 路由、CLI、MCP、ACL、审计读视图、日志压缩、运维指标均以实例为键维度——原「以域名为唯一会话键」的语义收窄为「以实例为会话键」。
- **默认回退**：不指定实例且无活动指针 ⇒ 默认实例（`default`）；活动指针存在 ⇒ 短路由落到活动实例（FR26/D39）。既有 `--domain` 调用在未交接时仍落到 `default`，语义不变。
- **旧日志不兼容（用户裁决）**：双键布局下只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**，启动恢复时 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移，不静默丢数据。
- **实例键承载形态（用户裁决）**：实例名作为**路径段**承载（`/domains/{d}/instances/{i}/…`），`/domains/{d}` 解析域的**活动实例**（无指针回退 `default`，FR26/D39）；`DomainLoaded` 事件**不新增 `instance` 字段**（实例由文件路径承载）。
- **声明原文另存**：`declare` 时把声明原文写入 `<data_dir>/<domain>/declaration.pact`，`recover` 用它回填声明注册表（支持重启后新增实例）。
- **形式依据**：论文 rev3 §1.3「Counter instantiation and partitioning」——实例划分属**实现自由**，论文全部规则与性质**按实例成立**（Rule-T 各实例独立终止；§1.4.1 TokenId 唯一性作用域为「该 Counter」，实现以 ULID 加强为跨实例唯一，见 W-Q4 裁定）；跨实例协同不在范式范围内。
- **通用护栏（用户裁决）**：同一 `data_dir` 上 daemon 启动须获取独占 flock（`<data_dir>/pactd.lock`），第二实例启动即拒绝；对**非空实例**（Counter/Archive 非空，或 `last_seq > 基线 seq`；含 `Imported` 接手状态）的再加载默认拒绝（`DomainBusy`），须显式 `force` 才重置并解冻。
- **归属**：`pact-store` + `pact-engine` + 接口层（`pact-http` / `pact-cli` / `pact-mcp` / `pact-runtime`）；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动（形式语义不变）。
- **实现状态**：已实现（阶段 10，路径 β）。存储 `(domain, instance)` 双列 + 域子目录布局；引擎声明注册表（`declare` / `instantiate` 分离）；接口全链（REST 默认族 + 实例镜像族、CLI `--instance`/`--force`/`instantiate`/`instances`、MCP 21 工具含 `pact_domain_instantiate` 与四个状态交接工具、`pact-runtime` 实例透传）；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。

### FR20 CPN 编码生成（阶段 6，D12/D28）

- **目标**：把一份良构域声明离线编码为 CPN Tools 彩色 Petri 网模型（论文 §4.5/附录 B），供形式验证与可视化检查；**不进入**引擎、存储、接口运行时路径。
- **入口**：新 crate `pact-cpn`（lib）+ CLI 离线命令 `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；不依赖 daemon。
- **单实例作用域（D12/D27）**：一次生成只编码**一个 Counter 实例**，库所/迁移以实例名命名空间化，不跨实例共享；跨实例协同不进入 `N(D)`。
- **编码映射**：每类型一个库所 + 一个 Archive 库所；token 颜色 = `TokenId × Participant × ThingRef × PrecondRefs × Timestamp`；生产迁移对前驱使用 test arc（输入绑定 + 同变量输出，不消费）；读迁移自环；消费迁移消耗目标并输出到 Archive；种子迁移无输入弧。
- **条件编码**：`and`/`or`/`not` 先化 NNF 再化 DNF；`or` 展开为多条迁移；`not` 下推为 `count T <= k`，用互补库所编码（调用方须为涉及类型提供 `--bound T=N` 的**真实并发上界**）；`not via` 无法用计数互补表达，显式拒绝；条件恒假的迁移保留并置 `guard=false`；DNF 项数与 `count >= n` 的展开规模各设 1024 上限，超限显式失败。
- **输出与验收**：CPN Tools XML（`format='6'`）；生成结果须通过 CPN DTD 校验、XML 良构与结构断言（库所/迁移/弧计数、test arc、互补不变量、单实例前缀）。CPN Tools GUI 内的交互式加载与状态空间实验由人工验收，不纳入 CI（沙箱无 GUI）。
- **实现状态**：已实现（2026-10-21）；`pact-cpn` 11 项测试 + CLI 离线 e2e 覆盖结构与错误路径。

### FR21 WASM 本地分发形态（D31/D33）

- **定位**：WASM 构建是 `pactd` 之外的**可选本地分发形态**，供 Node/桌面宿主内嵌运行；**权威仍是原生引擎**，NFR4 的单一静态二进制形态不变，WASM 不替代 `pactd`、不承担服务端角色。
- **实现状态**：**已稳定（2026-10-21，D33）**。稳定化前置 (1) W-Q7 独立 CI job、(2) 金标覆盖扩展、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视均已完成；WASM 作为受支持的本地分发形态维护，但**不在 `dist/` 产物集内**，仍不替代 `pactd`。
- **能力范围（W-S2 已实现，2026-10-21；D50 于 2026-10-31 补入口校验）**：语义核心（解析/良构校验/静态分析/片段组合/CPN 投影）、内存多实例会话（`(domain, instance)` 键）、Rule-S/P/R/E 与 Rule-T、两阶段提交（`prepare → 宿主持久化 → commit` / `abort`）、事件日志 JSONL 导出、自足快照导出与 `restore`、查询视图；事件 schema、快照结构与磁盘布局与原生共用（`pact-store-model` / `pact-views`）。写入前的票面校验与 REST 共用 `pact-ref-spec::validate_entry`：宿主可注入 RefBindings（`refBindings` / `PACT_WASM_REF_BINDINGS`），未注入时只做结构校验（FR30/D50）。
- **宿主职责**：持久化（JSONL append + fsync、快照 tmp+rename）、崩溃恢复（日志优先、末尾半行截断）、单写者（lockfile）由宿主适配层承担；WASM 内核不假定拥有 fsync、flock、socket 或进程能力。Node 适配层已交付（`crates/pact-wasm/js/`）。
- **能力边界**：不含浏览器 OPFS/SharedWorker、多用户 ACL/读审计、MCP/HTTP 服务、TLS、turso 索引、日志压缩与 `pactd` 运维接口；DSH `govern` 与 NFR6 v2 无硬依赖（见范围外）。**安全边界为本地单用户**，不得视为多用户安全边界。
- **ABI 策略**：`pact-wasm/0.x`，全部方法使用 JSON 字符串；破坏性变更须提升 ABI 版本并记入 `docs/design-change.md`；版本探测入口为 `abi_version`。去除非实验性标记不改变 ABI 版本策略。
- **稳定化验收（已完成）**：(1) W-Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）——**已实现（2026-10-21，`ci.yml` 的 `wasm` job）**；(2) 金标覆盖扩展（多实例、组合域、条件分支、压缩后恢复）——**已实现（2026-10-21，`crates/pact-wasm/tests/parity.rs` 7 项金标）**；(3) `pact-engine-kernel` 抽取——**已实现（2026-10-21，原生 `pact-engine` 与 `pact-wasm` 共用实例会话语义，D32）**；(4) 推送/发布前宿主边界安全审视——**已完成（2026-10-21，无新增可利用发现）**。四项验收完成后按 D33 去除实验性标记。

### FR22 TLS 与 Ed25519 请求签名身份（NFR6 v2）

- **TLS（可选）**：daemon 在 TCP 监听上可选终止 TLS（rustls，TLS 1.2/1.3）；`[tls]` 提供 PEM 证书链与私钥，`client_ca` 存在时要求客户端证书（mTLS）；`listen_unix` 不受影响。内置客户端（CLI / `pact-mcp` / `pact-runtime`）以 `--https` 连接、以 `--tls-ca` 信任自定义 CA；内置客户端不携带客户端证书，mTLS 面向外部客户端。
- **签名身份（可选）**：`[auth.keys] 参与者 = base64(32 字节 Ed25519 公钥)` 非空即启用。客户端以 base64(32 字节) Ed25519 私钥种子对请求签名，携带 `x-pact-participant` / `x-pact-timestamp` / `x-pact-nonce` / `x-pact-signature`；规范化串为 `PACT-SIGNATURE-V1`，覆盖方法、路径+查询、请求体 SHA-256、时间戳、nonce、参与者。服务端校验时间窗 `±300s`、nonce 去重（TTL 300s、上限 8192）、签名与请求体（> 8 MiB 返回 413）；通过后把已验证参与者写入可信头 `x-pact-verified-principal`，并剥离客户端伪造的同名头。
- **身份绑定**：签名与 Bearer 可同时配置；一旦 `[auth.keys]` 非空，除豁免端点外的 REST 请求**必须携带有效签名**（Bearer 不能替代签名；两者同时出现时以签名身份为准）。请求体/查询中的 `participant` 必须与解析出的身份一致，否则 `401 Unauthorized`（不是 403）；ACL 照常生效。签名身份仅用于 REST（含 CLI/MCP stdio 转发），不用于 `/mcp` 的 OIDC 路径。
- **豁免与信任模式**：`/health` `/ready` `/metrics` `/mcp` 与 OIDC 元数据端点豁免验签；REST 未配置任何身份（无令牌、无密钥；OIDC 只影响 `/mcp`）时为信任声明模式，仅在单机/回环部署使用。
- **线上契约与错误码**：签名请求头四件套；可信头 `x-pact-verified-principal`；进程内能力头 `x-pact-internal-auth`（随机 32 字节，不落日志）；缺/错身份 `401`、participant 不匹配 `401`、ACL 越权 `403`、签名请求体超限 `413`、nonce 重放 `429`、IdP 不可达 `503`（OIDC，见 FR24）。

### FR23 MCP Streamable HTTP（NFR6 v2）

- daemon 在 `[mcp].enabled = true` 时于同一监听端口内建 `/mcp`（MCP Streamable HTTP），工具面与 stdio 完全一致（21 个 `pact_*` 工具）；`pact-mcp` 保留 stdio 形态，作为薄封装经 REST 接入 daemon。
- **线上契约**：`POST/GET/DELETE /mcp`；`[mcp].public_url` 为对外可达基址，用于 `allowed_hosts` 与 OIDC 元数据。
- **鉴权**：令牌表非空时要求 `Authorization: Bearer <token>`；配置 OIDC 时按 FR24 校验 JWT；两者皆无时为本地信任模式（仅单机/回环，启动打印警告）。`[auth.keys]` 非空而令牌表为空且无 OIDC 时拒绝启动，避免 `/mcp` 仅凭公钥冒用身份。
- **本地调用**：daemon 内建 MCP 通过进程内 tower 路由调用自身 REST，不发起网络自连接；工具调用使用随机内部能力令牌 `x-pact-internal-auth`，并仅按当前请求注入已验证参与者，禁止跨会话串用或把外部 JWT 转发给 REST。
- **宿主保护**：rmcp 侧 `allowed_hosts` 仅允许回环与 `[mcp].public_url` 的 authority；不新增公网多租户、限流或 WAF。

### FR24 MCP OIDC 身份（资源服务器模式，NFR6 v2）

- **定位**：daemon 作为 OIDC **资源服务器**，只验证 IdP 签发的 JWT，不内置授权服务器、不实现授权码 + PKCE、登录页或 token 端点；授权码流程由 MCP 宿主/客户端向 IdP 完成。
- **范围**：OIDC 身份**仅 `/mcp` 接受**；REST 仍为静态 Bearer 或 Ed25519 签名；OIDC 令牌不缓存、不转发给 REST。
- **配置**：`[mcp.oidc]` 的 `issuer`、`audience`、`principal_claim`（缺省 `sub`）、`principal_map`、`jwks_uri`、`discovery`、`allowed_algs`、`ca`、`allow_static_tokens`、`allow_insecure_http`；启用 OIDC 时 `[mcp].enabled = true` 且 `public_url` 必填。
- **校验**：alg 白名单（缺省 RS256/ES256/EdDSA，拒绝 `none` / HS* / 非空 `crit`）；`iss` 精确匹配、`aud` 包含配置值、`exp` 必需、`nbf` 存在时校验，允许 60s 时钟偏移；JWKS 缓存 TTL 300s、最多 64 键，未知 `kid` 限频（最短 30s）重取一次；`principal_claim` 取值须为字符串（或唯一字符串元素的数组），经 `principal_map` 映射后须匹配 `[A-Za-z0-9_-]{1,128}`。
- **线上契约**：`GET /.well-known/oauth-protected-resource` 返回 `{resource, authorization_servers, bearer_methods_supported}`；`/mcp` 的 401 响应携带 `WWW-Authenticate: Bearer resource_metadata="<public_url>/.well-known/oauth-protected-resource"`；令牌非法 `401`，IdP 发现/JWKS 不可达 `503`，**REST 不受 IdP 故障影响**。

### FR25 DSH Agent Loop 集成（D37，阶段 11）

- **定位**：交付 DSH（DeepSeek Harness）插件包 `dsh-plugin-pact`（独立仓库 `onlookor/pact-dsh`，零运行时 npm 依赖；2026-10-23 自本仓库 `plugins/dsh/` 迁出，运行时行为未改），把 PACT 门控接入 DSH 的 agent step 与 tool 边界；**不重写** DSH 的 `ReactLoopAgent` 调度，也不接管模型调用、工具执行与对话式 ReAct。
- **两档运行**：`observe`（缺省）只镜像 `session/event` 到有界审计缓冲（可选 JSONL，目录 `0700`/文件 `0600`），不阻断任何动作；`govern` 在 `agent/pre-step` 与 `tools/pre-execute` 查询 PACT 门，未启用或策略拒绝时分别返回 `{kind:'reject'}` / `{kind:'deny'}`，后端异常时 fail-closed（`observe` 档 fail-open 并留痕）。
- **选择性工厂接管**：`cordis.patch.yml` 禁用原始 `agent-loop` 行，并插入使用同一 `agentLoop` 服务的 `pact-agent-loop` 行（`dsh-plugin-pact/loop`）；DSH patch 不能改写行的 `name`，只能按 `id` 覆盖或禁用。该子类复用官方 `AgentLoop`，仅对 `agentPreset` 以 `pact-` 开头的 agent 组合 `setup` 并安装 agent 作用域钩子，其余 agent 原样委托，行为与官方一致。
- **后端可注入**：插件只依赖 `PactBackend` 契约（`enabled/gates/status/seed/produce/consume/close`），本轮提供内存 `FakePactBackend` 与惰性 `WasmPactBackend` 适配层（`openPactHost(options)`），WASM 产物未就绪时以 `BackendUnavailable` 显式失败；不引入守护进程、端口或网络依赖。WASM 适配把宿主的 `enabled`（Rule-W 可生产选项数组）收窄为布尔语义——非空即“当前可执行”，与插件契约一致。
- **审计边界**：只记录 `type/seq/工具名/call id`，**不记录**工具参数、结果内容、密钥或用户数据；实例键由 session id 消毒后生成（`[A-Za-z0-9_-]{1,128}`）。
- **已知限制**：`resume` 的钩子在会话发布后安装，恢复后的首步不受门控；本轮不做 token 级工具映射、saga 补偿与 PACT 原生工具注册（阶段 12 评估）；依赖 DSH 预发布 API，DSH 升级须复核。
- **实现状态**：已实现（2026-10-22，阶段 11），Node 22 内置测试 49 项通过；已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上完成真实装配验证（profile 安装、`--dump-config` 组合、启动后 `ctx.agentLoop` 为 `PactAgentLoop`）。已在真实 `deepseek-official/deepseek-flash` 模型会话中验证 `pact-*` 门控三条路径：允许（bash 执行并返回 `PACT_TOOL_OK`）、`denyTools:["bash"]`（工具结果 `Error: pact: tool denied by policy`）、`fakeEnabled=false`（`agent/pre-step` 拒绝，`turn/start` 后直接 `turn/end`，无模型请求）。**WASM 后端已 live（2026-10-23）**：以官方 `wasm-bindgen 0.2.127` 构建 `pact-wasm` Node 产物，经新增 `crates/pact-wasm/js/open-host.js` 组装 `PactHost + FileStore`，在 DSH headless profile 中复跑同一三条路径——允许（bash 实际执行，`isError:false`）、`denyTools` 拒绝、空 Counter 实例的 `agent/pre-step` 拒绝（仅 `turn/start → turn/end`），WASM 存储日志由插件运行时创建。**npm 分发（2026-10-23）**：`dsh-plugin-pact@0.1.2` 经 tag `v0.1.2` 由 `release.yml` OIDC 发布；全新 profile 以 `add dsh-plugin-pact` 解析为 `^0.1.2` 并启动，两条 patch 行均激活。

### FR26 显式状态交接与活动实例别名（D39）

- **定位**：在域双键（FR19）基础上提供「声明换版但不中断协作」的显式路径。该路径 = 冻结源实例 → 以新声明创建目标实例 → 迁移 Counter / Archive / Rule-E 幂等表 → 可选切换活动实例 → 源实例只读。不替代原位保留状态的声明换版，也不提供双活、流量分割或跨实例协同。
- **`Imported` 自足基线**：交接事件 `Imported` 携带完整 `CounterState`、幂等表、源域/源实例/源 `seq` 与目标声明原文；与 `DomainLoaded` 同为恢复基线（取最后一个），单行即可独立审计与重放（NFR2）。
- **事件 schema v2**：普通事件 `version = 1`；`Imported` 写 `version = 2`；读取上限 `SUPPORTED_EVENT_SCHEMA_VERSION = 2`，更高版本 **fail-fast**（`UnsupportedSchemaVersion`），不静默降级。
- **迁移范围与兼容性下界**：迁移 Counter、Archive 与幂等表；源 Counter 中所有 token 类型必须仍被新声明声明（Archive 可保留已删除类型，历史只读）。不满足 ⇒ `HandoffIncompatible`（409），交接失败且无副作用。
- **冻结**：交接成功后源实例持久只读（`<domain>/<instance>.sealed`）；拒绝新的 Seed / Produce / Consume，Rule-R 读取与幂等重放仍返回首次归档；冻结标记跨重启保持，可显式解冻。**卸载不解除冻结**：已冻结实例的重载/实例化在未显式 `force` 时返回 `InstanceFrozen`，`force` 重置才清除标记；交接目标若残留 `.sealed` 视为已存在并拒绝。
- **活动实例别名**：域级指针 `<data_dir>/<domain>/active.json`；短路由 `/domains/{d}/…` 解析活动实例，无指针回退 `default`；实例级路由 `/domains/{d}/instances/{i}/…` 始终显式。指针切换是域级运维动作，ACL 要求 `domain/*` 授权。
- **控制面认证与授权**：配置了任何身份（令牌或签名，至少其一）后，`handoff` / `freeze` / `activate` / `active` 必须可认证；信任声明模式（两者皆空）免身份。ACL 启用时，`handoff` 要求源实例键、目标实例键与域通配键 `domain/*`（交接会改写域级 `declaration.pact`）；`freeze` 与 `GET active` 要求解析到的实例键；`PUT active` 要求 `domain/*`。
- **控制面审计**：`.sealed` 与 `active.json` 记录 `ts`（注入时钟）与 `by`（已认证身份，信任声明模式为 `None`）；`GET /domains/{d}/active` 返回这两个字段。旧版 `.sealed`（纯文本）与无审计字段的 `active.json` 保持可读。
- **原子性与激活**：先冻结源，再追加 `Imported` 作为提交点；追加失败回滚冻结且不创建目标。活动指针切换为 best-effort，结果携带 `activated` 与 `active_instance`；指针失败不回滚已提交的交接。
- **恢复与卸载**：恢复以最后基线解释状态；活动指针悬空 ⇒ 启动 fail-fast；卸载活动实例时清除指针。
- **接口面**：REST `POST /domains/{d}/handoff`、`GET/PUT /domains/{d}/active`、`PUT /domains/{d}/instances/{i}/freeze`；CLI `pact domain handoff|activate|active|freeze`；MCP `pact_domain_handoff|activate|active|freeze`（工具总数 21）。
- **实现状态**：已实现（2026-10-25）。模型/内核/存储/引擎/HTTP/CLI/MCP/WASM 视图贯通；测试覆盖交接迁移、冻结只读与幂等重放、别名切换与回退、重启恢复、类型闭包失败无副作用、目标冲突与 ACL。

### FR27 物引静态检查命令（D41）

- **定位**：提供离线命令 `pact ref lint`，在进入运行前检查 RefBindings、ContractCard 与 `.pact` 声明的静态一致性。不解析具体 `ThingRef` 的存在性、不访问网络、不读写引擎状态。
- **输入**：`--bindings PATH`（可重复；文件或目录，目录展开 `.yaml`/`.yml`/`.json`）、`--contracts PATH`（可重复）、`--tombstones PATH`（可重复，D44）、`--declaration FILE`（可选 `.pact`）、`--name N`（期望域名；缺省取声明文件 stem）、`--strict`（警告视为失败）。
- **检查项**：绑定/契约/Tombstone 解析与规范校验；重复域、重复契约与重复 tombstone；绑定类型键与声明类型一致；契约引用可解析；绑定接受的 scheme 与契约声明的 scheme 一致；`required` 但无契约；Identifier 放行；未注册 scheme 放行；`declarationDigest` 缺失或不一致；契约状态（retired / deprecated）；未被引用的契约；归档钉版契约的 retention 声明；tombstone 的契约存在性、删除策略一致性与引用 scheme/长度；`required=true` 类型的可发现性（读取路径、视图暴露、保留证据与删除可区分性）。
- **输出与退出码**：`--json` 输出 `{bindings, contracts, tombstones, errors, warnings, findings[]}`，finding 含 `severity / code / path / message`；人类模式逐条打印并给出摘要。存在 error 退出 `1`，否则 `0`；`--strict` 下 warning 也退出 `1`。
- **边界**：不替代运行期 resolve/invoke；`declarationDigest` 按 `spec/ref/v1/declaration-digest.md` 的 v1 规范计算并比对（FR34）——缺失为 warning，不一致为 error；不检查实例的动态引用集合，不验证票面可达性。静态检查通过不代表运行期引用可用。
- **实现状态**：已实现（2026-10-27）。`pact-cli` 单元测试与端到端测试覆盖一致输入、未知类型、缺失/未使用契约、scheme 冲突、Identifier 放行、重复/退役契约、解析失败与退出码。

### FR28 物引可见性过滤（D42）

- **定位**：`GET /counter` 与 `GET /archive` 返回的 token 中，`thing_ref` 默认按 `read-gate` 过滤；token 的存在性、类型、状态与计数不被过滤。
- **主体**：调用者身份来自 Bearer / OIDC / Ed25519 签名 / 本地已验证主体。配置了令牌或签名时，匿名读取视图返回 `401`；信任声明模式无主体。
- **默认策略**：无显式 RefBindings 时全部走 `read-gate`；只有调用者对该类型有**无条件** `reads` 或 `consumes` 子句时才返回票面，`produces` 不授予可见性。条件子句暂不逐 token 求值，按不可见处理（保守）。
- **显式覆盖**：daemon `[ref_bindings].paths` 指向 RefBindings 文件或目录；`spec.*.ref.visibility` 按 `defaults → scheme → type → instance/type` 匹配决定 `domain`（直接可见）、`hidden`（永不返回）或 `read-gate`。
- **失败模式**：RefBindings 读取/解析/规范校验失败或同一域重复声明 → daemon 拒绝启动（fail-closed）。
- **边界**：只过滤 Counter/Archive 的 `thing_ref`；`GET /tokens/{id}` 仍由 Rule-R 的 read gate 授权。视图级读审计（D43）在 `read_audit = true` 时按视图/类型记录暴露计数，不记录票面值。
- **实现状态**：已实现（2026-10-27）。`pact-http` 单元测试与端到端测试覆盖 read-gate、domain、hidden、匿名 401、信任模式、Archive 与视图级审计（含 fail-closed）。

### FR29 引用保留与 tombstone（D44）

- **定位**：把「内容保留多久、删除时是否留下可发现证据」变成可静态校验的契约声明，覆盖 `docs/thingref-governance.md` 的保留阶段。PACT 引擎仍不 fetch / delete / 解析引用；tombstone 由域作者以正常 `seed` / `produce` 产生。
- **保留声明**：ContractCard `spec.lifecycle.retention` 只接受 `permanent` 或正的 ISO 8601 duration（`P[nY][nM][nW][nD][T[nH][nM][nS]]`，周不得与其他日期分量组合）；`deletionPolicy` 只接受 `retain` / `delete` / `tombstone-then-delete`。
- **tombstone 联动**：`tombstone-then-delete` 必须声明 `tombstoneRef`（URI），其余策略不得声明；归档钉版（`pinning.requiredForArchive`）的契约禁止使用 `delete`。保留期长度本身由域作者按下游最晚行动时间决定，PACT 不猜测。
- **Tombstone 记录**：`apiVersion: pact.ref/v1`、`kind: Tombstone`；`metadata.id` 为全局唯一 URI，`spec` 含 `contract`（`id@version`）、`ref`、`reason`（expired/replaced/revoked/compromised/legal/manual）、`deletedAt`（RFC3339 UTC `Z`）、可选 `retainUntil`（必须晚于 `deletedAt`）、可选 `replacedBy`（`reason=replaced` 时必填）与 `note`（≤2048 字节、无控制字符）。规范见 `spec/ref/v1/tombstone.schema.json`。
- **静态检查**：`pact ref lint --tombstones` 校验 Tombstone 解析/规范、重复 id、重复 (contract, ref)、契约存在性、删除策略一致性与引用 scheme/长度；归档钉版但未声明 `lifecycle` 的契约给出警告。
- **可发现性检查**：对 `required=true` 的绑定，静态验收下游是否真的能拿到票面与保留证据：`ref.discoverability.required_hidden`（effective `visibility=hidden`）、`view_read_path_missing`（read-gate 但声明中无无条件 `reads`/`consumes`）、`no_reader`（声明中无任何 `reads`/`consumes` 路径）、`retention_missing`（被要求的契约未声明 `lifecycle` 且非归档钉版）、`deletion_without_tombstone`（被要求的契约使用 `deletionPolicy=delete`）。全部为 warning，`--strict` 视为失败。
- **边界**：不校验内容是否真的已删除、tombstone 是否已发布或解析可达；不做运行期保留计时与自动删除；不改变引擎/存储/Archive 的既有语义。可发现性检查只证明静态路径存在，不保证运行期一定可读。
- **实现状态**：已实现（2026-10-27）。`pact-ref-spec` 规范校验、`pact-cli` 单元与端到端测试覆盖 retention 语法、策略组合、Tombstone 字段、交叉检查与可发现性五项检查。

### FR30 ThingRef 写入路径校验（D45/D50）

- **定位**：把 `pact-ref-spec` 的 URI profile 与 RefBindings 策略接到**写入路径**，让 `ThingRef` 在进入引擎前就被拒绝或规范化。校验位于 REST 写入边界（`pact-http` 的 `POST /domains/{d}[/instances/{i}]/seeds` 与 `.../tokens`）；引擎、存储与 `pact-core` 仍把 `ThingRef` 当作不透明字符串，不依赖治理 crate。
- **三态**：
  - 字段**缺失** → `400 ThingRefMissing`（“没说”不等于“确认没有”）；
  - 显式 `null` → `∅`，成功写入，但仍按绑定策略校验（`required=true` 时 `400 ThingRefRequired`）；
  - 字符串 → 有引用，按下列结构规则与绑定策略校验，成功时写入**规范化**值。
- **结构规则**（不依赖绑定）：空串或全空白、首尾空白、控制字符、裸路径（`/`、`./`、`../`、`~/`、`\`、`C:\` 等）、密码型 userinfo（authority 中 `user:pass@`）→ `400 InvalidThingRef`；纯用户名 userinfo（如 `git+ssh://git@example.com/...`）允许。长度先按硬上限 65536 字节拦截（RefBindings `maxLength` 不得声明超过该值）。
- **形态判定**：带合法 scheme（首字符字母、后续字母/数字/`+`/`-`/`.`、≤32）且 `:` 后有内容 → URI，scheme 规范化为小写后写入；否则按 Identifier 文法 `1*(ALPHA / DIGIT / "_" / "-" / "." / ":" / "/" / "@" / "#")` 校验，字符集外（空格、`?`、`%` 等）→ `400 InvalidThingRef`。
- **绑定策略**：daemon 已加载该域 RefBindings 时，按 `defaults → scheme → type → instance/type` 合并出规则并强制执行：
  - `required=true` 且值为 `∅` → `400 ThingRefRequired`；
  - Identifier 且 `allowIdentifier` 不为 `true` → `400 InvalidThingRef`（有绑定即默认只接受 URI，旧 Identifier 需显式放行）；
  - `schemes` 非空且不含 `*` 时，URI 的 scheme 必须在集合内；
  - `maxLength` 覆盖默认 2048 字节上限，超过 → `400 InvalidThingRef`。
  未加载该域绑定时只做结构规则，不做策略校验。
- **继承与边界**：CLI 与 MCP 经 REST client 调用，自动继承同一套校验；MCP 工具参数省略时 client 发送显式 `null`（等价 `∅`），无法表达“缺失”。`pact-runtime` 的进程内路径同样经 REST 继承。`pact-wasm` 本地宿主不经过 REST，但在 `prepare_seed` / `prepare_produce` 进入 kernel 前复用同一 `pact-ref-spec::validate_entry` 纯函数（D50）：宿主以 JSON 注入 RefBindings（`refBindings` 选项 / `PACT_WASM_REF_BINDINGS`），未注入时只做结构校验；JS/WASM 无法表达“字段缺失”，省略与 `null` 都映射为 `∅`，因此 WASM 路径不产生 `ThingRefMissing`。ContractCard 归档钉版与 Archive 检查仍只在 REST 边界。
- **未生效项**：`declarationDigest` 的语义比对由 FR34 在 `pact ref lint` 与 `POST /domains`（`[ref_digest] mode`）实现（不一致按 error / `400 DeclarationDigestMismatch`）；`enforce` 需显式开启，默认 `warn` 不阻塞。`onUnknownScheme`、`value`、`pinning` 已由 FR31 接入运行期，ContractCard 的 `artifact.digestPolicy` / `reference.pinning` 运行期加载由 FR32 实现，显式重载由 FR33 实现（2026-10-30）；见 `todo.md` P1。
- **错误码**：均为 `400`，沿用 §6.3 结构化错误体；`code` 为 `ThingRefMissing` / `ThingRefRequired` / `InvalidThingRef`，`message` 携带原始规范错误文本（不含票面全文以外的敏感信息，票面本身可能含位置信息，按调用方权限返回）。
- **实现状态**：已实现（2026-10-28；WASM 侧 2026-10-31，D50）。`pact-ref-spec` 修复 userinfo 与 Identifier 文法，并把入口语义收为 `validate_entry` 纯函数；`pact-http` 用 `ThingRefField` 保留三态后委托该函数；`pact-wasm` 在 `prepare_seed` / `prepare_produce` 前复用同一函数，宿主经 `set_ref_bindings` 注入绑定。单元测试覆盖三态、结构规则、`git+ssh`、Identifier 与绑定策略；端到端测试覆盖缺失/`null`、userinfo、裸路径、Identifier、超长、大写 scheme 规范化与绑定策略；WASM 单测与 Node 测试覆盖绑定注入、拒绝、规范化与运行期移除。

### FR31 引用形态、未知 scheme 与钉版的运行期强制（D46）

- **定位**：把 RefBindings 中已声明、但此前只在 `pact ref lint` 生效的三类策略接到运行期：`value`（期望形态）、`onUnknownScheme`（未知 scheme 处理）、`pinning`（钉版）。执行点仍在 REST 边界（D45/D46），引擎不感知票面。
- **`value`**：约束**非空**票面的形态——`none` 只允许 `∅`；`uri` 只允许 URI；`identifier` 只允许 Identifier；未声明不限制。`∅` 是否允许只由 `required` 决定。`required=true` 与 `value=none` 互斥，声明即校验失败。形态不符 → `400 InvalidThingRef`。
- **`onUnknownScheme`**：有效 `schemes` 非空且不含 `*` 时，scheme 不在白名单内视为“未知”；白名单为空或含 `*` 表示不限。
  - `reject`（缺省）：`400 InvalidThingRef`；
  - `allow`：放行该引用（仍执行长度与 `value`/钉版检查）；
  - `treat-as-identifier`：按 Identifier 文法重新分类，且必须有 `allowIdentifier: true`，否则 `400 InvalidThingRef`。
- **`pinning`**：
  - `required`：写入（seed/produce）时即要求钉版；
  - `required-at-archive`：写入不拦，但在 consume（进入 Archive 的唯一转换）前要求钉版；
  - `optional`（缺省）：不要求。
  钉版判定为 URI 携带内容摘要 `sha256:<64 位小写十六进制>`，接受 URL fragment `#sha256=<hex>` 与 OCI `@sha256:<hex>` 两种写法；`∅` 表示没有外部制品，不受钉版约束；Identifier 不是钉版。失败 → `400 ThingRefUnpinned`。
- **执行点**：`pinning: required` 与上述形态/未知 scheme 策略在 seed/produce 前判定；`required-at-archive` 在 `/consume` 进入引擎前读取待消费 token 的票面判定。consume 只读取继承出的 `pinning`，不重复执行写入期规则，避免历史票面因后续收紧而无法归档。
- **边界**：ContractCard 的 `artifact.digestPolicy: required-at-archive` 与 `reference.pinning.requiredForArchive` 已由 FR32 在运行期加载；FR33 已实现显式重载。`pact-wasm` 本地宿主在写入侧复用同一 `value` / `onUnknownScheme` / `pinning: required` 判定（D50），但不加载 ContractCard、不做 consume 前的 `required-at-archive` 检查。
- **实现状态**：已实现（2026-10-29）。单元测试覆盖未知 scheme 三分支、`value` 三态、写入钉版与归档钉版；端到端测试覆盖 `ThingRefUnpinned`（写入与 consume）及钉版引用正常归档。

### FR32 ContractCard 运行期加载与版本不可变（D47）

- **定位**：让 daemon 在启动时加载 ContractCard，使绑定已声明、lint 已检查的卡片在运行期可解析；引擎、存储与 `pact-core` 仍不读卡片，卡片校验与消费只在 REST 边界（D45/D46 同一层）。
- **加载**：daemon 配置新增可选 `[ref_contracts].paths`（文件或目录，可重复；目录展开 `.yaml`/`.yml`/`.json`，规则与 `[ref_bindings]` 一致）。每个文件必须能解析为 `ContractCard` 并通过 `ContractCard::validate`；按 `contract_id()`（`metadata.id@version`）建索引，与绑定的 `contract` 引用键一致。
- **按数据触发闭环**：任一 RefBindings 有效规则（合并继承后）声明了 `contract` ⇒ 必须配置 `[ref_contracts]` 且每个被引用的契约可解析，否则 **daemon 拒绝启动**（fail-closed）；完全没有 `contract` 引用的部署无需配置，行为不变。
- **同版本不可变**：同一 `contract_id()` 出现两次（无论内容是否相同）⇒ 拒绝启动；卡片内容变更必须提升 `metadata.version`。运行期加载与 `pact ref lint` 的重复契约检查语义一致。
- **有效钉版（只加严）**：consume 进入 Archive 前的钉版要求 = 绑定 `pinning` **并集** 卡片 `reference.pinning.requiredForArchive` 或 `artifact.digestPolicy: required-at-archive`；卡片不得放宽绑定。判定与错误码沿用 FR31（`400 ThingRefUnpinned`）。
- **未生效项**：`artifact.kind` / `mediaType` 检查（票面不携带 media type，当前无法判定）、`lifecycle` 的运行期计时/删除；均不在 FR32 范围。显式重载由 FR33 承接（已实现）。
- **失败语义**：加载失败、卡片规范校验失败、重复 `contract_id()`、绑定引用的契约缺失 ⇒ 启动失败，错误信息指名文件与契约；无部分加载。
- **实现状态**：已实现（2026-10-30，B0）。`pact-http` 单测覆盖加载/索引、按数据触发、重复 `id@version`、卡片钉版并集；`visibility_e2e` 覆盖卡片要求归档钉版时的 `ThingRefUnpinned` 与重复版本拒绝启动。显式重载见 FR33（已实现）。

### FR33 ContractCard 与 RefBindings 显式重载（D48）

- **定位**：提供卡片与绑定的**控制面显式激活**，替代“改卡片必须重启 daemon”。不做文件监听、不做 SIGHUP 隐式重载；域声明的 D27/D39 迁移机制不变，卡片换版不触发状态交接。
- **接口面**：REST `POST /ref/reload`，请求体 `{"dryRun": bool}`（缺省 `false`）；CLI `pact ref reload [--dry-run] [--json]`。**不提供 MCP 工具**，避免 agent 触发全局治理变更。
- **事务与原子性**：读取 `[ref_contracts]` 与 `[ref_bindings]` 配置源 → 解析并规范校验全部卡片与绑定 → 校验所有 `contract` 引用可解析 → 计算影响报告 → 写控制面审计 → 原子替换内存索引与规则。任一步失败保持旧状态并返回结构化错误；不做部分生效。并发重载单飞，冲突返回 `409 RefReloadBusy`。在途请求持旧快照完成，新请求使用新索引。
- **预检**：`dryRun=true` 完成全部校验与影响报告但不替换。影响报告按 `(domain, instance, type)` 统计“重载后因缺摘要而无法归档”的存量 Counter token 数，不输出票面值。
- **审计**：`<data_dir>/ref-reload.jsonl`（append+fsync）每次记录 `ts`、`by`（无身份为 `null`）、`dryRun`、`outcome`（`applied`/`rejected`）、`files[{path,sha256,kind}]`、卡片/绑定计数与可选 `error`。审计写失败 ⇒ 拒绝本次重载（不替换索引）。
- **认证与授权**：配置任何身份（令牌或签名）后必须认证；ACL 启用时，调用者须对当前与新的 RefBindings 索引中每个域持有 `{domain}/*`；索引为空时只需认证；信任声明模式放行匿名。
- **错误码**：解析/规范校验失败、重复 `contract_id()`、契约引用不可解析 ⇒ `400 RefReloadInvalid`；并发重载 ⇒ `409 RefReloadBusy`；ACL 越权沿用 `403`；审计写失败 ⇒ `500`。均为 §6.3 结构化错误体。
- **回溯性加严**：按 FR31/D46，归档钉版在 consume 时按当前有效规则判定；把绑定切到更严的卡片版本会让存量未钉版 token 不可归档。这是运维决策，由预检报告与发布说明承担，不改变判定时机；不做“只影响切换后 token”的豁免（那需要记录 token 产生时的规则版本，另立事项）。
- **明确不做**：文件监听自动热加载、自动回滚、按域灰度（一次重载对全部域生效）、把卡片/绑定写入事件日志或 `seq`。
- **实现状态**：已实现（2026-10-30，B1）。`pact-http` 单测覆盖审计 JSONL 与审计失败；e2e 覆盖 `dryRun` 影响报告、成功重载后新规则生效、校验/审计失败保持旧状态、ACL 覆盖当前与新域、令牌模式认证与信任模式匿名；CLI e2e 覆盖真实二进制 `pact ref reload --dry-run`。

### FR34 域声明语义接口摘要 `declarationDigest` v1（D49）

- **定位**：让 RefBindings 的 `metadata.declarationDigest` 从“只校验格式的死字段”变成可核对的声明版本证据。摘要对象取**语义接口**，规范化与 golden 向量冻结在 `spec/ref/v1/declaration-digest.md`；离线工具与运行期比对按 D49 两步落地。
- **摘要对象**：域名（`load_domain` 的 `name`；CLI 取 `--name`，缺省文件 stem，须匹配 `[A-Za-z0-9_-]{1,128}`）+ `types` + `preconditions` + `gates`（每个 `(participant, action, type)` 及其条件表达式）+ `seed_authority`。注释、空白、换行/缩进、段内顺序、文件名、实例名、分析结果与卡片/绑定内容都不参与。
- **规范化**：UTF-8 紧凑 JSON（无缩进、无尾换行），顶层固定 `{"kind":"pact.declaration.interface","v":1,"domain":…,"types":[…],"preconditions":[…],"gates":[…],"seed":[…]}`；集合按字符串字典序升序，`gates` 按 `(participant, action, type)` 升序（action 用 `read`/`consume`/`produce` 字符串键，不是枚举序）；条件用自定文法文本 `via(T)` / `exists(T)` / `count(T,>=,n)` / `count(T,=,n)` / `and(A,B)` / `or(A,B)` / `not(A)`，不使用 `serde` 派生表示。摘要 = `sha256:` + 规范化字节的小写十六进制 SHA-256。
- **第一步（已实现）**：新增离线 `pact ref digest <file.pact> [--name <domain>] [--json]`；`pact ref lint --declaration` 计算摘要并与绑定比对——缺失仍为 warning `ref.bindings.digest_missing`（`--strict` 失败），已记录且不一致为 error `ref.bindings.digest_mismatch`（退出码 1）。实现落在 `pact-analysis` 纯函数（CLI 与未来运行期共用），`pact-ref-spec` 保持零 `pact-core` 依赖；规范文件为 `spec/ref/v1/declaration-digest.md`。
- **第二步（已实现，2026-10-31）**：运行期 `[ref_digest] mode = off|warn|enforce`（默认 `warn`），仅在 `POST /domains`（`load_domain`）进入引擎前比对；绑定未记录摘要或该域无绑定时不比较；`warn` 写警告日志且加载成功，`enforce` 仅“摘要存在且不一致”返回 `400 DeclarationDigestMismatch`（`detail` 含 `domain` / `expected` / `actual` / `digestVersion`）；`off` 不比较；声明解析失败交由引擎返回既有 `Syntax` / `WellFormedness`，不按摘要拒绝。启动恢复不比较（既有实例持声明快照，D27），显式重载（D48）不复核已登记声明。
- **边界**：摘要是“绑定对应哪版声明”的证据，不证明票面可达、不验证卡片内容、不进入引擎/存储/事件日志；格式保持 `sha256:<64hex>`，v1 语义冻结，未来换义须新规范版本与新编号。
- **实现状态**：第一步与第二步均已实现（2026-10-31）。`pact-analysis` 单测覆盖格式/顺序不变、语义变化、规范化 JSON 形状、条件文本与 golden 向量；`pact-cli` 单测覆盖缺失/不一致；e2e 覆盖真实二进制 `ref digest`、`--name` 参与摘要、非法域名 `Config` 错误、不一致退出 1 与 `--strict`；`pact-http` 单测覆盖三模式与缺失/一致/不一致/解析失败分支，e2e 覆盖 `enforce` 拒绝且不留已加载域、`warn`/`off` 放行、未记录摘要与无绑定不比较。

### FR35 refd OIDC 身份验证（D51）

- **定位**：`refd` 作为 OIDC 资源服务器，只验证 IdP 签发的 JWT，不内置授权服务器、不做发现文档；`[auth] mode = "oidc"` 时启用。
- **配置**：`[auth] mode = "oidc"` 时必需 `[auth.oidc]`：`issuer`（必填、必须 `https://`、不得含 userinfo/fragment）、`audience`（缺省 `pact-ref`）、`jwks_uri` 与 `jwks_file` 恰好一个（后者为常规文件、≤ 1 MiB）、`algorithms`（缺省且上限 `RS256`/`ES256`/`EdDSA`）、`leeway_seconds`（缺省 60、上限 60）、`cache_ttl_seconds`（缺省 300、> 0）、`timeout_ms`（缺省 5000、> 0）。配置非法即拒绝启动（fail-closed）。
- **验签与声明**：白名单外算法（含 `none`、`HS*`）拒绝；`crit` 头非空拒绝；`kid` 须在 JWKS 中匹配，无 `kid` 时仅当唯一可用密钥才接受（避免算法混淆）；必须校验 `iss`/`aud`/`exp`/`nbf`（`leeway` 按配置），缺 `sub` 或 `sub` 为空拒绝。
- **身份与授权**：actor = 验证后的 `sub`；`scope` claim（空格分隔字符串或字符串数组，其他形态视为空）映射为端点 scope，与既有 ACL 两层判定；scope 不足 → `403`，不透露对象存在性。
- **JWKS 缓存**：有界缓存（≤ 64 密钥）按 TTL 复用；未知 `kid` 触发一次强制刷新，30s 内不重复刷新（限频窗口内复用当前缓存）；拉取、解析或空密钥集失败一律 `401`。
- **错误语义**：缺凭据、格式错误、验签失败、未知 `kid`、JWKS 不可达均返回 `401`，统一带 `WWW-Authenticate: Bearer realm="pact-ref"`，不区分失败原因；认证调用在阻塞线程池执行，避免 JWKS 拉取阻断异步运行时。
- **边界**：不做 discovery、令牌撤销/内省、`groups`/`roles` 到 ACL 的自动映射；`/readyz` 仍只反映审计 sink 健康，不探测 JWKS 可达性（JWKS 失败在请求期以 `401` 体现）。
- **实现状态**：已实现（2026-10-31，跨仓库 `pact-ref`）。`ref-auth` 单测 11 项覆盖有效/过期/`nbf`/`iss`/`aud`/`alg=none`/`HS256`/未知 `kid` 限频刷新/JWKS 失败/缺凭据；`refd` 集成测试 3 项覆盖 `200`、过期与错误 `aud`/`iss` 的 `401`（含 `WWW-Authenticate`）、无端点 scope 的 `403`；全量 99 passed / 0 failed，`fmt`/`clippy`/`cargo audit` 通过。

### FR36 refd 动态只读调用与结果物化（D52）

- **定位**：`refd` 新增 `POST /v1/invoke`，把 `mcp-read` 能力调用结果转换为新的静态引用，形成“动态只读 → resolve → witness token”的闭环。治理平面仍不写 PACT Counter/Archive，也不校验 PACT read gate。
- **授权**：端点 scope `ref:invoke`；对象 ACL 的 operation 固定为 `invoke`（与 `resolve:read`/`resolve:verify`/`describe` 并列，并集允许、默认拒绝）。任一层不满足返回 `403`，不区分“无权”与“不存在”。
- **请求**：`{domain, type, reference, operation, idempotencyKey, input?}`；`operation` 是能力侧操作名，不参与授权求值；`idempotencyKey` 为 `[A-Za-z0-9._:-]{1,128}`；未知字段拒绝（`400 InvalidRequest`）。
- **只读闸门**：`profile.capability.sideEffect` 必须为 `read-only`，否则 `403`；`reference` 必须指向卡片声明的服务标识（`<capability.server.id>/<tool-path>`），否则 `400 InvalidReference`。
- **调用**：`mcp` adapter 以 JSON-RPC 2.0 `tools/call` 调用上游；端点默认必须 `https://`，仅回环主机可经 `[[mcp_servers]].allow_plaintext` 显式放宽；每服务 `timeout_ms` ≤ 30000，`refd` 另有 30s 总上限。
- **物化**：成功结果按内容寻址写入 `[materialize].dir/<sha256>.json`（原子写），响应 `200 {status: "succeeded", resultRef, resultDigest}`；`resultRef` 为 `file://` 引用，可交给 `/v1/resolve` 与参与者 `seed`/`produce`。单次结果上限 `[materialize].max_bytes`（缺省 1 MiB，≤ 16 MiB）。
- **失败语义**：上游明确失败 → `502 UpstreamFailure`，审计 `status=failed`，响应不泄露上游细节；`refd` 不隐式重试、不轮询。
- **启动约束（fail-closed）**：存在只读能力 profile 时，其 `capability.server.id` 必须已登记于 `[[mcp_servers]]`，且必须配置 `[materialize]`；`bearer_token_env` 未设置或为空拒绝启动。
- **边界**：`mcp-mutating`（动态写阶段）、审计边车、witness/compensation 不在本需求；`refd` 不提供写操作执行路径。
- **实现状态**：已实现（2026-10-31，跨仓库 `pact-ref`）。`pact-ref-spec` 只读卡片校验 5 项；`refd` `POST /v1/invoke` 的 7 项集成测试覆盖成功闭环、只读闸门 `403`、ACL `403`、归属 `400`、上游 `502` 与物化/审计；`[materialize]` 与 `[[mcp_servers]]` 启动 fail-closed。

### FR37 refd 动态写调用与幂等台账（D53）

- **定位**：`refd` 新增 `POST /v1/mutate`，执行卡片声明为 `capability.sideEffect = mutating` 的能力；结果仍物化为静态引用，witness token 由参与者以正常 `seed`/`produce` 产生。治理平面仍不写 PACT Counter/Archive，也不校验 PACT gate。
- **授权（两层，独立于只读）**：端点 scope `ref:mutate`；对象 ACL operation `mutate`。`ref:invoke` 不蕴含 `ref:mutate`，`/v1/invoke` 与 `/v1/mutate` 互不接受对方的 `sideEffect`；任一层不满足返回 `403`。
- **`mcp-mutating` profile**：卡片必须声明 `capability.kind`、`server.id`、`tool.name`、`tool.inputSchemaDigest`、`sideEffect = mutating`、`idempotency {key, semantics}`、`result.materialization ∈ {required, optional}`、`auth.audience` 与非空 `failure`；缺失或不满足在启动与请求期均拒绝（fail-closed）。`failure` 中保留键 `compensation` 可给出补偿能力引用，但 `refd` 不解析、不自动触发。
- **请求**：`{domain, type, reference, operation, idempotencyKey, input?, pactTokenId?}`；`reference` 必须严格等于 `<capability.server.id>/<capability.tool.name>`；`operation` 必须等于 `capability.tool.name`；`pactTokenId` 仅作审计关联，`refd` 不校验其在 PACT 中的存在性；未知字段拒绝（`400 InvalidRequest`）。
- **幂等台账**：`[mutate].journal` 为 append-only JSONL，每条 attempt 先写 `intent`（键 + 请求摘要）fsync，再写 `result`（`succeeded` / `failed` / `in-doubt`）fsync；键作用域为 `(actor, domain, type, reference, operation, idempotencyKey)`。同键同摘要：`succeeded` 返回原结果（`replayed=true`），`failed` 重放原失败，`in-doubt` 返回 `409 IdempotencyInDoubt`；同键异摘要返回 `409 IdempotencyConflict`；台账达到 `max_entries` 返回 `503 JournalFull`。
- **崩溃语义**：`intent` 落盘后、`result` 落盘前崩溃（含上游超时或连接中断）→ 同键后续请求 `409 IdempotencyInDoubt`；`refd` 不自动重试、不猜测上游是否已执行、不提供自动对账；`result` 落盘后崩溃 → 同键可安全重放。
- **审计边车**：沿用 `ref-audit`，每次写 attempt 记 `intent` 与 `result` 两条，含 `operation = "mutate"`、`sideEffect = mutating`、`settlement`、`idempotencyKey`、`resultDigest`；不记录 `input`、上游响应正文、物化内容或 bearer 令牌。`intent` 写失败 → `500 AuditUnavailable` 且不触达上游；`result` 写失败 → `500 AuditUnavailable`，台账保留可核对结果。
- **失败语义**：上游明确失败 → `502 UpstreamFailure`（台账 `failed`，审计 `status=failed`）；超时或连接中断 → `504 UpstreamTimeout`（台账与审计 `in-doubt`）；物化超限或写入失败 → 不返回部分结果。
- **启动约束（fail-closed）**：存在 `mutating` 能力 profile 时，必须配置 `[mutate].journal`（父目录存在、可解析、条目数 ≤ `max_entries`）与 `[materialize]`，`capability.server.id` 必须登记于 `[[mcp_servers]]`；台账中同键异摘要或非法行拒绝启动。
- **边界**：不做自动重试、轮询、后台任务、跨服务路由、自动 saga/compensation 或结果缓存清理；`revoke`、可选注册表与 conformance runner 不在本需求。
- **实现状态**：已实现（2026-10-31）。规范层 `pact-ref-spec` 强制 `mcp-mutating` **卡片级**必填字段（`tool.inputSchemaDigest`、`idempotency {key, semantics}`、`result.materialization ∈ {required, optional}`、`auth.audience`、非空 `failure`），单测 6 项；`pact-ref` 已落地 `ref-core` `MutateRequest`/`Mutation`、`ref-auth` `ref:mutate` scope、`refd` 幂等台账（intent/result、重启恢复、半行截断、满额）、`POST /v1/mutate` handler（请求级严格归属与 `operation` 校验、两阶段审计、错误映射）与启动 fail-closed；台账单测 8 项、`refd` 集成测试 12 项。witness/compensation 可运行示例见 `pact-ref/examples/witness-compensation/`（`README.md` 两条路径 + `services/refd/tests/witness_compensation.rs` 4 项离线回归）；FR37 不要求自动补偿。

### FR38 refd 的 git / oci scheme adapter（D54）

- **定位**：`refd` 补齐 `git+https` 与 `oci` 两类 scheme 的只读解析，使 `/v1/resolve` 能对代码提交与容器镜像给出可钉版句柄；仍只做静态解析，不取回正文、不执行构建或拉取镜像。
- **`git+https` 解析**：引用形如 `git+https://<authority>/<path>[@<rev>]`，`rev` 缺省 `HEAD`。adapter 请求 smart HTTP 的 `info/refs?service=git-upload-pack`，按 pkt-line 解析引用通告，支持 40/64 位十六进制 commit、`refs/...` 全名、`refs/heads/<rev>`、`refs/tags/<rev>`（peeled `^{}` 优先）。解析成功返回 `handle = git+https://<authority>/<path>@<commit>`，`media_type` 留空由 profile 回填。
- **`git+ssh` 与 `git+file`**：`git+ssh` 在 v1 显式不支持，但 scheme 已注册，请求返回 `400`（`UnsupportedIdentifier`）而非 `UnknownScheme`；`git+file` 不实现。SSH 用户名（`git@`）属合法 authority，密码型 userinfo 一律拒绝。
- **`oci` 解析**：引用形如 `oci://<registry>/<repository>[:tag][@sha256:<hex>]`，缺省 tag `latest`。adapter 以 `HEAD /v2/<repository>/manifests/<tag|digest>` 携带 OCI/Docker manifest `Accept` 头取 `Docker-Content-Digest`，归一化为 `sha256:<64 位小写 hex>`；引用已钉版且与响应不一致返回 `409`（`DigestMismatch`）。成功返回 `handle = oci://<registry>/<repository>@<digest>` 与非空 `digest`。
- **git 摘要边界**：git commit 只写入 `handle`，不填入 `digest`——`uri-profile.md` §4 的 `digest` 固定为 `sha256:` 形式，commit 哈希不是内容摘要，混用会破坏钉版语义。
- **防护（fail-closed）**：主机 allowlist 非空且精确匹配（`host` 或 `host:port`，不含 scheme/userinfo/path）；拒绝控制字符、空白、`?`、`#`、userinfo 与 `.`/`..` 路径段；HTTP 不跟随重定向；单次总超时上限 30s；响应大小上限为 git 引用通告 2 MiB、OCI manifest 头 1 MiB；`http://` 仅在 `allow_plaintext` 显式开启且目标为回环主机时允许。
- **配置**：可选 `[adapter.git]`（`allowed_hosts`、`timeout_ms` 缺省 10000、`allow_plaintext`）与 `[adapter.oci]`（`registries`、`timeout_ms` 缺省 10000、`allow_plaintext`）；未配置的 scheme 不注册，请求返回 `UnknownScheme`。配置非法（allowlist 为空、条目含 scheme/userinfo/path、超时越界）拒绝启动。
- **错误语义**：`408`/`504` → `Timeout`；`404`/`410` → `NotFound`；`401`/`403` → `Unauthorized`；其他上游状态与传输失败 → `Upstream`；钉版不一致 → `DigestMismatch`；引用语法或 scheme 不支持 → `UnsupportedIdentifier`。
- **实现状态**：已实现（2026-10-31，跨仓库 `pact-ref`）。`adapters/git` 单测 8 项（引用解析、非法引用、通告解析、HEAD/分支/标签/commit 解析、未知 rev 与未列主机、状态映射、畸形通告、`git+ssh` 显式拒绝）；`adapters/oci` 单测 7 项（引用解析、非法引用、allowlist 无网络、假 registry 钉 handle、钉版不一致/缺 digest、状态映射、无 `Docker-Content-Digest` 回退钉版）；`refd` 配置单测 3 项；`pact-ref` 全量 **172 passed / 0 failed**，`fmt`/`clippy`/`cargo audit` 通过。

## 5. 非功能需求

- **NFR1 语义保真**：核心规则实现为纯函数；以论文中的执行轨迹（软件工程生命周期、医疗路径、ReAct 循环、§4.2 区分性场景 State A/B）作为黄金测试用例。
- **NFR2 审计完整性**：事件日志自身即完整审计记录，人类可直接阅读；每条事件自足（消费事件携带 token 全量快照与可选幂等键），不依赖同一域实例其他行。
- **NFR3 确定性**：时钟抽象注入（系统时钟/逻辑时钟），支持确定性测试与重放。
- **NFR4 部署**：单一静态二进制（daemon + CLI），零运行时依赖。
- **NFR5 身份模型**：采用信任声明（客户端自报参与者名）+ 可选的按参与者令牌认证；v2 增补 Ed25519 请求签名身份与 MCP OIDC 资源服务器身份。身份校验全部位于接口层，核心与引擎不感知认证方式。
- **NFR6 联网演进**：Counter 单权威架构不变，联网形态为 client/server（无分布式共识需求）；传输层（stdio/本地 HTTP/远程 TLS）与核心逻辑分离。**v2 已实现（2026-10-22）**：daemon 内 TLS 1.2/1.3 与可选 mTLS；Ed25519 请求签名身份（防重放、与 ACL 绑定）；MCP Streamable HTTP（daemon 内建 `/mcp`，stdio 薄封装保留）；MCP OIDC 资源服务器（IdP 发现/JWKS、JWT 校验、claim 映射、RFC 9728 元数据）。OIDC 仅作用于 `/mcp`，REST 仍为静态 Bearer 或签名；引擎、存储与核心语义零改动。
- **NFR7 可运维性**：生产形态须可被外部进程管理器托管（可探活、可优雅停机），运行状态可观测（指标），数据可备份恢复；事件日志的长期增长须有可控的压缩/保留手段。**多实例边界（D27）**：同一 `data_dir` 上并发承载多实例时须以独占锁（flock）保证单写者，禁止交错写同一日志；实例间不共享 Counter/Archive，跨实例协同不在范式保障范围内。

## 6. 范围外（显式排除）

- CPN 模型的图形布局美化、CPN Tools GUI 内交互式状态空间实验的自动化：生成器只负责**结构等价编码 + 可加载性**（FR20）；CPN Tools 内的探索/可视化操作不纳入仓库自动化与 CI。
- Archival Policy（论文 §2.2 可选第五注入点）：引擎不内置；域内可由持有 `consumes` gate 的系统参与者以正常 Rule-E 承担。
- 多机分布式 Counter、OR 分支、时间约束、概率行为（范式本身不可表达）。
- 分布式事务 / 2PC / 共识协议，以及由引擎执行的外部副作用：引擎不执行副作用（FR10/FR15）；单写者 actor + 事件日志已满足线性化与崩溃一致性，引入这些机制只增复杂度（决策 D23）。
- 示例接入 CI 回归：`examples/**/*.pact` 不由 cargo test 遍历校验；示例脚本为教学资产，其正确性由人工执行与录制轨迹对照保证（决策 D22）。
- WASM 浏览器本地运行时（OPFS/SharedWorker）、多用户 ACL/读审计、DSH 团队级原生运行时与 NFR6 v2 远端模式：WASM 本地分发形态只覆盖本地单用户 Node/桌面宿主（FR21）；DSH 集成只覆盖 observe/govern 与 `pact-*` preset 的治理型 loop 接管（FR25），不含团队级运行时；上述能力待触发条件满足后另立范围。
