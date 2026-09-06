# 架构变更记录

## 2026-09-04（第 4 轮，阶段 4：接口层落地）

- **接口接入拓扑变更（D19）**：pact-mcp 与 pact-cli（在线）不再规划内嵌引擎，统一作为 pact-http daemon 的 REST 客户端；REST 通道成为通往引擎的唯一状态变更路径，三接口能力等价由构造保证。§1 总览图按此重绘（含图例更新：MCP stdio 为双向 JSON-RPC 流，箭头仅示发起方）。
- **组件集变更**：原 `pact-server` 组装 bin 取消；可执行物收敛为三个静态二进制——`pact`（CLI + daemon start）、`pactd`（裸 daemon）、`pact-mcp`（stdio MCP server）。§1/§3 相应更新。
- **传输范围收窄（D20）**：MCP v1 仅 stdio；Streamable HTTP 随联网演进（v2）落地，§3 部署视图移除该行。
- §2.1 数据流补上 REST 一跳（pact-mcp → REST → pact-http → engine）；§2.3 启动恢复主体由 `pact-server` 改为 `pact daemon start / pactd`。
- 结构类别覆盖不变：模块视图（§1 分层 + crate 依赖，见 design.md §1）、C&C 视图（§1/§2）、分配视图（§3）各一；无新增质量属性手段（§5 仅既有属性落点不变）。

## 2026-08-26（第 3 轮，阶段 3 落地核对）

- 清除 §2.1/§2.3/§3 残留的三处 `index.sqlite`/SQLite 旧称，统一为 `index.tursodb`/派生索引（turso）（D13 早已决策，本轮落地实现后核对补正）。
- §2.3 启动恢复流程按实现补全：快照有效性条件（seq ≥ 最后一次 DomainLoaded）、中部损坏行 fail-fast 语义（对应引擎恢复失败统一应答）。
- 阶段 3（pact-store + pact-engine）实现完成，架构图与数据流描述与代码一致，无结构性变更。

## 2026-08-25（第 2 轮）

- 存储层派生索引引擎由 rusqlite/SQLite 改为 turso（设计决策 D13），存储布局 `index.sqlite` → `index.tursodb`，经 `IndexStore` trait 隔离。
- 架构总览图增加图例（箭头语义、分层说明），对齐 documenting-software-architectures 技能的七规则之三（避免歧义）。
- 质量属性表更新：语义收口至 core（D14）、四层测试组织、索引可重建性表述。

## 2026-08-25（第 1 轮）

- 初始架构建档（分层视图、数据流、部署视图、联网演进、质量属性落点）。
