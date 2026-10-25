# PACT 域作者契约

> **用途**：把一个真实协作场景，稳定地转成一份能通过 `pact analyze` 的 `.pact` 域声明。
> **读者**：没有 PACT 背景的人；以及被要求代为建模的模型。
> **性质**：这是**生成协议**，不是规范。语义以 `paper/pactv1_rev3.md` 为准；语法细节见
> [`manual.md`](manual.md) §3；边界编号见 [`expressiveness-boundaries.md`](expressiveness-boundaries.md)；
> 消费时机见 [`token-consumption-timing.md`](token-consumption-timing.md)。
> **纪律**：本文件与引擎版本同步；新增/修改配方必须实际跑过 `pact analyze`（§9）。

---

## 0. 两种用法

### 0.1 有分析器（推荐）

```bash
pact analyze my_domain.pact          # 人读：六项分析，exit 0 = 良构
pact --json analyze my_domain.pact   # 机器读：供模型定位并修复错误
```

回环规则：

1. 先写，再跑。**非良构域不会输出六项报告**：命令返回
   `{"error":{"code":"WellFormedness","detail":[...]}}`、exit 1，`detail` 就是违规列表
   （即 `precondition_wf` 的 violations）。必须清零。
2. 良构后才会输出六项分析。其余五项非零时**不得静默忽略**：要么改，要么在决策记录（§7）
   里写明“这是设计意图”。
3. 修错时只看 JSON 的 `error.detail` 或报告字段，不要重写整个域。

`analyze` 是离线命令，不需要 daemon。装载前自检也用它（`domain validate` 需要 daemon）。

通过后启动 daemon，再装载：`pact domain load my_domain.pact`（域名缺省取文件 stem）。
装载后的运行流程见 [`manual.md`](manual.md) §4。

### 0.2 没有分析器（公开 chat / 纸笔）

把本文件**整段**粘给模型，再附场景描述；产出后按 §8 自检。可以直接用这句开头：

> 请严格按《PACT 域作者契约》工作：先过 §1 闸门，再按 §3 逐步提问；信息不足时提问，
> 不要编造；最终只输出 §2 要求的两份产物（`.pact` + 决策记录）。

> **不要给模型喂论文。** 论文是语义规范（规则、定理、附录），不是生成规格；它不会告诉模型
> “先问什么、产出什么格式、哪些错误必须先排除”。本文件才是给生成器看的。

---

## 1. 闸门：先决定“该不该用 PACT”

### 1.1 五条适用判据（全部为“是”才继续）

| # | 判据 | 判“否”时 |
|---|------|----------|
| G1 | 能列出**离散产物**（有明确的存在/不存在状态） | 先把连续过程离散成里程碑 token |
| G2 | 存在“某产物必须先有另一些产物”的关系 | 可能根本不需要 PACT |
| G3 | 产物能被不同参与者读取/使用/移交 | 同上 |
| G4 | 至少有一个参与者（单人也合法） | 同上 |
| G5 | 每个没有前置的产物，都能指出**谁有权首次注入** | 补齐入口；补不出就不要建模 |

### 1.2 五个硬停（命中就不要硬写，按处置改写）

| 代号 | 症状 | 处置 |
|------|------|------|
| H1 | “A 或 B 任一出现，就能产出 C” | OR 不能汇聚到同一 RHS（B4/X1）。分支各自成链、统一类型名、或拆域 |
| H2 | “任意两个同类型实例同时在场才能产出 C” | 不能写 `{A + A} → C`（B10/X2）。按角色拆成不同 `TypeName`，或让参与者产协调 token |
| H3 | “一次动作必须原子地产生多个 token” | 拆成多次 Rule-P；原子性不保（B15/X3） |
| H4 | “按内容/版本/身份决定能否生产” | Rule-P 不读 `ThingRef`（B11/B12）。把有限取值编成类型，或用见证 token |
| H5 | 需要超时、截止、严格时钟顺序 | PACT 没有时钟（B2/B8）。交给外部系统，或改用见证存活 |

命中 H1–H5 仍然可以建模，但**必须把改写方式写进决策记录**，并注明丢失了什么性质。

---

## 2. 输出契约

每次建模必须交付两份东西，缺一不可：

