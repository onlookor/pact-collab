# 设计变更记录

## 2026-10-21（第 36 轮，D32：抽取 `pact-engine-kernel`，原生与 WASM 共用实例会话语义）

- **新增纯内核 crate `pact-engine-kernel`**（无 IO、无 tokio；依赖 `pact-core`/`pact-parser`/`pact-analysis`/`pact-store-model`/`pact-views`）：
  - `Session` 统一持有 `declaration`/`domain`/`state`/`last_seq`/`next_seq`/`dl_seq`/`terminated`/`consumed_requests`；
  - 提供 `fresh`、`recover`（快照锚点 + 日志重放 + 幂等表 + 终止态）、`prepare_seed`/`prepare_produce`/`prepare_consume`/`read`、`mark_prepared`/`abort_prepared`（WASM 两阶段提交）、`commit`/`record_terminated`、`counter`/`archive`/`gates`/`enabled`/`status`/`snapshot`/`summary` 与 `analysis_report`；
  - `KernelError` 聚合 Parse/Rule/Model/Recovery/IdempotencyConflict，由原生与 WASM 各自映射到既有错误码（对外错误码不变）。
- **原生迁移**：`pact-engine` 的 `InstanceSession` 改为 `{ core: Session, events_since_snapshot, notify }`，经 `Deref`/`DerefMut` 复用内核方法；`recover_instance`、`instantiate`/`load_domain` 重置、`seed`/`produce`/`consume`/`read`、查询视图与 `analyze` 全部委托内核；`post_commit` 接收 `terminated_seq` 并在追加 `Terminated` 后调用 `record_terminated`。
- **WASM 迁移**：`pact-wasm` 的 `InstanceSession` 改为 `{ core: Session, events }`，删除约 500 行重复状态逻辑；`prepare_*`/`commit`/`abort`/`restore`/视图/分析全部委托内核。
- **行为契约不变**：DomainLoaded seq 从 1 起；失败命令仍推进时钟；Rule-T 终止 `seq = last_seq + 1` 且只报告一次；压缩水位 `base_seq` 恢复；Rule-E 幂等表跨快照恢复；`ConsumePreparation::Prepared` 以 `Box` 承载完整事件（控制枚举尺寸）。
- **前置进度**：FR21 稳定化前置 (3) 标记完成；仅余 (4) 推送/发布前宿主边界安全审视。
- **验证**：`cargo test --workspace` 184 项全绿（含 kernel 3 项单测、parity 7 项）；`cargo fmt --all --check` 通过；原生 `clippy --workspace --all-targets -D warnings` 与 wasm32 `clippy -p pact-wasm -D warnings` 均零告警；wasm-release 构建 + bindgen + `ci-smoke`（raw 455,929 B / gzip 142,236 B）+ Node 7 项测试通过。
- **未变**：事件 schema、磁盘布局、ABI `pact-wasm/0.2`、原生接口与错误码、依赖方向、`dist/` 产物。

## 2026-10-21（第 35 轮，金标扩面：多实例 / 组合域 / 条件分支 / 压缩恢复）

- **测试覆盖扩展**（`crates/pact-wasm/tests/parity.rs`，新增 4 项，共 7 项 parity）：
  1. `multi_instance_isolation_matches_native`：同一域声明并发两实例，各自独立 `DomainLoaded`/seq/idempotency；逐实例事件日志与原生逐行相等，Counter/Archive 隔离，`list_domains` 实例清单一致。
  2. `composed_domain_matches_native`：两片段经 §1.8 组合后 `DomainSummary` 完全相等；种子/生产事件日志逐行相等；四个参与者的 `my_gates` 逐项相等。
  3. `conditional_gate_branches_match_native`：`count T >= 2`、`count T >= 1`、`not exists T` 三种 GateExpr 分支的放行与 `GateConditionFailed` 拒绝逐项对账，含失败命令后的时钟推进差异。
  4. `compacted_log_recovery_matches_native`：原生 `compact(keep_last=0)` 后「快照 + 空日志」由 WASM `restore` 恢复，视图相等；WASM 续写 seq 接续快照水位（尾部仅 1 行），原生 daemon 可再打开该尾部日志并得到一致视图。
- **前置进度**：FR21 稳定化前置 (2) 金标扩面标记完成；(3) `pact-engine-kernel` 抽取与 (4) 安全审视仍待办。
- **验证**：`cargo test -p pact-wasm` 15 项全绿（8 单测 + 7 parity）；`cargo clippy -p pact-wasm --all-targets -D warnings` 零告警；`cargo test --workspace --locked` 全绿。
- **未变**：实现代码、事件/磁盘格式、ABI、CI 门禁；本轮仅扩展测试与文档。

## 2026-10-21（第 34 轮，Q7：WASM 独立 CI job）

