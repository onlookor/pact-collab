# 架构变更记录

## 2026-10-20（第 14 轮，阶段 9：分发产物落地）

- **§6 新增「分发形态（阶段 9 已完成）」**：`dist/` 提供三条部署路径——容器（多阶段 musl 构建、非 root uid 10001、`/var/lib/pact` 卷、`EXPOSE 8787`、`HEALTHCHECK`）、systemd（`pact` 用户、`StateDirectory`/`RuntimeDirectory`、20s 停机窗口、`ProtectSystem=strict` 等加固）、静态包（`x86_64-unknown-linux-musl` tar.gz + `SHA256SUMS`）。三者载荷为同一批静态二进制（`pact`/`pactd`/`pact-mcp`/`pact-runtime`）。
- **§6 部署图与归属边界更新**：图中 `dist/` 标注「阶段 9 已交付」；归属边界由「阶段 9 未开始」改为「已交付」，L1 生产化标「阶段 8 已交付」；§5 层边界行同步。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机进程/部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变；本轮仅新增**部署/分发描述与产物**，不改数据流、启动恢复与任何代码路径。

## 2026-10-20（第 13 轮，D25 子项 3：域级 ACL 与读审计落地）

- **§2.5 新增读审计与域级授权数据流**：Rule-R 成功后由引擎 actor（单写者）向 `<domain>/<instance>.read-audit.jsonl` 追加一行 + fsync；失败即 `500`（fail-closed）；审计流不产生事件、不推进 `seq`、不参与重放 / Rule-T / 快照 / 压缩。REST 入口在 ACL 非空时由 Bearer 令牌反查参与者并按 `(domain, instance)` 授权（支持 `domain/*`），覆盖生命周期 / 动作 / 查询 / compact / 审计视图，`GET /domains` 按授权过滤。
- **组件内新增（不改变分层与依赖方向）**：`pact-store` 新增 `audit.rs`（`ReadAuditRecord`）与 `Store::append_read_audit` / `read_audit_tail`（独立文件 append + fsync；尾部读取）；`pact-engine` 新增 `EngineConfig.read_audit`、Rule-R 审计写入、`Command::QueryReadAudit` + `Engine::read_audit`；`pact-http` 新增配置 `read_audit` / `[auth.acl]` + `Config::validate`、`ServerState::with_acl` 与鉴权辅助、读审计视图路由、client 方法。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **§5 质量属性更新**：安全边界行改为「启用 ACL 后全端点实例授权（401/403、列表过滤、配置校验），未启用保持信任声明」；可运维性行由「子项 1+2 已实现」扩为「**子项 1+2+3 已实现**」，并登记可选读审计流与只读视图。
- **无结构性变更**：crate 划分、依赖方向、单机单进程部署、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变。本轮变化限于**接口层鉴权 / 审计旁路 + 存储新增独立文件族**。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **155 项全绿**（基线 145 + 本轮 10）。

## 2026-10-20（第 12 轮，D25 子项 1：日志压缩与 schema 版本化落地）

- **组件内新增（不改变分层与依赖方向）**：`pact-store` 新增压缩能力——`WireEvent{version, #[serde(flatten)] event}`（事件与线上格式统一带顶层 `version`）、`logmeta` 水位旁路清单（`<instance>.logmeta.json`）、`rewrite_retaining`（tmp+fsync+rename 原子前缀重写）、`IndexStore::prune_events`、`Store::compact`；`Snapshot` 扩为**自足锚**（`version`/`seq`/`dl_seq`/`declaration`/`state`/`consumed_requests`）。`pact-engine` 增 `Command::Compact` + 公开 `Engine::compact` + `EngineConfig::compact_on_start/compact_keep_last` + `recover_instance` 的 `snapshot_is_anchor` 分支；`Notification` 增顶层 `version`。`pact-http` / `pact-cli` / `pact-mcp` 仅**增入口**（`POST .../compact`、`pact compact`、`pact_compact`）。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **§1 架构总览图更新**：存储层框补 `.logmeta.json（压缩水位）`，`.snapshot.json` 标注为「自足锚」。
- **§2.3 启动恢复流程更新**：补入 logmeta 水位读取、快照锚/重放双分支、`SnapshotRequiredForCompactedLog` 护栏、`base_seq > 0` 时索引补 `prune_events` 自愈、`compact_on_start` 自动压缩、事件 schema 版本兼容规则。
- **新增 §2.4 日志压缩数据流**：五步序（写自足快照为提交点 → 写 logmeta 水位 → 原子重写日志 → 裁剪索引 → 重置追加句柄），水位单调不回退；崩溃于任一中间点落到两个一致态之一（未截断超集态 / 已截断+水位+快照）。
- **§5 质量属性更新**：审计完整性行补「压缩后审计视图 = 自足快照 + 保留尾部事件，被丢弃前缀不再可查」；可恢复性行补「快照锚恢复 + 水位护栏 + 版本 fail-fast」；可运维性行由「子项 2 已实现」扩为「**子项 1+2 已实现**」。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机单进程部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变。本轮变化限于**存储层文件族 + 恢复流程分支 + 接口面新增一个运维端点**，属既有分层内的落地细化。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **145 项全绿**（基线 131 + 本轮 14）。

## 2026-10-20（第 11 轮，D25 子项 2：运维接口落地）