1. **一个 `pact` 代码块**（四段，顺序严格：`types` → `preconditions` → `gates` → `seed_authority`）；
2. **一份决策记录**（§7 模板）。

硬性禁止：

- 不编造业务事实。槽位缺失就提问，不要自行补角色、补产物。
- 不用中文、空格、连字符作标识符。标识符只允许 `[A-Za-z_][A-Za-z0-9_]*`。
- 不把**动作**（“部署”）、**角色**（“Alice”）、**指标**（“完成度 90%”）写成类型；类型是产物。
- 不在 `produces` 后面加条件（语法会拒绝）。
- 不为了“看起来完整”而添加无人读、无人产、无人消费的类型。

---

## 3. 五个建模步骤（按顺序提问）

### 第 1 步：列离散产物 → `types`

对候选词逐个问三个问题，三问全“是”才是类型：

1. 它是不是一个**结果**，而不是一个动作或状态？
2. 它能不能被“放上 Counter / 从 Counter 拿走”（离散存在性）？
3. 它是不是一个**可移交的件**，而不是某个角色或某个百分比？

| 候选 | 判定 |
|------|------|
| `requirement`、`design`、`test_report` | 是 |
| `review_meeting`、`deploy` | 否，是动作；改用其结果 `review_record`、`deployment` |
| `90%_complete`、`quality_score` | 否，是连续量；离散化为里程碑或交给 `ThingRef` |
| `alice`、`reviewer` | 否，是角色；角色进 `gates` |

命名用单数、简短、领域通用；同一概念只用一个名字。

### 第 2 步：写前置图 → `preconditions`

格式：`A → B`、`{A + B} → C`、`A → A`（自环合法）。

四条硬规则：

1. **只有 AND**。写两条 `A → C`、`B → C` 就是 OR，会触发 `DuplicatePrecondRule`（H1）。
2. **RHS 唯一**：同一类型只能有一条前置规则。
3. **LHS 非空、不重复**：`{ } → B` 和 `{A + A} → B` 都非法。
4. **没有前置规则的类型 = 源类型**，只能经种子进入，不能 `produces`。

自检：每个类型要么在**某条规则的 RHS 上**，要么是源类型（有种子权威）。两者都不是 → 它永远不可能出现。

### 第 3 步：写权限矩阵 → `gates`

格式：`参与者: reads T, consumes T, produces U;` 一个参与者可有多条子句。

- `reads`：只读，不改变状态；消费后就再也读不到。
- `consumes`：归档，不可逆；**不会**级联删除下游，也**不会**自动清理。
- `produces`：**不能带条件**；生产合法性完全由前置图决定。要给生产加条件，只能改前置图或加见证 token。
- 条件只能写 `reads` / `consumes`，表达式只读**当前 Counter**：
  `exists T`、`not exists T`、`count T >= n`、`count T = n`、`via T`。
- `via T` 的精确含义是：“动作依赖的那个 **T 实例**现在仍在 Counter 上”。
  它不是“T 发生在之前”。**必须同时决定：谁负责消费 T，且在依赖动作完成前不能消费它。**

### 第 4 步：定入口 → `seed_authority`

格式：`参与者: seeds T`。

- 每个源类型至少要有一个种子权威（否则 A.1 报不可达）。
- 纯循环域没有源类型，但**仍要一个 seed 点火**（seed 豁免前置条件）。
- 同一类型既有 `seeds` 又有 `produces` → `SEED_PRODUCE_OVERLAP` 警告：确认这是“双入口”设计，而不是漏改。

### 第 5 步：定消费 → 决策记录里的逐类型表

对**每一个类型** `T` 依次回答（TC 逼问，规则见 `token-consumption-timing.md` §4）：

| 问题 | 若为“是” |
|------|-----------|
| TC-Q1 有 `via T` 依赖吗？ | 见证必须活到依赖动作完成（TC-P1/TC-P2）；跑 A.6，非零要裁决 |
| TC-Q2 `T` 还是未来生产的前驱吗？ | 不要提前消费，否则终止未来扇出 |
| TC-Q3 有多个读者/多个消费者吗？ | 读不消耗；多消费者必须指定**谁**负责消费；没有“末位消费者”机制（TC3） |
| TC-Q4 有 `count T` 阈值吗？ | 消费会改变阈值；决定阈值失效后由谁消费 |
| TC-Q5 消费是否代表外部动作已完成？ | 用 readiness 见证前移（TC-P6），或安排补偿；不要假设原子 |
| TC-Q6 域要求最终终止吗？ | 要求终止 ⇒ **每个** token（含 ack、readiness、见证）都要有最终消费者；否则接受不终止并写明 |
| TC-Q7 只是审计/长期观察吗？ | 不消费；用 Read + Archive 查询（TC-P9），不要靠留在 Counter 当审计 |

