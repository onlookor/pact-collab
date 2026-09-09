# PACT 实现设计

对应需求：`docs/requirements.md`。范式规范：`docs/pactv1_rev2.md`（章节引用均指该论文最新版）。本文件只保留当前设计，变更历史记入 `docs/design-change.md`。

## 0. 决策记录

| # | 决策 | 结论 | 依据 |
|---|------|------|------|
| D1 | 实现语言 | Rust；不设 Python 学术层 | `docs/pact工程落地语言选择.md` 论证成立；补充：零隐式动作等不变量可由所有权/借用系统编译期强制；agent 经 MCP 接入，引擎语言与 agent 框架语言解耦，无需 Python 胶水层 |
| D2 | 持久化载体 | JSONL 事件日志为真相源 + SQLite 派生索引；索引引擎采用 turso（原 Limbo，Rust 重写的 SQLite） | 用户选定；turso 评估见 D13 |
| D3 | v1 接口范围 | MCP + REST 并行交付 | 用户选定；两者共享引擎门面，增量成本低 |
| D4 | Rule-W 唤醒 | 订阅推送 + 轮询兜底 | 用户选定 |
| D5 | 无人值守模式 | 引擎被动 + agent 自驱 | 用户选定；保持引擎的范式中立性（零隐式动作） |
| D6 | 并发模型 | 单权威 actor：全部状态变更经一个串行化执行点 | 论文中 Counter 是唯一协调介质；GateHolds 检查与状态变更必须原子 |
| D7 | 域管理 | 一个引擎实例承载多个命名域会话 | 避免联网阶段多协作流到来时重构 |
| D8 | 时钟 | Clock trait 注入（SystemClock / LogicalClock） | 确定性测试与重放（NFR3） |
| D9 | 读事件持久化 | v1 不持久化 Rule-R（无状态变更，重放不需要） | 保持事件日志与状态变更一一对应；审计流扩展位保留 |
| D10 | TokenId | ULID（有序、唯一、可读） | 便于日志排序与人工检视 |
| D11 | 身份模型 | v1 信任声明 + 可选按参与者令牌；接口层校验，核心不感知 | NFR5，联网版只替换接口层认证实现 |
| D12 | CPN 生成器 | 延后为可选阶段 6 | 不阻塞核心价值交付 |
| D13 | SQLite 索引引擎 | 采用 turso，经 pact-store 内部 `IndexStore` trait 隔离 | turso 现状（2026-08）：BETA，官方自述未达 SQLite 级可靠性，但已在多家生产环境运行；本架构中 SQLite 仅是**可随时重建的派生索引**（真相源为 JSONL），beta 风险被完全对冲——最坏情况是删库重放重建。收益：纯 Rust 工具链（无 C 编译依赖，musl 静态编译顺滑）、async-first API 与 tokio 引擎契合、MVCC/`BEGIN CONCURRENT` 为联网版并发留余量。退路：若 turso 出现阻塞性问题，实现 `IndexStore` 的 rusqlite 后备并切换，仅动 pact-store 一个 crate |
| D14 | 语义收口 | 域良构校验、Rule-W 可满足性判定、Rule-T 终止判定均为 pact-core 纯函数；引擎层只编排不判定 | 三者均为范式语义（§1.4.4、Rule-W、Rule-T），若留在引擎层会造成语义分裂；同时供 analysis/compose/测试复用 |
| D15 | ~~Rule-T 语义~~（**已回滚**，2026-08-26） | ~~termination_holds 排除空前置组合（源类型生产）~~ | 论文 rev1（pactv1_rev1.md）以源生产约束原生解决该问题（Rule-P 前提 Precond(T) ≠ ∅ + 声明期拒绝）：含源类型 produce gate 的域直接 ill-formed，不存在"永不终止"的合法域。临时豁免失去存在意义，回滚并由 D17 取代 |
| D16 | parser 宽容度 | 忠于 §3.1 文法骨架（四段固定顺序、段落完整）；宽容：段内尾分号、preconditions/gates/seed_authority 空段（types 至少一项）；扩展："->" 等价 "→"、`//` 行注释、GateExpr 优先级 not > and > or（文法未规定结合性） | 严格拒绝合法语义的空段比接受更糟；扩展均为可用性增益且不改变可表达语义 |
| D17 | 源类型生产约束（论文原生语义，rev2 §1.4.4/§3.1 沿用） | 三层强制：(1) Rule-P 前提 `Precond(T) ≠ ∅`——源类型（无前置规则）不可生产，运行时返回 `RuleError::SourceTypeProductionDenied`；(2) 声明期良构校验 `DomainError::SourceTypeProduceGate`（§3.1 约束 6）；(3) `is_source_type` 查询函数 + A.5 静态分析独立复核。Rule-T 按论文 Rule-P-enabled 三条件定义实现（源类型天然排除）；enabled_productions 不返回源类型 | 论文 §1.4/§1.4.3/§3.1 的规范语义：源类型唯一入口是 SeedAuthority（入口唯一性定理），消除 D15 指出的"永不终止"异常于声明期；良构域上 termination_holds 与"Counter 为空"等价（终止论证）。影响：LIFECYCLE 黄金域更新（analyst 角色改为 seed authority、operator via 子句移除——论文 Part II §1.3） |
| D18 | compose C2 实践语义 | 共享类型前置一致性判定：**双方都声明规则且集合不同 → 冲突**；单侧声明合法（缺席方对 T 的前置无约束，合并取并） | 论文 §1.8 示例的规范语义：D₂ 共享 design 而不重复声明其前置（顺序组合正依赖此机制），若按 C2 字面（一侧 ∅ 一侧 {…} 即冲突）论文自身示例无法组合。C4 安全性不受影响：`produces T` 只可能来自视 T 为非源的良构片段（必有规则），规则并集只增不减 |
| D19 | 阶段 4 范围与接口接入方式 | pact-http **提前至阶段 4** 与 pact-cli、pact-mcp 同批交付；MCP 与 CLI 均不内嵌引擎，而是经 `pact_http::client`（REST over unix socket）接入同一 daemon；原规划的独立 `pact-server` bin 取消，daemon 由 `pact daemon start`（pact-cli 子命令）与 `pactd`（pact-http 裸 daemon bin）提供 | 用户决策。三接口共享单一 daemon 即单一 Counter 权威（D6），REST 成为唯一状态变更通道——接口等价性由构造保证（MCP/CLI 只是 REST 的形态转换），e2e 只需各走一遍 REST 即覆盖契约一致性；联网版（v2）只换传输与认证，接口层结构零改动 |
| D20 | CLI 参数风格与 MCP 传输范围 | CLI 域操作命令一律 `--domain` / `--participant` / `--token` / `--type` flag（不采用设计稿中 `pact enabled <participant>` 式位置参数简写）；连接参数 `--unix` / `--tcp` / `--bearer` 与输出开关 `--json` 为全局 flag；MCP v1 仅 stdio，Streamable HTTP 移交联网演进（v2） | flag 风格消除位置参数歧义、便于脚本化与 `--json` 组合；MCP 经 REST 接入（D19）后 Streamable HTTP 与 REST 同端口价值消失，随 v2 TLS/远程接入一并落地 |
| D21 | rev2 良构约束强制风格与 A.5 重构（用户选定推荐项） | (1) 空 LHS `{ } → T` 解析期语法拒绝（错误指名 Non-Empty LHS 约束），`validate_domain` 另设 `EmptyPrecondLhs` 声明期防御（覆盖程序化构造/组合出的 Domain）；(2) LHS 重复 `{A + A} → B` 在 parser `build_domain` 保留原始 `Vec<TypeName>` 序列时检测（`DuplicatePrecondLhs`）——`&Domain` 的 BTreeSet 会静默去重，此为唯一强制点；RHS 重复由 BTreeMap 键唯一构造性保证；(3) count 字面量 PosNat（`pos_nat()` 拒绝 0 与前导零，`ZeroCountLiteral` 二次防御）；(4) A.5 = `precondition_wf` 三元报告 `{well_formed, violations: Vec<DomainError>, warnings: Vec<WfWarning>}`，违规判定委托 `core::validate_domain`（单一判定源），Seed/Produce 重叠（§1.4.3 Advisory）为 `SeedProduceOverlap` 咨询性警告；聚合报告字段前置（附录 D：A.5 先于其余分析）；(5) `DomainError::UndeclaredType.referenced_by` 由 `&'static str` 改 `String`，使错误可内嵌进 serde 报告 | 解析期拒绝保持文法忠实；表示层（BTreeSet）无法承载的约束在序列仍在手时检测；A.5 委托避免判定逻辑分裂；前驱互异仅按 TokenId（Rule-P Note），类型级去重由 LHS Uniqueness 良构性免除 |

