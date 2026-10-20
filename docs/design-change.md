# 设计变更记录

## 2026-10-20（第 22 轮，过程产物移除与阶段 8 排期重挂）

- **触发**：用户「移除这两个文件，然后继续开发」。阶段 10 交付后，模型 C 评估与咨询的**过程产物**已并入正式文档，不再需要留存。
- **移除**：`model-c-impact.md`、`model-c-expert-brief.md`（根目录临时产物，此前随 `a65ce34` 入库）删除。
- **悬空引用消解（仅正文指向，未改结论）**：`design.md` D27 行末的「评估与执行序见 …」与 §11 阶段 8 行的「（见 `model-c-impact.md` §6）」改写为自足表述——阶段 8 与 D27 的耦合点（(1) 压缩部分、(3) 全部按双键重述、`:compact` 路由随实例段重设计）已内联在该行。
- **排期**：阶段 8（D25 生产化三项）解除暂停（其暂停原因即与实例化耦合，现已随阶段 10 消除），成为下一实施目标；阶段 8 行状态仍为「未开始」。
- **零代码改动**：本轮不触碰任何 crate；`docs/pact*` 母本未动。

## 2026-10-20（第 21 轮，术语精化：「域实例」）

- **触发**：用户指出上一轮 G2 把「域」改为「实例」的措辞会与工程上的「实例」混淆，且不排除将来细化出更多类型-实例分离（如分布式 counter），要求统一改为「域实例」。
- **术语定案**：中文文档/注释中，指 D27 实例键的概念一律写「**域实例**」——即论文 `Counter instance`（§1.3 描述性短语），**与论文已废弃的英文 `domain instance` 无关**，亦不替代论文术语；映射登记于 `design.md` D27「术语登记」。论文母本 `docs/pactv1_rev3.md` 未触碰。
- **改点（正文措辞，零语义）**：`design.md` D23（2 处）、§1 crate 注释、§6.3（411）、§7.1（445）、§8.1（521/523）、§8.2（527）、§8.3（531）；`requirements.md` FR9/FR10/FR12/FR15/NFR2；`manual.md` Rule-T 行；`architecture.md` §2.3 启动恢复步骤。**有意保留**：论文条款名「单实例作用域」及 G1 CPN 行的论文锚定表述（保持与论文一致，不用中文「域实例」）。
- **代码侧（无行为变化）**：注释口径「每实例/同一实例/逐实例/不锁定实例」→「每域实例/同一域实例/逐域实例/不锁定域实例」——`pact-store/src/{log,event,store}.rs`、`pact-engine/src/{engine,lib,notify}.rs`、`pact-http/src/dto.rs`；另修 `pact-core/src/domain.rs` 中误用「域实例」指 **Domain 声明**的文档注释 → 「域声明」，消除与中文「域实例（= 实例键）」的撞义。
- **形式层语义不变**：`pact-parser`/`pact-analysis`/`pact-compose` 零改动；`pact-core` 仅 `domain.rs` 一处文档注释措辞（无类型/无行为变化）。
- **验证**（2026-10-20 收尾重跑）：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警、`cargo test --workspace` **123 passed / 0 failed**。

## 2026-10-20（第 20 轮，多实例约束覆盖核对后的文档补齐 G1/G2/G3）

- **触发**：见 `docs/requirements-change.md` 第 12 轮。核对基准 = rev3 与多实例直接相关的条款（§1.3 两半、§1.4.1 唯一性、Rule-T scope note、§4.5/§B.2 CPN scope、§1.8 及 §4.1–4.3 术语）；结论：C1/C2/C3/C6 全覆盖，C4/C5 未登记（G1）、幂等作用域措辞滞后（G2）、母本 §1.4.1 Rationale 与 Definition 措辞不一致（G3）。
- **G1（补登记）**：`design.md` 决策 **D12** 行与 §11 **阶段 6** 行补入论文 §4.5/§B.2 的 **单实例作用域**约束——每个 Counter 实例独立编码、库所与迁移不跨实例共享、跨实例协同不入 `N(D)`；作为 `pact-cpn`（阶段 6，未开始）的实现前置条件。
- **G2（措辞收窄）**：把「以域名为会话键」的残留文本统一收窄为「以实例为会话键」（D27/FR19）。改动点：决策 **D23**（「同域其他行」「同域内重复 `request_id`」）、§6.3 幂等段（411 行）、Consumed 自足段（489 行）、§8.1 启动恢复（518 行「逐域取最后一次 DomainLoaded」）与 Rule-E 处理顺序（520 行「同域 `request_id`」）、§8.2 通知（524 行「每域一个 broadcast」）、§8.3 Rule-T（528 行「不锁定域」）、§1 crate 注释（48 行「多域会话」→「多域/多实例会话」），以及 **§7.1/§7.2 存储布局**修正为 D27 布局（`<domain>/<instance>.events.jsonl` + `<instance>.snapshot.json` + `declaration.pact` + `pactd.lock`；`seq` 明确为**每实例**单调）。
- **G3（母本就地修订，不新增版本）**：经用户**特批**，就地修订 `docs/pactv1_rev3.md` §1.4.1 的 **Rationale**——原文 "tokens carry globally unique IDs" 与同节 Definition（"`TokenId` is unique within its Counter; implementations may strengthen this to cross-instance uniqueness…"，121 行）不一致，改为与 Definition 一致（保留「实现可加强为跨实例唯一」的 Q4 口径）。**不产生修订版本**：仍为 rev3（文件就地修改，不新建 rev4、不改任何 rev3 指针）；rev2→rev3 **差异块由 9 增至 10**（新增该 1 处纯措辞一致性修订，零语义）。
- **代码侧（无行为变化）**：仅修正 6 处**文档性注释**口径（`pact-store/src/log.rs`、`pact-store/src/event.rs` ×2、`pact-engine/src/notify.rs`、`pact-engine/src/engine.rs` 配置项注释、`pact-http/src/dto.rs`）——「每域」/「同域」→「每实例」/「同一实例」。无代码语义改动。
- **形式层不变**：`pact-core`/`pact-parser`/`pact-analysis`/`pact-compose` 零改动；本轮仅文本与注释。
- **验证**（2026-10-20 收尾重跑；沙箱宿主链接器方案）：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警、`cargo test --workspace` **123 passed / 0 failed**（与阶段 10 基线一致）。