TC-Q1–TC-Q7 全为“否” → 可以消费。任一为“是” → 按右列处理，或明确接受后果。

---

## 4. 形状配方库：先选形状，再填空

非专业用户最缺的不是语法，而是“我该选哪种形状”。先匹配形状，再替换类型名和参与者名。
每个配方都实际跑过 `pact analyze`（§9），预期输出写在配方末尾。

### R1 线性流水线

**信号**：产物一件接一件，每件只有一个主要生产者。

```pact
types { request; answer }

preconditions { request → answer }

gates {
    responder: reads request, consumes request, produces answer;
    client:    consumes answer
}

seed_authority { client: seeds request }
```

- 必须决定：每个生产者在产出后是否立刻消费前驱。此处是“是”（无 `via`、无未来扇出）。
- 常见误用：生产者先 `consumes` 再 `produces`。运行时顺序必须是
  `read → produce(--predecessors) → consume`；反过来会报 `PredecessorNotOnCounter`，且不可撤销。
- 预期：六项分析全零。

### R2 只读评审支线

**信号**：有人只审阅、不产出、不消费。

```pact
types { draft; review; final }

preconditions { draft → review; review → final }

gates {
    author:      reads draft, consumes draft, produces review;
    reviewer:    reads review;
    owner:       reads review, consumes review, produces final;
    stakeholder: consumes final
}

seed_authority { intake: seeds draft }
```

- 必须决定：评审发生在 `review` 被消费**之前**。`reads` 不保留 token，结构不保证评审窗口。
- 两个选项：(a) 接受“评审必须先完成”的外部纪律；(b) 让 reviewer 产出 `approval`，把
  `approval` 写进下一环的前置图（结构强制）。
- 预期：六项分析全零。

### R3 审批关联（`via` 见证）

**信号**：处置某产物时，必须能证明它的来源仍在场。

```pact
types { design; test_report; deployment }

preconditions { design → test_report; test_report → deployment }

gates {
    engineer:    reads design, produces test_report;
    reviewer:    reads test_report via design;
    deployer:    reads test_report via design,
                 consumes test_report via design,
                 produces deployment;
    stakeholder: consumes deployment
}

seed_authority { intake: seeds design }
```

- 必须决定：`design` 作为见证**保留**，因此谁都不消费它 ⇒ 域无法终止。
- 若要终止：加一个显式释放动作（如 `archivist: consumes design when count deployment >= 1`），
  并接受 A.6 会报搁浅风险；或改用更长寿的见证类型。
- 预期：`consumption_reachability: 1`（`design` 无消费者）、`stranding_risk: 0`。前者是设计意图。

### R4 外部物化见证

**信号**：消费会触发外部动作，必须先确认外部产物已就位。

```pact
types { work_item; readiness; completion }

preconditions { work_item → completion }

gates {
    coordinator: consumes work_item when exists readiness,
                 consumes readiness,
                 produces completion;
    reviewer:    consumes completion
}

seed_authority {
    requester:       seeds work_item;
    external_system: seeds readiness
}
```

- 要点：外部状态不能写进 GateExpr，只能编码成 token；`exists readiness` 与消费落在同一次准入判定。
- 必须决定：`readiness` 自身也要清理；外部失败时不要假装消费可以回滚（TC12/TC-D5）。
- 预期：六项分析全零。

### R5 扇出 / ack 汇聚（屏障）

**信号**：一个共享产物要被多个下游各自使用，全部完成后才归档。