## 1. 总体结构

Cargo workspace，crate 划分（每个保持小而聚焦）：

```
crates/
  pact-core      # 六原语、Domain、GateExpr、SOS 规则（纯函数，零 IO）
  pact-parser    # .pact 域声明文法解析 -> pact_core::Domain（含良构校验）
  pact-analysis  # 六项静态分析（论文 Appendix A.1–A.6；依赖 pact-core + petgraph）
  pact-compose   # 片段并行组合 + 四条件一致性检查（论文 §1.8）
  pact-store     # 事件溯源：JSONL 追加日志、快照、turso 派生索引、重放
  pact-engine    # 引擎 actor：命令处理、订阅总线、Rule-T 检测、多域会话
  pact-mcp       # MCP server（rmcp 实现），工具经 pact-http client 映射到引擎命令
  pact-http      # axum REST + SSE，路由映射到引擎命令；含类型化 client 与 pactd bin（裸 daemon）
  pact-cli       # clap 命令行（daemon 控制 + 领域操作 + 离线分析），单二进制 `pact`
```

依赖方向（单向，禁止回指）：

```
parser / analysis / compose ──> core
store ──> core
engine ──> core, store
http ──> engine
mcp ──> http（client，经 REST 接入 daemon）
cli ──> http（在线命令经 client；daemon start 复用其运行时）
cli ──> parser / analysis / compose / core（离线命令，不经 daemon）
```

## 2. pact-core：核心语义（阶段 1，接口定义）

### 2.1 纯度约束（硬性清单）

pact-core 是范式的唯一权威语义实现，必须满足：

1. **无 IO**：不依赖任何 IO/async crate；不使用 `std::fs`、`std::net`、`std::process`、环境变量读取。
2. **无日志**：不使用 tracing/log 宏——日志是 IO 副作用，破坏可重放性。可观测性通过返回值（`Result`、报告结构）传递，由引擎层负责落日志。
3. **无时钟**：不读取系统时间。所有时间戳由调用方作为 `now: Timestamp` 参数传入（引擎侧注入 Clock，见 D8）。
4. **无随机**：不生成 TokenId。所有 id 由调用方作为参数传入（引擎侧注入 IdGen，见 D10）。
5. **无全局可变状态**：无 `static mut`、无 `thread_local!` 可变状态。
6. **依赖白名单**：`serde`（仅 derive，纯数据转换）为唯一非 std 依赖；`proptest`、`ulid`（仅测试辅助）只出现在 dev-dependencies。
7. **规则函数只读状态**：全部规则函数以 `&CounterState` 入参、以返回值（新 token / ArchiveEntry / 报告）出参，状态应用由调用方完成——签名层面强制"检查不改状态"。

审计结论（对照初版设计的修正）：
- Rule-T 终止判定、Rule-W 可满足性判定、域良构校验从引擎/parser 层**移入 core**（D14）；
- 增加显式 `Effect` 模型与 `CounterState::apply`，状态变更路径唯一且可测；
- `rule_s` 不读 CounterState（论文 Rule-S 前提不含 Counter 条件），签名相应省略。

