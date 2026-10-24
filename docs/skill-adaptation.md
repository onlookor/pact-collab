# 技能适配：RE / SE 的 PACT 形式层

本文件只保留当前有效内容；变更追加记入 `docs/skill-adaptation-change.md`。

范式规范：`paper/pactv1_rev3.md`（下文 §x.y 引用均指该文档）。产物落在 **另一个仓库** `~/projects/skills`，本文件是其设计说明、验收清单与差异矩阵的唯一权威记录。

> **母本版本说明（2026-10-20）**：母本已由 rev2 切换为 rev3，改动仅 5 类术语与作用域增补——§1.3 增列「Counter instantiation and partitioning」、§1.4.1 TokenId 作用域、Rule-T scope note、§4.5/§B.2 CPN 单实例作用域、§1.8 及 §4.1–4.3 的 `domain instance` → `domain declaration`；**§3.1 文法与 Appendix A 算法零变化**。故本文件正文中出现的 "rev2" 一词（如 §7.2「rev2 不表达的七类维度」）与 rev3 指同一文法，逐处改写无信息增益，未改写。

---

## 0. 为什么单独建档

| 既有文档 | 记什么 | 本项工作是否属于 |
|:--|:--|:--|
| `docs/design.md` | PACT **引擎实现**设计：crate 划分、决策记录 D1–D22、接口形态 | 否——引擎代码零改动 |
| `docs/architecture.md` | 引擎**架构**：组件、连接器、运行时视图 | 否——不新增组件、不改数据流 |
| `docs/requirements.md` | 引擎需求 | 否——未提出新引擎需求 |
| `docs/manual.md` | **权威用户手册**：适用判据、三接口完整命令面、六项静态分析、错误码全集、故障排查、已知偏差 | 否——本文件不重复手册内容，只记技能侧的建模裁决 |
| `examples/` | **领域无关的通用示例**：RE / SE / ReAct-agent 通用域 + 组合片段 + 离线/在线两条上手脚本与录制轨迹 | 否——与本项工作正交，见下 |

本项工作是 PACT 的**外部应用**：把两个技能目录（需求工程、软件工程）中的结构命题投影成 `.pact` 域声明，使其可被既有 `pact analyze` / `pact compose` 机械校验。它消费引擎能力，不改变引擎能力。硬塞进 design.md 会把"实现设计"与"使用案例"混为一谈，且三份既有文档的变更文件都已承载各自语义裁决，混入会断链。故单独建档，并按 `AGENTS.md` 的"变更追加"纪律配套 `skill-adaptation-change.md`。

**与 `examples/` 的关系（正交，勿混）**：`examples/domains/` 下的 `se_lifecycle.pact` / `re_pipeline.pact` 同样取材于软件工程与需求工程，但它们是**面向范式系统用户的教学示例**——不绑定任何技能目录、粒度刻意压低（45 / 50 行），随 `examples/README.md` 与 `docs/manual.md` 讲解使用场景与方法（需求侧登记为 FR14、设计侧为 D22）。本文件记录的 4 件 `.pact` 是**面向技能落地的生产级适配**——落在 `~/projects/skills/` 内、逐条对应技能文档的结构命题、SE 侧按裁剪模板分三档（149–425 行）。二者领域相邻而绑定对象不同，不构成重复维护；技能适配层的建模裁决只记在本文件，范式系统的通用用法只记在手册与示例。

## 1. 范围与产物

### 1.1 形式层（`.pact`，4 件）

| 文件（相对 `~/projects/skills/`） | 归属 | 类型 | 前置规则 | 角色位 | seed 权威 | 行数 |
|:--|:--|--:|--:|--:|--:|--:|
| `requirements-engineering/pact/re_fragment.pact` | RE 技能（单一，无档位） | 8 | 6 | 8 | 2 | 149 |
| `software-engineering/pact/se_tier1_solo.pact` | SE 档 I：单人 / 单 Agent | 40 | 31 | 15 | 9 | 395 |
| `software-engineering/pact/se_tier2_small_team.pact` | SE 档 II：小团队 | 41 | 32 | 18 | 9 | 388 |
| `software-engineering/pact/se_tier3_multi_team.pact` | SE 档 III：中大型 / 多团队多服务 | 41 | 32 | 20 | 9 | 425 |

