# PACT 共享 Token 消费时机

对应规范：`paper/pactv1_rev3.md`（下称“母本”）§1.4（Rule-P/R/E/T）、§1.4.0（`via`）、§1.6、§1.7.3、附录 A.6。
本文件是**实现侧与域设计侧**对“一个 token 被多个下游共享时，何时可以消费”的集中论述。

与 [`docs/expressiveness-boundaries.md`](expressiveness-boundaries.md) 的分工：

- 边界文档回答“范式能不能直接表达某个需求”，用 `B<编号>`；
- 本文件回答“何时消费安全、由谁消费、按什么顺序消费、如何验证”，用 `TC<编号>`；
- 两者交叉引用，不复制对方内容；冲突时以母本为准。

阅读顺序：§2 缺口 → §4 判定准则 → §5 模式 → §8 评审清单。

## 1. 问题定义

消费（Rule-E）把 token 从 Counter 移入 Archive。对共享 token，这不是目标 token 的局部动作，而是全局决策：

- **早消费**：剥夺下游的观察（read）、未来生产（Rule-P 前驱）、见证（`via`）、阈值（`count`）；
- **晚消费**：token 累积；Rule-T 要求 `Counter = ∅`，永久保留即永不终止；
- **顺序错误**：通过 `via` 依赖它的下游 token 会搁浅（stranded）。

本文的“共享”指一个 token 实例 `t` 被多个下游关系引用，包括：

1. 多个参与者对 `type(t)` 持有 read/consume gate；
2. `t` 是某条 Rule-P 规则的前驱类型实例，且该规则可被多次执行；
3. 多个下游 token 的 `depends_on` 含 `t.id`；
4. 某个 `count type(t)` 阈值依赖 `t` 留在 Counter。

> `depends_on` 只是 provenance，不是 demand。一个 token 被多少下游 `depends_on` 引用，并不等于“还有多少下游需要它”。范式没有引用计数。

## 2. 母本现状与缺口

### 2.1 母本已明确

| 语义 | 母本位置 | 对消费时机的含义 |
|---|---|---|
| Rule-P 不消费前驱 | §1.4 Rule-P Note | 前驱保持，可被再次读取或作为后续生产的前驱 |
| Rule-R 幂等、非破坏 | §1.4 Rule-R | 多读者可重复读，读不改变 Counter |
| Rule-E 单 token、无级联 | §1.4 Rule-E Note | 消费不影响已产生的下游 token，也不反向传播 |
| `via` 见证必须在当前 Counter | §1.4.0、§1.7.2 | 见证归档后 `GateHolds` 失败；同类型其他实例不能替代 |
| GateExpr 只看当前状态 | §1.7.3 | 不能表达“先消费 A 才能消费 B” |
| 上游不保证晚于下游存活、无悬空引用、不保证清理、见证不保证存活 | §1.6 | 消费顺序是域的责任，不是范式的保证 |
| 搁浅后果 | §1.6 | 无 `via` 的参与者不受影响；有 `via` 的参与者被阻塞 |
| 搁浅风险与三种缓解 | 附录 A.6 | 约定、Archival Policy、调整前置图以避免共享见证 |
| 消费时机是域策略；无 demand registry、引用计数、last-consumer 信号 | §1.6（2026-10-24 补写） | 何时消费不可由声明推导，必须由域指定 |
| Read 不是 reservation；Archive 不在 Counter，不能 read/consume/作 `via` 见证 | §1.6（2026-10-24 补写） | 读后仍可能被他人消费；消费后不可再观察 |
| ack 屏障：`consumes t when count ack >= n` | §1.7.3（2026-10-24 补写） | 可近似“所有下游完成”，但不绑定消费者实例，ack 自身需清理 |

### 2.2 母本已补写与仍存边界

2026-10-24 经用户授权，母本 §1.6 与 §1.7.3 已补写消费时机说明。缺口状态如下：