### 2.2 数据类型

```rust
pub struct TokenId(pub String);          // ULID，newtype 防混用
pub struct TypeName(pub String);
pub struct ParticipantName(pub String);
pub struct ThingRef(pub Option<String>); // URI 或 ∅；对引擎不透明
pub type Timestamp = String;             // ISO-8601 UTC

pub struct Token {
    pub id: TokenId,
    pub ty: TypeName,
    pub thing_ref: ThingRef,
    pub produced_by: ParticipantName,
    pub depends_on: Vec<TokenId>,        // 直接前驱实例 id，生产时记录
    pub produced_at: Timestamp,
}

pub struct ArchiveEntry {
    pub token: Token,
    pub exhausted_by: ParticipantName,
    pub exhausted_at: Timestamp,
}

pub enum Action { Read, Consume, Produce }

pub enum CountOp { Ge, Eq }

pub enum GateExpr {
    Via(TypeName),
    Exists(TypeName),
    Count { ty: TypeName, op: CountOp, n: u64 },  // n 受 PosNat 约束（§1.7.3：n ≥ 1）
    And(Box<GateExpr>, Box<GateExpr>),
    Or(Box<GateExpr>, Box<GateExpr>),
    Not(Box<GateExpr>),
}

pub struct Domain {
    pub types: BTreeSet<TypeName>,
    /// rhs -> 前驱集合；良构保证每个 rhs 至多一条规则（§1.4.4）
    pub preconditions: BTreeMap<TypeName, BTreeSet<TypeName>>,
    pub gates: BTreeMap<(ParticipantName, Action, TypeName), Option<GateExpr>>,
    pub seed_authority: BTreeSet<(ParticipantName, TypeName)>,
}

pub struct CounterState {
    pub counter: BTreeMap<TokenId, Token>,
    pub archive: Vec<ArchiveEntry>,
}
```

`GateExpr` derive `PartialEq`——直接复用为组合时的"语法同一性"判定（§1.8 条件 3）。

### 2.3 域良构校验（D14，从 parser 移入）

```rust
pub enum DomainError {
    DuplicatePrecondRule { ty: TypeName },                       // §3.1 约束 1 RHS Uniqueness
    DuplicatePrecondLhs { ty: TypeName, lhs_ty: TypeName },      // §3.1 约束 2 LHS Uniqueness（`{A+A}→B`；仅 parser 可报，D21）
    EmptyPrecondLhs { ty: TypeName },                            // §3.1 约束 3 Non-Empty LHS（`{ }→T`；声明期防御）
    UndeclaredType { referenced_by: String, ty: TypeName },      // §3.1 约束 4 Type Closure
    ZeroCountLiteral { participant, action: Action, ty, count_ty }, // §3.1 约束 5 PosNat（程序化构造防御）
    ProduceWithCondition { participant: ParticipantName, ty: TypeName },
    SourceTypeProduceGate { participant: ParticipantName, ty: TypeName }, // §3.1 约束 6（D17）
    DuplicateGateClause { participant: ParticipantName, action: Action, ty: TypeName },
}

/// 源类型判定（论文 §1.4.3）：无前置规则（Precond = ∅，含规则缺席）即源类型，
/// 唯一入口为 SeedAuthority / Rule-S。
pub fn is_source_type(domain: &Domain, ty: &TypeName) -> bool;

/// 良构性检查（§1.4.4 + §3.1 六项约束 + 引用封闭性）。
/// 解析后、组合后、加载前调用；返回全部错误（非首错即停）。
/// 约束 2（LHS Uniqueness）在 &Domain 表示层由 BTreeSet 去重结构化排除，
/// 其强制点在 parser build_domain（D21）；约束 1（RHS）由 BTreeMap 键唯一保证。
pub fn validate_domain(domain: &Domain) -> Result<(), Vec<DomainError>>;
```

### 2.4 规则函数（纯函数）

```rust
pub enum RuleError {
    StaticGateDenied { participant, action: Action, ty: TypeName },
    GateConditionFailed { participant, action: Action, ty: TypeName, expr: GateExpr },
    PreconditionMismatch { ty, expected: BTreeSet<TypeName>, provided: BTreeSet<TypeName> },
    SourceTypeProductionDenied { participant, ty: TypeName }, // Rule-P 前提 Precond(T) ≠ ∅（D17）
    PredecessorNotOnCounter { token_id: TokenId },
    DuplicatePredecessor { token_id: TokenId },
    TokenNotOnCounter { token_id: TokenId },
    SeedAuthorityDenied { participant, ty: TypeName },
}

/// Rule-S（§1.4.3）：仅检查 (participant, ty) ∈ SeedAuthority；不查前置条件、不读 Counter。
pub fn rule_s(domain: &Domain, participant: &ParticipantName, ty: &TypeName,
              thing_ref: ThingRef, id: TokenId, now: Timestamp)
    -> Result<Token, RuleError>;

/// Rule-P（论文 rev1 §1.4）：检查静态 produce gate + Precond(T) ≠ ∅（源类型排除，D17）；
/// Precond(ty) 集合与提供的前驱类型集合相等（AND 语义）；
/// 前驱均在 Counter 上且互异。前驱不被消费——&CounterState 在类型层面保证只读。
pub fn rule_p(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              ty: &TypeName, predecessors: &[TokenId], thing_ref: ThingRef,
              id: TokenId, now: Timestamp)
    -> Result<Token, RuleError>;

/// Rule-R：token ∈ Counter；静态 read gate；GateHolds。无状态变更，返回 token 克隆。
pub fn rule_r(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              token_id: &TokenId)
    -> Result<Token, RuleError>;

/// Rule-E：检查同 Rule-R（action=Consume）；效果 = Counter 移除 + Archive 追加，无级联。
pub fn rule_e(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              token_id: &TokenId, now: Timestamp)
    -> Result<ArchiveEntry, RuleError>;
```