## 2026-10-20（第 19 轮，阶段 10 实施：域双键实例化落地，路径 β）

- **触发**：用户「按计划实施」——实施阶段 10（`docs/design.md` §11 行 10，D27 路径 β）。**范围仅 D27 双键本体**：存储双列 + 域子目录、引擎声明注册表 + 实例会话、接口全链、通用护栏；**阶段 8 仍暂停**（schema 版本化、日志压缩、health/metrics/PID、ACL/审计读视图本轮不做）。
- **D27 行更新**：把「派生待定」节替换为「派生细节定案 + 已实现（见 §9.5 / §11 行 10）」——旧日志 fail-fast、实例键路径段承载、`DomainLoaded` 不新增 `instance`、声明原文另存 `declaration.pact`、护栏 flock + `DomainBusy`。
- **实现决策（本轮推导 R1–R10）**：
  - R1 新增 `InstanceKey{domain, instance}`（定义于 `pact-store`，engine 复用；`Display = "domain/instance"`）；
  - R3 `DEFAULT_INSTANCE = "default"` 常量置于 `pact-engine`；
  - R4 `pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**（形式语义不变）；
  - R5「实例非空」判据 = `last_seq > dl_seq`（`DomainLoaded.seq`）；
  - R6 `POST /domains` = declare + instantiate `default`；R7 `GET /domains` 响应由 `Vec<String>` 改 `Vec<DomainListing{name, instances}>`；
  - R8 recover 顶层遗留旧文件 → `LegacyLayoutDetected` fail-fast；R9 flock 独占锁落在 `pact-http::daemon` 启动路径（新依赖 `fs4`）；
  - R10 引擎内部 `declare()` / `instantiate()` 私有方法 + 公开命令 `LoadDomain` 与新增 `Instantiate`。
- **逐层落地**：
  - **pact-store**：`key.rs`（`InstanceKey` + `validate`）、`error.rs`（`InvalidInstanceName` / `LegacyLayoutDetected` / 共享谓词 `valid_key_name`）、`log.rs`（`domain_dir` + 三参 `events_path`/`snapshot_path` + `declaration_path`）、`store.rs`（重写：`logs: Mutex<BTreeMap<InstanceKey, (EventLog, u64)>>`、`RecoveredInstance{key, events, truncated}`、先探测旧布局再两层扫描、`write_declaration`/`load_declaration`、`last_seq(&InstanceKey)`、`list_instances`/`list_domains`）、`index.rs`（`events(domain, instance, …, PRIMARY KEY(domain,instance,seq))`、`tokens(token_id PK, domain, instance, …)`；`open()` 自愈：无 instance 列则 DROP 重建）。
  - **pact-engine**：`declarations: BTreeMap<String, DeclarationEntry>` + `instances: BTreeMap<InstanceKey, InstanceSession>`；`InstanceSession{declaration, domain, state, last_seq, dl_seq, events_since_snapshot, terminated, consumed_requests, notify}` + `is_occupied()`；`EngineError` 删 `DomainNotFound`、增 `InstanceNotFound`/`DomainNotDeclared`/`InstanceExists`/`DomainBusy`；facade 11 动作方法收 `key: InstanceKey`、新增 `instantiate`/`list_domains`、`load_domain` 增 `instance`/`force` 并先做 `validate_domain_name` + `validate_instance_name`（防路径穿越）；`views.rs` 增 `DomainSummary.instance` 与 `DomainListing`；`notify.rs` `Notification.instance`（非 Option）。
  - **pact-http**：路由增 `POST /domains/{d}/instances` 与 `/domains/{d}/instances/{i}` 任何段 + 实例级镜像组；所有 handler 抽 `*_inner(&ServerState, key, …)` + default/instance 双薄 wrapper（auth 统一在 `*_inner`，实例路由同受令牌约束）；`dto.rs` 增 `LoadDomainReq.instance/force`、`InstantiateReq`、`DomainsResp → Vec<DomainListing>`、`DomainInfo.instance`；`client.rs` `base_path(domain, instance)`（None/`default` ⇒ 默认族）+ 22 方法加 `instance` + `instantiate`；`daemon.rs` 增 `acquire_data_dir_lock`（`<data_dir>/pactd.lock`）；`From<EngineError>` 增 `InvalidDomainName`/`InvalidInstanceName` → 400（原落 500）。
  - **pact-cli**：`DomainCmd::Load` 加 `instance`/`force`；新增 `Instantiate`/`Instances`；`Info`/`Unload`/Seed/Produce/Read/Consume/`CounterCmd::Ls`/`ArchiveCmd::Ls`/`GatesCmd::Show|Enabled|Status` 加 `--instance`；`list` 打印 `name [instances…]`。
  - **pact-mcp**：15 → **16** 工具，新增 `pact_domain_instantiate`；`LoadIn.instance/force`；各 In 加 `#[serde(default)] instance: Option<String>`；`ComposeIn` **移除** `instance`（compose 只作用 default，消除 clippy dead_code）。
  - **pact-runtime**：`ParticipantClient` 增 `fn instance()`；`RestClient`/`McpClient` 增 `instance` + `with_instance`；新增自由函数 `bound_ready(listings, domain, instance)`；`drive()`/`executor` 域发现改用 `bound_ready` + `list_domains`；`Orchestrator` 增 `instance` + `with_instance`；bin `FileConfig` 增 `#[serde(default)] instance`。