SE 侧按《软件工程元框架-部署规模裁剪模板.md》§3 的三档装配画像各出一份。三档**不是人数门槛**，选档判据是该模板 §3 的三空间承载事实。

RE 侧无档位：RE 技能的 `stages/` 是固定六阶段链，裁剪模板不适用，故只有一份片段。

### 1.2 配套内容层（本轮改动的既有文件）

`.pact` 只能表达结构。凡 rev2 无词汇的命题（时间、频次、概率、OR 路由、成本数值、角色互斥……见 §7.2）一律落配套层，禁止在 `.pact` 里造语法。

| 文件 | 改动性质 |
|:--|:--|
| `software-engineering/pact/README.md` | **新建**：目录导览六节（权威关系 / 文件清单 / 校验 / rev2 不表达的维度 / E1–E7 索引 / 变更纪律） |
| `software-engineering/README.md` | §2 新增"形式层 `pact/`"子节；§4 补依赖关系一行；§5 阶段 A 第 2 步补选档后取结构位基线；§6 补当前状态一行 |
| `software-engineering/编译规范-changlog.md` | 追加 2026-10-14 批次（新增目录的规程面登记） |
| `software-engineering/todo.md` | 新增工作项 3.10（首批闭合 + 三项续办） |
| `requirements-engineering/stages/s2-elicitation.md` | 修 §2 非法形态 `reads ∅`；补源类型说明；front-matter 箭头统一 |
| `requirements-engineering/stages/s3-analysis.md`、`s4-specification.md` | front-matter 箭头 `->` → `→`（使配套层可与 `.pact` 机械比对） |
| `requirements-engineering/stages/s5-validation.md` | front-matter 补 `release_manager` 与 `validation_report → re_baseline`；新增 §8「Composition interface (§1.8)」段 |
| `requirements-engineering/stages/s6-management.md` | **语义级修正**：变更环四参与者链重写（详见 §8.1） |
| `requirements-engineering/SKILL.md` | §4.6 从 4 条扩到 10 条 + 权威声明行 |
| `requirements-engineering/README.md`、`PROGRESS.md` | rev1 → rev2 指向更新（3 处）+ analyze/compose 结果登记 |

## 2. 建模方法与三条纪律

**权威方向是单向的**：`成文法/技能正文 → .pact → 文件头注释`。`.pact` 是下游投影，不反向约束规范源。

1. **不得新增规范命题**。母本规则若无法用 rev2 文法表达，处置是：记入该档文件头"已知表达边界"（E 系列）→ 把规则留在配套内容层执行 → 按母本 §0.13 提请裁决。**不是**在 `.pact` 里造语法。
2. **逐条可回指**。`.pact` 中每个类型与每条前置规则都必须能回指规范源条款，文件头已逐类型标注指针。回指不上即幻觉条目，删除。
3. **只承载"粒度"轴**。裁剪模板 §1.2 给出三条合法裁剪轴（频次 / 粒度 / 工具形态）；rev2 只有粒度轴有词汇（角色位增减、`count` 阈值）。频次与工具形态两轴全部落配套层。

语法基线（每份产物落盘后逐条机械核对）：注释只用 `//`；域声明四段固定顺序 `types → preconditions → gates → seed_authority`；`∅` 在域声明中非法；`count` 字面量须为 PosNat；`produces` 不得携带条件。

## 3. 验收清单

全部条目已通过。命令见 §5.1，逐项证据见 §5.2。

### 3.1 语法与良构（A.5，§3.1）