### 2.5 GateHolds 与 GateExpr 求值（§1.4.0 / §1.7.3）

```rust
pub enum GateFailure {
    StaticDenied,                                    // (p, action, type(t)) ∉ gates
    ConditionFailed { expr: GateExpr },              // cond(t, Counter) = false
}

/// GateHolds：静态 gate + 条件子句合取；仅作用于 Read/Consume。
/// cond 为 None 或无匹配子句 → 空真成立。
pub fn gate_holds(domain: &Domain, state: &CounterState,
                  participant: &ParticipantName, action: Action /* Read|Consume */,
                  token: &Token) -> Result<(), GateFailure>;

impl GateExpr {
    /// 纯函数求值。签名只暴露 (target token, CounterState)——
    /// 接口层面强制 §1.7.1 的搜索空间（当前 Counter + depends_on）与纯度约束。
    pub fn evaluate(&self, target: &Token, state: &CounterState) -> bool;
}
```

`via` 求值（溯源敏感，回归测试锚点，论文 §4.2 State A/B）：

```rust
GateExpr::Via(ta) => state.counter.values()
    .any(|w| w.ty == ta && target.depends_on.contains(&w.id))
// 两条件须同时成立：w 在 Counter 上 ∧ w.id ∈ t.depends_on（实例级，非类型级）
```

### 2.6 效果应用（纯）

```rust
pub enum Effect {
    Seeded(Token),
    Produced(Token),
    Consumed(ArchiveEntry),
}

impl CounterState {
    /// 状态变更唯一路径：apply 规则函数的成功返回值。
    /// 引擎与测试共用，保证重放与在线行为逐字节一致。
    pub fn apply(&mut self, effect: Effect);
}
```

### 2.7 Rule-W / Rule-T 判定（D14，纯函数）

```rust
/// Rule-W 服务：参与者在当前状态下可满足的全部 Rule-P 组合。
/// 对每个 produce gate 类型 T：源类型排除（Rule-P 前提 Precond ≠ ∅，D17），
/// 其余检查每种前驱类型是否各有一枚实例（贪心见证选取）。
/// 不枚举全部组合（组合爆炸），只报可满足性与一组见证。
pub struct ProductionOption {
    pub ty: TypeName,
    pub witness_predecessors: Vec<TokenId>, // 一组满足前置的前驱实例
}
pub fn enabled_productions(domain: &Domain, state: &CounterState,
                           participant: &ParticipantName) -> Vec<ProductionOption>;

/// Rule-T 判定（论文 §1.4）：Counter = ∅ ∧ ¬∃ p: p is Rule-P-enabled。
/// Rule-P-enabled 三条件：(1) Precond(T) ≠ ∅；(2) produce gate；(3) 全部前驱
/// 类型在 Counter 上各有一枚实例。Counter = ∅ 时条件 (3) 必然失败，
/// 故良构域上本判定等价于 Counter 为空（终止论证）。
pub fn termination_holds(domain: &Domain, state: &CounterState) -> bool;
```

## 3. pact-parser：域声明文法（阶段 1，已实现）

- 手写递归下降解析器（文法极小，不引入 parser 生成器依赖；`logos` 做词法）。
- 文法即论文 §3.1：`types` / `preconditions`（含 `{A + B} → C` 多前驱形式）/ `gates`（`reads|consumes T [via T_a | when GateExpr]`、`produces T`）/ `seed_authority`。
- `via` 糖形式解析时脱糖为 `when via T_a`（§1.7.4），AST 唯一。
- 宽容度与扩展遵循 D16（段序严格、types 非空；尾分号/空段宽容；`->`、`//` 注释、优先级 not > and > or 为扩展）。
- rev2 解析期拒绝（D21）：空 LHS `{ } → T`（指名 Non-Empty LHS 约束）、count 字面量非 PosNat（0 / 前导零，指名 §1.7.3 与 Normalization rule）；LHS 类型重复 `{A + A} → B` 在 build_domain 报 `DuplicatePrecondLhs`（良构层）。

```rust
pub struct Span { pub line: usize, pub column: usize }

pub struct SyntaxError { pub span: Span, pub message: String }  // 含期望符号

pub enum Problem {
    Syntax(Vec<SyntaxError>),
    WellFormedness(Vec<pact_core::DomainError>),  // 转调 core::validate_domain（D14）
}

/// 一步到位：文本 -> 良构 Domain。返回全部问题（非首错即停）。
pub fn load_domain(src: &str) -> Result<Domain, Problem>;

/// 仅语法层（供工具链分步报告：先语法后语义）。
pub fn parse_raw(src: &str) -> Result<RawDeclaration, Vec<SyntaxError>>;
```

## 4. pact-analysis：静态分析（阶段 2，已实现，2026-09-09 按 rev2 重构 A.5）

六项算法按论文 Appendix A（rev2 版）直译，输入 `&Domain`，输出结构化报告（非仅布尔）：

| 分析 | 输出 |
|------|------|
| A.1 可达性 | `ReachabilityReport { seed_types, reachable, unreachable }`；单目标查询 `is_reachable` |
| A.2 生产覆盖 | 无任何 producer 的**非源类型**清单（源类型经 SeedAuthority 进入，排除） |
| A.3 消费可达性 | 无任何 consumer 的类型清单（技术债/滞留信号，含源类型） |
| A.4 结构死锁 | 无种子入口且不可外部馈入的 SCC 清单（`DeadlockComponent`） |
| A.5 前置与源生产良构性 | `PreconditionWfReport { well_formed, violations: Vec<DomainError>, warnings: Vec<WfWarning> }` 三元输出（rev2 Appendix A.5）；违规判定委托 `core::validate_domain`（单一判定源，D21）；`SeedProduceOverlap`（§1.4.3 Advisory）为咨询性警告 |
| A.6 搁浅风险 | `StrandingRisk { ty_b, ty_a, via_holder, consumer, potential }` 四元组清单；无环精确判定，有环标记 `potential`（保守） |

