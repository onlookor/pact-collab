# PACT 实现需求

来源：`docs/pactv1_rev3.md`（范式论文最新版）+ 用户指示。本文件只保留当前有效需求，变更历史记入 `docs/requirements-change.md`。

## 1. 项目目标

实现论文提出的 PACT（Precondition-governed Autonomous Counter-based Token）范式的完整工程实现：一个可运行的单机协调引擎，忠实实现论文的全部形式语义（六原语、SOS 规则、GateHolds、GateExpr、静态分析、片段组合）。

## 2. 总体约束

- **语言**：Rust（决策依据见 `docs/pact工程落地语言选择.md` 及 `docs/design.md` 决策 D1，无反对意见）。
- **部署范围**：当前仅单机运行；架构设计必须为将来联网运行预留演进路径（见 NFR6）。
- **开发环境**：所有开发工作在 Trae 沙箱环境中进行。
- **无人值守**：整个协调工作从启动到完成（含 Rule-T 终止）可全程无人工介入。

## 3. 接口需求

按优先级排序：AI agent（主要）≈ 普通应用程序（主要）> 人类。

### FR1 AI Agent 接口（MCP 协议）
- 提供 MCP server，agent 以参与者身份接入，执行全部领域操作。
- 工具集覆盖：域加载/校验、Rule-S/P/R/E 四类动作、Counter/Archive 查询、静态分析、片段组合、参与者能力查询（`my_gates`）、Rule-P 可满足性查询（`enabled`，服务 Rule-W）、Counter 变更订阅（Rule-W 唤醒）、终止状态查询。

### FR2 应用程序接口（REST/JSON）
- 提供 REST API，与 MCP 共享同一引擎门面，能力对等（除订阅用 SSE 外）。

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

## 5. 非功能需求

- **NFR1 语义保真**：核心规则实现为纯函数；以论文中的执行轨迹（软件工程生命周期、医疗路径、ReAct 循环、§4.2 区分性场景 State A/B）作为黄金测试用例。
- **NFR2 审计完整性**：事件日志自身即完整审计记录，人类可直接阅读；每条事件自足（消费事件携带 token 全量快照与可选幂等键），不依赖同一域实例其他行。
- **NFR3 确定性**：时钟抽象注入（系统时钟/逻辑时钟），支持确定性测试与重放。
- **NFR4 部署**：单一静态二进制（daemon + CLI），零运行时依赖。
- **NFR5 身份模型**：v1 采用信任声明（客户端自报参与者名）+ 可选的按参与者令牌认证；为联网版真实认证预留抽象。
- **NFR6 联网演进**：Counter 单权威架构不变，联网形态为 client/server（无分布式共识需求）；传输层（stdio/本地 HTTP/远程 TLS）与核心逻辑分离。
- **NFR7 可运维性**：生产形态须可被外部进程管理器托管（可探活、可优雅停机），运行状态可观测（指标），数据可备份恢复；事件日志的长期增长须有可控的压缩/保留手段。**多实例边界（D27）**：同一 `data_dir` 上并发承载多实例时须以独占锁（flock）保证单写者，禁止交错写同一日志；实例间不共享 Counter/Archive，跨实例协同不在范式保障范围内。

## 6. 范围外（显式排除）

- CPN 编码生成器（论文 §4.5/附录 B）：列入后期可选阶段（决策 D12）。**实现约束（G1）**：须遵论文 §4.5/§B.2 的**单实例作用域**——每个 Counter 实例独立编码，库所与迁移不跨实例共享，跨实例协同不入 `N(D)`；CPN 编码的「域」为域声明，实例划分属运行期（D27）。
- Archival Policy（论文 §2.2 可选第五注入点）：引擎不内置；域内可由持有 `consumes` gate 的系统参与者以正常 Rule-E 承担。
- 多机分布式 Counter、OR 分支、时间约束、概率行为（范式本身不可表达）。
- 分布式事务 / 2PC / 共识协议，以及由引擎执行的外部副作用：引擎不执行副作用（FR10/FR15）；单写者 actor + 事件日志已满足线性化与崩溃一致性，引入这些机制只增复杂度（决策 D23）。
- 示例接入 CI 回归：`examples/**/*.pact` 不由 cargo test 遍历校验；示例脚本为教学资产，其正确性由人工执行与录制轨迹对照保证（决策 D22）。
