# 技能适配变更记录

本文件只追加，不改写已登记批次。当前有效内容见 `docs/skill-adaptation.md`。

## 2026-10-15（第 4 轮，通用示例与用户手册落盘后的边界澄清）

- **背景**：用户指出本文件一族的适配结构与两技能强绑定，另需一套与技能无关的通用结构 + examples（已落盘 `examples/`，需求侧 FR14、设计侧 D22，见 `docs/requirements-change.md` 第 6 轮与 `docs/design-change.md` 第 14 轮）。
- **§0 表新增两行**：`docs/manual.md`（权威用户手册）与 `examples/`（领域无关通用示例）均不承载本项工作，避免读者把技能适配层与通用示例层混为一谈。
- **§0 新增"与 `examples/` 的关系"段**：明确二者领域相邻而绑定对象不同——示例不绑定技能目录、粒度压低、服务于范式系统用户上手；本文件的 4 件 `.pact` 落在 `~/projects/skills/`、逐条对应技能文档命题、SE 分三档。
- **`docs/design.md` 决策编号引用**：§0 表中 D1–D21 → D1–D22。
- **对技能侧产物零影响**：`~/projects/skills/` 下 4 件 `.pact` 未改动，§1–§10 的产物清单、验收清单、三档粒度差异矩阵、机械校验结论均不变。引擎代码零改动。

## 2026-10-15（第 3 轮，论文改动回退，规范母本维持原文）

- **裁决**：用户指出第 2 轮同步论文 §1.4.3 / A.5 超出实现方范畴，三处（§1.4.3 Advisory / A.5 伪代码 / A.5 Remediation）逐字回退；论文维持参与者级 Advisory 原文。
- **对本文档的影响**：§10 落点清单中"论文三处同步"改写为"论文维持原文 + 实现登记为保守超集"；第 2 轮"论文……同步"仅存档不生效。
- **保留**：`lib.rs` 注释/Display 类型级表述（与回退后的论文恰构成保守超集关系）、跨参与者回归块、design.md D21 类型级定案（措辞修正为保守超集表述）。
- **验证**：纯文档回退，代码零改动，第 2 轮测试结论（103 项全绿）不变。
- **纪律沉淀**：与 `design-change.md` 第 13 轮同——规范母本改动须事前单列确认，不随概括性指令默认授权。

## 2026-10-15（第 2 轮，`SEED_PRODUCE_OVERLAP` 处置定案：采 (b) 改注释承认现状）

- **裁决**：用户选定第 1 轮 §10 登记的处置 (b)。理由：本次适配中该警告确实起到了提示部署决策点的作用（SE 单独部署须保 seed 权威、组合部署应撤除，见 §6.3），收窄实现反而会静默掉真实裁决点；实现方向为假阳性不漏报，无正确性风险。
- **落点**：`lib.rs` 文档注释与 `Display` 改为类型级判定表述；论文 §1.4.3 Advisory 段与 Appendix A.5 伪代码/Remediation 同步；`design.md` D21(4) 补记；`analysis_domains.rs` 增补跨参与者回归块（警告 `participant` 字段断言为 seed 持有者）。明细见 `docs/design-change.md` 2026-10-15 批次。
- **对本文档的影响**：§10 由"待裁决"改写为"已定案"（含终态行号 185-191 / 202 / 228-242）；§6.3 现象引用的警告文本随 `Display` 更新。
- **验证**：fmt --check / clippy -D warnings / 全量测试 103 项全绿（净增 0 项，扩充既有用例）；命令与 CI 一致，宿主机工具链 cargo 1.98.1（distrobox 不可达偏差沿用第 1 轮登记，容器内复跑仍列 §9.2 续办 3）。
- **`.pact` 产物零影响**：警告为咨询性且 JSON 序列化形态不变，4 份片段与 3 份合并域的 analyze/compose 结果不受本轮影响，无需重跑。

## 2026-10-14（第 1 轮，RE / SE 两技能 PACT 形式层适配建档）

- **建档理由**：本项工作是 PACT 的**外部应用**（把 `~/projects/skills` 下两个技能目录的规范命题投影为 `.pact` 域声明），引擎代码零改动、不新增 crate、不改接口，既不属 `design.md`（引擎实现设计）也不属 `architecture.md`（引擎架构）。用户指示"如果既有设计和架构文档不合适，则单独建一个文档记录及其对应的变更文档"，遂建 `docs/skill-adaptation.md` + 本文件。
- **模型选型**：动手前先比较 qwen3.8-max / qwen3.8-flash / GLM5.3-flash / seed-evolving 四候选，经用户选定 **qwen3.8-max** 主刀。判据是该任务需同时持有 rev2 文法约束、两份技能母本的条款语义、以及组合性 C1–C4 三条线索并做长程一致性推理，属长上下文 + 强约束遵从型任务。
- **产物（形式层 4 件）**：
  - `requirements-engineering/pact/re_fragment.pact` **rev2 重写**（149 行；8 类型 / 6 前置规则 / 8 角色位 / 2 seed 权威）。原片段按 rev1 语法书写，rev2 增加了"前置规则 LHS 类型两两相异"约束（§1.4.4 LHS Uniqueness），故整体重写而非增量修补。
  - `software-engineering/pact/se_tier{1,2,3}_*.pact` **新建**（档 I 40/31/15/9、档 II 41/32/18/9、档 III 41/32/20/9）。按《部署规模裁剪模板》§3 三档装配画像各出一份。