- **新增 CI job `wasm`**（`.github/workflows/ci.yml`）：安装 stable + `wasm32-unknown-unknown` → wasm 目标 `clippy -D warnings`（仅 lib，避免 `--all-targets` 拉入 `pact-engine`/`tokio` 等原生 dev-deps）→ `cargo build -p pact-wasm --profile wasm-release` → `wasm-bindgen --target nodejs` → ABI/语义/体积门禁 → Node 持久化测试。
- **wasm-bindgen CLI 版本锁定**：`0.2.127`（须与 crate 版本一致），经 `actions/cache` 缓存 `~/.cargo/bin/wasm-bindgen`；未命中时才 `cargo install wasm-bindgen-cli --version … --locked`。
- **新增门禁脚本** `crates/pact-wasm/js/ci-smoke.js`：断言 `abi_version() === "pact-wasm/0.2"`；`validate`/`analyze` 最小语义路径（类型 2、gate 子句 4、seed_authority 1、A.5 良构、源类型可达）；体积上限 raw ≤ 600 KiB、gzip ≤ 200 KiB（基线 450,190 B / gzip ~140 KiB，留约 30% 余量）。
- **设计 §10 测试策略**新增「WASM 门禁」行。需求 FR21 的稳定化前置 (1) 标记为已实现，其余 (2)–(4) 不变。
- **本地实测**：wasm32 clippy 清零；wasm-release 构建与 bindgen 通过；`ci-smoke` 通过（raw 450,190 B / gzip 140,289 B）；Node 7 项持久化测试全绿。
- **未变**：crate 代码、事件/磁盘格式、原生 CI `check` job；本轮只加 CI 与门禁脚本。

## 2026-10-21（第 33 轮，P6 裁决 A：WASM 本地分发形态条件立项（D31））

- **新增决策 D31**：WASM 本地运行登记为**实验性可选分发形态**（Node/桌面宿主），权威仍是原生引擎；`pactd`、NFR4 单静态二进制与 FR18 生产分发承诺不变。
- **新增 §16「WASM 本地分发形态（实验性）」**：16.1 定位与权威边界（本地单用户、无 ACL/审计、不提供 MCP/HTTP）；16.2 组件与宿主职责（`pact-wasm` 内核 / JS 宿主适配层 / 原生 store+engine 三方边界）；16.3 ABI `pact-wasm/0.x` + JSON 字符串与 `prepare → append+fsync → commit` 唯一写路径（abort 回收 seq、`Terminated` 派生追加、ID/ts 宿主注入）；16.4 稳定化前置（Q7 CI → 金标扩面 → `pact-engine-kernel` 抽取 → 安全审视）与不纳入项（浏览器 OPFS/SharedWorker、多用户、压缩/索引、DSH `govern`、NFR6 v2）。
- **crate 清单注释**：`pact-wasm` 由「spike S1」改为「WASM 本地分发形态（实验性，D30/D31）」。
- **未变**：事件 schema、磁盘布局、原生 API、依赖方向、`dist/` 产物；本轮仅登记立项、分发形态与 ABI 策略，未改代码。
- **配套**：需求侧新增 FR21（`docs/requirements-change.md` 第 18 轮），架构侧新增组件边界（`docs/architecture-change.md` 第 20 轮），可行性报告状态改为已条件立项。

## 2026-10-21（第 32 轮，P4/S2：Node 持久化宿主）

- **JS 存储适配器**（`crates/pact-wasm/js/pact-store.js`）：与原生同布局（`<root>/<domain>/<instance>.{events.jsonl,snapshot.json,lock}`）；`append` = `open('a') + write + fsync + close`；`writeSnapshot` = tmp + `fsync` + `rename` + 目录 `fsync`；`readEvents` 用字节级 `lastIndexOf('\n')` 截断崩溃残留半行并 `fsync`；域名/实例名以 `[A-Za-z0-9_-]{1,128}` 校验，杜绝路径穿越。
- **单写者**：`.lock` 以 `O_EXCL` 创建并写 pid；冲突时若持有者进程不存在（`kill(pid,0)` 失败）则回收一次，否则抛 `InstanceLocked`；`close` 删除锁。仅进程级近似，不等价于原生 `flock`（已在文档标注）。
- **最小宿主**（`pact-host.js`）：`PactHost.open` 获取锁后按「日志/快照优先」恢复，空日志且无快照才新建并追加 `DomainLoaded`；`_finish` 严格 `prepare → append+fsync → commit`，追加失败 `abort`；`commit` 返回的 `Terminated` 事件随后追加（信息性，失败不回滚状态）。
- **demo**（`js/demo.js`）：生命周期 + 快照 + 重启校验，便于人工观察。
- **测试**：Node `node:test` 7 项（重启一致、append 后未 commit 的崩溃恢复、半行截断、单写者、快照自足、幂等跨重启、Rule-T 终止落盘）；Rust 新增 `parity.rs::native_and_wasm_alternate_on_shared_log`（原生→WASM→原生→WASM 交替打开同一日志，视图一致）。全仓 **177 项测试全绿**，fmt/clippy（native + wasm32）清零。
- **边界**：浏览器 OPFS/SharedWorker 不做（Q6）；WASM CI job 不做（Q7 待决策）；不改需求、不动原生磁盘格式。