- **§5 可运维性行**（由「未实现」转**子项 2 已实现**）：daemon 增加三个**免令牌**运维端点 `/health`、`/ready`、`/metrics`（Prometheus 文本），可选 `pid_file`（陈旧检测），SIGINT/SIGTERM 优雅停机（停止 accept → 有界 10s drain → 终态快照）。明确**未实现**：日志压缩/保留与 schema 版本、域级 ACL、读审计扩展。
- **组件内新增**（不改变分层与依赖方向）：`pact-http::server` 增加 `ConnTracker`/`ConnGuard`（在途连接计数，供有界 drain）与 `watch` 停机广播；`serve_unix` / `serve_tcp` 签名增加停机接收器与计数器，**仅 `pact-http` 内部与测试调用点**受影响。`pact-engine` / `pact-store` / `pact-core` 零改动，依赖方向不变。
- **无结构性变更**：crate 划分、部署拓扑（单机单进程）、引擎被动性（D5）、单一状态变更通道（D6/D19）均不变。

## 2026-10-20（第 10 轮，术语精化：「域实例」）

- **触发**：用户要求把指 D27 实例键的「实例」统一写「域实例」（避免与工程实例混淆，为将来类型-实例分离留余地）；见 `docs/design-change.md` 第 21 轮。
- **改动**：`architecture.md` §2.3 启动恢复步骤「逐实例（<domain>/<instance>.events.jsonl）」→「**逐域实例**（…）」。§5 多实例承载行的「Counter 实例」「实例键」等为论文/机制术语，保留不改。
- **形式层与实现零变化**：仅中文措辞；`docs/pact*` 母本未触碰。

## 2026-10-20（第 9 轮，阶段 10 实施：域双键实例化落地）

- **§5 多实例承载行更新**（由第 8 轮的「未实现」转**已实现**）：补入**存储布局定案**——域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、声明原文 `declaration.pact` 同目录）；索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键）；顶层遗留单层文件 fail-fast（`LegacyLayoutDetected`）；同一 `data_dir` 以 flock（`‹data_dir›/pactd.lock`）保证单写者；非空实例再加载默认拒绝（`DomainBusy`，须 `force`）；实例键以路径段承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` ≡ `default`），键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。
- **§1 架构总览图更新**：引擎层框由「域会话表（`domain → {Domain, CounterState, 订阅者}`）」改为「**声明注册表（`domain → 声明`）+ 实例会话表（`(domain,instance) → {声明, 状态, 订阅者}`）**」；存储层框由「`<domain>.events.jsonl` / `<domain>.snapshot.json` / `index.tursodb`」改为「`<domain>/<instance>.events.jsonl` / `<domain>/<instance>.snapshot.json` / `<domain>/declaration.pact` / `index.tursodb (domain,instance) 双列`」。
- **§2.3 启动恢复流程更新**：补入「先取 data_dir 独占 flock（`pactd.lock`，第二实例拒绝启动）」「顶层遗留单层文件 → fail-fast（`LegacyLayoutDetected`）须人工迁移」「逐**实例**恢复」「重放后恢复**声明注册表**（读 `<domain>/declaration.pact`）」。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机进程/部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。本轮变化限于**存储布局细节、引擎状态表形态与接口路由形状**，属既有分层内的落地细化。
- **质量属性关联**：审计完整性行维持（`DomainLoaded` 存档声明原文）；可恢复性行维持（索引用 `(domain,instance)` 双列，可重建自愈）。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **123 项全绿**（基线 110 + β 专项）。

## 2026-10-20（第 8 轮，D27 域双键实例化立项：形式层裁定与论文 rev3）

- **本轮无结构性变更**：crate 划分、单机进程/部署拓扑、单一状态变更通道（D6/D19）、数据流、纯函数语义层均不变；引擎被动性（D5）不变。**零代码改动**。
- **§5 质量属性表新增一行**「多实例承载（FR19，D27，未实现）」：同一域声明可并发承载多个 Counter 实例；实例为接口层一等键，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表；同一 `data_dir` 上多实例须以 flock 保证单写者；存储布局（单层文件名 vs 域子目录）待裁决。
- **背景**：域名今天既是定义标识又是会话键，`LoadDomain` 的替换语义（design.md §8.1）在两个并行项目共用同一域声明时会**销毁**对方 Counter 与快照——这是 D27 立项的直接动因。评估见根目录 `model-c-impact.md`（五层 × 三档判定）。
- **形式层依据**：领域专家裁定该拆分**不构成形式冲突**，属论文 §1.3「Physical implementation of the Counter」既有的实现自由（Q1 (a)）；论文据此修订为 **rev3**（9 处增补/术语替换，零文法、零语义变化，`diff -u` 已核），母本引用全仓同步至 rev3。裁定要点见 `docs/design-change.md` 第 18 轮。
- **架构影响待评估**：存储布局（单文件 vs 子目录）、引擎会话表形态（声明注册表 + 实例会话）、事件日志与索引的实例维度、REST 路由形态——均待 3 项细节裁决后在「阶段 10 域双键实例化」实施时评估并回填本文件 §4（数据流 / 启动恢复）与 §5。
- **可运维性行（NFR7/D25）**仍标"未实现"（属阶段 8），且其验收与实例化耦合（见 `model-c-impact.md` §6）。

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
