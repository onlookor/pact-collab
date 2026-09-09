# 设计变更记录

## 2026-09-09（第 10 轮，论文 rev2 对齐重构）

- **论文切换**：范式规范源由 `pactv1_rev1.md` 切换为 `pactv1_rev2.md`（design.md 头部引用更新）。rev2 实质差异六项：§1.4.4 良构约束扩展（新增 LHS Uniqueness、Non-Empty LHS；自环合法）、Rule-P Note（前驱互异只按 TokenId）、§1.4.3 Advisory（Seed/Produce 重叠为警告非违规）、§1.7.3 PosNat（count 字面量 `[1-9][0-9]*`）、§3.1 六项良构约束重述、Appendix A.5 重写（三元输出 + 附录 D 前提顺序）。
- **D21 新增（用户选定三项推荐方案）**：良构约束强制风格与 A.5 重构——空 LHS 解析期拒绝 + `EmptyPrecondLhs` 声明期防御；LHS 重复在 `build_domain` 原始序列上检测（BTreeSet 表示层静默去重，唯一强制点）；`pos_nat()` 拒绝 0/前导零 + `ZeroCountLiteral` 防御；A.5 = `precondition_wf` 三元报告（委托 `validate_domain` 单一判定源 + `SeedProduceOverlap` 咨询性警告）；`AnalysisReport.precondition_wf` 字段前置。
- **代码变更**：pact-core 新增 3 个 `DomainError` 变体（`DuplicatePrecondLhs` / `EmptyPrecondLhs` / `ZeroCountLiteral`）+ `validate_domain` 空 LHS 与零计数检查 + `GateExpr::zero_count_refs`；parser `nat()` → `pos_nat()`、花括号形式空 LHS 语法拒绝；pact-analysis 删除 `SourceTypeProduction` / `source_production_legality`，新增 `WfWarning` / `PreconditionWfReport` / `precondition_wf`；engine `AnalysisReport` 字段重命名前置；cli 摘要输出改造（`precondition_wf: 良构（n 项违规，m 项警告）` + 警告逐条）；rules.rs 按 Rule-P Note 更新注释（逻辑零变更，`provided.len() != predecessors.len()` 非冗余——排除同类型多实例冒充）。
- **表示层决策**：`DomainError::UndeclaredType.referenced_by` 由 `&'static str` 改 `String`——`&'static str` 使含该错误的容器无法 derive `Deserialize`（serde 生命周期约束），A.5 报告需内嵌 `Vec<DomainError>` 故必须自有。序列化形态不变（同为 JSON 字符串）。
- **测试**：新增 rev2 专项（parser：`{A+A}→B` 拒绝、空 LHS 拒绝、count 0 / 前导零拒绝、多位 PosNat 接受、自环接受；analysis：`precondition_wf` 良构/违规/警告三元结构、`EmptyPrecondLhs` / `ZeroCountLiteral`（含 Not 嵌套）防御、`SeedProduceOverlap` 正负用例）；更新既有用例（`count b = 0` → `count b = 2`；三接口 e2e 的 analyze 断言改为三元结构）。workspace 97 → **103 项测试全绿**；fmt/clippy 清零。
- **无架构变更**：crate 结构、依赖方向、数据流、部署拓扑不变（architecture.md 不动）。

## 2026-09-07（第 9 轮，CI 接入 + lint 缺口归零）

- **新增 GitHub Actions**（`.github/workflows/ci.yml`）：push(main)/PR/workflow_dispatch 触发；单 job 依次执行 `cargo fmt --all -- --check` → `cargo clippy --workspace --all-targets --locked -- -D warnings` → `cargo test --workspace --locked`；dtolnay/rust-toolchain@stable（含 rustfmt、clippy）+ Swatinem/rust-cache。无设计变更。
- **历史缺口归零**：rustfmt/clippy 不可用问题已解决——沙箱为独立 rootfs（Fedora，无 rustup），经官方 rustup 装入 `~/.local/share/rustup`（stable 1.98 + rustfmt 1.9 + clippy 0.1.98）。
- **全仓 lint 规范化**：①`cargo fmt --all` 一次性基线（rustfmt 2024 风格，纯机械重排）；②clippy 1.98 全量 36 处告警清零——31 处机械修复（`is_none_or`、`keys()` 迭代、`io::Error::other`、`from_ref` 代 clone、未用导入、布尔比较等），5 处手工（doc 列表缩进 ×3 文件、`ClausesByParticipant` 类型别名、测试模块更名 `mod unit` 消 inception、`&PathBuf`→`&Path`、cli_e2e 失败路径 kill+wait 防僵尸进程）。
- 回归：stable 1.98 工具链下 fmt --check ✓ / clippy -D warnings ✓ / **97 项测试全绿**。

