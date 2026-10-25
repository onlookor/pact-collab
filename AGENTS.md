- 需求文档为 `docs/requirements.md`，只保留当前有效需求，需求变更追加记入 `docs/requirements-change.md`。
- 设计文档为 `docs/design.md`，只保留当前设计，设计变更追加记入 `docs/design-change.md`。
- 架构文档为 `docs/architecture.md`，只保留当前架构，架构变更追加记入 `docs/architecture-change.md`。使用 `documenting-software-architectures` 技能。
- 用户手册为 `docs/manual.md`，只保留当前有效内容。
- 技能适配文档为 `docs/skill-adaptation.md`，记录 `~/projects/skills` 下 RE / SE 两技能的 PACT 形式层适配（产物清单、验收清单、三档粒度差异矩阵、表达边界），只保留当前有效内容，变更追加记入 `docs/skill-adaptation-change.md`。
- 论文母本为 `paper/pactv1_rev3.md`，是范式的唯一规范来源；项目内只保留当前有效版本（历史版本以 git 记录与变更文档为准）。**任何对母本的改动必须事前获得用户显式确认**，不得随概括性指令默认授权。
- 文档标识符必须“先定义、后使用”：`D*`、`FR*`、`TC-*`、`W-*`、`B*`、`X*` 等字母+数字编号只能来自权威文档，不得在对话中临时自创后于文档中裸用；新增编号先在 `docs/architecture.md` 附录 A 登记，再引用。跨文件引用必须带命名空间前缀（如 `TC-` / `W-`），同一字母序列不得在两处表示不同含义；架构分层、上手阶梯等描述性概念一律用名称，不用编号标签；`*-change.md` 保留历史编号，不回改。

- 每一轮工作前，读取 `docs/` 目录下的文档。

- 每一轮工作后，更新 `docs/` 目录下的文档。

- 随变更组织单元测试、集成测试、端到端测试、回归测试。

- 推送到github之前以及部署之前，做一次全面的安全审视。