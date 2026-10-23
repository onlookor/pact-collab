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

### FR15 消耗原子性、幂等与物化边界
- 提交契约：Rule-E 的提交点 = 事件日志 fsync；其后步骤（索引投影、快照、Terminated、通知）失败不回滚、不失败本次调用（决策 D23）。
- 原子域：GateHolds 检查（含 via/exists/count/not 读集）与「Counter 移除 + Archive 追加」在同一临界区完成，无 TOCTOU、无并发双消费。
- 幂等：consume 可选 `request_id`；同一域实例内重复请求返回首次归档结果，不重复消费；跨重启（由事件日志重建幂等表）仍生效；同键异 token → `IdempotencyConflict`。
- 边界：引擎不执行外部副作用，`consume` 成功 ≠ 外部动作成功；"产物未就位"应前移为 gate 准入条件（witness-token 模式）或后置为显式补偿（saga 式），**不得**表现为消费回滚。
- 文档化：上述契约与边界须写入 `docs/design.md`、`docs/manual.md` 并在 `examples/` 提供对应示例。

### FR16 生产运行层（agent 驱动运行时 + 外部副作用执行器）

引擎保持被动（FR11、决策 D5）：不编排、不拉起 agent。生产系统须在引擎之外补齐：

- **可复用的参与者驱动运行时**：给定参与者名与其门（`my_gates` / `enabled` / `subscribe`），实现「等待 → 决策 → 行动」自驱循环，并负责 N 个 agent 进程的启动与编排。当前该逻辑仅存在于 e2e 测试（`pact-mcp/tests/unattended_e2e.rs` 的 P1/P2/P3），无可复用交付物。
- **外部副作用执行器**：订阅 `Consumed` 事件（或经 outbox）真正落地外部动作；失败按 FR15 边界以追加补偿事实（saga）承担，不表现为消费回滚。
- **归属**：本仓库新增 crate（决策 D24）。引擎、存储、核心语义零改动。

### FR17 引擎生产化

- **事件日志压缩与 schema 版本化**：`pact-store` 提供快照之后的事件日志截断/轮转与保留策略；事件与线上格式补 `version` 字段与兼容契约（旧日志重放语义明确）。
  - **截断机制**：**前缀删除 + 旁路水位清单**——快照为提交点，`<instance>.logmeta.json` 记 `base_seq` 水位（先于日志重写），日志原子重写（tmp+fsync+rename）仅保留 `seq > base_seq` 行；水位单调不回退。
  - **快照自足与幂等存活**：压缩后日志可能不含 `DomainLoaded`，故快照须承载 `version` / `seq` / `dl_seq` / `declaration` / `state` / `consumed_requests`（**Rule-E 幂等表随快照持久化**，否则前缀内 `Consumed` 行被丢弃会破坏跨重启幂等）；恢复以快照为锚补尾部事件。
  - **数据安全优先**：日志已截断而快照缺失或不覆盖水位 ⇒ **fail-fast**（`SnapshotRequiredForCompactedLog`），不静默丢数据。
  - **版本兼容契约**：`EVENT_SCHEMA_VERSION = 1`；缺 `version` 的旧日志/旧快照视同 v1（向后兼容）；`version > 当前` ⇒ **fail-fast `UnsupportedSchemaVersion`**（不静默按 v1 重放）。
  - **触发方式**：运维显式接口（REST `POST .../compact`、CLI `pact compact`、MCP `pact_compact`）+ 可选启动自动压缩（`pact.toml` 的 `compact_on_start` / `compact_keep_last`；缺省关闭）。
  - **固有取舍（须文档化）**：压缩后被丢弃前缀不再可查——审计视图退化为「快照 + 保留尾部事件」，`events` 索引行被裁剪（`tokens` 投影行保留）；NFR2 的「日志即完整审计」在压缩后由「快照 + 保留尾部」共同承担。v1 不做自动周期性轮转。