```pact
types { shared; ack_a; ack_b; merged }

preconditions {
    shared → ack_a;
    shared → ack_b;
    { ack_a + ack_b } → merged
}

gates {
    worker_a:    reads shared, produces ack_a;
    worker_b:    reads shared, produces ack_b;
    coordinator: consumes shared when count ack_a >= 1 and count ack_b >= 1,
                 produces merged;
    cleaner:     consumes ack_a when count merged >= 1,
                 consumes ack_b when count merged >= 1;
    stakeholder: consumes merged
}

seed_authority { intake: seeds shared }
```

- 要点：`count` 是类型级，不绑定具体消费者；ack 与消费者实例不能一一对应（TC4）。
- 必须决定：ack 自身也要消费，否则阻止终止；cleaner 的 `when count merged >= 1` 是把
  “先屏障、后清理”变成准入条件的写法。
- 预期：六项分析全零。

### R6 循环点火

**信号**：产物环环相扣，没有天然起点，但必须能启动。

```pact
types { observation; thought; action; tool_result }

preconditions {
    observation → thought;
    thought → action;
    action → tool_result;
    tool_result → observation
}

gates {
    agent: reads observation, consumes observation, produces thought,
           reads thought, consumes thought, produces action,
           reads action, consumes action, produces tool_result,
           reads tool_result, consumes tool_result, produces observation
}

seed_authority { user: seeds observation }
```

- 要点：环不阻碍终止；终止取决于 Counter 是否为空。首个 `observation` 由 seed 点火。
- 必须决定：收工时由谁消费最后一个 token；`observation` 同时可 seed 和可 produce，
  `SEED_PRODUCE_OVERLAP` 是**预期**警告，不是错误。
- 预期：良构 0 违规 / **1 警告**；其余五项全零。

---

## 5. 良构硬约束（八类错误与修法）

| 错误码 | 触发 | 修法 |
|--------|------|------|
| `DuplicatePrecondRule` | 同一 RHS 有两条前置规则 | 每条 RHS 只留一条；OR 走 H1 |
| `DuplicatePrecondLhs` | `{A + A} → B` | LHS 去重 |
| `EmptyPrecondLhs` | `{ } → B` | 删掉规则，把 `B` 当源类型并给种子权威 |
| `UndeclaredType` | 引用了未在 `types` 声明的类型 | 补进 `types`（每个引用点都会单独报） |
| `ZeroCountLiteral` | `count T >= 0`、前导零 | 阈值写正整数 |
| `ProduceWithCondition` | `produces T when …` | 删条件；条件进前置图或改用见证 token |
| `SourceTypeProduceGate` | 给源类型写 `produces` | 源类型只能 seed；确实要生产就先给它加前置 |
| `DuplicateGateClause` | 同一 `(参与者, 动作, 类型)` 重复 | 合并子句 |

解析期还会直接拒绝：空 LHS、非正整数的 `count` 字面量、四段顺序错误、标识符不合法。
**解析返回全部问题，不是首错即停**，可一次改完。

---

## 6. 六项分析怎么读

| 输出 | 它在问 | 非零意味着 | 必须做什么 |
|------|--------|-----------|-----------|
| `precondition_wf` | §3.1 良构 + Seed/Produce 重叠 | violations > 0：**不可用** | 必须清零；此时命令以 `WellFormedness` 错误拒绝，违规在 `error.detail` 里，不会出报告 |
| `reachability` | 从种子权威 BFS | 有类型永远产生不了 | 补种子、补生产、或删掉该类型 |
| `gate_coverage` | 非源类型有没有生产者 | 有人想用却没人能产 | 补 `produces` 子句 |
| `consumption_reachability` | 每个类型有没有消费者 | 可能技术债，也可能是有意保留 | 按 TC-Q7 裁决并记录 |
| `structural_deadlock` | 无种子入口的强连通分量 | 环起不来 | 给环加 seed 点火（R6） |
| `stranding_risk` | `via` 见证会不会先被消费 | 存在真实顺序风险 | 按 TC-P1/TC-P2 排序，或改见证类型 |

**裁决原则**：violations 必须改；warnings 和其余五项**不要求为零**，但每一处非零都要有
一句“为什么这样是对的”。示例域里就有三处刻意的非零（`se_lifecycle` 的搁浅风险、
`re_pipeline` 的无消费者、`react_loop` 的重叠警告）。

---

## 7. 决策记录模板