| 编号 | 原缺口 | 状态 |
|---|---|---|
| a | 没有“共享 token 的消费所有者” | 母本已声明“消费时机是域策略”，具体所有者仍由域指定 |
| b | 没有 demand / reference count / reader count | 已由母本 §1.6 明确声明不存在；`depends_on` 仅为 provenance |
| c | 没有“末位消费者”事件或 last-reader 检测 | 已由母本 §1.6 明确声明不存在 |
| d | Read 与 consume 之间没有 reservation/lease | 已由母本 §1.6 明确“Read 不是 reservation” |
| e | 没有标准消费顺序建议 | 母本 §1.7.3 给出 ack 屏障近似；仍不强制顺序 |
| f | Rule-T 与“保留见证/审计保留”的冲突 | 已由母本 §1.6 明确：要求终止则须为每个 token 指定最终消费者 |
| g | 没有范式级声明“消费成功 ≠ 外部动作成功” | 仍为范式外边界，由实现设计 D23 补充 |

补充：实现中 Archive **不参与 Rule-R/Rule-E**，用于审计查询与 TokenId 唯一性守卫（`Session::used_ids` 由 Counter + Archive 重建）；母本 §1.6 补写后已明确“归档 token 不在 Counter，不能 read/consume/作 `via` 见证”，与本文件 **Counter-only** 的实现口径一致。

## 3. 能力总表（TC1–TC13）

| ID | 需求 | 母本依据 | 能否直接表达 | 可行模式 | 丢失 / 代价 | 静态检查 |
|---|---|---|---|---|---|---|
| TC1 | 多读者共享只读 | §1.4 Rule-R | 可 | 任意多个 read gate；无需复制 | 读不改变状态，但消费后不可再读 | A.3 可提示无 consumer；无 reader 计数 |
| TC2 | 前驱复用与重复生产（扇出预算） | §1.4 Rule-P Note | 可 | 保持前驱在 Counter；每次 Rule-P 可复用同一实例 | 消费前驱即终止未来扇出；无“扇出次数”计数 | A.1/A.2；无扇出计数 |
| TC3 | 末位消费者归档 | 无 | 不可直接表达 | 外部协调者、ack 汇聚（P4）、每消费者副本（P3）；或约定唯一消费者 | 无 last-consumer 事件；协调者必须可信 | A.3；无静态保证 |
| TC4 | 全下游完成后消费（屏障） | §1.4 Rule-E | 仅可近似 | P4：下游 read 后 produce 每角色 ack；`consumes t when count ack >= n` | ack 与消费者实例不能一一对应；ack 自身需清理；参与者可能未真正完成工作 | A.3/A.6；无 ack 完整性检查 |
| TC5 | 按消费者复制（copy fan-out） | §1.4 Rule-P | 可 | 每次生产一份下游副本；共享前驱保留到所有副本产生完毕；各副本独立消费 | 副本是不同 `TokenId`；“同一对象”需域解释；副本数量无结构约束 | A.1/A.2；无副本数约束 |
| TC6 | 见证存活（`via`） | §1.4.0 | 可（正向） | P2：所有 `via` 依赖动作完成前不消费见证 | 见证长期占用，可能阻止 Rule-T；`not via` 不可编码（B23） | A.6 精确/保守报告 |
| TC7 | 消费导致下游搁浅 | §1.6 | 现象可表达，避免需设计 | P1 下游优先排空；P2；P6 见证 token；设计避免共享见证 | 无法自动恢复、无撤销；只能重载域 | A.6 |
| TC8 | 保留 / 租约 / 资源占用 | §1.4 Rule-E；B17 | 仅可近似 | P5：consume 资源=占用；释放时重新 seed 一个资源 token | 新 `TokenId`，资源谱系不连续；崩溃可能泄漏；无自动过期 | 无；需外部锁/审计 |
| TC9 | 幂等重试与至多一次 | §1.4 Rule-E；实现 D23 | 实现层可 | P8：`request_id` 同键同 token 重放首次结果；异 token 冲突 | 不阻止其他参与者抢先消费；不是分布式锁；并发结论依赖宿主单写者串行化（P8） | 集成测试覆盖；无静态分析 |
| TC10 | 自动清理 | §1.6；§2.2 Archival Policy | 仅可近似 | P7：清洁工参与者 / Archival Policy，仍是显式 Rule-E | 引擎不自动执行；策略需可信；可能误删见证 | A.3 提示无 consumer |
| TC11 | 引用计数 / 存活判定 | 无 | 范式外 | 外部索引、协调者 token、ack 计数；或把“仍需要”编码成显式 token | PACT 不知道谁还需要；无法 O(1) 判定 | 无 |
| TC12 | 消费成功 ≠ 外部动作成功 | 实现 D23；§1.6 | 实现层可 | P6 witness-token 前移；或后置 saga 补偿 | 二者最终一致；消费不回滚 | 无范式级检查 |
| TC13 | 终止压力与消费顺序 | §1.4 Rule-T | 可计算，不可强制 | P1 下游优先；最后消费见证/ack；P7 归档策略；或接受不终止 | **Counter 上**保留任何 token 即不终止（Archive 中的 token 不阻止终止）；顺序错误会搁浅 | `termination_holds`；A.6 |