A.5 是其余分析的前提（附录 D 顺序）：聚合报告 `AnalysisReport` 将 `precondition_wf` 置于首位（引擎 `analyze_loaded` 经 `load_domain` 已拒绝非良构域，线上报告该字段恒为良构——独立复核语义保留给直接持有 `&Domain` 的调用方）。

实现要点：
- 内部用 `petgraph`（Tarjan/Kosaraju SCC、拓扑排序判环），前置图以 `DiGraphMap<&TypeName, ()>` 构建（边 X → T 当且仅当 X ∈ Pre(T)）；
- A.4 的"外部馈入"判定 = SCC 直接入边的前驱是否在 A.1 种子可达闭包内；
- A.6 提取复合 GateExpr 中全部嵌套 `Via` 原子（非仅顶层）；`potential` 按全局拓扑有环性判定（论文按拓扑性质在精确/保守两算法间选择）；
- 全部输出按 BTreeSet/BTreeMap 序确定排序（可作黄金断言与线上 JSON）。

## 5. pact-compose：片段组合（阶段 2，已实现）

```rust
pub enum Conflict {
    PreconditionMismatch { ty, pre_a, pre_b },   // C1+C2
    GateConditionMismatch { participant, action, ty, cond_a, cond_b }, // C3
    IllFormed(DomainError),                       // C4 终检透传
}

/// 多片段并行组合：从空域（组合单位元）折叠二元组合链；
/// 任一步冲突返回该步全部冲突明细。
pub fn compose(fragments: &[Domain]) -> Result<Domain, Vec<Conflict>>;

/// 二元并行组合 D₁ ∥ D₂（论文 §1.8）：四条件检查 → 四元组集合并 → 良构终检。
pub fn compose_pair(a: &Domain, b: &Domain) -> Result<Domain, Vec<Conflict>>;
```

四条件（论文 rev1 §1.8）：C1 前置唯一性保持、C2 共享类型前置一致（**D18 实践语义**：双侧都声明且不同才冲突，单侧声明合法——论文示例规范语义）、C3 gate 条件语法同一（`GateExpr: PartialEq` 深度相等，`Option<GateExpr>` 的 None 与 None 同一）、C4 源生产封闭（良构片段 + C2 + C3 下自动保持，经 `validate_domain` 终检兜底）。C1 的跨域情形（同 rhs 双规则不同前驱）为 C2 冲突特例（rhs 必属双方 Types，良构引用封闭）。

## 6. 领域数据格式（线上格式，三接口共享）

全部接口（MCP 工具参数/返回、REST 载荷、CLI 的 `--json` 输出、SSE/通知事件）共用以下 JSON 格式。字段名 snake_case；时间戳 ISO-8601 UTC。

### 6.1 `.pact` 域声明文件（注入点载体，论文 §3.1 文法）

```pact
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, consumes implementation, produces test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
```

（论文 rev1 Part II §1：requirement 为源类型，"analyst 引出需求"建模为种子权威而非生产 gate——D17 源生产约束。）

### 6.2 Token 与 ArchiveEntry（JSON）

```json
{
  "id": "01J9ZQ5T8W3N2R4Y6A7B8C9D0E",
  "type": "design",
  "thing_ref": "file://artifacts/design-001.md",
  "produced_by": "architect",
  "depends_on": ["01J9ZQ3M..."],
  "produced_at": "2026-08-25T10:15:30Z"
}
```

ArchiveEntry = Token + `"exhausted_by"` + `"exhausted_at"`。

### 6.3 动作请求/响应

以 Rule-P 为例（其余动作同构）：

```json
// 请求：POST /domains/{d}/tokens  /  MCP pact_produce 参数
{ "participant": "developer",
  "type": "implementation",
  "predecessors": ["01J9ZQ…"],
  "thing_ref": "file://src/impl-001" }

// 成功响应：HTTP 201 / MCP 工具返回
{ "token": { …Token… } }

// 失败响应：HTTP 4xx / MCP 工具 is_error 返回
{ "error": { "code": "GateConditionFailed",
             "participant": "operator",
             "action": "read",
             "type": "test_report",
             "expr": "via implementation",
             "message": "见证 token 不在 Counter 上：依赖的 implementation 实例已被消费" } }
```

`error.code` = `RuleError` / `Problem` 变体名，机器可判；`message` 面向人类。拒绝动作一律整体失败，无部分效果。

### 6.4 Counter 变更通知事件（Rule-W 唤醒，SSE `data:` / MCP notification）

```json
{ "event": "Produced",          // Seeded | Produced | Consumed | Terminated
  "domain": "software-lifecycle",
  "seq": 42,
  "ts": "2026-08-25T10:15:31Z",
  "token_id": "01J9ZQ…",        // Terminated 时省略
  "summary": { "counter_size": 3 } }
```

### 6.5 查询响应

- `GET /domains/{d}/counter?type=` → `{"tokens": [Token…], "size": n}`
- `GET /domains/{d}/enabled?participant=` → `{"options": [{"type": "implementation", "witness_predecessors": ["01J…"]}] }`
- `GET /domains/{d}/status` → `{"counter_size": n, "archive_size": m, "terminated": false}`

## 7. pact-store：事件溯源存储（阶段 3）

### 7.1 事件类型（真相源）

```rust
pub enum Event {
    DomainLoaded { name: String, declaration: String }, // 原文存档，可复现解析
    Seeded    { seq: u64, ts: Timestamp, token: Token },
    Produced  { seq: u64, ts: Timestamp, token: Token },  // token 内含 depends_on/produced_by
    Consumed  { seq: u64, ts: Timestamp, token_id: TokenId, by: ParticipantName },
    Terminated{ seq: u64, ts: Timestamp },                // Rule-T 检测结果（信息性）
}
```

JSONL：每域一个 `<domain>.events.jsonl`，逐行 JSON 追加，`seq` 全局单调。