- **测试（FR13 四层）**：改造既有锚点（`engine_integration.rs` 全文重写、`daemon_e2e.rs`、`cli_e2e.rs`、`mcp_e2e.rs` 工具数 15→16、`unattended_e2e.rs`、`runtime_e2e.rs`）+ 新增 β 专项：engine 集成 3（`same_declaration_supports_multiple_instances` / `replacing_declaration_does_not_disturb_running_instance` / `declaration_backfill_supports_new_instances_after_restart`）；daemon e2e 2（`instance_routes_are_isolated` / `invalid_key_names_rejected_without_touching_fs`）；cli e2e 1（`instance_commands_and_default_fallback`）；runtime e2e 1（`instance_binding`，两实例并发自驱）；daemon 单元 1（`data_dir_lock_is_exclusive`）。
- **两处安全修复（详见「安全审视」）**：① flock 护栏因 `fs4::fs_std::FileExt::try_lock_exclusive` 返回 `Result<bool>` 而被 `Ok(false)` 静默放行 → 改为检查 bool；② 非法域名/实例名原落 HTTP 500 → 加 `From<EngineError>` 映射到 400 `InvalidDomainName`/`InvalidInstanceName`。
- **验证输出摘录**（沙箱，宿主机工具链包装 `/tmp/hostlink`）：
  ```
  cargo fmt --all --check                               → 无输出（FMT_OK）
  cargo clippy --workspace --all-targets -- -D warnings → 零警告
  cargo test --workspace                                → TOTAL_PASSED=123（基线 110 + β 专项，全绿）
  ```
- **文档**：`requirements.md`（FR19 细化 + 实现状态）、`design.md`（D27 行、§9.2 实例路由 + 错误码小节、§9.3 CLI 实例命令、§11 阶段 10 状态 → 已完成）、`architecture.md`（§1 图：引擎声明注册表 + 实例会话表 / 存储域子目录布局；§2.3 启动恢复；§5 多实例行 → 已实现 + 存储布局定案）、`manual.md`（§7.1 存储布局、§8.1 CLI 表、§8.2 REST 表、§8.3 MCP 16 工具、§9.4 错误码、§10 故障排查、§11 限制 9/10）、四份 change 文档。
- **规范母本未触碰**：`docs/pact*`（rev1/rev2/rev3）全程未改；本轮为实现落地，不新增范式语义（不触发边界判据 4）。
- **沙箱构建偏差**：沙箱 rootfs 无链接器/C 编译器，构建测试经宿主机 `/run/host` 工具链包装（`/tmp/hostlink`，`ld`→`ld.bfd`）完成，**未安装任何包、未改 /usr**（同第 15/17 轮做法）。

## 2026-10-20（第 18 轮，域双键实例化的形式层裁定与论文 rev3 切换）