## 4. 判定准则

对目标 token `t`，消费前依次检查 D1–D6。**任一为“仍需要”即不得消费**，除非先完成对应动作或明确接受其后果。

| 编号 | 问题 | 判据 | 处置 |
|---|---|---|---|
| D1 | 未来 Rule-P 是否还需要 `type(t)` 作前驱？ | 存在生产规则以该类型为前驱，且域仍希望该规则可执行 | 保留；或明确放弃未来扇出后再消费 |
| D2 | 是否存在 `via type(t)` 依赖，且目标 token 的 `depends_on` 含 `t.id`？ | 有 read/consume gate 使用 `via`，且依赖 token 尚未被消费 | 先处理下游（P1），或保留见证（P2），或改用 ack（P4） |
| D3 | 是否存在 `count type(t) >= n` / `= n` gate？ | `count` 在**准入时**按当前 Counter 求值；若 `T = type(t)`，目标自身仍计入阈值，消费后计数才减一 | 保留至阈值不再需要；或拆类型 |
| D4 | 是否仍有参与方需要持续观察？ | 报表、审计、复核依赖 Read；Read 只在 Counter 上有效 | 不消费；改用外部索引/归档查询（P9） |
| D5 | 本次消费是否代表外部副作用？ | 外部产物是否已就位，或已有补偿方案 | P6 witness-token，或 saga 补偿 |
| D6 | 域是否要求 Rule-T？ | `Counter` 必须最终为空 | 制定完整排空顺序；否则接受不终止 |

决策：

- 全部为否 → 可消费；
- D1 为是 → 保留，或接受“最后一次生产”。
- D2 为是 → 下游优先排空；或见证保留；或 ack 汇聚。
- D3 为是 → 保留至阈值失效；若阈值本意是“累积”，永不消费该类型。
- D4 为是 → 不消费；审计需求用 Read + 外部索引，而不是把 token 留在 Counter 当作审计。
- D5 为是 → 先确认物化（P6），或安排补偿。
- D6 为是 → 所有 token（含 ack、readiness、见证）都必须有最终消费者。

## 5. 消费模式

### P1 下游优先排空（downstream-first drain）

- **做法**：先消费叶子 token（无 `via` 依赖）→ 中间 token → 根/见证最后。
- **保证**：`via` 见证在依赖动作发生时仍在 Counter 上。
- **丢失**：若下游本身要保留，根也不能消费；顺序需人工或协调者执行。
- **适用**：生命周期有明确汇点、要求终止。
- **检查**：A.6 报告为空或仅 `potential`。

### P2 见证保留（keep-as-witness）