### 7.2 存储布局

```
<data_dir>/
  <domain>.events.jsonl      # 真相源，只追加
  <domain>.snapshot.json     # 定期快照（seq + CounterState），原子写（tmp+rename）
  index.tursodb              # 全部域共用：events 与 tokens 的查询索引
```

### 7.3 IndexStore trait（D13 隔离层）

```rust
/// 派生索引抽象。默认实现 TursoIndexStore（turso）；
/// 阻塞性问题出现时可实现 RusqliteIndexStore 替换，仅动本 crate。
#[async_trait]
pub trait IndexStore: Send {
    async fn open(path: &Path) -> Result<Self, StoreError>;
    async fn upsert_event(&self, ev: &Event, domain: &str) -> Result<(), StoreError>;
    async fn upsert_token(&self, t: &Token, domain: &str, status: TokenStatus) -> Result<(), StoreError>;
    async fn rebuild(&self, events: &[Event]) -> Result<(), StoreError>; // 从日志全量重建
}
```

索引 schema（turso 兼容 SQLite 方言）：

```sql
CREATE TABLE events(seq INTEGER PRIMARY KEY, domain TEXT, kind TEXT, ts TEXT, payload TEXT);
CREATE TABLE tokens(token_id TEXT PRIMARY KEY, domain TEXT, type TEXT, produced_by TEXT,
                    thing_ref TEXT, depends_on TEXT /*JSON数组*/, produced_at TEXT,
                    status TEXT /*counter|archived*/, exhausted_by TEXT, exhausted_at TEXT);
CREATE INDEX idx_tokens_domain_type ON tokens(domain, type, status);
```

写入路径（引擎临界区内）：追加 JSONL（fsync）→ 更新索引 → 更新内存态。启动路径：校验索引与日志末尾 seq 一致，不一致则 `rebuild`；加载最近快照 + 重放尾部事件恢复内存态。

### 7.4 崩溃一致性

- 快照写失败不影响正确性（重放兜底）；
- JSONL 半行写入（崩溃残留）→ 启动时检测并截断至最后一个完整行；
- 索引文件永远可丢弃重建——真相源唯一。

## 8. pact-engine：引擎 actor（阶段 3）

### 8.1 执行模型

tokio 任务 + mpsc 命令通道，独占持有全部域的 `CounterState`。每个命令处理 = 查找域 → 调用 pact-core 纯函数 → 成功则 `state.apply(effect)` + 持久化 + 广播通知。检查与应用在同一临界区内原子完成（D6）。引擎**只编排不判定**（D14）：GateHolds、Rule-W、Rule-T 的判定全部转调 core。

```rust
pub enum Command {
    LoadDomain { name, declaration, resp },       // 替换语义：同名额重置会话、清理快照、seq 续接
    Validate   { declaration, resp },             // 只校验不落盘
    Seed     { domain, participant, ty, thing_ref, resp },
    Produce  { domain, participant, ty, predecessors, thing_ref, resp },
    Read     { domain, participant, token_id, resp },
    Consume  { domain, participant, token_id, resp },
    QueryCounter { domain, filter, resp },
    QueryArchive { domain, filter, resp },
    QueryGates   { domain, participant, resp },
    QueryEnabled { domain, participant, resp },   // 转调 core::enabled_productions
    QueryStatus  { domain, resp },                // 含 core::termination_holds 结果
    ListDomains  { resp },
    Subscribe    { domain, resp },                // 返回 broadcast::Receiver<Notification>
    Analyze      { text, resp },                  // 六项静态分析聚合报告
    ComposeAndLoad { name, fragments, resp },     // 组合 + emit 回写 + 加载
    Shutdown     { resp },                        // 全部域终态快照后退出
}
```

启动恢复（actor 首要动作，失败则 fail-fast 排空后续命令）：逐域取**最后一次** DomainLoaded 解析声明；快照仅在 `snapshot.seq ≥ 该 DomainLoaded.seq` 时有效（防替换后旧快照复活）；重放尾部事件重建 CounterState 与 terminated 标记。Terminated 事件只在 false→true 转移时追加一次。

### 8.2 通知（Rule-W 唤醒，D4）

每域一个 `tokio::sync::broadcast` 通道，状态变更事件（Seeded/Produced/Consumed/Terminated）即时广播（格式见 §6.4）。MCP 端：工具调用返回 + 订阅通知（rmcp 支持 server notification）；REST 端：SSE `GET /domains/{d}/events`。轮询兜底 = `QueryEnabled` / `QueryStatus`。

### 8.3 Rule-T 检测

每次状态变更后惰性求值 `core::termination_holds` → 广播 `Terminated`。信息性事件，不锁定域（种子仍可注入）。

### 8.4 时钟与 id 生成（D8/D10）

`trait Clock { fn now(&self) -> Timestamp }` + `trait IdGen { fn next(&self) -> TokenId }`，由引擎构造时注入（生产: SystemClock + UlidGen；测试: LogicalClock + 顺序 id）。

## 9. 接口层（阶段 4，已实现）

三接口共享 §6 线上格式，能力对等；差异仅在传输与形态。接入方式（D19）：daemon 是唯一权威进程（`pact daemon start` 或 `pactd`，REST + SSE）；pact-mcp 与 pact-cli 在线命令均经 `pact_http::client` 走 REST 接入 daemon，不内嵌引擎。

### 9.1 pact-mcp（AI agent 主接口，已实现）

MCP server（rmcp），v1 传输仅 stdio（D20；Streamable HTTP 随 v2 联网接入落地）。`pact-mcp --unix <sock>`（或 `--tcp`）由 agent 拉起为子进程，工具经 REST 转发至 daemon。工具清单（前缀 `pact_`）：