- **触发**：用户提问「两个同一个域的项目并行如何处置？」→ 指示「暂停实施阶段 8，评估模型 C 的全面影响，包括论文（可能需要领域专家）」。评估产出根目录 `model-c-impact.md`（五层 × 三档判定 + 阶段 8 交叉标注 + 专家问题清单）。
- **触发事实（评估核出）**：域名今天**既是定义标识又是会话键**——`LoadDomain` 为替换语义（同名额重置会话、清快照、seq 续接），两个并行项目共用同一域名时第二次 load 会**销毁**第一本的 Counter 与快照；域名直接作文件名与 REST 路由键；`{d}:analyze`/`{d}:compose` 已占用 `:suffix` 命名空间；全仓无 `data_dir` 锁。
- **领域专家裁定**（咨询简报 `model-c-expert-brief.md`，答复全文带回）：Q1 **(a)** 概念承认——以 §1.3「实现自由」形式落地；Q2 **(a)** 术语消歧——指「声明/组合结果」的 `domain instance` 统一改称 `domain declaration`；Q3 **(a)+(c)** 限定重述——§1.3 集中说明 + Rule-T 增 scope note；Q4 保留「globally unique」但**作用域收窄至该 Counter**，实现以 ULID 加强为跨实例唯一；Q5 确认**无任何论文级不变量或定理依赖「一个域名至多一本 Counter」**；Q6 确认 CPN 编码需加单实例作用域说明；Q7 **不新增正式术语**（工程侧用描述性短语 `Counter instance`）。总括口径：不涉 SOS 规则/GateHolds/GateExpr/良构约束/静态分析算法的语义修改；实例划分属实现自由**不越界**；母本修订须用户显式确认。
- **用户裁决**：破例授权改论文为 rev3；**立项模型 C**；实现走**路径 β**（`pact-store`/索引显式 `(domain, instance)` 双列 + 引擎**声明注册表**，`declare` 与 `instantiate` 分离）。
- **论文 rev3 落地**（新增 `docs/pactv1_rev3.md`，`pactv1_rev2.md` 留作历史）：5 类改动共 **9 个 diff 块**——§1.3 增列「Counter instantiation and partitioning」条款；§1.4.1 定义段重写（TokenId 作用域）；Rule-T 后增「Scope note」；§4.5 与 §B.2 各增 CPN 单实例作用域说明；§1.8 定理行及 §4.1/§4.2/§4.3 四处 `domain instance` → `domain declaration`。**`diff -u` 核对结论：仅此 9 处增补/替换，零文法变化、零语义变化、零行丢失**（§x.y 编号不变，全部既有引用仍有效）。
- **母本引用同步**：`design.md` / `requirements.md` / `manual.md` / `skill-adaptation.md` / `examples/README.md` / `examples/README.zh-CN.md` 的 `docs/pactv1_rev2.md` 指针全部改为 rev3（历史变更文档中的 rev2 记述保留不动）。
- **文档落地**：`design.md` 新增 **D27**（域双键实例化与形式层裁定：术语登记 + 三项待定细节）＋ §11 新增**阶段 10**、阶段 8 验收注明与实例化耦合；`requirements.md` 新增 **FR19 域双键实例化**、FR10 多域会话行补实例维度、NFR7 补多实例边界（同 `data_dir` 须 flock 保证单写者）。
- **零代码改动**：本轮全程未改任何 crate；`pact-core` 纯度白名单不变；**阶段 8 实施保持暂停**（其三项中 (1) 压缩与 (3) ACL/审计与实例化强耦合，(2) 的 `:compact` 路由与既有 `:suffix` 分发冲突，须随双键一并重设计）。
- **待裁决（D27 细节 3 项，用户）**：存储布局（单层文件名 vs 域子目录）、默认实例名取值、实例与声明版本的绑定（创建时快照声明 vs 引用最新）。

## 2026-10-19（第 17 轮，阶段 7 `pact-runtime` 落地）

- **触发**：用户批准实施阶段 7（`docs/design.md` §11 行 7；D24），按 `spec.md`/`tasks.md`（**临时规划产物**，实现并更新 `docs/` 后删除）执行；硬约束为 pact-core / pact-store / pact-engine / 三接口 **零改动**。
- **新增 crate `crates/pact-runtime/`**（lib `pact_runtime` + bin `pact-runtime`，根 `Cargo.toml` members 追加）：
  - `src/client.rs`：`ParticipantClient` trait（`Send + Sync`）、`WakeStream`、`Endpoint`（`Unix`/`Tcp`）——传输层抽象；
  - `src/rest.rs`：`RestClient` 包装 `pact_http::client`；`src/mcp.rs`：`McpClient`+`EventNotifier`（rmcp **stdio 子进程** MCP 客户端，镜像 `unattended_e2e` 用法）；
  - `src/driver.rs`：`Bootstrap`/`DriverConfig`/`AgentReport`/`absorb`/`drive()`——P1（生产优先）/P2（汇消费）/P3（等待）策略、订阅推送 + 轮询兜底、`pristine` 守卫、500ms 终止宽限，与 `unattended_e2e` **逐条同构**（差异仅"错误以 `Result` 返回"而非 panic）；
  - `src/orchestrator.rs`：`Transport`/`AgentSpec`/`BootstrapSpec`/`Orchestrator::run()`——每 agent `tokio::spawn`，未来对象 `Box::pin` 统一（REST/MCP 两具体 client 不引入 dyn）；
  - `src/executor.rs`：`ExecutorConfig`/`Compensation`/`SideEffectHandler`/`SideEffectExecutor`/`StopSignal`——订阅 `Consumed` 通知、archive 游标增量处理、失败以 **Rule-S 追加补偿事实**（saga），不表现为消费回滚（FR15/D23 边界）；
  - `src/bin/pact-runtime.rs`：TOML 配置驱动的生产编排入口（装配编排 + 可选执行器，输出各 agent 报告 JSON）。