## 2026-09-06（第 8 轮，推送前全面安全审视）

- **范围**：整个 workspace（9 crate + docs + 仓库文件面），AGENTS.md 推送/部署前强制约束。方法：项目安全基线 → 偏差图 → 源到汇三遍过；横切扫描 `unsafe`/进程/环境变量/密钥样式串/SQL 拼接/路径拼接。
- **结论**：路径穿越（域名 `[A-Za-z0-9_-]{1,128}` 强校验）、SQL 注入（turso 全参数化）、不安全反序列化（仅 serde 强类型）、命令注入（生产代码零进程 spawn）、密钥泄露（仓库零命中、bearer 不落日志、`.gitignore` 覆盖 target/data/sock）均清洁；core/parser/analysis/compose 保持零 IO 纯度。
- **唯一 LOW 级发现（已修复，用户选定"现在修复"）**：[CWE-208] `ServerState::authorize` 的 bearer token 比较为非恒时的 `String ==`（TCP 模式下可计时侧信道）。修复：pact-http `server.rs` 新增零依赖恒时比较 `ct_eq`（长度差计入 diff + 字节异或折叠），并附单元测试锁定语义。**无设计变更**（§9.4"接口层校验"语义不变）。
- **设计内部署注意（记录，非变更）**：读/管理端点不校验 bearer 为 D11 信任模型既定（TCP 模式下本机进程可读状态/装卸域，v1 默认 unix socket 为边界）；`pact.toml [auth] tokens` 明文，部署建议 chmod 600；`--bearer` 经命令行本机 ps 可见，v2 换签名密钥；client URL 拼接未编码但影响仅 404/405；turso BETA 由 D13 隔离层对冲。
- 回归：workspace 累计 **97 项测试全绿**（新增 1 项 `ct_eq` 单元测试）。

## 2026-09-05（第 7 轮，阶段 5：无人值守场景验证）

- **无设计变更**：阶段 5 为纯验证阶段，未改动引擎/核心语义。新增 `pact-mcp/tests/unattended_e2e.rs`（两场景合并为单 `#[tokio::test]` 顺序执行），落实 FR11 与 §11 阶段 5 验收——测试进程只做"外部启动机制"（拉起 daemon + N 个真实 `pact-mcp` stdio 子进程），此后零 Rule-S/P/R/E 介入，agent 经 P1/P2/P3 策略 + Rule-W（订阅推送 + 轮询兜底）自驱至 Rule-T。
- **验证发现（印证既有设计，非变更）**：域加载与种子注入之间存在窗口，此时 Counter=∅ 且无 token，`termination_holds` 平凡为真、`pact_status` 返回 `terminated=true`。这与 §"Terminated 为信息性事件、不锁定域（种子仍可注入）"一致，属**预期语义**；无人值守 agent 驱动层须自行防护——仅当 `archive_size>0`（确有真实工作发生）才承认终止，避免 worker 在播种前误判退出。修复落在测试的 agent 驱动器，引擎零改动。
- **agent 驱动层竞态修复（测试内）**：①上述过早终止防护；②Terminated 一次性推送与轮询判定竞态——轮询可能先于推送判定终止，故终止后开 500ms 宽限窗口排空在途推送，确保 `notified_terminated` 不漏计；③P2 汇消费收紧为"仅当无 produces 门或生产预算耗尽"才允许，防止有生产门的 agent 误食自己的见证前驱破坏 consume-after-produce 纪律。
- **测试基架**：两场景合并单 runtime 顺序执行，规避同进程多 runtime 并发 reap `pact-mcp` 子进程的 tokio 全局孤儿队列陷阱。
- 阶段 5 完成：workspace 累计 96 项测试全绿；unattended_e2e 20/20 连跑稳定。§11 阶段表阶段 5 标记为已完成。