- [x] 4 份 `.pact` 均被 `pact-parser` 接受，无解析错误
- [x] 4 份均 `well_formed: true`，`violations` 为空
- [x] 单片段 `warnings` 为空（RE / 档 I / 档 II / 档 III 各自 0 项）
- [x] 无 `SourceTypeProduceGate`：源类型（`business_goal`、`change_request`、以及 SE 侧 9 个 seeded 类型）无任何 `produces` gate
- [x] 无 `ProduceWithCondition`：全部阈值/存在性判据挂在 `reads` / `consumes` 上（E2）
- [x] 无 `DuplicatePrecondLhs` / `EmptyPrecondLhs` / `UndeclaredType` / `ZeroCountLiteral`
- [x] 注释只用 `//`；域声明体内无 `∅` 字符

### 3.2 静态分析（A.1–A.6）

- [x] `reachability`：4 份各自 `unreachable` 均为 **0**
- [x] `gate_coverage`：4 份均为 **0**（每个非源类型都有 producer）
- [x] `structural_deadlock`：4 份均为 **0**
- [x] `stranding_risk`：4 份均为 **0**
- [x] `consumption_reachability` 非零项已**逐项**归类定性，无未解释残留（§6.1、§6.2）

### 3.3 组合性（§1.8 C1–C4）

- [x] RE × 档 I、RE × 档 II、RE × 档 III 三次 `pact compose` 全部 exit 0、stderr 空、**无冲突输出**
- [x] C1 前置唯一性在合并域保持
- [x] C2 共享类型 `re_baseline` 的前置由 RE 侧**单侧声明**，SE 侧只声明类型不重复声明规则（D18 语义）
- [x] C3 共享 `(participant, action, type)` 三元组无 GateCond 语法分歧
- [x] C4 三份合并域输出重定向为 `.pact` 后 `pact analyze` 终检：`well_formed: true`、`violations: 0`、`unreachable: 0`、`gate_coverage: 0`、`deadlock: 0`、`stranding: 0`
- [x] 合并域唯一警告 `SEED_PRODUCE_OVERLAP(re_baseline)` 已定性为设计意图并登记裁决归属（§6.3）

### 3.4 配套层一致性

- [x] RE `stages/s2`–`s6` 的 `pact_rule` / `pact_participant` front-matter 与 `re_fragment.pact` 逐条比对一致
- [x] `stages/s6-management.md` 变更环描述与片段的实质矛盾已修复（§8.1）
- [x] `SKILL.md` §4.6 覆盖 rev2 全部关键约束（四条规则、Gate Matrix、零隐式动作、seed 权威、源类型禁产、溯源敏感性、组合接口、内容 vs 结构）
- [x] rev1 → rev2 指向更新完成，`build_tree.py` 完整性检查 39 文件全 OK
- [x] SE 目录三条治理纪律履行：根 `README.md` 登记 `pact/`、`编译规范-changlog.md` 追加批次、`todo.md` 记续办

### 3.5 未通过项 / 遗留

无未通过项。遗留三项属接地阶段动作，登记于 `software-engineering/todo.md` 3.10 续办：E7 部署前人类复核、`SEED_PRODUCE_OVERLAP` 部署裁决、容器内复跑校验（§9）。

## 4. 三档粒度差异矩阵

核心结论：**档间差异不在结构命题，只在角色位与阈值。** 档 II → 档 III 的类型集与前置图**完全一致，零增删**——多团队不引入新的结构命题。

### 4.1 计数与阈值

| 维度 | 档 I | 档 II | 档 III |
|:--|--:|--:|--:|
| 类型 | 40 | 41 | 41 |
| 前置规则 | 31 | 32 | 32 |
| 角色位（gate 声明的参与者） | 15 | 18 | 20 |
| seed 权威 | 9 | 9 | 9 |
| 批准链深度 `m`（`count approval_escalation >= m`） | 1 | 2 | 3 |
| 消解链上限 `N`（`not count concession_record >= N`） | 1 | 2 | 3 |

seed 权威三档**完全相同**：全部 9 项由 `responsible_human` 持有，覆盖 `tri_space_fact`、`external_mandate`、`new_fact`、`method_registry_entry`、`approval_registry_entry`、`guard_rule_entry`、`fvr_entry`、`projection_rule_entry`、`re_baseline`。裁剪不触碰种子入口。

