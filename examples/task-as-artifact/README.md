# 任务即制品示例：任务板与任务 DAG

本示例演示 PACT 的两种**当前可写**用法：任务记录作为 `ThingRef`，任务状态或位置作为 `TypeName`。
它们都是“把任务当制品”，但都不是“任务图执行器”。完整论述见专题
[`docs/task-as-artifact.md`](../../docs/task-as-artifact.md)；它也是本示例的阅读前提。

## 1. 这个示例演示什么，不演示什么

**演示：**

- 任务身份放在 `ThingRef`（如 `task://BOARD-42`），范式不读它；
- 任务在流程中的位置/状态放在 `TypeName`，由前置图与 gate 治理；
- 两种写法：通用状态列（`task_board.pact`）与任务产出类型（`task_dag.pact`）；
- `via` 的直接前驱见证、多读者、`Archive` / `depends_on` 审计。

**不演示（也不承诺）：**

- 同一任务恰好执行一次；
- 两个下游 token 按任务实例配对；
- 自动推进、失败重试、超时、动态加任务、OR 分支、原子多输出。

这些是能力边界，不是本示例的配置项。原因见专题 §6 与
[`docs/expressiveness-boundaries.md`](../../docs/expressiveness-boundaries.md) 的 B2、B4、B15、B16、B20、X4。

## 2. 文件

| 文件 | 用法 | 类型表示什么 | 适合 |
|---|---|---|---|
| [`task_board.pact`](task_board.pact) | 用法一：状态作为类型 | 看板列：`task_open / task_doing / task_in_review / task_done` | 任务种类不确定、所有任务共享同一块看板 |
| [`task_dag.pact`](task_dag.pact) | 用法二：任务产出作为类型 | 任务产出：`triage_report / build_artifact / test_report / release_record` | 任务拓扑固定、每个位置有自己的准入门 |
| [`run.sh`](run.sh) → [`trace.txt`](trace.txt) | 离线 analyze 轨迹 | — | 不需要 daemon，验证两份声明良构 |

## 3. 运行

```bash
cargo build -p pact-cli

examples/task-as-artifact/run.sh              # 打印
examples/task-as-artifact/run.sh > trace.txt  # 录制
```

实测输出见 [`trace.txt`](trace.txt)。两份域都 `exit=0`；`task_board` 的 `stranding_risk=1`
是刻意的顺序约束，不是缺陷：

- `releaser` 读/消费 `task_in_review` 时带 `via task_doing`，要求“生产这份评审时用的那枚
  `task_doing`”仍在 Counter 上；
- `archivist` 有权消费 `task_doing`；
- 如果 `archivist` 先消费，`releaser` 的 `consume task_in_review` 会被 Gate 拦下
  （`produce task_done` 仍可成立，因为 `produces` 子句不带条件）。范式只报风险，不替你排序
  （详见 [`docs/token-consumption-timing.md`](../../docs/token-consumption-timing.md)）。

## 4. 用法一：任务板

`task_board.pact` 的类型是**通用状态列**，不是任务身份。任务身份在物引里：

```text
TypeName = 任务当前在哪一列
TokenId  = 这一枚状态 token 的实例身份
ThingRef = task://BOARD-42
```

### 4.1 在线动作序列（需要 daemon）

以下命令演示“位置在类型、身份在物引”。token id 需从上一条命令的输出里取；
完整 daemon 用法见 [`examples/online/run.sh`](../online/run.sh)。

```bash
pact domain load examples/task-as-artifact/task_board.pact

pact seed --domain task_board --participant planner --type task_open \
          --thing-ref 'task://BOARD-42'

pact produce --domain task_board --participant planner --type task_doing \
             --predecessors <task_open-id> --thing-ref 'task://BOARD-42'

pact consume --domain task_board --participant planner --token <task_open-id>

pact produce --domain task_board --participant worker --type task_in_review \
             --predecessors <task_doing-id> --thing-ref 'task://BOARD-42'

# 多读者：reviewer 不改变 Counter，也不要求任务归它所有
pact read --domain task_board --participant reviewer --token <task_in_review-id>

pact produce --domain task_board --participant releaser --type task_done \
             --predecessors <task_in_review-id> --thing-ref 'task://BOARD-42'

pact consume --domain task_board --participant releaser --token <task_in_review-id>
pact consume --domain task_board --participant archivist --token <task_doing-id>
pact consume --domain task_board --participant stakeholder --token <task_done-id>
```

要点：

1. **`produce` 不消费前驱。** `task_doing` 生产出 `task_in_review` 之后仍留在 Counter；
   想让它消失必须显式 `consume`（§1.4 Rule-P Note）。