- **依赖方向落地**：`runtime → pact-http`（类型化 REST client）+ `pact-engine`（线上视图 `CounterView`/`StatusView`/`GateClause`/`Notification`）+ `pact-core`（`Token`/`ArchiveEntry`/标识符 newtype）；`pact-mcp` 仅作可执行文件由编排 spawn、**不**作 lib 依赖（避免给 `pact-http` 扩大公共 re-export 面，spec D-A）。engine/store/core/三接口 **零改动**（`git status` 核对）。
- **两处修复**：
  - ①**执行器域发现**：编排与执行器并发启动时，执行器可能先于引导者装载域而收到 `DomainNotFound` → 执行器增**域发现轮询**（对齐 `drive()` 的 `list_domains` 轮询再 `subscribe`）；
  - ②**驱动 P1 需读见证**：P1 对每个见证前驱先 `read()` 再生产，测试域 `worker` 补 `reads task` 门（P1 见证可满足性前提）。
- **测试**：新增 5 项——集成 1（`tests/runtime_e2e.rs`，单 `#[tokio::test]`(multi_thread) 内起 **in-process daemon**（`Store::open`+`Engine::spawn`+`ServerState`+`router`+`serve_unix`）顺序跑：REST 编排 SE 生命周期（Σ 产 4/消 5/读 4、溯源链、`max_counter ≤ 5`、终态 `counter_size:0/archive_size:5/terminated:true`）｜REST 编排 ReAct 循环（7 产/8 消/7 读、`max_counter ≤ 2`、archive 序两轮同构）｜副作用执行器（失败 → 补偿事实 `task_failure` 由 `materializer` 追加、成功计数、游标无重复））+ 单元 4（`absorb` 计数/`max_counter`/`notified_terminated`；`effect_types` 过滤；`Endpoint::args` 生成；`RuntimeError::Display` 与 `From<ClientError>` 映射）。MCP 路径代码镜像已验证可编译的 e2e 用法，不在测试内 spawn（spec D-D：跨 crate 取不到 `CARGO_BIN_EXE_pact-mcp`）。
- **验证输出摘录**（沙箱，宿主机工具链包装）：
  ```
  cargo fmt --all --check                              → 无输出（FMT_OK）
  cargo clippy --workspace --all-targets -- -D warnings → Finished `dev` profile … in 1.20s（零警告）
  cargo test --workspace                                → TOTAL_PASSED=110（基线 105 + 本轮 5，全绿）
    Running tests/runtime_e2e.rs  test result: ok. 5 passed; 0 failed; …
  ```
- **文档**：`docs/design.md`（D24 行补"已实现"+依赖细化；§1 crate 表去"未实现"、依赖行改 `runtime → http/engine/core` 并注明 `pact-mcp` 仅 bin；§11 阶段 7 状态 → 已完成并附验收摘要；§13 L3 与构建形态措辞更新）；`docs/architecture.md`（§6 标题去"未实现"+新增"落地形态"条+依赖方向细化；§5 层边界行更新）。`docs/requirements.md`/`docs/manual.md` **不改**（FR16 已存在）。
- **规范母本未触碰**：`docs/pact*` 全程未改；D24 为**实现层**生产运行层，不新增范式语义（不触发边界判据 4）。
- **执行偏差**：沙箱 rootfs 无任何链接器与 C 编译器，构建/测试经宿主机（`/run/host`）工具链包装（`/tmp/hostlink`，gcc/ld→ld.bfd/ar/…）完成，**未安装任何包、未改 /usr**；沙箱与宿主 glibc 同版，链接产物在沙箱内直接运行。命令语义与 CI 一致（同第 15 轮做法）。
- **安全审视**：新增为客户端/编排/执行器代码——仅经 `pact_http::client` 与 `spawn` 子进程连接既有 daemon；无 `unsafe`；无新增密钥（bearer 由配置注入，不硬编码）；`McpClient` 的 `spawn` 仅拉 `pact-mcp` 可执行文件（路径来自配置）；无新依赖引入除 workspace 既有项之外（`toml`/`async-trait` 等已在 workspace）；无文件路径注入（配置项仅作连接参数与类型名）。无新增攻击面。

