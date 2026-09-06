- 需求文档为 `docs/requirements.md`，只保留当前有效需求，需求变更追加记入 `docs/requirements-change.md`。
- 设计文档为 `docs/design.md`，只保留当前设计，设计变更追加记入 `docs/design-change.md`。
- 架构文档为 `docs/architecture.md`，只保留当前架构，架构变更追加记入 `docs/architecture-change.md`。使用 `documenting-software-architectures` 技能。
- 用户手册为 `docs/mannual.md`，只保留当前有效内容。

- 每一轮工作前，读取 `docs/` 目录下的文档。

- 每一轮工作后，更新 `docs/` 目录下的文档。

- 所有开发工作在 distrobox 容器 `pact-dev` 中进行。

- 随变更组织单元测试、集成测试、端到端测试、回归测试。

- 推送到github之前以及部署之前，做一次全面的安全审视。