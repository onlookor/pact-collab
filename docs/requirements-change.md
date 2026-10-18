# 需求变更记录

## 2026-10-18（第 7 轮，消耗原子性、幂等与物化边界）

- **触发**：对"消耗原子性"组织评审后，用户裁决采纳"文档化实现契约 + 补幂等 + witness-token 示例（2a）与 outbox 语义（2b）并行"组合，要求契约表述自足、不依赖外部讨论语境。
- **新增 FR15 消耗原子性、幂等与物化边界**：明确引擎对 Rule-E（消费）在并发、崩溃、重试与外部副作用四个维度上的承诺与不作为——提交点（JSONL fsync）前无部分效果、提交点后派生步骤尽力而为、Rule-E 支持可选幂等键、引擎不执行外部副作用。
- **FR9 补两条**：事件自足（Consumed 事件携带 token 全量快照，单行即可审计与重放）、提交契约（JSONL fsync 为提交点，其后派生动作不回滚）。
- **FR10 补一条外部副作用边界**：`ThingRef` 不透明、引擎不执行外部动作，"consume 成功" 不等于 "外部动作成功"，物化确认由 witness-token 或 saga 语义在域外承载。
- **FR12 补一条**：Rule-E 消费请求支持可选 `request_id` 幂等键；同键同 token 重试返回首次归档结果，同键异 token 报 `IdempotencyConflict`（HTTP 409）。
- **FR13 集成测试补幂等**：新增 request_id 去重、跨重启幂等表重建、同键异 token 冲突三类断言。
- **FR14 域数四 → 五**：通用域新增 `external_materialization.pact`（witness-token 物化确认模式）。
- **NFR2 强化**："每条事件自足"——单行日志即可独立审计与重放，不依赖同域其他行。
- **§6 范围外新增一条**：分布式事务 / 2PC / 分布式共识、以及由引擎执行外部副作用，均不在实现范围内（单写者 actor 已满足线性化需求，见 design.md D23）。

## 2026-10-15（第 6 轮，通用示例与用户手册）

- **触发**：用户指出既有 `docs/skill-adaptation.md` 一族的适配结构与 requirements-engineering / software-engineering 两技能强绑定，需补一套与技能无关的需求工程/软件工程通用结构，并配 examples 向用户说明整个 PACT 范式系统的使用场景与方法。
- **新增 FR14 通用示例与用户手册**：`examples/domains/` 四份通用域、`examples/fragments/` 两份组合片段、`examples/offline/run.sh` 与 `examples/online/run.sh` 两条上手路径（各带录制轨迹）、`examples/README.md` 场景导航；`docs/manual.md` 为权威用户手册。明确二者分工（可运行教学路径 / 参考权威）与交叉引用义务。
- **§6 范围外新增一条**：示例不接入 CI 回归——`examples/**/*.pact` 不由 cargo test 遍历校验，示例正确性由人工执行与录制轨迹对照保证（决策 D22）。
- **用户裁定的范围边界**：通用域含 RE / SE / ReAct-agent 三类，**不含**医疗路径域；SE 通用域取中等粒度；examples 深度为离线三件套 + 在线 daemon 全生命周期轨迹；用户向文档落点为 `examples/README.md` 与 `docs/manual.md` 两者。
- 既有 FR1–FR13、NFR1–NFR6 无实质变更。

## 2026-09-09（第 5 轮，论文 rev2 对齐）

- 范式规范来源切换：`pactv1_rev1.md` → `pactv1_rev2.md`。
- FR4：Rule-P 前驱互异补充"只按 TokenId 判定"（rev2 Rule-P Note）。
- FR5：新增 count 字面量 PosNat 约束（`[1-9][0-9]*`，拒绝 0 与前导零；缺席语义用 `not exists T` 表达，Normalization rule）。
- FR6：良构校验由"四条零散规则"重述为 §3.1 六项约束（新增 LHS Uniqueness、Non-Empty LHS 两条；自环 `A → A` 显式合法）；明确解析期语法拒绝 + 声明期 validate_domain 防御两层。
- FR7：A.5 由"源生产合法性"重定义为"前置与源生产良构性"（rev2 A.5）——§3.1 六项约束违规清单 + Seed/Produce 重叠咨询性警告，三元输出 Violations/Warnings/WF(D)，按附录 D 为其余分析的前提、置于报告首位。

## 2026-08-26（第 4 轮）

- 范式规范来源切换：`pactvfinal.md` → `pactv1_rev1.md`（论文最新版解决了 D15 指出的 Rule-T 终止问题，以 rev1 为准）。
- FR4：Rule-P 补充前提 `Precond(T) ≠ ∅`（源类型不可生产）；Rule-T 按 rev1 Rule-P-enabled 三条件定义。
- FR6：良构校验补充 WF-3——源类型（无前置规则）不得声明 produce gate。
- FR7：静态分析由五项扩为六项（新增 rev1 A.5 源生产合法性）。
- FR8：片段组合一致性由三条件扩为四条件（新增 C4 源生产封闭）。

## 2026-08-25（第 2 轮）

- 文档标题去掉"（当前有效）"后缀（三文档统一）。
- 新增 FR12 领域数据格式：`.pact` 声明文件为注入点唯一载体；定义 Token/ArchiveEntry/动作请求响应/结构化错误/通知事件的 JSON 线上格式；三接口共享同一套格式；时间戳 ISO-8601 UTC、字段 snake_case。
- 新增 FR13 测试组织：随变更组织单元/集成/端到端/回归四类测试（对应 AGENTS.md 新增规则）。

## 2026-08-25（第 1 轮）

- 初始需求建档（FR1–FR11，NFR1–NFR6，范围外清单）。