- **产物（配套层）**：新建 `software-engineering/pact/README.md`（六节导览）；SE 根 `README.md` 四处（§2 新增"形式层"子节、§4 依赖关系、§5 阶段 A 第 2 步、§6 当前状态）；`编译规范-changlog.md` 追加 2026-10-14 批次；`todo.md` 新增工作项 3.10。RE 侧 `stages/s2`–`s6`、`SKILL.md` §4.6、`README.md`、`PROGRESS.md` 对齐（明细见 `skill-adaptation.md` §1.2、§8）。
- **机械校验**：4 份单片段 `pact analyze` 六项全绿（`well_formed: true`、violations 0、warnings 0、unreachable 0、`gate_coverage` 0、`structural_deadlock` 0、`stranding_risk` 0）；RE × 三档 `pact compose` 三次均 exit 0 / stderr 空 / 无冲突；三份合并域终检良构（11 源类型，47/48/48 可达，0 不可达，gate_coverage 0，死锁 0，滞留 0），唯一输出为 1 项咨询性警告。
- **发现并修复的实质矛盾（1 处语义级）**：`requirements-engineering/stages/s6-management.md` 正文称 `change_controller consumes old specification`，与片段声明 `consumes re_baseline, consumes change_approval` 冲突。推理确认**片段正确**（`Precond(specification) = {raw_requirement}`；旧 `specification` 早在 S5 被 `validator` 消费；变更到达时 Counter 上是 `raw_requirement` + `re_baseline`）。同步发现正文缺 `change_requester` / `change_analyst` / `ccb` 三个参与者。修复后 `change_approval` 只有一个 producer gate（`ccb`）且 `Precond(change_approval) = {change_impact_report}`，**"Agent 永不代 CCB 批准"红线由结构强制而非散文**。其余为形态级不一致（`reads ∅` 非法形态、front-matter 箭头 `->` 与 `→` 不统一、`s5` front-matter 漏 `release_manager`、`SKILL.md` §4.6 覆盖不全、rev1 指向残留 3 处）。
- **验收清单与三档粒度差异矩阵**：按用户指示收入 `skill-adaptation.md` §3 与 §4。矩阵核心结论——**档间差异不在结构命题，只在角色位与阈值**；档 II → 档 III 类型集与前置图零增删；档 I → 档 II 只增 `contract_confirmation` 及其唯一前置规则；裁剪模板三合法轴中只有**粒度**轴在 rev2 有词汇。
- **两项非零咨询输出的定性**：`consumption_reachability` 30/31 项逐项归为常驻注册表资产 8 项 / 审计只读证据 10 项 / 终态审计工件 12–13 项（§6.1）；RE 侧 `raw_requirement` 无消费者是溯源敏感性的必需代价，副作用是 Counter 永不为空 ⇒ Rule-T 在 RE 单独部署时不可达（§6.2，**不掩盖**）。`SEED_PRODUCE_OVERLAP(re_baseline)` 定性为设计意图，裁决点在部署实例化时，须记入裁剪 ADR（§6.3）。
- **表达边界登记**：SE 侧 E1–E7（E7"角色位互不兼任"不可表达，是档 III 最需人类复核的缺口）；rev2 不表达的七类维度表；RE 侧 `change_approval` 不入 `depends_on`（改由 Archive 承载批准链）。全部落配套内容层执行，**未在 `.pact` 中造任何语法**。
- **PACT 仓库侧线索（未改动）**：`crates/pact-analysis/src/lib.rs:224-238` 的 `SeedProduceOverlap` 实现把 `produced` 收集为全局集合，**宽于** `lib.rs:185-187` 文档注释所述"同一参与者"，故跨参与者组合也报警（假阳性方向，不漏报）。既有测试 `analysis_domains.rs:425-451` 只覆盖同参与者场景。本轮不修改（超出用户请求范围，且改行为会牵动 D21 的 A.5 定义与既有测试语义），登记两个可选处置并倾向"改注释以匹配实现"（§10）。
- **执行偏差**：`AGENTS.md` 要求在 distrobox 容器 `pact-dev` 内开发，本次 `distrobox` 命令不可达，改用宿主机 `./target/debug/pact` 只读离线校验；PACT 仓库 `crates/` 零改动。接地验收时须在容器内复跑 analyze × 4 与 compose × 3 并留痕。
- **测试**：本项工作未新增引擎测试——`.pact` 是数据产物不是代码，其验证即 §5 的 analyze/compose，已全绿。未触碰 `crates/`，既有单元/集成/e2e 测试语义不变。
- **安全审视**：本轮产物为纯文本域声明与文档，无网络、无进程、无密钥、无路径拼接、无 `unsafe`；PACT 仓库代码零改动，故 2026-09-06（第 8 轮）的安全审视结论继续有效。`.pact` 文件不含可执行内容与外部引用，`compose` 输出仅写入 `/tmp`。