### 4.2 类型与前置图差异

| 迁移 | 类型增删 | 前置规则增删 |
|:--|:--|:--|
| 档 I → 档 II | `+ contract_confirmation` | `+ {decision_landing + re_baseline} → contract_confirmation` |
| 档 II → 档 III | 无 | 无 |

`contract_confirmation` 是档 II 起才存在的结构命题：小团队及以上有独立的契约责任方，决策落地须回签确认。档 I 无此角色，故无此类型。

### 4.3 角色位差异（逐档累积）

| 迁移 | 新增角色位 | 承载的结构命题 |
|:--|:--|:--|
| 档 I → 档 II | `approver_backup` | 批准链第二级：`reads approval_registry_entry, reads adr_package via selected_entry, reads approval_escalation, produces approval_record` |
| | `oncall_responder` | 从 `responsible_human` **接管**守护信号处置：`reads guard_rule_entry, reads guard_signal, consumes guard_signal, produces waiver_request` |
| | `contract_owner` | `reads decision_landing, reads re_baseline, produces contract_confirmation` |
| 档 II → 档 III | `approver_external` | 批准链第三级，gate 与 `approver_backup` 同构 |
| | `service_owner` | 多服务边界的服务责任人：`reads decision_landing, reads guard_rule_entry, produces guard_signal, reads contract_confirmation, reads view_instance, reads reopen_proposal` |

三档**共有**的 15 个角色位：`responsible_human`、`approver_primary`、`review_design`、`review_shorthand`、`review_materiality`、`review_verif_plan`、`review_waiver`、`external_arbiter`、`agent_preparer`、`system_automaton`、`guardian`、`verifier`、`projector`、`compiler`、`auditor`。

### 4.4 角色合并度（同一结构命题的持有者迁移）

裁剪模板的"粒度"轴在 `.pact` 中表现为**同一 gate 子句在不同档由不同角色位持有**：

| 结构命题 | 档 I 持有者 | 档 II / III 持有者 |
|:--|:--|:--|
| `consumes guard_signal, produces waiver_request` | `responsible_human` | `oncall_responder` |
| `produces guard_signal` | `guardian` | `guardian` + `service_owner`（档 III） |
| `produces approval_record` | `approver_primary`（1 个角色位） | + `approver_backup`（档 II）+ `approver_external`（档 III） |
| `reads approval_escalation`、`reads contract_confirmation` | 无（`auditor` 也不读） | `auditor` 读（档 II/III） |
| `reads guard_signal`、`reads waiver_grant` | `responsible_human`（含消费） | `responsible_human` 仅只读 |

`approval_escalation` 在**任何档都只读不消费**（E6）：一旦消费，`count approval_escalation >= m` 就永远无法累积到 `m`，链耗尽判定失效。这属于“保留见证/阈值”模式（TC3/TC6/TC13）；判定准则与替代方案见 [`docs/token-consumption-timing.md`](token-consumption-timing.md)。

### 4.5 落在配套层的档间差异（`.pact` 无词汇）

频次（复审周期、批准时限）、工具形态（CI / 人工执行 CH-B）、注册表条目子集、检查单粒度——四类差异全部在裁剪模板 §2 参数包与 §3 装配画像中登记，`.pact` 不表达。这是设计而非缺陷：见 §7.2。

## 5. 机械校验

### 5.1 命令

```bash
cd ~/projects/pact

# 单片段静态分析（六项）
./target/debug/pact analyze --json <file>.pact

# 组合后终检（C1–C4）
./target/debug/pact compose \
  ~/projects/skills/requirements-engineering/pact/re_fragment.pact \
  ~/projects/skills/software-engineering/pact/se_tier3_multi_team.pact > /tmp/composed.pact
./target/debug/pact analyze --json /tmp/composed.pact
```

### 5.2 结果（2026-10-14 实测）