## 2026-10-19（第 16 轮，生产化边界裁决：D24–D26）

- **触发**：用户就"生产系统功能边界"提问并四问裁决（详见 `docs/requirements-change.md` 第 9 轮）。
- **新增 D24 生产运行层归属**：本仓库新增 crate（暂名 `pact-runtime`），承载参与者驱动循环（订阅/轮询 → `my_gates`/`enabled` → 决策 → 行动）+ 多 agent 编排 + 外部副作用执行器（outbox/saga 消费端）；引擎保持被动（D5）、MCP 维持 stdio-only（D20）均不变。
- **新增 D25 引擎生产化三项**：事件日志压缩与 schema 版本化（pact-store）；运维接口 health/ready + metrics + PID 文件 + 信号化优雅停机（pact-http/engine）；域级授权与审计读视图（pact-http 接口层，读审计**可选开启、默认关闭**——落地 D9 保留的审计扩展位，修订其默认值表述）。
- **新增 D26 分发产物归属**：本仓库 `dist/`（Dockerfile、systemd unit、musl 静态打包脚本），不接入 CI 语义测试（同 D22）。
- **结构更新**：§1 crate 划分增 `pact-runtime`（标注未实现）与其依赖方向（`runtime → mcp / http`，不反向）；§11 实现阶段增阶段 7/8/9（均"未开始"）；新增 §13「生产运行层与生产化边界」（含边界判据四条与构建形态）；§12 补 MCP 提供形态确认。
- **代码/测试零变更**：本轮纯文档与设计裁决；workspace 105 项全绿结论沿用第 15 轮。
- **规范母本未触碰**：`docs/pact*` 未改；本裁决为实现层工程边界，不涉及范式语义（不触发边界判据 4）。
- **安全审视**：本轮仅新增文档，无网络、无密钥、无 `unsafe`、无依赖变更。**登记待办风险**：FR17 的域级 ACL 与读审计属访问控制面，实现时须单独做安全设计（不属本轮）。

## 2026-10-18（第 15 轮，消耗提交契约与 Rule-E 幂等：发现 A/B 修复 + D23）

- **触发**：用户就"消耗原子性"组织评审后裁决——修复评审发现 A/B，采纳"文档化实现契约 + 补幂等 + witness-token 示例（2a）与 outbox 语义（2b）并行"组合。
- **发现 A（高）**：`Event::Consumed` 只记 `token_id`，重放时从 Counter 反查 token（`StoreError::ConsumeTargetMissing`）——单行日志不自足，与 NFR2"日志自身即完整审计记录"不符。**修复**：`Consumed` 改携 `token` 全量快照（`exhausted_at`=`ts`、`exhausted_by`=`by`）+ 可选 `request_id`；`apply_event` 与索引 `rebuild` 直接取事件内 token，删除 `ConsumeTargetMissing` 变体。
- **发现 B（中）**：`append`/`append_consumed` 先落日志再写索引，索引失败即返回 Err → 引擎不 apply、不推进 `last_seq`，日志/内存瞬态分歧直至重启。**修复**：确立**提交点 = JSONL fsync**；索引投影改为 `index_project`（尽力而为，失败不回滚、不失败本次调用），`append_consumed` 并入统一 `append`。同一原则推广到提交点之后的所有派生步骤：`post_commit` 改为**不可失败**（快照失败跳过并保留计数、Terminated 追加失败不回滚已提交命令）。
- **新增决策 D23**：消耗提交契约与 Rule-E 幂等（提交点 / 事件自足 / 幂等 / 原子域含 gate 读集 / 物化边界五点），显式拒绝库事务、2PC、分布式共识。
- **代码变更**：pact-store（`event.rs`/`index.rs`/`store.rs`/`error.rs`）、pact-engine（`EngineError::IdempotencyConflict`、`Command::Consume` 加 `request_id`、会话内幂等表 `consumed_requests`、recover/load 重建、`post_commit` 去 `Result`）、三接口（dto 新增 `ConsumeReq`、server 路由与错误映射、client、mcp `ConsumeIn`、cli `--request-id`）。
- **测试**：store 单元 18（新增 Consumed 自足（空 Counter 上重放）、幂等键序列化，替换 2 项旧用例）；engine 集成 11（新增 request_id 去重 + 跨重启生效 + 同键异 token 冲突）；daemon e2e 补幂等重试断言；`daemon_e2e`/`engine_integration` 全部 consume 调用补 `request_id` 形参。workspace **105 项全绿**；fmt/clippy 清零。
- **文档**：design.md（D23、§6.3 consume 请求、§7.1/§7.3/§7.4、§8.1、新增 §8.5、§9.2/§9.3、§10/§11 计数）；requirements.md（FR9 事件自足/提交契约、FR10 外部副作用边界、FR12 request_id、FR13、FR14 五份域、**新增 FR15**、NFR2、§6 范围外新增一条）；manual.md（§4.2 `--request-id`、新增 §4.5/§4.6、§7.2 Consumed 字段、§9.4 与 §10 增 `IdempotencyConflict`）；architecture.md（§2.1 提交点标注、§5 质量属性）。
- **2a/2b 落盘**：`examples/domains/external_materialization.pact`（witness-token 物化确认示例）+ 手册 §4.6；离线脚本新增 step 1.5、重录 trace（187→200 行，仍为 5 个顶层步骤）、examples/README 与根 README 域数由四改五；`examples/offline/fixtures/se_lifecycle.events.jsonl` 的 5 条 `Consumed` 行补为 token 全量快照以匹配新事件形状（离线 `replay` 因此自洽，已实测通过）。
- **磁盘事件格式变更（无旧格式兼容层）**：`Consumed` 由 `token_id` 改携 `token` 全量对象，属**不向后兼容**的日志格式变更——早于本轮的 `events.jsonl` 在重启重放时会被判为非法行（fail-fast）。本轮为**未发布**的 v1 早期，无历史部署需要迁移，故不加兼容垫片；既有示例 fixture 已重录为新格式。
- **规范母本未触碰**：`docs/pact*` 全程未改；D23 为**实现层**契约（论文仅给单步 SOS 语义），不涉及修订母本。
- **执行偏差**：沙箱缺 `cc` 与 glibc 开发文件，测试链接改用宿主机编译器 + `--sysroot=/run/host`（RUSTFLAGS），命令语义与 CI 一致。
- **安全审视**：仅事件字段、存储写序、可选幂等键与文档/示例；无网络、无 sudo、无密钥、无 unsafe、无新依赖变更；幂等键为客户端自报字符串，仅作 map 键与日志字段，不参与文件路径构造，无新增攻击面。