- **运维接口**：daemon 提供 `GET /health`（存活）、`GET /ready`（就绪：引擎可应答）、`GET /metrics`（Prometheus 文本格式运行指标，含 uptime 与域实例计数）；可选 `pid_file`（缺省不写，含陈旧检测）；信号化优雅停机（SIGINT/SIGTERM → 停止接受新连接 → 有界等待在途 → 终态快照 → 清理）。运维端点**不校验令牌**（仅供本地/内网探活与抓取）。
- **域级授权与审计读视图**：接口层按域 ACL（阻止任意客户端 `load` / `unload` 任意域）；读审计（Rule-R 记录）为**可选开启**扩展，默认关闭（修订决策 D9 的默认值表述，见 D25）。
  - **ACL 粒度**：条目为 `domain/instance`，支持 `domain/*` 通配（实例键，FR19）；配置形如 `[auth.acl] principal = ["proj/*", "other/default"]`。
  - **ACL 范围**：启用后覆盖全部域作用域端点——生命周期（load / instantiate / unload / compact / `:analyze` / `:compose`）、四类动作（seed / produce / read / consume）、查询（counter / archive / gates / enabled / status / events）与读审计视图；`GET /domains` 按授权过滤（空域剔除）。无令牌 → `401`，越权 → `403`。**未配置 ACL 时保持既有行为**（生命周期与查询不绑定身份，向后兼容）。
  - **配置合法性**：ACL 非空而令牌表为空 ⇒ daemon 启动失败（身份不可判定）；每条目须为合法 `domain/instance` 或 `domain/*`。
  - **读审计**：开启 `read_audit = true` 后，每次成功的 Rule-R 读取在 `<data_dir>/<domain>/<instance>.read-audit.jsonl` 追加一条自足记录（`version/ts/domain/instance/participant/token_id`）并 fsync。**fail-closed**：审计写失败则本次读取以 `500` 拒绝，不允许「读成功但无审计」。审计流**不进入**事件日志 / `seq` / 重放 / Rule-T，不参与快照与压缩。
  - **只读视图**：REST `GET /domains/{d}[/instances/{i}]/read-audit?limit=N`（缺省 100，上限 1000，时间正序）返回 `{records:[...]}`。

### FR18 部署与分发产物

- 本仓库 `dist/` 目录交付：容器镜像（Dockerfile）、systemd unit、静态二进制（musl）打包脚本。
- 与 FR14/决策 D22 同精神：发行产物属交付资产，不接入 CI 的语义测试（`examples/**` 仍不被 cargo test 遍历）。

### FR19 域双键实例化（D27）

- **定义键 / 实例键分离**：同一域声明（定义键 `domain`）可并发承载多个 **Counter 实例**（实例键 `instance`），各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表。
- **实例为接口层一等键**：REST 路由、CLI、MCP、ACL、审计读视图、日志压缩、运维指标均以实例为键维度——原「以域名为唯一会话键」的语义收窄为「以实例为会话键」。
- **默认回退**：不指定实例 ⇒ 默认实例（`default`）；既有 `--domain` 调用与既有脚本语义不变（落到 `default` 实例）。
- **旧日志不兼容（用户裁决）**：双键布局下只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**，启动恢复时 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移，不静默丢数据。
- **实例键承载形态（用户裁决）**：实例名作为**路径段**承载（`/domains/{d}/instances/{i}/…`），`/domains/{d}` 语义等价于 `default` 实例；`DomainLoaded` 事件**不新增 `instance` 字段**（实例由文件路径承载）。
- **声明原文另存**：`declare` 时把声明原文写入 `<data_dir>/<domain>/declaration.pact`，`recover` 用它回填声明注册表（支持重启后新增实例）。
- **形式依据**：论文 rev3 §1.3「Counter instantiation and partitioning」——实例划分属**实现自由**，论文全部规则与性质**按实例成立**（Rule-T 各实例独立终止；§1.4.1 TokenId 唯一性作用域为「该 Counter」，实现以 ULID 加强为跨实例唯一，见 Q4 裁定）；跨实例协同不在范式范围内。
- **通用护栏（用户裁决）**：同一 `data_dir` 上 daemon 启动须获取独占 flock（`<data_dir>/pactd.lock`），第二实例启动即拒绝；对**非空实例**（`last_seq > DomainLoaded.seq`）的再加载默认拒绝（`DomainBusy`），须显式 `force` 才重置。
- **归属**：`pact-store` + `pact-engine` + 接口层（`pact-http` / `pact-cli` / `pact-mcp` / `pact-runtime`）；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动（形式语义不变）。
- **实现状态**：已实现（阶段 10，路径 β）。存储 `(domain, instance)` 双列 + 域子目录布局；引擎声明注册表（`declare` / `instantiate` 分离）；接口全链（REST 默认族 + 实例镜像族、CLI `--instance`/`--force`/`instantiate`/`instances`、MCP 16 工具含 `pact_domain_instantiate`、`pact-runtime` 实例透传）；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。