## 2026-09-04（第 6 轮，阶段 4：pact-http + pact-cli + pact-mcp）

- **D19 新增（用户决策）**：pact-http 提前至阶段 4，与 pact-cli、pact-mcp 同批交付；MCP 与 CLI 不内嵌引擎，统一经 `pact_http::client`（REST over unix socket）接入同一 daemon；原规划的 `pact-server` 组装 bin 取消，daemon 由 `pact daemon start` 与 `pactd` 提供。接口等价性由构造保证（三接口只是同一 REST 的形态转换）。
- **D20 新增**：CLI 域操作一律 `--domain/--participant/--token/--type` flag（放弃 §9.3 原稿 `pact enabled <participant>` 位置参数简写）；全局连接参数 `--unix/--tcp/--bearer/--json`；MCP v1 仅 stdio，Streamable HTTP 移交联网演进（v2）。
- §1 crate 清单更新：删 pact-server 行；pact-http 注明含类型化 client 与 pactd bin；依赖方向改为 `http → engine`、`mcp/cli → http(client)`、`cli → parser/analysis/compose/core`（离线命令）。
- §9.1/§9.2/§9.3 按实现回填：MCP 经 REST 转发；`{d}:analyze/:compose` 段内冒号在 `/domains/{d}` 统一入口分发（matchit 限制，对外 URL 不变）；独立 `POST /validate`；CLI 完整命令/flag 清单逐条对齐实现。
- §11 阶段表：阶段 4 完成（95 项测试全绿：新增 HTTP daemon e2e、CLI e2e×2、MCP stdio e2e）；阶段 5 范围调整为纯无人值守场景验证（pact-http 已提前交付）。

## 2026-08-26（第 5 轮，阶段 3：pact-store + pact-engine）

- **store schema 修正**（§7.3 落地偏差，代码注释已先行标注）：
  - `events` 表主键由 `seq` 改为 `(domain, seq)`——seq 是域内单调序，跨域必然碰撞；
  - `DomainLoaded` 事件携带 `seq`/`ts`——使"日志逐行 +1 单调"不变量对所有行统一（含域加载行）。
- **engine 命令全集落定**（§8.1 枚举补全）：新增 `ListDomains`、`Shutdown`（全部域终态快照后退出）；`Compose` 定名 `ComposeAndLoad`（组合 + emit 回写 + 加载一体）；`Subscribe` 返回 `broadcast::Receiver<Notification>`。
- **域替换语义明确**：同名额重载 = 重置会话状态 + 清理陈旧快照 + seq 续接；恢复时快照仅在 `snapshot.seq ≥ 最后一次 DomainLoaded.seq` 时有效，且只重放该事件之后的事件（防替换后旧状态复活）。
- **Terminated 事件一次性**：仅在 `termination_holds` false→true 转移时追加并广播，重复求值不重复落盘。
- **恢复 fail-fast**：日志中部损坏（完整行非法 JSON）或声明不可解析时，actor 以统一 `EngineError::Recovery` 排空命令队列（需人工介入）；尾部半行照 §7.4 截断自愈。
- **Clock 无 chrono 依赖**：`SystemClock` 以 civil-from-days 手算实现 ISO-8601 UTC（修复实现中月份变量遮蔽分钟的 bug，测试锁定闰日纪元）。
- 阶段 3 完成：31 项新增测试全绿（store 17 / engine 单元 4 / engine 集成 10），workspace 累计 80 项；纯度核验通过（pact-core 依赖树仅 serde）。

## 2026-08-26（第 4 轮，论文 rev1 对齐 + 阶段 2）

