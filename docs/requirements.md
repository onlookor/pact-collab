# PACT 实现需求

来源：`docs/pactv1_rev2.md`（范式论文最新版）+ 用户指示。本文件只保留当前有效需求，变更历史记入 `docs/requirements-change.md`。

## 1. 项目目标

实现论文提出的 PACT（Precondition-governed Autonomous Counter-based Token）范式的完整工程实现：一个可运行的单机协调引擎，忠实实现论文的全部形式语义（六原语、SOS 规则、GateHolds、GateExpr、静态分析、片段组合）。

## 2. 总体约束

- **语言**：Rust（决策依据见 `docs/pact工程落地语言选择.md` 及 `docs/design.md` 决策 D1，无反对意见）。
- **部署范围**：当前仅单机运行；架构设计必须为将来联网运行预留演进路径（见 NFR6）。
- **开发环境**：所有开发工作在 distrobox 容器 `pact-dev` 中进行。
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
- 崩溃恢复：快照 + 日志尾部重放。

### FR10 引擎服务
- 单权威：所有状态变更经串行化执行点，GateHolds 检查与状态变更原子完成。
- 多域会话：一个引擎实例可承载多个命名域，各自独立 Counter/Archive。
- Rule-W 唤醒：订阅推送（状态变更事件主动通知订阅者）+ 轮询查询兜底。
- Rule-T 检测：Counter 为空且无任何参与者的 Rule-P 可满足时发出终止事件（信息性，不锁定域）。

### FR11 无人值守运行
- 引擎被动：不编排、不拉起 agent 进程；agent 由外部机制启动后通过 MCP 自驱接入，靠 Counter 状态自组织（决策 D5）。
- 提供从域加载 → 种子注入 → 协作推进 → 终止检测的完整无介入闭环所需全部协议能力。

### FR12 领域数据格式
- `.pact` 域声明文件为四个注入点的唯一载体，格式遵循论文 §3.1 文法。
- 定义线上数据格式（JSON）：Token、ArchiveEntry、规则动作请求/响应、结构化错误（错误码 + 载荷）、Counter 变更通知事件。
- 全部接口（MCP / REST / CLI 输出）共享同一套 JSON 线上格式，保证三接口行为可对照。
- 时间戳统一 ISO-8601 UTC；字段名 snake_case。

### FR13 测试组织
- 随变更组织四类测试：单元测试（各 crate 纯函数）、集成测试（store/engine 命令流）、端到端测试（MCP/REST/CLI 完整生命周期）、回归测试（论文 §4.2 区分性场景 State A/B 及全部黄金轨迹）。

## 5. 非功能需求

- **NFR1 语义保真**：核心规则实现为纯函数；以论文中的执行轨迹（软件工程生命周期、医疗路径、ReAct 循环、§4.2 区分性场景 State A/B）作为黄金测试用例。
- **NFR2 审计完整性**：事件日志自身即完整审计记录，人类可直接阅读。
- **NFR3 确定性**：时钟抽象注入（系统时钟/逻辑时钟），支持确定性测试与重放。
- **NFR4 部署**：单一静态二进制（daemon + CLI），零运行时依赖。
- **NFR5 身份模型**：v1 采用信任声明（客户端自报参与者名）+ 可选的按参与者令牌认证；为联网版真实认证预留抽象。
- **NFR6 联网演进**：Counter 单权威架构不变，联网形态为 client/server（无分布式共识需求）；传输层（stdio/本地 HTTP/远程 TLS）与核心逻辑分离。

## 6. 范围外（显式排除）

- CPN 编码生成器（论文 §4.5/附录 B）：列入后期可选阶段（决策 D12）。
- Archival Policy（论文 §2.2 可选第五注入点）：引擎不内置；域内可由持有 `consumes` gate 的系统参与者以正常 Rule-E 承担。
- 多机分布式 Counter、OR 分支、时间约束、概率行为（范式本身不可表达）。