## 2026-10-15（第 14 轮，通用示例与用户手册落盘）

- **触发**：用户要求补一套与两技能无关的通用结构 + examples，向用户说明 PACT 范式系统的使用场景与方法（需求侧登记为 FR14，见 `docs/requirements-change.md` 第 6 轮）。
- **新增决策 D22**：通用示例与用户手册作为交付资产——不接入 CI、不改任何 crate、运行产物入 `.gitignore`；取证中发现的三处实现与文档不一致只登记不修码。
- **落盘资产**：
  - `examples/domains/`：`se_lifecycle.pact`(45) / `re_pipeline.pact`(50) / `react_loop.pact`(38) / `cross_harness.pact`(36)
  - `examples/fragments/`：`security_review.pact`（成功合并）/ `security_blocking.pact`（C2 冲突）
  - `examples/offline/run.sh`(168) + `trace.txt`(187)：analyze × 4、compose × 2、replay × 1，含三类不良构域与陷阱域
  - `examples/online/run.sh`(251) + `trace.txt`(238)：daemon 启动 → 装载 → 六规则全流程 → Rule-T → 卸载 → 清理（`trap ... EXIT`，因 CLI 无 `daemon stop`）
  - `examples/README.md`(413)：场景导航与逐例讲解
  - `docs/manual.md`(653)：12 章权威用户手册
- **代码零改动**：本轮未修改任何 crate；`cargo test` 结论沿用第 12/13 轮（workspace 103 项全绿）。`.gitignore` 已含 `/examples/online/run-data`、`/examples/offline/out`（脚本每次运行重建）。
- **architecture.md 无变更**：未引入新组件、未改变 crate 划分、进程与部署视图、数据流与质量属性落点，故不追加 architecture-change 记录。
- **规范母本未触碰**：本轮全程未修改 `docs/pact*`（遵守第 13 轮纪律沉淀）；示例与手册中凡引用论文处均以"§x.y → pactv1_rev2.md"形式指向，不复制原文。
- **安全审视结论**：本轮仅新增 Markdown 文档与 shell 教学脚本。脚本无网络出访、无 sudo、无密钥/令牌硬编码（在线示例走信任声明模式，`pact.toml` 中 `[auth.tokens]` 为空表）、无 `unsafe`、无依赖变更；`trap` 清理确保 daemon 与 unix socket 不残留。无新增攻击面，无需调整既有安全控制（恒时令牌比较、事件日志只追加、快照原子写）。

## 2026-10-15（第 13 轮，论文改动回退：规范母本不动，实现登记为保守超集）

- **触发**：用户指出第 12 轮"规范同步（pactv1_rev2.md 三处）"超出实现方工作范畴——论文是规范母本，实现方不得顺手修订。裁决：恢复原文。
- **回退**：§1.4.3 Advisory 段、Appendix A.5 伪代码、A.5 Remediation 三处逐字恢复为参与者级原文（该文件不入仓库，纯本地还原，无版本控制影响）。
- **D21 措辞修正**：删去"与论文一并对齐定案"，改为"论文维持参与者级原文，实现登记为参考算法的**保守超集**；规范是否修订由作者另行裁决"。
- **保留**：`lib.rs` 注释/Display 的类型级表述仍然准确（回退后论文为参与者级、实现更宽，恰构成保守超集关系）；跨参与者回归用例；第 12 轮全部测试结论（103 项全绿，本轮代码零改动）。
- **纪律沉淀**：涉及规范母本（`docs/pact*`）的任何文字改动，事前必须单列请求用户显式确认，不得随"更新文档"类概括指令默认授权；"实现宽于规范"的偏差以保守超集登记于 design.md，不回灌论文。