| 检查项 | RE | 档 I | 档 II | 档 III | RE×档I | RE×档II | RE×档III |
|:--|--:|--:|--:|--:|--:|--:|--:|
| `well_formed` | true | true | true | true | true | true | true |
| `violations` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `warnings` | 0 | 0 | 0 | 0 | 1 | 1 | 1 |
| `seed_types` | 2 | 9 | 9 | 9 | 11 | 11 | 11 |
| `reachable` | 8 | 40 | 41 | 41 | 47 | 48 | 48 |
| `unreachable` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `gate_coverage` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `consumption_reachability` | 1 | 30 | 31 | 31 | 30 | 31 | 31 |
| `structural_deadlock` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `stranding_risk` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

三次 `compose` 均 exit 0、stderr 0 字节、无冲突输出。

## 6. 非零咨询输出的定性

### 6.1 `consumption_reachability` 30 / 31 项

该分析列出**没有任何 Rule-E 消费者**的类型。SE 三档的非零项不是技术债，逐项归为三类：

**A 类：常驻注册表资产（8 项，三档相同）**

`approval_registry_entry`、`external_mandate`、`fvr_entry`、`guard_rule_entry`、`method_registry_entry`、`projection_rule_entry`、`re_baseline`、`tri_space_fact`

理由：这些是五注册表与三空间事实的 Token 化形态，母本 §10.2.5 判据 2 要求注册表**非空且长期在场**。消费即入 Archive，等于注销注册表条目。它们被大量 `reads`（Rule-R 非破坏），这正是注册表的语义。

**B 类：证据常驻供审计只读（10 项，三档相同）**

`adr_package`、`candidate_set`、`cost_ledger`、`feasible_set`、`mandate_intake`、`selected_entry`、`selection_context`、`survey_record`、`survivor_set`、`verdict_table`

理由：`auditor` 对全部 10 项持 `reads` gate。母本 §8.2.6 要求审计口长期可达；消费会切断审计。选型链的中间证据（候选集 → 存活集 → 可行集 → 判定表 → 成本台账）属"过程留痕"，其归宿是 Archive 之外的常驻 Counter + 审计视图。

**C 类：终态审计工件（档 I 12 项 / 档 II、III 13 项）**

`approval_escalation`、`approval_record`、`concession_record`、`decision_landing`、`grounding_status`、`human_ruling`、`plan_review`、`provisional_ruling`、`reopen_proposal`、`shorthand_pack`、`view_instance`、`waiver_grant`，档 II/III 另有 `contract_confirmation`

理由：这些是流程的**终点产物**，其价值在于可被事后追溯，无下游步骤需要销毁它们。`decision_landing` 被 `guardian` / `verifier` / `projector` / `responsible_human` 多方 `reads`；`grounding_status` 是接地验收结论；其余是裁决/批准/让步/豁免的留痕。

计数核对：8 + 10 + 12 = 30（档 I）；8 + 10 + 13 = 31（档 II / III）。

### 6.2 RE 侧 `raw_requirement`（1 项）

`raw_requirement` 无消费者是**设计意图**，且是必需的：`validator` 持 `reads specification via raw_requirement`，按 §1.4.0 GateHolds 的见证语义，读 `specification` 时要求 Counter 上存在一个 `raw_requirement` Token 且其 id 在 `specification.depends_on` 中。若消费 `raw_requirement`：

1. `validator` 的 via 见证被搁浅（`stranding_risk`）；
2. 变更环无法再生产 `specification`（`Precond(specification) = {raw_requirement}`，Rule-P 要求前置在场）——死锁。

**已知副作用**：`raw_requirement` 永驻 Counter ⇒ Counter 永不为空 ⇒ Rule-T 终止条件（Counter = ∅ 且无参与者 Rule-P-enabled）在 RE 片段单独部署时不可达。这是溯源敏感性与终止性的真实张力，登记在此，不掩盖。RE 片段的实践终止判据是 `re_baseline` 产出，而非 Rule-T。

### 6.3 合并域 `SEED_PRODUCE_OVERLAP(re_baseline)`（1 项）

**现象**：RE × SE 三档合并域均报 `SEED_PRODUCE_OVERLAP: responsible_human 对非源类型 re_baseline 持有 seed 权威，且该类型存在 produces gate`。