- **做法**：在依赖 `T_a` 的所有 `via` 动作完成前，不消费 `T_a` 的见证实例。
- **保证**：依赖动作不会因见证消失而失败。
- **丢失**：见证长期占用；若无最终消费者，Rule-T 不触发；可能累积。
- **适用**：审计、复核需要上游在场。
- **检查**：A.6；`termination_holds` 预期为 false，或需要 P7。

### P3 按消费者复制（copy fan-out）

- **做法**：每次需要独立下游时，用 Rule-P 产生一份新的下游 token（或按角色生产不同 RHS 类型）；共享前驱保留到所有副本产生完毕。
- **保证**：每个副本可独立消费，不互相阻塞。
- **丢失**：副本是不同 `TokenId`；“同一逻辑对象”需域解释；副本数量无结构约束；前驱仍需最终清理。
- **适用**：多个下游独立完成、互不影响（母本 §4.2 fan-out 的结构）。
- **检查**：A.1/A.2；无副本计数。

### P4 ack 汇聚（barrier）

```text
consumer_a: reads shared, produces ack_a
consumer_b: reads shared, produces ack_b
coordinator: consumes shared when count ack_a >= 1 and count ack_b >= 1
```

- **保证**：至少所需数量的 ack 在场时才消费 `shared`；消费与计数在同一次 GateHolds 判定。
- **丢失**：
  - 不能保证 ack 与消费者实例一一对应（`count` 只数类型，`ThingRef` 不被 gate 读取）；
  - ack 自身也是 token，需要后续消费，否则累积并阻止 Rule-T；
  - 参与者可能未真正完成工作就 produce ack，结构不强制；
  - 某个下游失败时，屏障永远不满足。
- **适用**：需要“所有下游完成才归档”的显式汇点。
- **检查**：A.3（ack 是否有 consumer）、A.6（ack 的消费顺序）。

### P5 租约 / 资源

- **做法**：资源 = 可消费 token；acquire = `consume`；release = 重新 seed/produce 一个同类 token。
- **保证**：同一时刻只有一个持有者（资源实例被移除）。
- **丢失**：释放产生新 `TokenId`，资源谱系不连续；崩溃或忘记释放导致资源消失（无自动过期）；释放路径需要 seed 权限。
- **适用**：互斥资源、许可、配额。
- **检查**：A.1/A.2；外部审计。

### P6 外部物化见证

- **做法**：外部系统就绪后 seed/produce `readiness`；消费 gate 写 `when exists readiness`。
- **保证**：就位确认与消费在同一次 GateHolds 判定（无 TOCTOU）。
- **丢失**：`readiness` 的语义由域约定；PACT 不验证外部内容；`readiness` 自身要清理。
- **适用**：consume 触发外部动作。
- **检查**：A.3/A.6；见 `docs/design.md` §8.3。

### P7 归档策略 / 清洁工

- **做法**：指定参与者持有 `consumes` gate，按域策略定期消费。Archival Policy 是域/工具层的约定（可选第五注入点），**等价于**由该参与者显式执行 Rule-E；引擎本身不内置 AP。
- **保证**：清理是显式、可审计的。
- **丢失**：引擎不自动执行（`docs/manual.md` §3.2/§6：无内置 AP）；策略可能误删见证；需要调度者。
- **适用**：长期运行、需要控制累积。
- **检查**：A.3；A.6。

### P8 幂等重试

- **做法**：consume 带 `request_id`；同键同 token 返回首次归档结果；同键异 token 返回 `IdempotencyConflict`。
- **保证**：网络重试不会重复消费。
- **丢失**：不提供跨参与者互斥；不同 `request_id` 的并发**在单写者串行化下**只有一个成功，另一个得到 `TokenNotOnCounter`。该结论依赖宿主契约：原生引擎由单写者 actor 串行化；WASM 两阶段宿主须保证同一实例 `prepare → commit` 之间不插入其他写命令（见 `docs/wasm-local-spike-feasibility.md` §4.3）。宿主违反契约时，第二次提交由 `apply_event` 以 `ConsumeTargetNotOnCounter` fail-closed 拒绝，不会产生重复归档。
- **适用**：客户端重试、at-least-once 传输。
- **检查**：集成测试；无静态检查。