| 工具 | 语义 |
|------|------|
| `pact_domain_validate` | 校验声明文本，返回良构/分析报告 |
| `pact_domain_load` | 加载/替换命名域（幂等按 name） |
| `pact_list_domains` | 已加载域清单 |
| `pact_analyze` | 六项静态分析聚合报告（A.5 `precondition_wf` 前置） |
| `pact_compose` | 组合多片段并加载 |
| `pact_seed` | Rule-S |
| `pact_produce` | Rule-P（显式传前驱 token id 列表） |
| `pact_read` | Rule-R（返回 token 全量数据） |
| `pact_consume` | Rule-E |
| `pact_counter_query` / `pact_archive_query` | 状态查询（按类型过滤） |
| `pact_my_gates` | 参与者的全部 gate 子句（含条件），agent 自省 |
| `pact_enabled` | Rule-W 轮询：当前可满足的 Rule-P 组合 |
| `pact_status` | Counter 概要 + terminated |
| `pact_subscribe` | 注册 Counter 变更通知（Rule-W 唤醒） |

工具的输入输出 schema 由类型派生（serde + schemars），错误以 §6.3 结构化 JSON 返回，agent 可编程处理（如 gate 被拒后转入等待）。

### 9.2 pact-http（应用程序接口，已实现）

axum，JSON over HTTP，监听 unix socket（默认，单机安全）与/或 127.0.0.1（可配）。载荷格式一律取 §6。实现注记：`{d}:analyze` / `{d}:compose` 因路由库不支持同段"参数+字面量"，在 `/domains/{d}` 统一入口内按段内冒号后缀分发（对外 URL 不变）；另提供独立 `POST /validate`（声明校验，不触域）与类型化 Rust client（pact-mcp / pact-cli 共用）。

```
POST   /domains                       {name, declaration}
GET    /domains                       域清单
GET    /domains/{d}                   域信息 + 状态摘要
DELETE /domains/{d}                   卸载（内存态，日志保留）
POST   /domains/{d}:analyze | :compose
POST   /domains/{d}/seeds             Rule-S
POST   /domains/{d}/tokens            Rule-P {participant, type, predecessors[], thing_ref}
GET    /domains/{d}/counter?type=
GET    /domains/{d}/archive?type=
POST   /domains/{d}/tokens/{id}/read     {participant}   Rule-R
POST   /domains/{d}/tokens/{id}/consume  {participant}   Rule-E
GET    /domains/{d}/gates/{participant}
GET    /domains/{d}/enabled?participant=
GET    /domains/{d}/status
GET    /domains/{d}/events            SSE 订阅（Rule-W 唤醒）
```

被拒动作返回 4xx + §6.3 结构化错误体（错误码 = RuleError 变体名）。

### 9.3 pact-cli（人类接口，已实现）

单二进制 `pact`（D20：域操作一律 `--domain` / `--participant` / `--token` / `--type` flag，不用位置参数简写）。全局 flag：`--unix <sock>`（缺省 ./pact.sock）/ `--tcp <addr>`（优先于 --unix）/ `--bearer <token>` / `--json`（§6 线上格式输出）。

- 注入点：`pact domain validate <file.pact>`；`pact domain load <file.pact> [--name N]`；`pact seed --domain D --participant P --type T [--thing-ref R]`。
- 领域操作：`pact produce --domain D --participant P --type T [--predecessors id1,id2] [--thing-ref R]`；`pact read|consume --domain D --participant P --token ID`；`pact counter ls --domain D [--type T]`；`pact archive ls --domain D [--type T]`；`pact gates show --domain D --participant P`；`pact enabled --domain D --participant P`；`pact status --domain D`。
- 域管理：`pact domain list|info --domain D|unload --domain D`。
- 离线能力（不经 daemon）：`pact analyze <file.pact>`、`pact compose f1.pact f2.pact ...`、`pact replay <domain>.events.jsonl`。
- daemon 控制：`pact daemon start --config pact.toml`（前台运行，与裸 `pactd` bin 等价）。
- 在线命令经 unix socket 调 REST API（复用 `pact_http::client`）；错误按 §6.3 结构化输出，exit code 非 0。

### 9.4 配置与身份（D11）

`pact.toml`：

```toml
data_dir = "./data"
listen_unix = "./pact.sock"        # 默认；listen_tcp 可选
[mcp] enabled = true
[auth]
# 可选：参与者令牌表；缺省 = 信任声明模式
[auth.tokens]
architect = "…"   # Authorization: Bearer
```

接口层在命令进入引擎前完成参与者身份校验；核心与引擎不感知认证方式（联网版只换此层）。

## 10. 测试策略（FR13，四层）

| 层次 | 范围 | 位置 |
|------|------|------|
| 单元测试 | core 规则函数/GateExpr 求值/validate_domain；parser 文法与错误报告；analysis/compose；store 读写 | 各 crate `#[cfg(test)]` |
| 属性测试 | Rule-R 后 Counter 逐字节不变；Rule-P 后前驱仍在 Counter；Rule-E 只影响目标 token 与 Archive；GateExpr 求值不改状态（纯度）；apply+重放等价 | pact-core proptest |
| 集成测试 | 引擎命令流（多域、并发命令串行化）、store 崩溃恢复（JSONL 半行截断、索引重建）、订阅通知时序 | crates/tests |
| 端到端 + 回归 | 黄金轨迹逐事件比对：软件工程生命周期 t₀–t₁₀、医疗路径 t₁–t₁₃（via 全部通过点）、ReAct 循环（循环拓扑 + Counter O(1)）、**§4.2 区分性场景 State A/B**（via 溯源敏感性回归锚点）；MCP 与 REST 契约等价；无人值守全流程 | 阶段 4/5 的 e2e 测试 |

## 11. 实现阶段