**成因**：SE 侧 `responsible_human` 持 `seeds re_baseline`（单独部署时基线由外部导入）；RE 侧 `release_manager` 持 `produces re_baseline`（验证链产出基线）。`re_baseline` 因有前置规则 `validation_report → re_baseline` 而非源类型，故触发该咨询性警告。

**意图与裁决归属**：

| 部署形态 | `re_baseline` 来源 | SE 侧 seed 权威 |
|:--|:--|:--|
| SE 单独部署 | 外部导入（seed） | **必须保留**，否则 `re_baseline` 不可达，档内 20+ 条依赖链全部断裂 |
| RE + SE 同时部署 | RE 验证链生产 | **应当撤除**，避免双入口 |

撤除 seed 会让 SE 单独部署不可用，保留 seed 会在组合时报一项咨询性警告。两份文件都保留现状，**裁决点在部署实例化时**，须记入裁剪 ADR（母本 §8.2.7）。该警告是咨询性的（§1.4.3 Advisory），不阻塞加载。

## 7. 已知表达边界

本章只记录 RE/SE 适配中实际撞到的边界。通用编号、编码模式与“不可编码”清单见
[`docs/expressiveness-boundaries.md`](expressiveness-boundaries.md)；下文处置若与总表冲突，以总表为准，
并同步修订本章。引用时使用 `B<编号>`。

### 7.1 SE 侧 E1–E7（就地记录于各档文件头，跨档累积）

| 编号 | 命题 | 出现于 |
|:--|:--|:--|
| E1 | 前置唯一性（RHS Uniqueness，§3.1 约束 1）禁止 `selected_entry` 经"完整 ADR"与"简记包"两路汇聚到同一 `decision_landing`。故 ADR 路径与简记路径建成**两条不相交 Token 流**；互斥性编码为 `reads selected_entry when not exists adr_package` | 档 I 起 |
| E2 | `produces` 不得携带条件（`DomainError::ProduceWithCondition`），故一切阈值/存在性判据改挂 `reads` / `consumes` | 档 I 起 |
| E3 | 权威六级全序 A0–A5（母本 §0.19.2）中只有 A0 有 Token（`external_mandate`）；A1–A5 的比较是配套层谓词求值 | 档 I 起 |
| E4 | 让步记录的三个强制消费端在形式层是 `reads`（Rule-R 非破坏）+ Archive 审计链 | 档 I 起 |
| E5 | rev2 无时序词汇，"会签先于批准"编码为 Counter 在场性 | 档 I 起 |
| E6 | `approval_escalation` **只读不消费**——否则 `count approval_escalation >= m` 永远无法累积，链耗尽判定失效 | 档 II 起 |
| E7 | "角色位互不兼任"不可表达 | 档 III |

**E7 是档 III 最需人类复核的缺口**：PACT 参与者是**不透明标识**，无自然人身份、无互斥约束词汇。形式层能保证的只是"存在三个独立的 `produces approval_record` 角色位（`approver_primary` / `approver_backup` / `approver_external`）"，**不能保证它们由三个不同的人填充**。同理，Waiver 的利益无关性、三级批准链的独立性都只能靠 `approval_registry_entry` 条目内容 + 部署前检查单强制。

### 7.2 rev2 不表达的七类维度（一律落配套内容层）

| 维度 | 规范源出处举例 | 落点 |
|:--|:--|:--|
| 时间 / 频次 / 时限 | 批准时限、复审周期、"连续 3 轮未过闸则升级" | 各档装配说明、检查单 |
| 概率与模糊判据 | 风险 likelihood、"实质性影响"的程度判断 | 方法注册表条目的谓词 |
| OR 多路由 | 批准 / 驳回 / 延期三分支 | 表现为"不产生任何 Token"（§1.4 零隐式动作） |
| 图结构判据 | 冲突成环检测、血缘下游目检 | 走查规程（母本 x.6） |
| 三值语义与 unknown 传播 | M15 勘察义务占位符 | 注册表台账 |
| 成本数值 / SLO / 包络界表达式 | §0.15 包络、成本台账数值 | `cost_ledger` Token 的 ThingRef schema |
| 角色位互不兼任 | 三级批准链独立性、Waiver 利益无关性 | `approval_registry_entry` 条目 + 部署前检查单 |

