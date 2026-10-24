# PACT 表达力边界与编码方案

对应规范：`paper/pactv1_rev3.md`（下称“母本”）。本文件是**实现侧**对表达力边界的集中登记：
母本声明“不能直接表达什么”，本文件补充“在现有范式内通常怎么表达、代价是什么、哪些只能交给范式外”。
两者冲突时以母本为准；母本新增或修订边界声明后，必须同步本文件。

阅读顺序：先看 §1 的分类，再按编号查 §2 总表；需要落地时看 §3 的编码模式。

## 1. 分类

| 类别 | 含义 | 处置 |
|---|---|---|
| **可编码** | 母本语法/语义不能直接写出，但可用类型、前置图、GateExpr、SeedAuthority 组合出等价或近似结构 | 按 §3 模式编码，并在域声明处注明模式编号 |
| **仅可近似** | 能表达可观察结果，但不能保持原语义的全部性质（如原子性、严格时序、任意配对） | 必须显式登记丢失了哪条性质，不得声称等价 |
| **范式外** | 需要时钟、外部状态、事务、动态 schema 或无限值域，PACT 不承诺 | 交给参与者、外部系统或部署约定；PACT 只承接结果 token |
| **工具层** | 范式可表达，但某个编码/工具（如 CPN 生成器）不能无损承载 | 在工具入口报错或要求显式参数，不得静默生成错模型 |

## 2. 总表

### 2.1 过程形态

| ID | 边界 | 母本依据 | 可行表达方式 | 代价 / 限制 |
|---|---|---|---|---|
| B1 | 连续过程（信号、流量、进度） | §2.3 | 离散化为里程碑 token；连续量放 `ThingRef` 指向的外部系统 | 丢失区间内的连续变化，只能看到采样点 |
| B2 | 实时约束（超时、截止、时延） | §2.3、§1.3（`Timestamp` 无语义） | 截止逻辑放参与者/外部调度器；需要“到期”时由外部系统 seed 一个到期 token | PACT 不比较时间戳、不保证单调，不能靠 `Timestamp` 做超时判定 |
| B3 | 概率分支 | §2.3 | 随机决策留在参与者内部，Counter 只承接被选中的结果 token | 分支概率不可在域声明中表达，也不可静态审计 |
| B4 | OR 分支（A **或** B 都能触发 C） | §1.4.4、§3.1 约束 1（RHS Uniqueness） | 见 §3.1：分支各自成链并复制下游；或把选择内化到参与者；或按分支拆成多个域声明/实例；或把 A、B 统一成同一 `TypeName`（区别放 `ThingRef`） | **不能**用两条规则汇聚到同一 RHS（违反 RHS Uniqueness）。母本 §1.4.4 已明确：上述编码都不能在保留分支身份的同时汇聚到同一后继类型 |
| B5 | 产物内部结构（章节、字段、AST） | §1.3、§2.3 | 用 `ThingRef` 委派给外部系统 | PACT 不解释内容，不能做字段级 gate |
| B6 | 自动生命周期排序（自动推进） | §1.6、零隐式动作 | 参与者显式 produce/consume；需要自动清理时用可选 Archival Policy 或“清洁工”参与者 | 引擎不替域做隐式动作；Archival Policy 仍需经 Rule-E 执行 |
| B7 | 生产时的额外 gate 条件 | §1.5.2、§3.1 `produces` 无 `GateCondOpt` | 把条件表达进前置图（增加前驱类型），或增加一个见证/放行 token | 不能写 `produces T when …`；条件只能间接影响生产 |
| B8 | 两次消费之间的时序（先消费 A 才能消费 B） | §1.7.3 | 把时序编码为见证存活：`reads/consumes B via A`，让 A 在 B 被消费前一直留在 Counter；或用外部协调者发放许可 token | `via` 只能保证“产生 B 时用的那个 A 实例仍在”，不能表达“B 一定在 A 之后发生”；A 被消费后 B 会搁浅（§A.6） |
| B9 | 内部生产源类型 | §1.4、§1.4.3 | 给源类型指定 SeedAuthority，改由 Rule-S 注入；或把它改成有前驱的非源类型 | 源类型唯一入口是 Rule-S；声明期会拒绝 `produces` 源类型 |
| B10 | 同类型多实例生产（`{A + A} → B`） | §1.4.4 LHS Uniqueness | 见 §3.2：按角色拆成 `A_1`、`A_2` 等不同 `TypeName`；或让参与者在 read 侧用 `count A >= 2` 自检后产生协调 token；或做范式扩展 | 角色拆分只能表达**固定角色**，不能表达“任意两个 A”；`count` 只作用于 read/consume，不能直接约束生产 arity |
| B11 | Referent 敏感的生产（按 `ThingRef` 的版本/身份决定能否生产） | §1.4 Rule-P、§4.4 | 把版本/身份编码成不同 `TypeName`，由 SeedAuthority/种子方在注入时分类；或由外部校验者产生“已校验”见证 token 作为前驱 | 需要有限、预先声明的分类；无界版本流需要重声明/新实例。Rule-P 永远不读 `ThingRef` |
| B12 | Referent 敏感的读/消费 | §1.7.3、§4.4 | 见证 token 模式：外部系统校验后 seed/produce `readiness`/`approval` token，gate 写 `when exists …`；或把版本编成类型后写 `via` | GateExpr 没有 referent 级原子；仍受有限分类限制 |
| B13 | 生产时的否定/上下文条件（`when not exists T`） | §1.5.2 | 引入“放行/清障”前驱类型（由参与者维护）；或把否定条件后移到 read/consume gate | 前置图只有 AND，没有 NOT；清障 token 的存在性由域约定维护，不是结构保证 |
| B14 | 逻辑对象演进的同一性（新 token 是旧对象的新版本） | §1.3、§1.4.4 自环注 | 显式版本类型（`T_v1`、`T_v2`）；或增加 lineage/版本 token；`depends_on` 只记录实例链 | PACT 只有 token 实例 provenance，没有“同一逻辑对象”概念；对象同一性由域解释 `ThingRef`/`depends_on` |
| B15 | 原子多输出（一次动作同时产生多个 token） | §4.3 | 拆成两次 Rule-P + 中间类型；或用复合类型 token 承载多个引用；或用外部事务包装器暂存后统一提交 | **不能保持原子性**：中间状态可见。复合类型会失去对子输出的独立访问。下游可用 `exists` 门控隐藏中间态，但仍不是单步原子转移 |

