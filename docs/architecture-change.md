# 架构变更记录

## 2026-10-19（第 7 轮，阶段 7：L3 生产运行层落地）

- **§6 落地核对**：标题去"未实现"、增注"L3 运行层已实现"；新增「落地形态（阶段 7 已完成）」条——`crates/pact-runtime/` 交付传输抽象（`ParticipantClient` / `RestClient` / `McpClient`）、驱动循环 `drive()`、编排 `Orchestrator`、副作用执行器 `SideEffectExecutor`、TOML 配置驱动 bin；部署图标注由"（目标）"改为已交付（L3 lib + bin）。
- **§6 依赖方向细化**：由 `pact-runtime → pact-mcp / pact-http` 修正为 `pact-runtime → pact-http / pact-engine / pact-core`（lib 层）；`pact-mcp` 仅作可执行文件由编排 spawn、不作 lib 依赖——与 design.md §1 依赖行一致（D24 行依赖细化）。
- **§5 质量属性更新**：层边界行"生产驱动逻辑收于新增 `pact-runtime`…"改为"已实现，经 REST 接入 daemon"。
- **无结构性变更**：crate 划分（除 `pact-runtime` 由"未实现"转"已实现"，见 design.md §1）、单机进程/部署拓扑、单一状态变更通道、数据流、纯函数语义层均不变；引擎被动性（D5）不变。可运维性行（NFR7/D25）仍标"未实现"（属阶段 8）。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **110 项全绿**（基线 105 + 本轮 5）；engine/store/core/三接口零改动。

## 2026-10-19（第 6 轮，生产运行层与生产化边界）

- **新增 §6「生产运行层与分发」**：在既有单机架构之上补 **L3 生产运行层**（新增 crate `pact-runtime`），给出目标进程拓扑图、依赖方向（`runtime → mcp / http`，不反向）、进程拓扑（systemd 托管的常驻编排进程拉起 daemon 与 agent 子进程；MCP 仍由 agent 宿主以 stdio 子进程拉起）、归属边界与 MCP 提供形态（权威恒为 daemon）。
- **§5 质量属性增两行**：可运维性（NFR7 / D25，未实现）、层边界（D24–D26）。
- **无既有结构变更**：crate 划分（除新增 `pact-runtime`，见 design.md §1）、依赖方向、单机进程/部署视图、数据流、单一状态变更通道均不变；§2 数据流、§3 v1 部署视图、§4 联网演进不动。
- **未实现标注**：§6 与 §5 新增行均标注"未实现"，本轮仅为架构裁决落档（对应 design.md D24–D26、requirements.md FR16–FR18）。

## 2026-10-18（第 5 轮，提交契约与事件自足落地核对）

- **§2.1 数据流补正**：Rule-P 路径标注"追加 JSONL(fsync，**提交点**) → 派生索引投影(turso，尽力而为) → state.apply"，补提交点前后失败语义（提交点前无部分效果、提交点后不回滚）与 Rule-E 可选幂等键说明（对应 design.md D23）。
- **§5 质量属性两行更新**：审计完整性行补"每条事件自足（Consumed 携 token 全量快照），单行即可审计与重放"；可恢复性行补"索引滞后由启动一致性校验触发重建自愈"。
- **无结构性变更**：crate 划分、依赖方向、进程/部署拓扑、单一状态变更通道、纯函数语义层均不变；本轮仅数据流时序标注与质量属性表述随实现契约补正。

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