RE 侧同类维度：阶段时限、评审频次、验证抽样比例、"实质性变更"判定——落 `stages/` 与 `validators/`。

### 7.3 RE 侧：`change_approval` 不入 `depends_on`

新 `specification` 的 `depends_on` 记 `{raw_requirement id, change_request id}`，**不能**记 `change_approval`：`depends_on` 条目是按前置类型为键的 via 见证，而 `change_approval ∉ Precond(specification)`。批准链改由 **Archive** 保存——`change_approval` 的 `exhausted_at` / `exhausted_by` 记名 `change_controller` 与消费时刻。审计读 Archive，不读 `depends_on`。

## 8. 配套层对齐中发现并修复的不一致

### 8.1 `stages/s6-management.md` 变更环（语义级矛盾）

原文正文称 `change_controller consumes old specification`，而 `re_fragment.pact` 声明 `consumes re_baseline, consumes change_approval`。推理确认**片段正确**：

- `Precond(specification) = {raw_requirement}`，故 Rule-P 要求 `raw_requirement` 仍在 Counter；
- 旧 `specification` 早在 S5 被 `validator` 消费（`consumes specification, produces validation_report`）；
- 变更到达时 Counter 上是 `raw_requirement` + `re_baseline`，**不是**旧 `specification`；
- 因此消费 `re_baseline` 是唯一正确的"退役旧状态"动作。

同时正文缺 `change_requester`（seed 权威）、`change_analyst`、`ccb` 三个参与者。修复：front-matter 展开完整链、§1 Identity 改写、§2 PACT Operation 整段重写（含四段"为什么"说明：为什么不消费 `specification`、为什么 `change_approval` 独立成类型、两个 `produces specification` 为何合法、`depends_on` 的表达边界）、Step 1/2/4/5.1 逐处补齐。

修复的额外收益：`change_approval` 只有一个 producer gate（`ccb`），且 `Precond(change_approval) = {change_impact_report}` ⇒ 无影响分析即无批准。**"Agent 永不代 CCB 批准"这条红线由结构强制，不再只靠散文。** Reject / defer 不产生任何 Token（零隐式动作），链不前进。

### 8.2 其余（形态级）

| 文件 | 问题 | 修复 |
|:--|:--|:--|
| `stages/s2-elicitation.md` | §2 代码块出现 `reads ∅`——rev2 中无 reads 子句就是不写，`∅` 在域声明中非法 | 删除该子句，改为 `Gate: consumes business_goal, produces raw_requirement`；补源类型说明段 |
| `stages/s5-validation.md` | front-matter 的 `pact_rule` / `pact_participant` 只写 `validator`，漏 `release_manager`（正文 §8 已有完整声明） | 补 `validation_report → re_baseline (baselining, §8)` 与 `+ release_manager (baselining)`；新增「Composition interface (§1.8)」段 |
| `stages/s2`–`s6` | front-matter 箭头 `->` 与 `.pact` 的 `→` 不一致，无法机械比对 | 统一为 `→` |
| `SKILL.md` §4.6 | 只有 4 条，缺 rev2 全部关键约束 | 扩到 10 条 + 权威声明行（不一致时改本节不改 `.pact`，且须重跑 analyze） |
| `README.md`、`PROGRESS.md` | 指向 rev1；PROGRESS 称"语法对照 rev1 核对" | 改指 rev2 §3.1，登记 analyze / compose 结果 |

`PROGRESS.md` 措辞自查：初稿写"旧 rev1 已被取代"，核实 `docs/` 下 rev1 与 rev2 并存且 rev1 未标废弃后，改为"rev2 为当前版；rev1 为历史版本，rev2 增加了'前置规则 LHS 类型两两相异'约束"。