### 2.2 结构与运行

| ID | 边界 | 母本依据 | 可行表达方式 | 代价 / 限制 |
|---|---|---|---|---|
| B16 | 动态 schema / 动态参与者 / 动态 gate 矩阵 | §2.2、§3.1（静态四注入点） | 预先声明超集；或为新结构创建新域声明/新实例 | 运行时不能新增类型、改前驱或改授权；变更要走重新声明 |
| B17 | 资源竞争 / 容量约束 | §1.4 Rule-P（生产不消费前驱） | 把资源建模为可消费 token：`consume R` 表示占用，释放时重新 seed/produce 一个 `R`；或交给外部锁服务 | 释放产生的是新 `TokenId`，资源谱系不连续，需域约定解释；生产本身不持有资源 |
| B18 | 无界循环 / 终止保证 | §1.4.2、§4.6、附录 A.4 | 循环类型用 SeedAuthority 点火；增加预算/计数器 token，用 gate 阈值或消费预算控制迭代 | PACT 不保证终止；Rule-T 只在 Counter 空且无参与者可生产时触发；静态分析只能报结构死锁/搁浅 |
| B19 | 内容寻址选择（无限值域） | §4.4（Linda 分离） | 把有限取值集编成不同 `TypeName`；或由外部选择器产生一个结果 token | 无限值域不可表达；类型枚举必须有限且静态声明 |
| B20 | Archive 的祖先/血缘查询 | §1.6 | Archive 扁平记录 + 外部物化 provenance index；或递归遍历 `depends_on` | 无 O(1) 祖先查询；跨 token 谱系需外部索引 |
| B21 | 上游必须晚于下游存活 / 无悬空引用 | §1.6、§A.6 | 用 `via` 见证把上游钉在 Counter 上；先消费下游再消费上游；用 A.6 静态分析预警搁浅 | 范式不保证；上游被消费后依赖它的 token 会搁浅 |
| B22 | 自动清理 / 无 token 累积 / 完整归档 | §1.6 | 显式消费策略；可选 Archival Policy；或“清洁工”参与者周期消费 | 范式不保证清理；未消费 token 留在 Counter |

### 2.3 工具层（非范式边界）