2. **gate 不按任务 id 路由。** `worker` 只被授权 `reads task_doing`；它拿到哪一枚
   `task_doing`、是不是 `BOARD-42`，由参与者自己读物引判断。
3. **同一枚 `task_doing` 可以产出多枚 `task_in_review`。** 若两个 worker 同时动手，
   两枚评审都会成立。要避免重复，只能靠参与者幂等、串行纪律或外置执行器；
   范式不保证恰好一次（专题 §6）。
4. **`via` 只认直接前驱。** 它保证“产生评审时用的那枚 `task_doing` 现在还在”，
   不保证“任务 42 的结果配任务 42 的审批”。

## 5. 用法二：任务 DAG

`task_dag.pact` 把每个任务映射成它的产出类型，前置图就是任务 DAG。这正是母本 §4.1
的构造编码（限 SOAD：单输出、AND 结构）：

```text
活动 A_i        → 类型 output-of-A_i
(A_i, A_j)      → T_i → T_j
能力映射        → gate 的 produce/read/consume
无入边的源活动  → seed_authority（对应类型是源类型，不可 produce）
```

在线动作序列与用法一相同，只是物引换成任务产出：

```bash
pact domain load examples/task-as-artifact/task_dag.pact

pact seed --domain task_dag --participant intake --type intake \
          --thing-ref 'task://DELIVERY-7/intake'

pact produce --domain task_dag --participant triager --type triage_report \
             --predecessors <intake-id> --thing-ref 'task://DELIVERY-7/triage'

pact consume --domain task_dag --participant triager --token <intake-id>

pact produce --domain task_dag --participant builder --type build_artifact \
             --predecessors <triage_report-id> --thing-ref 'task://DELIVERY-7/build'
```

后续 `test_report`、`release_record` 同理。`trace.txt` 显示六项分析全 0：拓扑固定、
每个非源类型都有 producer、每个类型都有 consumer。

注意：并发处理多个交付时，前置匹配只按 `TypeName` + `TokenId`，不按物引。
`triage_report` 可能来自交付 A，而下游 `build_artifact` 的物引写的是交付 B；
类型级 AND 不会发现这种错配。这正是专题 §6 的“同任务配对”缺口，不是本示例的配置问题。

**但“六项全 0”不等于“任务图执行器”。** 它只说明声明在结构上是闭合的；执行保证
（恰好一次、重试、超时、动态任务）仍不在范式内。

## 6. 与基于图的流程编排、agent 协调对比

| 问题 | 基于图的流程编排（LangGraph / DAG 调度 / BPMN） | 本例的 PACT |
|---|---|---|
| 节点是什么 | 任务 / 动作 / agent 步骤 | 类型化的状态或任务产出（CPN 里是 place） |
| 边是什么 | 控制流，常带数据依赖 | 存在性前置：生产 B 时 A 必须在场 |
| 谁决定下一步 | 调度器 / 图执行器 | 参与者自己看 Counter（Rule-W），没有中心派发 |
| 推进方式 | 自动、级联、可重试 | 显式动作；`produce` 不消费前驱，消费是另一个动作 |
| 同一任务只跑一次 | 通常由引擎保证 | 不保证：同一前驱可被多次生产下游 |
| 多任务并发配对 | 运行时按实例配对 | 类型级 AND；同任务配对不在范式内（B14、B20） |
| 失败 / 重试 / 超时 | 引擎原语 | 范式外（B2、B6） |
| 动态加任务 / 改图 | 支持 | 类型必须静态声明（B16、B19） |
| 审计 | 日志，通常要额外关联 | `Archive` + `depends_on` + `via` 原生 |
| 静态分析 | 有限，取决于图与代码 | 六项声明级分析（附录 A / D） |
| agent 怎么协作 | 调度器派活 / 消息 / handoff | 共享任务状态；多读者；不需要 agent 直接通信 |

一句话：

> **图编排回答“下一步跑什么”；PACT 回答“现在什么存在、谁可以动”。**
> 把任务放进物引，PACT 得到的是受治理的任务板；要任务图执行器的执行保证，
> 仍需要外部执行器。

## 7. 边界与下一步阅读

- 为什么“任务即制品”不是范式扩展：专题 §1–§3；
- 完整对比与能力缺口：专题 §4–§6；
- 边界编号与编码代价：`docs/expressiveness-boundaries.md`（B2、B4、B5、B6、B14、B15、B16、B19、B20、X1、X3、X4）；
- 共享 token 何时可以消费：`docs/token-consumption-timing.md`；
- 用户手册的工具分类对比：`docs/manual.md` §1.2。