## 9. 执行偏差与后续工作

### 9.1 偏差登记

`AGENTS.md` 要求所有开发工作在 Trae 沙箱环境中进行（该条目早先表述为 distrobox 容器 `pact-dev`，该容器已不存在；本次一并更正）。本项技能的 PACT 形式层校验在沙箱内以已构建的 `./target/debug/pact` 做**只读**离线校验。

- 未修改 PACT 仓库任何代码（`crates/` 零改动）；
- 校验结果可信（同一构建产物），校验即在正式开发环境（Trae 沙箱）内完成；
- **接地验收时须在沙箱内复跑** `pact analyze` × 4 与 `pact compose` × 3 并留痕。

测试组织：本项工作未新增引擎测试——`.pact` 是数据产物不是代码，其"测试"就是 §5 的 analyze/compose 本身，已全绿。引擎侧回归：未触碰 `crates/`，故既有单元/集成/e2e 测试语义不变；若未来因 §10 的线索改动 `pact-analysis`，须同步复核 `crates/pact-analysis/tests/analysis_domains.rs`。

### 9.2 后续工作（登记于 `software-engineering/todo.md` 3.10）

1. **E7 部署前人类复核**：随真实项目试点验证"角色位互不兼任"的检查单是否足够。
2. **`SEED_PRODUCE_OVERLAP(re_baseline)` 部署裁决**：记入裁剪 ADR。
3. **沙箱内复跑校验**（见 §9.1）。

## 10. PACT 仓库侧线索（已定案：2026-10-15 采处置 (b)）

第 1 轮登记的偏差：`SEED_PRODUCE_OVERLAP` 的实现范围**宽于其文档注释**——

- `crates/pact-analysis/src/lib.rs:185-191` 的文档注释原写"**同一参与者**对同一非源类型既持 SeedAuthority 又持 produces gate"；
- `crates/pact-analysis/src/lib.rs:228-242` 的实现把 `produced` 收集为**全局** `BTreeSet<&TypeName>`（`filter` 只比 `Action::Produce`，丢弃参与者维度），随后 `seed_authority.iter().filter(|(_, t)| !is_source_type(..) && produced.contains(&t))` **只比类型不比参与者**。

后果：跨参与者的 seed/produce 组合也会报警（本例即 SE 的 `responsible_human` seeds + RE 的 `release_manager` produces）。方向是**假阳性**（过度报警），不漏报，故不产生正确性风险。既有测试 `precondition_wf_warns_seed_produce_overlap` 原只覆盖同参与者场景，未捕获该偏差。

**2026-10-15 用户选定处置 (b)：改注释承认现状，保留实现不变。** 理由：本次适配中该警告**确实起到了提示部署裁决点的作用**（SE 单独部署须保 seed 权威、组合部署应撤除），收窄实现反而会静默掉真实裁决点。落点：

- `lib.rs` 文档注释（185-191）与 `Display` 文本（202）改为类型级判定表述：`participant` 字段是 seed 持有者，produces gate 取全体参与者并集，跨参与者重叠也告警（假阳性方向，不漏报）；
- 论文 `pactv1_rev3.md` **维持参与者级原文**：2026-10-15 曾将其三处（§1.4.3 Advisory / A.5 伪代码 / A.5 Remediation）改为类型级定义，经用户指出改规范母本超出实现方范畴后**当日逐字回退**；实现与论文的关系登记为**保守超集**——论文规定的同参与者重叠实现必然告警，跨参与者重叠是额外的假阳性方向告警，Advisory 本身是咨询性警告，不触碰良构性语义。规范是否修订由作者另行裁决（rev3 切换未触及 §1.4.3，本关系仍成立）；
- `docs/design.md` D21(4) 补记类型级判定定案；
- `analysis_domains.rs` 的 `precondition_wf_warns_seed_produce_overlap` 增补跨参与者回归块（p produces b、q seeds b、q ≠ p ⇒ 警告 `participant = q`），锁定类型级行为。

明细见 `docs/design-change.md` 2026-10-15 批次。