```markdown
## 决策记录

- 场景（一句话）：
- 适用闸门：G1=是 G2=是 G3=是 G4=是 G5=是（任一否 → 说明为何仍继续或停止）
- 硬停命中：无 / H?（改写方式与丢失的性质）
- 采用形状：R?（以及为什么）

| 类型 | 前驱 | 生产者 | 读者 | 消费者 | via 依赖 | count 阈值 | 保留策略 | 最终消费者 |
|------|------|--------|------|--------|----------|------------|----------|------------|
| ...  | ...  | ...    | ...  | ...    | ...      | ...        | 短用/见证/审计 | ... |

- 入口（源类型 → 种子权威）：
- TC 裁决：TC-Q1–TC-Q7 中所有“是”的处理方式
- B 裁决：涉及的 B 编号与改写代价
- 分析结果（有工具时）：六项输出 + 每处非零的理由
- 已知代价：不终止 / 仅近似 / 外部依赖
```

**没有决策记录的域视为未完成**：语法正确只是最低门槛。

---

## 8. 无工具自检清单（12 条）

1. 四段齐全且顺序为 `types → preconditions → gates → seed_authority`。
2. 标识符全部匹配 `[A-Za-z_][A-Za-z0-9_]*`。
3. 每个类型要么在某条规则的 RHS，要么是源类型并有种子权威。
4. 每个 RHS 只有一条前置规则。
5. 没有空 LHS、没有 LHS 重复、没有 `count 0`。
6. 没有给源类型写 `produces`。
7. 没有在任何 `produces` 后写条件。
8. 每个非源类型至少有一个 `produces` 子句。
9. 每处 `via T` 都有明确的“见证由谁、何时消费”的答案。
10. 每个多消费者类型都指定了消费负责人，或明确写了“常驻不消费”。
11. 若声称域会终止：每个 token（含 ack、readiness、见证）都有最终消费者。
12. 决策记录已填写，且每处已知代价都写明。

第 9–11 条是新手最常漏的；它们不是语法问题，分析器只能“提醒”，不能替你决定。

---

## 9. 最小完整示例（已实测）

```pact
types { request; answer }

preconditions { request → answer }

gates {
    responder: reads request, consumes request, produces answer;
    client:    consumes answer
}

seed_authority { client: seeds request }
```

```console
$ pact analyze minimal.pact
precondition_wf: 良构（0 项违规，0 项警告）
reachability: 1 源类型，2 可达，0 不可达
gate_coverage: 0 个无 producer 的非源类型
consumption_reachability: 0 个无 consumer 的类型
structural_deadlock: 0 个强连通分量
stranding_risk: 0 处滞留风险

$ pact analyze minimal.pact; echo $?
0
```

§4 的六个配方（R1–R6）均已用同一命令验证：R1/R2/R3/R5 为独立测试文件，
R4/R6 直接采用 `examples/domains/` 中的已验域；预期非零项已在配方末尾注明。

---

## 10. 与既有文档的分工

| 文档 | 管什么 | 本契约如何使用 |
|------|--------|----------------|
| `paper/pactv1_rev3.md` | 语义、规则、定理 | 只作最终裁决，不作生成提示 |
| [`manual.md`](manual.md) §3 | 语法细节、错误码、命令 | 查语法；本契约不重复长表 |
| [`expressiveness-boundaries.md`](expressiveness-boundaries.md) | B1–B25、X1–X6 | 命中硬停时查编号与改写代价 |
| [`token-consumption-timing.md`](token-consumption-timing.md) | TC1–TC13、TC-D1–TC-D6、TC-P1–TC-P9 | 第 5 步逐类型逼问的完整依据 |
| [`../examples/`](../examples/README.md) | 可运行域与实录轨迹 | 需要更复杂骨架时参考，不直接复制 |
| `docs/skill-adaptation.md` | RE/SE 两技能的适配 | 特定技能场景另查 |

---

## 11. 维护规则

1. 新增配方必须附 `pact analyze` 实测结果；预期非零项必须解释。
2. 引擎语义或错误码变化时，同步 §5/§6，并在 `docs/design-change.md` 记录。
3. 本文件只写“怎么生成”，不写“为什么这样规定”；理由回链到母本、B、TC，避免多处漂移。
4. 若本文件与母本冲突，以母本为准，并登记到 `docs/design-change.md`。