### FR20 CPN 编码生成（阶段 6，D12/D28）

- **目标**：把一份良构域声明离线编码为 CPN Tools 彩色 Petri 网模型（论文 §4.5/附录 B），供形式验证与可视化检查；**不进入**引擎、存储、接口运行时路径。
- **入口**：新 crate `pact-cpn`（lib）+ CLI 离线命令 `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；不依赖 daemon。
- **单实例作用域（G1/D27）**：一次生成只编码**一个 Counter 实例**，库所/迁移以实例名命名空间化，不跨实例共享；跨实例协同不进入 `N(D)`。
- **编码映射**：每类型一个库所 + 一个 Archive 库所；token 颜色 = `TokenId × Participant × ThingRef × PrecondRefs × Timestamp`；生产迁移对前驱使用 test arc（输入绑定 + 同变量输出，不消费）；读迁移自环；消费迁移消耗目标并输出到 Archive；种子迁移无输入弧。
- **条件编码**：`and`/`or`/`not` 先化 NNF 再化 DNF；`or` 展开为多条迁移；`not` 下推为 `count T <= k`，用互补库所编码（调用方须为涉及类型提供 `--bound T=N` 的**真实并发上界**）；`not via` 无法用计数互补表达，显式拒绝；条件恒假的迁移保留并置 `guard=false`；DNF 项数与 `count >= n` 的展开规模各设 1024 上限，超限显式失败。
- **输出与验收**：CPN Tools XML（`format='6'`）；生成结果须通过 CPN DTD 校验、XML 良构与结构断言（库所/迁移/弧计数、test arc、互补不变量、单实例前缀）。CPN Tools GUI 内的交互式加载与状态空间实验由人工验收，不纳入 CI（沙箱无 GUI）。
- **实现状态**：已实现（2026-10-21）；`pact-cpn` 11 项测试 + CLI 离线 e2e 覆盖结构与错误路径。

### FR21 WASM 本地分发形态（D31/D33）

- **定位**：WASM 构建是 `pactd` 之外的**可选本地分发形态**，供 Node/桌面宿主内嵌运行；**权威仍是原生引擎**，NFR4 的单一静态二进制形态不变，WASM 不替代 `pactd`、不承担服务端角色。
- **实现状态**：**已稳定（2026-10-21，D33）**。稳定化前置 (1) Q7 独立 CI job、(2) 金标覆盖扩展、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视均已完成；WASM 作为受支持的本地分发形态维护，但**不在 `dist/` 产物集内**，仍不替代 `pactd`。
- **能力范围（S2 已实现，2026-10-21）**：语义核心（解析/良构校验/静态分析/片段组合/CPN 投影）、内存多实例会话（`(domain, instance)` 键）、Rule-S/P/R/E 与 Rule-T、两阶段提交（`prepare → 宿主持久化 → commit` / `abort`）、事件日志 JSONL 导出、自足快照导出与 `restore`、查询视图；事件 schema、快照结构与磁盘布局与原生共用（`pact-store-model` / `pact-views`）。
- **宿主职责**：持久化（JSONL append + fsync、快照 tmp+rename）、崩溃恢复（日志优先、末尾半行截断）、单写者（lockfile）由宿主适配层承担；WASM 内核不假定拥有 fsync、flock、socket 或进程能力。Node 适配层已交付（`crates/pact-wasm/js/`）。
- **能力边界**：不含浏览器 OPFS/SharedWorker、多用户 ACL/读审计、MCP/HTTP 服务、TLS、turso 索引、日志压缩与 `pactd` 运维接口；DSH `govern` 与 NFR6 v2 无硬依赖（见范围外）。**安全边界为本地单用户**，不得视为多用户安全边界。
- **ABI 策略**：`pact-wasm/0.x`，全部方法使用 JSON 字符串；破坏性变更须提升 ABI 版本并记入 `docs/design-change.md`；版本探测入口为 `abi_version`。去除非实验性标记不改变 ABI 版本策略。
- **稳定化验收（已完成）**：(1) Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）——**已实现（2026-10-21，`ci.yml` 的 `wasm` job）**；(2) 金标覆盖扩展（多实例、组合域、条件分支、压缩后恢复）——**已实现（2026-10-21，`crates/pact-wasm/tests/parity.rs` 7 项金标）**；(3) `pact-engine-kernel` 抽取——**已实现（2026-10-21，原生 `pact-engine` 与 `pact-wasm` 共用实例会话语义，D32）**；(4) 推送/发布前宿主边界安全审视——**已完成（2026-10-21，无新增可利用发现）**。四项验收完成后按 D33 去除实验性标记。

### FR22 TLS 与 Ed25519 请求签名身份（NFR6 v2）

- **TLS（可选）**：daemon 在 TCP 监听上可选终止 TLS（rustls，TLS 1.2/1.3）；`[tls]` 提供 PEM 证书链与私钥，`client_ca` 存在时要求客户端证书（mTLS）；`listen_unix` 不受影响。内置客户端（CLI / `pact-mcp` / `pact-runtime`）以 `--https` 连接、以 `--tls-ca` 信任自定义 CA；内置客户端不携带客户端证书，mTLS 面向外部客户端。
- **签名身份（可选）**：`[auth.keys] 参与者 = base64(32 字节 Ed25519 公钥)` 非空即启用。客户端以 base64(32 字节) Ed25519 私钥种子对请求签名，携带 `x-pact-participant` / `x-pact-timestamp` / `x-pact-nonce` / `x-pact-signature`；规范化串为 `PACT-SIGNATURE-V1`，覆盖方法、路径+查询、请求体 SHA-256、时间戳、nonce、参与者。服务端校验时间窗 `±300s`、nonce 去重（TTL 300s、上限 8192）、签名与请求体（> 8 MiB 返回 413）；通过后把已验证参与者写入可信头 `x-pact-verified-principal`，并剥离客户端伪造的同名头。
- **身份绑定**：签名与 Bearer 可同时配置；一旦 `[auth.keys]` 非空，除豁免端点外的 REST 请求**必须携带有效签名**（Bearer 不能替代签名；两者同时出现时以签名身份为准）。请求体/查询中的 `participant` 必须与解析出的身份一致，否则 `401 Unauthorized`（不是 403）；ACL 照常生效。签名身份仅用于 REST（含 CLI/MCP stdio 转发），不用于 `/mcp` 的 OIDC 路径。
- **豁免与信任模式**：`/health` `/ready` `/metrics` `/mcp` 与 OIDC 元数据端点豁免验签；REST 未配置任何身份（无令牌、无密钥；OIDC 只影响 `/mcp`）时为信任声明模式，仅在单机/回环部署使用。
- **线上契约与错误码**：签名请求头四件套；可信头 `x-pact-verified-principal`；进程内能力头 `x-pact-internal-auth`（随机 32 字节，不落日志）；缺/错身份 `401`、participant 不匹配 `401`、ACL 越权 `403`、签名请求体超限 `413`、nonce 重放 `429`、IdP 不可达 `503`（OIDC，见 FR24）。

### FR23 MCP Streamable HTTP（NFR6 v2）

- daemon 在 `[mcp].enabled = true` 时于同一监听端口内建 `/mcp`（MCP Streamable HTTP），工具面与 stdio 完全一致（17 个 `pact_*` 工具）；`pact-mcp` 保留 stdio 形态，作为薄封装经 REST 接入 daemon。
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

- **定位**：在 PACT 仓库内交付 DSH（DeepSeek Harness）插件包 `plugins/dsh/`（`@pact/dsh-plugin`，零 npm 依赖），把 PACT 门控接入 DSH 的 agent step 与 tool 边界；**不重写** DSH 的 `ReactLoopAgent` 调度，也不接管模型调用、工具执行与对话式 ReAct。
- **两档运行**：`observe`（缺省）只镜像 `session/event` 到有界审计缓冲（可选 JSONL，目录 `0700`/文件 `0600`），不阻断任何动作；`govern` 在 `agent/pre-step` 与 `tools/pre-execute` 查询 PACT 门，未启用或策略拒绝时分别返回 `{kind:'reject'}` / `{kind:'deny'}`，后端异常时 fail-closed（`observe` 档 fail-open 并留痕）。
- **选择性工厂接管**：`cordis.patch.yml` 禁用原始 `agent-loop` 行，并插入使用同一 `agentLoop` 服务的 `pact-agent-loop` 行（`@pact/dsh-plugin/loop`）；DSH patch 不能改写行的 `name`，只能按 `id` 覆盖或禁用。该子类复用官方 `AgentLoop`，仅对 `agentPreset` 以 `pact-` 开头的 agent 组合 `setup` 并安装 agent 作用域钩子，其余 agent 原样委托，行为与官方一致。
- **后端可注入**：插件只依赖 `PactBackend` 契约（`enabled/gates/status/seed/produce/consume/close`），本轮提供内存 `FakePactBackend` 与惰性 `WasmPactBackend` 适配层（`openPactHost(options)`），WASM 产物未就绪时以 `BackendUnavailable` 显式失败；不引入守护进程、端口或网络依赖。WASM 适配把宿主的 `enabled`（Rule-W 可生产选项数组）收窄为布尔语义——非空即“当前可执行”，与插件契约一致。
- **审计边界**：只记录 `type/seq/工具名/call id`，**不记录**工具参数、结果内容、密钥或用户数据；实例键由 session id 消毒后生成（`[A-Za-z0-9_-]{1,128}`）。
- **已知限制**：`resume` 的钩子在会话发布后安装，恢复后的首步不受门控；本轮不做 token 级工具映射、saga 补偿与 PACT 原生工具注册（阶段 12 评估）；依赖 DSH 预发布 API，DSH 升级须复核。
- **实现状态**：已实现（2026-10-22，阶段 11），Node 22 内置测试 47 项通过；已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上完成真实装配验证（profile 安装、`--dump-config` 组合、启动后 `ctx.agentLoop` 为 `PactAgentLoop`）。已在真实 `deepseek-official/deepseek-flash` 模型会话中验证 `pact-*` 门控三条路径：允许（bash 执行并返回 `PACT_TOOL_OK`）、`denyTools:["bash"]`（工具结果 `Error: pact: tool denied by policy`）、`fakeEnabled=false`（`agent/pre-step` 拒绝，`turn/start` 后直接 `turn/end`，无模型请求）。**WASM 后端已 live（2026-10-23）**：以官方 `wasm-bindgen 0.2.127` 构建 `pact-wasm` Node 产物，经新增 `crates/pact-wasm/js/open-host.js` 组装 `PactHost + FileStore`，在 DSH headless profile 中复跑同一三条路径——允许（bash 实际执行，`isError:false`）、`denyTools` 拒绝、空 Counter 实例的 `agent/pre-step` 拒绝（仅 `turn/start → turn/end`），WASM 存储日志由插件运行时创建。

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