| 阶段 | 内容 | 验收 | 状态 |
|------|------|------|------|
| 1 | pact-core + pact-parser + 黄金轨迹/属性测试 | 论文全部示例轨迹通过；core 纯度约束清单（§2.1）逐条满足 | **已完成（2026-08-25，2026-08-26 按 rev1 更新）**：27 项测试全绿（11 单元含源类型约束专项 + 2 属性 × 512 用例 + 5 黄金 + 9 parser）；纯度核验（依赖树仅 serde、禁用 API 扫描零命中）；D15 已回滚、D17 生效（论文 rev1 源生产约束）；已知缺口：容器内无 rustfmt/clippy 组件（无 rustup/sudo），待环境补齐后补跑 |
| 2 | pact-analysis + pact-compose | 六项分析对论文域输出正确；组合一致性冲突可检出 | **已完成（2026-08-26）**：20 项测试全绿（analysis 12：四论文域验证 + 死锁/馈入/不可达/循环保守/间接祖先/嵌套 via 用例；compose 8：论文 §1.8 示例逐字段比对 + 四条件冲突各型 + 单位元/交换律/幂等合并）；D18 落定（C2 实践语义） |
| 3 | pact-store + pact-engine | 重放等价、崩溃恢复、多域会话 | **已完成（2026-08-26）**：新增 31 项测试全绿——store 17（事件 JSON 往返、Consumed 重放取全量、seq 逐行 +1、域名校验、索引滞后重建、快照原子写重启存活、日志清洗）；engine 单元 4（ISO-8601 纪元含闰日、逻辑时钟单调、顺序/ULID id）；集成 10（全生命周期 + 通知时序 seq/规模断言、快照路径与纯重放路径状态逐字段等价、崩溃半行截断后可续作、索引丢失重建、中部损坏 fail-fast、多域隔离、8 并发 seed 串行化、域替换恢复、validate/analyze/compose 命令、Clock/IdGen 确定性注入）。纯度核验：pact-core 依赖树仅 serde，IO/时钟/随机 API 扫描零命中。workspace 累计 80 项测试全绿 |
| 4 | pact-http + pact-cli + pact-mcp（stdio）（D19：pact-http 提前） | 人类与 agent 均可走完生命周期 | **已完成（2026-09-04）**：pact-http（axum daemon：unix socket 默认/TCP 可选、§6.3 类型化错误体、SSE 通知流、类型化 client、pactd bin）+ pact-mcp（rmcp stdio，15 个 `pact_` 工具，经 REST 接入，schemars 派生 schema）+ pact-cli（单二进制 `pact`：daemon start + 在线命令经 client + 离线 analyze/compose/replay + `--json`）。e2e 三绿：HTTP daemon 全生命周期（含 404/错误码映射、SSE、compose/unload）；CLI 真实子进程 daemon 全生命周期 + 错误路径 + 离线命令 + 事件日志回放对照；MCP 真实 stdio 子进程 15 工具全生命周期 + 3 错误路径。workspace 累计 95 项测试全绿。D19/D20 落定 |
| 5 | 无人值守场景验证（pact-http 已提前至阶段 4 交付） | 多 agent 经 MCP 自驱完成 ReAct/软件工程域，全程无介入至 Terminated；含 MCP Streamable HTTP 与否由联网演进决定（D20：v1 不做） | **已完成（2026-09-05；2026-09-09 修正场景 B 卫生断言）**：`pact-mcp/tests/unattended_e2e.rs` 单测试内顺序验证两场景，测试进程只做"外部启动机制"（拉起 daemon + N 个真实 `pact-mcp` stdio 子进程），此后零 Rule-S/P/R/E 介入。场景 A（ReAct）：user 播种 + agent 私有生产预算 7，自驱两轮循环至 Terminated，7 产/8 消/7 读、Counter 恒 ≤2（单 agent 串行，真不变量）、archive 序两轮同构；场景 B（软件工程生命周期）：external_stakeholder 播种 + 5 worker 并发自驱接力至 Terminated，4 产/5 消/4 读、角色分工与溯源链 requirement→…→deployment 逐项核验、Counter ≤ 类型数 5（结构上界；典型调度 ≤2，但 CI 慢调度下下一阶段可在上一阶段 produce→consume 窗口内合法生产，窗口因 produces 门无条件子句而语义上不可消除，2026-09-09 CI 实测 max_counter=3 后断言修正为结构上界）。agent 自驱策略 P1 生产优先/P2 汇消费/P3 等待（订阅推送 + 轮询兜底，Rule-W）。修复两类自驱竞态：①播种前过早终止（域加载与种子注入间 Counter=∅ 使 `termination_holds` 平凡为真，worker 误判退出）→ 仅当 `archive_size>0` 才承认终止（符合 §"Terminated 信息性、不锁定域"，修复落在 agent 驱动层非引擎）；②Terminated 一次性推送与轮询判定竞态 → 终止后 500ms 宽限窗口排空在途推送。合并为单 runtime 顺序执行以规避 tokio 多 runtime 子进程回收陷阱。D20 生效（v1 不做 Streamable HTTP） |
| 6（可选） | pact-cpn：CPN 编码生成（正片段直出，not 条件走互补库所编码） | CPN Tools 可加载验证 | 未开始 |
| 重构（2026-09-09） | 论文 rev2 对齐：良构约束六项化（新增 LHS Uniqueness / Non-Empty LHS / PosNat 解析期与声明期强制）、A.5 重构为 precondition_wf 三元报告（D21）、DomainError 自有 String | rev2 专项测试（`{A+A}→B` / `{ }→B` / count 0 / 前导零拒绝、自环接受、EmptyPrecondLhs / ZeroCountLiteral / SeedProduceOverlap 用例）+ 三接口 e2e 断言更新全绿 | **已完成**：workspace 103 项测试全绿（净增 6 项）；fmt/clippy 清零 |

## 12. 联网演进路径（NFR6）

v1 → v2 的变化被限制在接口层：TLS 终结、真实参与者凭据（签名密钥替代 bearer token）、MCP Streamable HTTP 远程接入。引擎、存储、核心语义零改动——因为 Counter 单权威不因联网改变（client/server，无共识）。turso 的 MVCC 并发写能力为 v2 多客户端并发查询提供余量。