## 2026-10-15（第 12 轮，SeedProduceOverlap 语义注释与规范对齐定案）

- **背景**：技能适配轮发现 `precondition_wf` 的 `SeedProduceOverlap` 实现**宽于**其文档注释——`produced` 收集为全体参与者 produces gate 的全局类型集，跨参与者组合（SE `responsible_human` seeds + RE `release_manager` produces）也告警，方向为假阳性不漏报；既有测试只覆盖同参与者场景（skill-adaptation.md §10 第 1 轮登记）。用户裁决采处置 (b)：改注释承认现状、保留实现。
- **代码变更（逻辑零变更）**：pact-analysis `WfWarning::SeedProduceOverlap` 文档注释改为**类型级判定**表述（produces gate 取全体参与者并集，`participant` 字段 = seed 持有者；比论文 §1.4.3 原表述更宽，假阳性方向不漏报）；`Display` 文本同步改写（旧文"同时持有 seed 权威与 produces gate"暗示同一参与者，与实现不符）。无测试断言依赖旧 Display 文本，JSON 序列化形态不变（变体名 + 字段）。
- **规范同步（pactv1_rev2.md 三处）**：§1.4.3 Advisory 段改为类型级定义——重叠 = 非源类型 T 同时可 seed 与可 produce，同参与者双入口是退化情形，跨参与者重叠亦须显式部署裁决（recall 偏向，不漏报同参与者情形），警告 `participant` 字段为 seed 持有者；Appendix A.5 伪代码改为 `Produced ← { T : ∃ p, ("produce", T) ∈ Gate(p) }` 后按类型判属；A.5 Remediation 补类型级说明。
- **测试**：`precondition_wf_warns_seed_produce_overlap` 增补第三块——跨参与者用例（p produces b、q seeds b、q ≠ p ⇒ 警告 `participant = q`），锁定类型级行为（第 1 轮偏差今后由测试捕获）。workspace 全量 **103 项全绿**（净增 0 项，扩充既有用例）；fmt/clippy 清零。
- **D21 同步**：design.md D21(4) 补记类型级判定定案（produced 并集、participant 字段语义、假阳性方向）。
- **执行偏差（沿用）**：distrobox `pact-dev` 仍不可达，验证在宿主机工具链（cargo 1.98.1，与 CI stable 同版）完成，命令与 CI 一致（fmt --check → clippy --locked -D warnings → test --workspace --locked）。
- **安全审视**：本轮仅注释、Display 字符串、测试断言与文档，无网络、无进程、无密钥、无 unsafe、无依赖变更（`--locked` 通过即证 Cargo.lock 未动）；不触发推送/部署前置审视的重新评估，2026-09-06 第 8 轮结论继续有效。

## 2026-09-09（第 11 轮，CI 偶发失败修正：场景 B Counter 卫生断言）

- **现象**：GitHub Actions 上 `unattended_e2e::unattended_scenarios_to_terminated` 偶发失败——`max_counter = 3` 违反"应恒 ≤ 2"断言；同轮本地（20+ 连跑）与全量回归均未复现。
- **根因（非引擎缺陷，为测试断言过强）**：场景 B 的 P1 纪律中，`pact_produce` 与 consume-after-produce 的 `pact_consume` 是同一 agent 内的两次串行调用，窗口内 Counter = 2；若下一阶段 agent（被 produces 推送或轮询唤醒）在该窗口内完成自己的 Rule-P，Counter 合法地达到 3（理论链式重叠上界为类型数 5）。Rule-P 只要求前驱在 Counter 上（不被消费），且 §3.1 规定 `produces` 门不接受条件子句——该窗口**语义上不可消除**，窗口重叠是 PACT 自治语义下的合法执行。本地 20 连跑通过纯属调度时序有利。
- **修正**：场景 B 卫生断言由 `≤ 2` 放宽为结构上界 `≤ 类型数 5`（每类型至多一实例：每类型仅一个 produce 门持有者 + P1 同迭代串行 produce→consume），注释记录竞态分析与 CI 实测证据。场景 A（单 agent 串行，无并发生产者）的 `≤ 2` 为真不变量，保留不动。
- **验证**：unattended_e2e 本地 10 连跑稳定；workspace 全量（104 项）+ fmt/clippy 清零。
- **顺带**：design.md §11 阶段 5 验收表述同步修正（场景 B 卫生上界）。

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