## 2026-10-21（第 31 轮，P3/S1：WASM 内存语义与原生对账）

- **实例会话**：`PactHost` 由「声明缓存」升级为「声明注册表 + `(domain, instance)` 会话表 + 待提交事务表」；每会话持有声明快照、`CounterState`、完整事件序列、`last_seq`/`next_seq`、`dl_seq`、`terminated` 与 Rule-E 幂等表。`is_occupied = last_seq > dl_seq` 与原生一致。
- **生命周期对齐原生**：`load_domain` 空实例可重置（seq 续接）、非空实例返回 `DomainBusy`；新增 `load_domain_with(..., force)` 显式替换；`instantiate` 从已登记声明创建额外实例；`load_domain_at` 注入确定性时间戳（`DomainLoaded` 现参与事件序列，P2 的 `last_seq = 0` 由此变为真实值）。
- **两阶段提交**：`prepare_seed/produce/consume` 校验并分配 seq，返回 `Prepared { tx_id, seq, event(JSONL 行), result, replayed }`；`commit(tx_id, terminated_ts)` 应用事件、写入幂等表、按原生 `post_commit` 顺序追加 Rule-T `Terminated`（可选独立时间戳）；`abort` 丢弃事务并回收未被后续分配占用的 seq，保证 JSONL 连续、可被原生 `Store` 的 `seq == last+1` 校验接受。`prepare_consume` 幂等命中返回 `tx_id = None`、`replayed = true`，不产生事件。
- **查询**：`read`（Rule-R）与 `counter`/`archive`/`gates`/`enabled`/`status`，直接复用 `pact-views` 类型与 `pact-core` 规则，字段顺序与原生一致。
- **导出/恢复**：`export_events` 逐行 `to_wire_json`（顶层 `version`）；`export_snapshot` 复用 `Snapshot::new`（自足）；`restore` 复刻 `recover_instance` 的锚点选择（最后一个 `DomainLoaded` 优先，否则快照锚）、`base_seq` 重放、幂等表重建与终止态计算。
- **错误码**：`FacadeError.code` 由自造小写改为与原生接口层同名（`InstanceNotFound`/`DomainBusy`/`InstanceExists`/`DomainNotDeclared`/`IdempotencyConflict`/`SeedAuthorityDenied`/`PredecessorNotOnCounter`/`TokenNotOnCounter`/`InvalidDomainName`/`InvalidInstanceName`/`Syntax`/`WellFormedness`）；`FacadeError.code` 类型改为 `String`。
- **ABI**：`abi_version` 升至 `pact-wasm/0.2`。新增导出 `load_domain_at`/`load_domain_with`/`instantiate`/`prepare_seed`/`prepare_produce`/`prepare_consume`/`commit`/`abort`/`read`/`counter`/`archive`/`gates`/`enabled`/`status`/`export_events`/`export_snapshot`/`restore`。
- **测试**：`crates/pact-wasm/tests/parity.rs` 两项（金标事件/视图/双向互通、错误码对账）——固定 `T0–T8` + `seq-0..2`，**8 条事件与原生 JSONL 逐行相等**（含 `Terminated`），视图与错误码逐项相等，WASM 日志+快照可被原生 daemon 恢复，原生日志可被 WASM 恢复。`pact-wasm` dev-dependencies 新增 `pact-engine`/`pact-store`/`tokio`/`tempfile`（仅测试）。全仓 **176 项测试全绿**（P2 的 171 + 5），fmt/clippy（native + wasm32）清零。
- **体积**：`wasm-release` 产物 bindgen 后 450,190 B（440 KiB）、gzip 140,457 B（137 KiB），较 S0 的 267 KiB 增加 173 KiB，登记为新基线。
- **边界**：仍不落盘（P4 的 JS `PactStore` 负责 append/fsync/崩溃恢复）；不接 CI（Q7 待 P3 退出后再议）；浏览器 OPFS/SharedWorker 不做（Q6）。

## 2026-10-21（第 30 轮，P2/S0：WASM 门面 `pact-wasm`（D30））

- **新增 `pact-wasm`**（`crate-type = ["cdylib", "rlib"]`）：宿主无关的 `PactHost` 承载解析/校验/静态分析/片段组合；`wasm-bindgen` 包装仅编译到 `wasm32`，原生构建不拉入该依赖。
- **ABI（Q2 冻结）**：全部方法使用 JSON 字符串；导出 `new`、`load_domain(name, declaration, instance)`、`validate(declaration)`、`analyze(declaration)`、`compose(name, fragments_json, instance)`、`list_domains`，另有 `abi_version` 探针；错误以 `{"code","message"}` JSON 字符串抛出。
- **构建配置**：根 `Cargo.toml` 新增 `wasm-release` profile（`opt-level="z"`、LTO、`codegen-units=1`、`panic="abort"`、`strip`）；`wasm-bindgen 0.2.127` 仅作为 wasm32 目标依赖。
- **实测**：release `.wasm` 273,394 B（267 KiB），gzip 89,054 B（87 KiB）；Node 22 模块加载 1.71 ms、构造 0.35 ms、首次 `analyze` 3.24 ms。
- **语义对账**：`react_loop.pact` 与 `cross_harness.pact` 的 `analyze` JSON 与原生 `pact --json analyze` 深度相等。
- **边界**：S0 不写事件、不落盘、无 `prepare/commit`（P3/P4）；浏览器绑定、OPFS、wasm-pack 与独立 CI job 均未纳入（Q6/Q7）。
- **验证**：`cargo fmt --all --check`、host 与 wasm 目标 `clippy -D warnings` 均干净；全仓 **171 项测试全绿**（P1 的 166 + `pact-wasm` 5）。