| ID | 边界 | 依据 | 处置 |
|---|---|---|---|
| B23 | CPN 的 `not via` 无法用互补库所编码 | `docs/design.md` §15、D28 | 生成器显式报 `NegatedVia`，不生成错模型；需要负向见证时改用正向见证或外部检查 |
| B24 | CPN 的 `not`/`count =` 需要类型上界 | `docs/design.md` §15、D28 | 调用方显式给 `--bound T=N`；上界不可从声明推导，正确性由调用方承担 |
| B25 | CPN 工具内状态空间实验 | `docs/design.md` §15 | CI 只保证 XML/DTD 与结构断言；GUI 加载与状态空间实验人工验收 |

## 3. 编码模式

### 3.1 OR 分支（B4）

在 RHS Uniqueness 下，**不能**写 `A → C` 与 `B → C` 两条规则。可选做法：

1. **分支各自成链 + 复制下游**（结构最保守）
   ```text
   A → C_A;  C_A → D_A
   B → C_B;  C_B → D_B
   ```
   两条链互不干扰；如果下游需要共同结果，只能在域外合并，或按第 3 种做法拆域。
2. **把选择内化到参与者**
   参与者持有 `produces C`，在内部决定走哪条路；`reads A when exists A or exists B` 只约束读，不约束生产。
   代价：生产合法性不再由结构强制，依赖参与者纪律。
3. **拆成多个域声明 / 多实例**
   每个分支一份声明，按部署选择实例；适合分支在部署期就固定的场景。
4. **统一前驱类型**
   若 A、B 可以共用一个 `TypeName`，则只写一条 `C` 的规则；分支标签放 `ThingRef`。
   代价：类型系统不再区分分支，`via`/`count` 也看不到分支差异。
5. **互斥准入**（两条分支流都已建模时）
   用 GateExpr 的否定条件在 read 侧互斥，例如 `reads selected_entry when not exists adr_package`，
   保证同一时刻只有一条分支被推进。这是**准入侧互斥**，不是生产侧 OR；它不改变两条流各自的前驱结构。

> GateExpr 自身的 `or` 只能用于 read/consume 准入（如 `when exists A or exists B`），不能把两条生产规则合并成一条。不要把它误当成生产 OR。
>
> 说明：母本 §1.4.4 已明确 OR 不能原生表达，并列出“分支各自成链 / 统一类型 / 参与者内化”三类编码；任何编码都不能在保留分支身份的同时汇聚到同一后继类型。若确实需要这种汇聚，那是范式扩展候选（例如放宽 RHS Uniqueness 或引入 OR 前驱），当前不在语法内。

### 3.2 同类型多实例（B10）

1. **固定角色拆分**（可静态验证）
   ```text
   preconditions { A_primary + A_replica → B }
   seed_authority { intake: seeds A_primary; intake: seeds A_replica }
   ```
   由 SeedAuthority/种子方把实例分类到两个角色。注意：B 只能有一条前置规则（RHS Uniqueness），所以角色类型必须同时出现在这一条规则的 LHS 中。
   限制：只能表达“一个主、一个备”这类固定角色，不能表达“任意两个 A”。
2. **参与者侧计数 + 协调 token**
   `coordinator: reads A when count A >= 2, produces pair_ready`；`B` 的前驱含 `pair_ready`。
   限制：`count` 检查发生在 read，不在 produce；协调者必须可信，结构不强制它真的等到两个。
3. **范式扩展**（候选，不在当前版本）：允许 LHS 多重集，或允许 `produces` 带条件。二者都会改变 §1.4.4 的良构约束，需单独评估。

> 母本附录 A.5 的 `LHS_DUPLICATE` remediation 已与本小节对齐：多前驱只能按角色拆成不同 `TypeName`，`count T >= n` 不能表达生产 arity。

### 3.3 Referent 敏感条件（B11/B12）

1. **版本/身份编成类型**
   ```text
   preconditions { requirement_v2 → design }
   seed_authority { intake: seeds requirement_v2 }
   ```
   由 intake 在注入时判定版本并选择类型。
   限制：版本集合必须有限且预先声明；无界版本流需重声明或新实例。
2. **见证 token 模式**（读/消费侧最常用）
   外部校验者校验 referent 后 seed/produce `readiness` 或 `approval` token；目标动作写 `when exists readiness` 或 `via approval`。
   限制：`readiness` 的语义由域约定；PACT 不知道它对应哪个 referent 版本，除非版本也编进类型名。