### P9 审计保留（不消费）

- **做法**：审计/报表需求用 Read + Archive 查询或外部索引；不为“清理”而消费仍需观察的 token。
- **保证**：持续可读。
- **丢失**：Counter 累积；不终止；祖先查询需外部索引（B20）。
- **适用**：合规、长期复核。
- **检查**：A.3 的“无 consumer”可能是有意为之。

## 6. 反模式

| 编号 | 反模式 | 后果 / 纠正 |
|---|---|---|
| A1 | 生产完立刻消费前驱 | 有 `via` 下游时直接搁浅；即使无 `via` 也终止未来扇出。先跑 D1/D2 与 A.6 |
| A2 | 把 Read 当租约 | Read 不改变状态、不保留；别人仍可 consume |
| A3 | 把 `depends_on` 当存活保证 | 它是 provenance，允许指向已归档 token；不阻止上游被消费 |
| A4 | 用 `Timestamp` 排消费序 | `Timestamp` 无语义、不保证单调（§1.3）；排序只能靠见证/外部协调 |
| A5 | 把 consume 当 release 但不重新 seed | 资源消失，后续 acquire 不可能 |
| A6 | 为触发 Rule-T 而消费共享 token | 可能搁浅下游或破坏未来生产；终止是派生结果，不是消费理由 |
| A7 | 指望引擎自动清理 | 零隐式动作；Archival Policy 也要显式 Rule-E |
| A8 | 用 `count T` 推断“最后一个读者” | `count` 是当前 token 数，不是读者数；没有 last-reader 事件 |
| A9 | 认为同一 `request_id` 能阻止他人抢先消费 | 幂等只对同键重试生效，不是锁 |

## 7. 实现映射

| 语义 | 位置 | 行为 | 对消费时机的含义 |
|---|---|---|---|
| Rule-R | `crates/pact-core/src/rules.rs` `rule_r` | 只查 Counter + gate；无事件、无状态变更 | 多读者可重复读；消费后 `TokenNotOnCounter` |
| Rule-P | 同文件 `rule_p` | 前驱保持；`depends_on` 记录实际实例 | 保留前驱=保留未来扇出；消费前驱不会删除已产生的下游 |
| Rule-E | 同文件 `rule_e` | Counter 移除 + Archive 追加；无级联 | 单 token 决策；下游是否搁浅由 gate 决定 |
| `via` | `crates/pact-core/src/gate.rs` `GateExpr::Via` | 只查当前 Counter 且 `id ∈ depends_on` | 见证归档即失效；同类型其他实例无效 |
| `count` | 同文件 `Count` | 当前 Counter 计数 | 消费改变阈值；不区分读者 |
| Rule-T | `crates/pact-core/src/wait.rs` `termination_holds` | Counter 空且无 Rule-P-enabled | Counter 上保留任何 token 即不终止（Archive 中的 token 不阻止终止） |
| 消费幂等 | `crates/pact-engine-kernel/src/lib.rs` `prepare_consume` | `request_id` 重放首次结果；异 token 冲突 | 重试安全；不是互斥；并发结论依赖宿主单写者串行化（P8） |
| TokenId 守卫 | 同文件 `used_ids` | Counter+Archive 历史 id 唯一 | 只防 id 复用，不代表存活或引用计数 |
| A.3 | `crates/pact-analysis/src/lib.rs` `consumption_reachability` | 无 consumer 类型清单 | 提示可能永不清理；也可能是有意保留 |
| A.6 | 同文件 `stranding_risk` | `via` 见证被消费的风险 | 消费顺序的静态预警 |
| Archive | `pact-store-model` `apply_event` | 目标在 Counter 时移除并追加 Archive | Archive 不参与 Rule-R/E；用于审计查询与 TokenId 唯一性守卫 |
| 重复消费守卫 | 同文件 `apply_event` | 目标不在 Counter → `ConsumeTargetNotOnCounter` | 日志与状态不一致时 fail-closed，不重复归档 |