## 2026-10-21（第 29 轮，P1：WASM 纯类型层拆分（D29））

- **新增 `pact-store-model`**：承载 `Event`（五变体）、`WireEvent`（顶层 `version` + `flatten`）、`EVENT_SCHEMA_VERSION`、`apply_event`/`ApplyOutcome`、`to_wire_json`、`Snapshot` 数据结构、`InstanceKey` 与 `ModelError`；依赖仅 `pact-core + serde + serde_json`，无 IO/tokio/unsafe，可编译到 `wasm32-unknown-unknown`。
- **新增 `pact-views`**：承载 `CounterView`、`ArchiveView`、`StatusView`、`GateClause`、`DomainSummary`、`DomainListing`、`CompactionOutcome`、`AnalysisReport`；依赖仅 `pact-core + pact-analysis + serde`，无 tokio，可编译到 wasm。
- **原生 API 保持**：`pact-store/src/event.rs` 与 `key.rs` 删除，`lib.rs` 从模型 crate 原样 re-export（`Event`/`WireEvent`/`InstanceKey`/`Snapshot`/`apply_event`/`EVENT_SCHEMA_VERSION` 等）；`pact-store::snapshot` 只保留原子读写与版本校验。`pact-engine/src/views.rs` 改为 `pub use pact_views::*`，`pact_engine::views::*` 路径不变。
- **错误边界**：`ModelError`（JSON/非法域名/非法实例名/不支持的 schema 版本）为纯模型错误；`StoreError` 保留全部既有变体并新增 `From<ModelError>` 一一映射；`EngineError` 新增 `From<ModelError>`（转 `Store`），调用方匹配路径不变。
- **零格式变化**：`Event`/`Snapshot` 的 serde 属性逐字迁移，JSONL 与线上 JSON 字段、默认值、`skip_serializing_if` 均未改；磁盘布局、索引 schema、压缩水位均未改。
- **测试**：事件/快照模型的原测试随类型迁入 `pact-store-model`；全仓 `cargo test --workspace` **166 项全绿**（与拆分前基线一致）；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo check --target wasm32-unknown-unknown -p pact-store-model -p pact-views` 通过。
- **未执行**：`pact-wasm` 门面、JS 存储适配、`prepare/commit` 实现仍属 P2/P3；本轮不接线、不改 `docs/pact*` 母本。

## 2026-10-21（第 28 轮，阶段 6：CPN 编码生成 pact-cpn（D28））

- **D28 新增**：`pact-cpn` 纯函数生成器 + CLI `pact cpn`；输出 CPN Tools XML（`generator format='6'`）；单实例作用域命名空间 `i_<instance>__`；颜色 `PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`、`PACT_ARCHIVE = PACT_TOKEN × Participant × Timestamp`、`PACT_COMPL = int`。
- **结构映射**：每类型库所 + Archive；生产迁移对每个前驱一条 test arc（输入绑定 + 同变量输出），输出依赖列表 `[#1 xi]`；读迁移 `x` 自环；消费迁移消耗 `x` 并输出 Archive；种子迁移无输入弧。
- **条件编码**：`GateExpr → NNF → DNF`；每个合取项一条迁移（`or` 多迁移，名称 `_c1/_c2…`）；`count T >= n` n 条 test arc + 两两不同 guard；`via T` test arc + `List.exists (fn z => z = #1 y1) (#4 x)`；`count T <= k` 用互补库所（初值 N、`M(p_T)+M(p_T_not)=N`、生产/种子消耗 1、消费归还 1、测试弧 N-k 借还）；`not via` 拒绝；缺上界/超大 DNF/`count >= n` 超过 1024/非良构域分别报错。
- **目标类型自查**：目标 token 计入目标类型 count，故 `exists T`/`count T >= 1` 在 T 为目标时无需额外弧，`count T >= n` 只需 n−1 个额外见证；`via T` 在目标类型上仍需额外见证。
- **CLI 接入**：`pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`，`--json` 输出 `CpnStats`；错误映射为 `Cpn` 代码（缺上界提示具体类型）；离线路径不触 daemon。
- **设计文档**：决策表新增 D28；§1 crate 清单与依赖图新增 `pact-cpn`（`cpn → core`）；新增 §15 完整编码设计；阶段表阶段 6 标记已完成。顺手修正 D25 行中与 §9.6 不一致的「(3) 未开始」为「已实现（2026-10-20）」。
- **验证**：`pact-cpn` 11 项测试（结构计数、via/test arc、not 互补、目标自查、or 展开、count 上界、not via 错误、实例命名空间、XML 良构、超限字面量拒绝）；CLI 离线 e2e 覆盖成功与缺上界；`examples/domains/*.pact` 生成物全部通过 `xmllint --dtdvalid`（CPN DTD）；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **166 项全绿**。
- **未执行**：CPN Tools GUI 内实际加载与状态空间实验（沙箱无 GUI）——列为人工验收；`--bound` 正确性由调用方承担，文档已明确。
- **安全审视（零可利用发现）**：生成器零 IO、无 `unsafe`、无新增第三方依赖（仅 pact-core + serde derive）；所有输出文本经 XML 转义，名称经 ML 标识符安全化，实例名进入命名空间前消毒；DNF 设 1024 项上限防指数膨胀；错误路径不回显文件内容。

## 2026-10-20（第 27 轮，阶段 9：分发产物 dist/（D26））

- **交付形态**：新增 `dist/` 六个文件——`Dockerfile`（多阶段 `rust:1.98-alpine` → `alpine:3.22`，非 root uid/gid 10001、`/var/lib/pact` 数据卷、`EXPOSE 8787`、`HEALTHCHECK` 探 `/health`、入口 `pactd --config /etc/pact/pact.toml`）、`Dockerfile.dockerignore`（排除 `.git`/`target`/`docs`/`examples`/本地数据，避免论文母本进入构建上下文）、`pact.container.toml`、`pactd.service`、`pact.toml.example`、`build-static.sh`。
- **systemd unit**：`User/Group=pact`、`StateDirectory=pact`（`/var/lib/pact` 0700）、`RuntimeDirectory=pact`（`/run/pact` 0700）、`Restart=on-failure`、`KillSignal=SIGTERM` + `TimeoutStopSec=20s`（覆盖 10s drain）、`UMask=0077`；加固 `ProtectSystem=strict`/`ProtectHome`/`PrivateDevices`/`ProtectKernel*`/`ProtectControlGroups`/`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`/`RestrictNamespaces`/`LockPersonality`/空 `CapabilityBoundingSet`。
- **静态打包**：`build-static.sh` 缺省目标 `x86_64-unknown-linux-musl`；自动探测 `musl-gcc` 或 musl 系 `cc` 并导出 `CC_<target>`/`CARGO_TARGET_<target>_LINKER`（turso 的 C 依赖需要）；`cargo build --release --locked --bins`；strip 后逐 bin `readelf -d` 无 `NEEDED` 校验；归档含 `bin/`、unit、配置示例、`LICENSE`、`SHA256SUMS`，并产出 tar.gz 与 `.sha256`；缺前置显式失败，不回退 glibc。
- **配置示例**：systemd 侧 `/var/lib/pact` + `/run/pact/pact.sock`；容器侧 `listen_unix=""` + `0.0.0.0:8787`，文档强制 `-p 127.0.0.1:8787:8787`（v1 无 TLS）。
- **形式层与代码零改动**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` / `pact-store` / `pact-engine` / 三接口与 `pact-runtime` 均未修改；`docs/pact*` 母本未动；`dist/**` 按 D26 不接入 CI 语义测试。
- **验证**：`bash -n dist/build-static.sh` 通过；两份 TOML 由 Python `tomllib` 解析通过；用临时目录替换路径后，`pactd` 实际以两份示例配置启动并优雅停机（unix 套接字与 TCP 两种监听均正常）；`systemd-analyze verify`（临时 root，补齐目标 stub）单元语义通过；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **155 项全绿**（无代码改动，与上一轮基线一致）。**未执行**：musl 静态构建与容器镜像构建——当前沙箱无 musl C 工具链、无 Docker/Podman；脚本对缺失前置显式失败，实际构建需在具备 `musl-gcc` 的主机（或 Docker 构建阶段）执行。
- **全面安全审视（零可利用发现）**：
  - **构建上下文**：`Dockerfile.dockerignore` 排除 `.git`、`docs`（含未公开论文母本 `docs/pact*`）、`examples`、`target` 与本地数据；该机制要求 BuildKit（Docker 23+ 默认），未使用经典构建器，手册与 Dockerfile 均标注。
  - **镜像运行**：非 root uid/gid 10001；数据目录为卷；运行阶段不额外 `apk add`（仅基础镜像 busybox，无额外 CVE 面）；配置为无密钥示例。
  - **网络暴露**：容器内 `0.0.0.0:8787` 仅为端口映射可达，手册强制 `-p 127.0.0.1:8787:8787`；v1 无 TLS 且默认信任声明模式，跨主机使用须外部 TLS + `[auth.tokens]`/`[auth.acl]`（已在 §2.4/§7.6 与示例注释登记）。
  - **systemd 权限**：`StateDirectory`/`RuntimeDirectory` 0700、`UMask=0077`、空 capability 集、`ProtectSystem=strict`/`ProtectHome`/`PrivateDevices`/`ProtectKernel*`；只放开 unix/inet 地址族；`TimeoutStopSec=20s` 覆盖 10s drain。配置文件按 `root:pact 0640` 安装（可含令牌）。
  - **打包脚本**：无 `eval`；strip 后以 `readelf -d` 拒绝任何 `NEEDED` 动态依赖（防误发 glibc 产物）；归档不含密钥或未公开文档；校验和未签名，已在设计 §14 列为非目标。
  - **代码面**：零 Rust 改动、无 `unsafe`、无新增依赖；`docs/pact*` 母本未动。

## 2026-10-20（第 26 轮，D25 子项 3：域级 ACL 与读审计视图实施）

- **四项设计取舍（用户裁决，均选推荐项）**：ACL 粒度 = 实例级条目 `domain/instance` + `domain/*` 通配；ACL 范围 = 生命周期（load / instantiate / unload / compact / `:analyze` / `:compose`）+ 四类动作 + 查询 + 读审计视图，未配置时保持既有行为；读审计 = 独立 `<instance>.read-audit.jsonl`（append + fsync），不进入主事件日志；审计语义 = fail-closed + REST 只读视图。
- **配置（pact-http）**：`Config.read_audit: Option<bool>`（缺省 false）；`AuthConfig.acl: BTreeMap<String, Vec<String>>`；新增 `Config::{acl(), read_audit(), validate()}`。`validate()` 拒绝「ACL 非空 + 令牌表为空」及非法条目（只接受 `domain/instance` / `domain/*`），daemon 启动前调用。匹配函数 `acl_allows` / `acl_allows_domain` 仅做字符串比较。
- **鉴权（§9.6）**：`ServerState` 增 `acl` 字段与 `with_acl`（`new` 保持无 ACL 兼容）。`authenticated_principal` 恒时遍历令牌反查参与者；`authorize_action`（动作端点：令牌归属 + 实例授权）、`authorize_key`（生命周期 / 查询 / compact / 读审计）、`authorize_domain`（`:analyze` 该域任一授权）。`GET /domains` 逐实例过滤并剔除空域。无令牌 `401`、越权 `403`（结构化错误体含 `domain` / `instance`）。覆盖端点：load / instantiate / unload / compact / `:analyze` / `:compose`、seed / produce / read / consume / gates / enabled、counter / archive / status / events、read-audit。
- **读审计（pact-store + pact-engine）**：`ReadAuditRecord{version, ts, domain, instance, participant, token_id}`；`Store::{append_read_audit, read_audit_tail}`（append + `sync_all`；尾部 N 条按时间正序；文件不存在 → 空；仅容忍末尾无换行的崩溃半行，其余坏行 fail-fast）。`EngineConfig.read_audit`；Rule-R 成功后写审计，`Store` 错误经 `EngineError::Store` 上抛 → REST `500`（fail-closed）。新增 `Command::QueryReadAudit` + `Engine::read_audit(key, limit)`。
- **接口面**：`GET /domains/{d}[/instances/{i}]/read-audit?limit=`（缺省 100、夹取 `[1, 1000]`）→ `{records:[...]}`；`pact_http::client::Client::read_audit(...)`。审计流不产生事件、不推进 `seq`、不参与重放 / Rule-T / 快照 / 压缩；实例未加载也可查询。
- **测试**：新增 10 项——config 3（ACL 无令牌拒绝、条目格式校验、通配/实例匹配）、store 3（append+tail 且独立于事件日志、末尾半行容忍、跨块反向扫描）、engine 2（Rule-R 审计字段 / limit / 重启持久化 / 不改事件日志、审计写失败 fail-closed 且关闭时不阻断）、http e2e 2（401/403/通配/列表过滤/查询与审计视图受控/卸载；审计视图记录 / limit / 实例隔离）。全仓 **155 项**全绿（基线 145 + 10）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动；`docs/pact*` 母本未动。CLI/MCP 未新增子命令（只读视图走 REST/client）。
- **安全审视待办**：提交/推送前按流程做一次全面安全审视。

## 2026-10-20（第 25 轮，推送前全面安全审视）

- **审查范围**：本轮全部改动（`pact-store` 版本化与压缩、`pact-engine` 快照锚与 Compact、`pact-http` 端点与压缩配置、`pact-cli`/`pact-mcp` 入口、文档）。
- **核对项与结论（零可利用漏洞）**：
  - **路径穿越**：`compact` 端点键与既有端点同经 `InstanceKey::validate()`（谓词 `[A-Za-z0-9_-]{1,128}`，天然禁 `..` 与分隔符），存储层路径构造仍集中在 `log::events_path` / `domain_dir`；新增 `logmeta_path` 同源复用同一 `(domain, instance)` 参数与校验，无新增可达的穿越面。
  - **SQL 注入**：`prune_events` 为 `DELETE … WHERE domain = ?1 AND instance = ?2 AND seq <= ?3` 全参数化绑定，无字符串拼接。
  - **内存安全**：全仓 `unsafe` 零出现；新增代码不含 `unsafe`、不含 `unwrap()` 于外部输入路径（仅锁中毒 `.expect()` 与测试）。
  - **崩溃原子性与数据丢失**：压缩五步序中「快照（提交点）→ logmeta 水位 → 日志 tmp+fsync+rename → 索引裁剪」保证任一中间点崩溃均落到两个一致态；`SnapshotRequiredForCompactedLog` 护栏防「日志已截断而快照缺失」的静默丢数据；索引自愈路径（`base_seq > 0` 补 `prune_events`）不删 tokens 投影。
  - **版本兼容**：读到更高 `version` 一律 fail-fast（日志/快照/logmeta 三处），不静默按 v1 重放未知语义；缺 `version` 才按 v1 兼容。
  - **拒绝服务面**：`keep_last` 为 `u64`，计算用 `saturating_sub`，无溢出/下溢；`/compact` 为 O(日志大小) 写放大操作，单个实例 flock 互斥故不可并发放大；`/health`/`/ready`/`/metrics` 及 `/compact` 均不参与令牌比较路径，不构成恒时比较旁路。
  - **信息泄露**：新增错误变体（`UnsupportedSchemaVersion`、`SnapshotRequiredForCompactedLog`）的 HTTP 映射只回显版本号/`base_seq` 等非敏感结构量，不额外暴露服务器路径（沿用既有 `Store` 500 通道）。令牌比较仍走恒时 `ct_eq`。
- **发现并处置（均为「补文档/补告警」，非代码缺陷）**：
  1. **压缩端点不校验令牌**（认证边界问题）：`POST .../compact` 与域生命周期端点一致不绑定参与者身份，且属**破坏性**操作。处置：在 `manual.md` §2.3.1/§7.4/§8.2 明确「令牌只覆盖四类动作端点，daemon 不得暴露给不可信网络，域级 ACL 属子项 (3) 未实现」，并在 `architecture.md` §5 安全边界行同步登记。
  2. **`compact_on_start` 静默丢弃审计前缀**：`compact_on_start = true` + `compact_keep_last = 0` 时每次重启都会丢弃全部已入快照前缀。处置：daemon 启动打印显式告警，manual §7.4 记入风险。
- **未处置（受限于范围，登记为已知限制）**：压缩端点无速率限制/并发配额（单写者锁已兜底）；域级 ACL 仍缺（D25 子项 3）。

## 2026-10-20（第 24 轮，D25 子项 1：日志压缩与 schema 版本化实施）

- **四项设计取舍（用户裁决，均选推荐项）**：压缩触发 = **显式运维接口 + 可选启动自动**；截断机制 = **前缀删除 + 旁路 base_seq 清单**；幂等表 = **纳入快照**；version 形态 = **每事件顶层 `version` 字段**。
- **schema 版本化（pact-store）**：新增 `WireEvent{version, #[serde(flatten)] event}` 与 `EVENT_SCHEMA_VERSION = 1`（唯一真源）。日志与线上格式统一经 `to_wire_json` 输出（实测行首为 `{"version":1,…}`）；读取经 `into_event()`，`version > 当前` ⇒ `StoreError::UnsupportedSchemaVersion` fail-fast；缺 `version` 由 serde `default` 视为 v1（旧日志/旧快照零迁移）。`Notification`（SSE）同步加顶层 `version`。
- **前缀压缩（pact-store）**：`Store::compact(key, snap, keep_last, compacted_at)` 五步序——① `write_snapshot`（**提交点**）② `base_seq = max(snap.seq - keep_last, prev_base)` 写 `<instance>.logmeta.json`（先于日志重写，水位**单调不回退**）③ `rewrite_retaining` 原子重写日志（保留 `seq > base_seq` 行原始字节，丢弃尾部半行，tmp+fsync+rename）④ `IndexStore::prune_events` 裁剪 `events` 行 ⑤ 移除并重建追加句柄，`last_seq = snap.seq`。返回 `CompactionResult{snapshot_seq, base_seq, retained, discarded}`。
- **快照自足锚（pact-store）**：`Snapshot` 扩为 `{version, seq, dl_seq, declaration, state, consumed_requests}`（`Snapshot::new(...)` 统一构造）。`consumed_requests` = Rule-E 幂等表（`request_id → (TokenId, ArchiveEntry)`）——压缩丢弃 `Consumed` 行后跨重启幂等仍生效。
- **恢复分支（pact-store + pact-engine）**：`read_and_sanitize(path, base_seq)` 的 seq 校验由「首行恒为 1」放宽为**连续性**（首行允许 ≤ `base_seq+1` 的未截断超集态，其余逐行 +1）。`Store::recover` 读 logmeta → 载快照 → 若日志无 `DomainLoaded` 则要求快照覆盖 `max(base_seq, first_seq-1)`，否则 `SnapshotRequiredForCompactedLog` fail-fast；`last_seq = max(日志末尾, base_seq, snap_seq)`；`base_seq > 0` 时补 `prune_events` 自愈崩溃窗口。索引一致性校验放宽为「索引不落后于日志」（`idx_last == log_last || (log_last == 0 && idx_last <= base_seq)`），避免误触发全量 `rebuild` 连带清空 tokens 投影。
- **引擎（pact-engine）**：`recover_instance` 新增 `snapshot_is_anchor` 分支（日志无 `DomainLoaded` ⇒ 以 `snap.declaration`/`snap.dl_seq`/`snap.state`/`snap.consumed_requests` 为基补尾部；无快照 ⇒ `EngineError::Recovery`）；幂等表基线 `idem_floor = base_seq`（快照锚）或 `dl_seq`（日志锚）。所有快照构造点统一走 `snapshot_of(&InstanceSession)`，保证恒自足。新增 `Command::Compact`、`Engine::compact(key, keep_last)`、`EngineConfig{compact_on_start, compact_keep_last}`；`Engine::spawn` 保持默认配置兼容，新增 `spawn_with_config`。恢复末尾按配置逐实例自动压缩（单个失败仅 `eprintln!` 告警，不阻断启动）。
- **接口面**：REST `POST /domains/{d}/compact` 与 `/domains/{d}/instances/{i}/compact`（`CompactReq{keep_last?}` → `CompactionOutcome{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events}`）；CLI `pact compact --domain D [--instance I] [--keep-last N]`（离线 `replay` 改经 `WireEvent` 校验版本）；MCP `pact_compact`（工具数 16 → 17）；`pact.toml` 新增 `compact_on_start`（缺省 false）与 `compact_keep_last`（缺省 0），`daemon.rs` 经 `spawn_with_config` 传递。
- **取舍与边界**：压缩后被丢弃前缀不再可查（审计视图 = 快照 + 保留尾部事件；`events` 索引行被裁剪、`tokens` 投影行保留）；v1 不做自动周期性轮转，压缩为运维显式动作。`SeqGap` 的既有语义（中部缺行 fail-fast）不变。
- **测试**：新增 14 项——store 单测 6（version 兼容/未来版本致命、截断后 base_seq 校验、logmeta 往返与缺失、compact 截断/keep_last/无快照 fail-fast）+ 既有 store 快照测试改造 + engine 集成 3（压缩保持状态与幂等、keep_last 保留尾部 + 线上 version、启动自动压缩）+ http e2e 2（压缩截断 + 幂等跨重启 + 404）+ CLI/MCP e2e 断言扩展。全仓 **145 项**全绿（基线 131 + 14）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**；`docs/pact*` 母本未动。

## 2026-10-20（第 23 轮，D25 子项 2：运维接口实施）

- **范围**：仅 D25 子项 (2)。(1) 日志压缩/schema 版本化、(3) 域级 ACL/审计读视图未动（均须按域实例双键重述）。
- **新增 §9.5 运维接口**：`GET /health`（存活，不依赖引擎）、`GET /ready`（就绪，探针 = 一次 `list_domains` 往返，不可应答 → 503）、`GET /metrics`（Prometheus 文本格式，`text/plain; version=0.0.4`）。指标：`pact_domains_declared`、`pact_instances_loaded`、`pact_counter_size{domain,instance}`、`pact_archive_size{domain,instance}`、`pact_terminated{domain,instance}`、`pact_uptime_seconds`；仅统计**已加载**实例（`InstanceNotFound` 跳过）。
- **认证口径**：三端点**免令牌**（用户裁决）。理由：探活/抓取惯例；部署默认 unix socket，TCP 建议 `127.0.0.1`。域操作端点认证不变。
- **PID 文件**：`Config.pid_file: Option<PathBuf>`（缺省不写，向后兼容）；启动写入自身 PID、退出（含信号路径）drop 删除；启动时陈旧文件且 PID 存活 → `AlreadyExists` 拒绝。Linux 以 `/proc/<pid>` 判定存活（PID 复用假阳性 → 保守拒绝），非 Linux 交由 `pactd.lock` flock 兜底。
- **优雅停机**：`pact-http::server` 引入 `watch` 停机广播 + `ConnTracker`（`AtomicUsize` + `Notify`）在途计数。SIGINT/SIGTERM 同路径 → 停止 accept → 有界（`DRAIN_TIMEOUT` = 10s）等待在途归零 → `engine.shutdown()` 终态快照 → 清理 PID/套接字。**有界是必须的**：SSE 长连接会占满窗口。监听循环签名增加 `watch::Receiver<bool>` 与 `ConnTracker`（5 处测试调用点同步）。
- **接口形状**：`ServerState` 增加 `started_at: Instant`（uptime 基准）；`text_response` 显式设 `Content-Type`。
- **测试**：新增 8 项——daemon 单元 3（PID 写入/删除、陈旧检测、未配置不写）+ daemon_e2e 3（免令牌、指标含已加载实例、停机停止 accept 且 drain 归零）+ 进程级 2（SIGTERM 优雅退出并清理 PID/套接字、陈旧 PID 拒绝启动）。全仓 **131 项**全绿（基线 123 + 8）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` / `pact-store` / `pact-engine` **零改动**；`docs/pact*` 母本未动。

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