3. **不要在 Rule-P/GateExpr 里读 `ThingRef`**：母本明确 `ThingRef` 对范式 opaque（§1.3、§4.4）。

### 3.4 时序与存活（B8/B21）

- `via T_a` = “产生 T_b 时用的那个 T_a 实例现在还在 Counter 上”，不是“T_a 发生在 T_b 之前”。
- 要让上游“钉住”，就别在 T_b 消费前消费 T_a；A.6 会报搁浅风险。
- 严格顺序（B 必须在 A 之后）只能靠外部协调者发放许可 token，或把顺序编码成新的类型链。
- 不要用 `Timestamp` 做顺序判断（B2）。

### 3.5 资源与循环（B17/B18）

- 资源 = 可消费 token；占用 = `consume`，释放 = 重新 seed/produce。释放后的 `TokenId` 是新的，资源谱系需域解释。
- 预算/计数器 = 普通 token + gate 阈值；每次迭代消费或递减预算；预算耗尽后自然无法继续。
- 循环点火 = SeedAuthority；纯循环域没有源类型，但必须有 seed 才能启动（母本 §1.4.3）。

### 3.6 外部编排（B15/B19/B20/B22）

- 原子多输出、无限值域选择、O(1) 血缘、自动清理都不该在 PACT 内伪造。
- 由外部系统暂存/提交、选择、建索引、定时消费；PACT 只记录结果 token 与 `depends_on`。
- 外部动作成功与 `consume` 成功是两个独立承诺（设计 D23）；不要把外部失败伪装成消费回滚。

## 4. 不可编码清单（需范式扩展）

以下不是“换个写法就行”，而是当前语法/语义在**精确语义**上确实做不到。§2 的近似编码仍可使用，
但它们只保持可观察结果，不保持结构保证（如任意配对、原子性、严格顺序）；不要把近似编码登记为等价。

| ID | 不可表达 | 为什么 | 扩展方向（候选，非承诺） |
|---|---|---|---|
| X1 | 真正的 OR 汇聚到同一 RHS | RHS Uniqueness | 允许 OR 前驱，或允许同一 RHS 多规则并定义选择语义 |
| X2 | 任意两个同类型实例作为前驱 | LHS Uniqueness | 允许 LHS 多重集，并明确多实例匹配语义 |
| X3 | 原子多输出 | Rule-P 每次一个 token | 多输出规则 + 原子提交语义 |
| X4 | 无界内容寻址选择 | TypeSchema 有限、`ThingRef` opaque | 引入内容谓词/无限值域选择语义 |
| X5 | 运行时动态 schema/gate | 四注入点静态声明 | 版本化声明 + 迁移语义 |
| X6 | 严格实时/概率/连续语义 | 无时钟、确定性、离散 | 范式外扩展，或保持外部系统 |

## 5. 引用位置

| 位置 | 引用方式 |
|---|---|
| `paper/pactv1_rev3.md` §1.3、§1.4 Rule-P Note、§1.4.4、§2.3 | 规范源；边界声明与最小澄清 |
| `docs/design.md` §18 | 实现侧索引与模式落点 |
| `docs/architecture.md` §5 | 质量属性表中的架构级引用 |
| `docs/manual.md` §1.5 | 用户视角“什么时候别用”与绕法 |
| `examples/README.md` §7.3 | 可运行示例中的边界与 workaround |
| `docs/skill-adaptation.md` §7 | RE/SE 技能适配中的具体边界 |
| `crates/pact-core/src/domain.rs` | `DuplicatePrecondLhs`（B4/B10）、`ProduceWithCondition`（B7/B13） |
| `crates/pact-analysis/src/lib.rs` | `stranding_risk`（B8/B21） |
| `crates/pact-cpn/src/lib.rs` | `NegatedVia` / `MissingBound`（B23/B24） |

## 6. 维护规则

1. 母本新增/修订边界声明时，先改母本，再同步本文件编号与总表。
2. 新增编码模式必须写明：适用边界、保持了什么、丢失了什么、是否有静态分析可验证。
3. 代码注释引用本文件时使用 `B<编号>`，不复制整段解释，避免多处漂移。
4. 本文件不替代母本；出现冲突以母本为准，并登记到 `docs/design-change.md`。