**实现刻意不做**：

- 不维护引用计数或读者数；
- 不发 last-consumer 事件；
- 不做 reservation/lease；
- 不做自动清理；
- 不因消费而级联删除下游；
- 不用 `Timestamp` 排序；
- 不校验外部动作是否成功。

这些不是遗漏，而是“消费时机 = 域策略”的边界。

## 8. 域设计评审清单

- [ ] 枚举每个共享类型 `T`：readers、consumers、producers、`via` 依赖者、`count` 阈值。
- [ ] 为每个需要清理的 `T` 指定消费所有者（参与者/角色）与触发条件。
- [ ] 标注 `T` 的保留策略：短期（用完即消费）/ 见证（保留至依赖完成）/ 审计（不消费）。
- [ ] 若存在 `via T`：选择 P1/P2 或 P4；跑 A.6，确认无不可接受的搁浅。
- [ ] 若存在 `count T >= n`：确认消费不会使阈值永久不可达；必要时拆类型或改用 ack。
- [ ] 若 consume 触发外部动作：选 P6（前移）或 saga（后置补偿），不要假设原子。
- [ ] 若域要求终止：确认每个 token（含 ack、readiness、见证）都有最终消费者；否则接受不终止。
- [ ] 若需要重试：统一使用 `request_id`，并明确它不提供跨参与者互斥。
- [ ] 跑 A.3/A.6，记录“无 consumer”是技术债还是有意只读。
- [ ] 把以上决策写入域 ADR 或声明注释，引用 `TC<编号>`。

## 9. 与表达力边界的关系

- B8（两次消费之间的时序）：本文件给出消费顺序与 ack 汇聚的落地程序。
- B17（资源竞争）：P5 是资源租约的具体编码。
- B21（上游存活 / 悬空引用）：D2 + P1/P2 是避免搁浅的判定与模式。
- B22（自动清理）：P7/P9 是清理与审计保留的分工。
- 本文件**不新增 B/X 编号**；引用计数、末位消费者若需范式原语，属扩展候选，另行评估。
- 边界文档回答“能不能表达”；本文件回答“何时消费、由谁消费、顺序如何、怎么验证”。

## 10. 引用落点

| 位置 | 引用方式 |
|---|---|
| `paper/pactv1_rev3.md` §1.4 Rule-P/R/E/T、§1.4.0、§1.6、§1.7.3、A.6 | 规范源；§1.6/§1.7.3 已于 2026-10-24 补写消费时机与 ack 屏障 |
| `docs/design.md` §18 | 索引行与实现映射 |
| `docs/architecture.md` §5 | 质量属性引用 |
| `docs/manual.md` §1.5、§4.5 | 用户视角“何时别消费” |
| `docs/skill-adaptation.md` §4.4 E6 | 只读不消费的适配实例 |
| `examples/README.md` §7.3 | 示例边界引用 |
| `crates/pact-core/src/rules.rs` | Rule-E 注释引用 TC2/TC7/TC13 |
| `crates/pact-core/src/gate.rs` | `Via` 注释引用 TC6/TC7 |
| `crates/pact-core/src/wait.rs` | Rule-T 注释引用 TC13 |
| `crates/pact-analysis/src/lib.rs` | A.3/A.6 注释引用 TC7/TC10/TC11/TC13 |
| `crates/pact-engine-kernel/src/lib.rs` | `prepare_consume` 注释引用 TC9 |

## 11. 维护规则

1. 母本新增或修订消费语义时，先改母本，再同步本文件 TC 表与判定准则。
2. 新增模式必须写明：保证什么、丢失什么、适用条件、静态检查。
3. 代码注释只引用 `TC<编号>`，不复制解释，避免多处漂移。
4. 与母本冲突时以母本为准，并登记到 `docs/design-change.md`。
5. 不得把近似模式登记为等价；没有 last-consumer/引用计数时，不得声称能精确判定消费时机。