- **论文切换**：范式规范源由 `pactvfinal.md` 切换为 `pactv1_rev1.md`（design.md 头部引用更新）。
- **D15 回滚**：论文 rev1 以源生产约束（Rule-P 前提 `Precond(T) ≠ ∅` + 声明期 WF-3 拒绝）原生解决 Rule-T 终止问题，临时豁免（termination_holds 排除空前置组合）失去存在意义，代码与文档全部回滚。
- **D17 新增**：源类型生产约束三层强制——(1) `rule_p` 运行时前提检查（`RuleError::SourceTypeProductionDenied`）；(2) `validate_domain` 声明期拒绝（`DomainError::SourceTypeProduceGate`，WF-3）；(3) `is_source_type` 查询函数 + A.5 静态分析独立复核。`termination_holds` 按 rev1 Rule-P-enabled 三条件重写；`enabled_productions` 不返回源类型。
- **D18 新增**：compose C2 实践语义——共享类型前置一致性判定为"双侧都声明规则且集合不同才冲突，单侧声明合法"（论文 §1.8 示例 D₂ 共享 design 而不重复声明其前置的规范语义）。
- 影响范围与同步更新：LIFECYCLE 黄金域改为 rev1 版（analyst 由 produce gate 改为 seed authority、operator via 子句移除）；pact-core 单元测试补源类型约束专项；property 宽容域去掉源类型 produce gate；parser 测试补 WF-3 用例。
- 阶段 2 完成：pact-analysis（六项分析 A.1–A.6，petgraph SCC/拓扑）+ pact-compose（四条件组合）落地，20 项测试全绿。
- 阶段 1 状态更新为"2026-08-26 按 rev1 更新"（27 项测试全绿）。

## 2026-08-25（第 3 轮，阶段 1 实现）

- **D15 新增**：Rule-T 终止判定的"enabled"排除空前置组合（源类型生产）——字面语义下含源类型 produce gate 的域永不终止，与论文 Part II 轨迹 t₁₀/t₁₃ 矛盾；排除后判定在 Counter=∅ 前提下等价于 Counter 为空。黄金测试 + 专项单元测试（`termination_semantics`）锁定。
- **D16 新增**：parser 宽容度决策（段序严格/尾分号与空段宽容/`->`、`//` 注释、GateExpr 优先级 not > and > or 为扩展）。
- 实现落定与接口微调：
  - `GuardedAction` 新类型（Read|Consume）类型层面强制 GateHolds 作用域（§1.4.0）；
  - `Token.ty` 线上序列化字段名 rename 为 `"type"`（§6.2 契约，测试锁定）；
  - `Effect::token_id()` 辅助方法（引擎广播/持久化用）；
  - `GateExpr::type_refs()`、`precond_of()` 辅助查询（良构校验/分析用）；
  - pact-core dev-dependencies 增补 serde_json（仅测试，验证线上 JSON 契约；不违反纯度白名单——白名单约束 runtime 依赖）。
- 阶段 1 状态：已完成（26 项测试全绿；纯度核验通过）。见 design.md §11。
- 环境缺口：pact-dev 容器无 rustup/sudo，rustfmt 与 clippy 不可用，待补齐后补跑。

## 2026-08-25（第 2 轮）

- **D13 新增**：派生索引引擎采用 turso（原 Limbo，Rust 重写的 SQLite，现 BETA）。可行性依据：索引在本架构中可随时从 JSONL 真相源重建，beta 风险被对冲；收益为纯 Rust 工具链与 async API。经 pact-store 内部 `IndexStore` trait 隔离，保留 rusqlite 后备退路。存储布局中 `index.sqlite` 改为 `index.tursodb`。
- **D14 新增（pact-core 纯度审计结论）**：
  - Rule-T 终止判定（`termination_holds`）、Rule-W 可满足性判定（`enabled_productions`）、域良构校验（`validate_domain`）从引擎/parser 层移入 pact-core 纯函数——语义收口，引擎只编排不判定；
  - 新增 `Effect` 枚举与 `CounterState::apply`，状态变更路径唯一；
  - 明确纯度硬性清单：无 IO / 无日志（tracing 宏禁用）/ 无时钟 / 无随机 / 无全局可变状态 / 依赖白名单（serde 仅 derive）；
  - `rule_s` 签名去掉 `state` 参数（论文 Rule-S 前提不含 Counter 条件）。
- 新增 §6 领域数据格式：`.pact` 声明文件示例、Token/ArchiveEntry JSON、动作请求/响应与结构化错误格式、通知事件格式、查询响应格式（三接口共享）。
- §2、§3 扩充为阶段 1 两模块的完整接口定义（类型 + 函数签名）。
- §10 测试策略改为四层组织（单元/属性/集成/端到端+回归），对齐 FR13。
- 文档标题去掉"（当前有效）"后缀。

## 2026-08-25（第 1 轮）

- 初始设计建档（D1–D12，crate 划分，核心类型，MCP 工具清单，REST 端点，测试策略，六阶段计划）。
