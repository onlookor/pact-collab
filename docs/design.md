# PACT 实现设计

对应需求：`docs/requirements.md`。范式规范：`docs/pactv1_rev3.md`（章节引用均指该论文最新版）。本文件只保留当前设计，变更历史记入 `docs/design-change.md`。

## 0. 决策记录

| # | 决策 | 结论 | 依据 |
|---|------|------|------|
| D1 | 实现语言 | Rust；不设 Python 学术层 | `docs/pact工程落地语言选择.md` 论证成立；补充：零隐式动作等不变量可由所有权/借用系统编译期强制；agent 经 MCP 接入，引擎语言与 agent 框架语言解耦，无需 Python 胶水层 |
| D2 | 持久化载体 | JSONL 事件日志为真相源 + SQLite 派生索引；索引引擎采用 turso（原 Limbo，Rust 重写的 SQLite） | 用户选定；turso 评估见 D13 |
| D3 | v1 接口范围 | MCP + REST 并行交付 | 用户选定；两者共享引擎门面，增量成本低 |
| D4 | Rule-W 唤醒 | 订阅推送 + 轮询兜底 | 用户选定 |
| D5 | 无人值守模式 | 引擎被动 + agent 自驱 | 用户选定；保持引擎的范式中立性（零隐式动作） |
| D6 | 并发模型 | 单权威 actor：全部状态变更经一个串行化执行点 | 论文中 Counter 是唯一协调介质；GateHolds 检查与状态变更必须原子 |
| D7 | 域管理 | 一个引擎实例承载多个命名域会话 | 避免联网阶段多协作流到来时重构 |
| D8 | 时钟 | Clock trait 注入（SystemClock / LogicalClock） | 确定性测试与重放（NFR3） |
| D9 | 读事件持久化 | v1 默认不持久化 Rule-R（无状态变更，重放不需要） | 保持事件日志与状态变更一一对应；审计流扩展位保留。（D25 将该扩展位落地为**可选开启**的读审计开关，默认仍关闭） |
| D10 | TokenId | ULID（有序、唯一、可读） | 便于日志排序与人工检视 |
| D11 | 身份模型 | v1 信任声明 + 可选按参与者令牌；接口层校验，核心不感知 | NFR5，联网版只替换接口层认证实现 |
| D12 | CPN 生成器 | 延后为可选阶段 6（**实现须遵论文 §4.5/§B.2 单实例作用域**：每个 Counter 实例独立编码、库所与迁移不跨实例共享、跨实例协同不入 `N(D)`；见 D27/G1） | 不阻塞核心价值交付 |
| D13 | SQLite 索引引擎 | 采用 turso，经 pact-store 内部 `IndexStore` trait 隔离 | turso 现状（2026-08）：BETA，官方自述未达 SQLite 级可靠性，但已在多家生产环境运行；本架构中 SQLite 仅是**可随时重建的派生索引**（真相源为 JSONL），beta 风险被完全对冲——最坏情况是删库重放重建。收益：纯 Rust 工具链（无 C 编译依赖，musl 静态编译顺滑）、async-first API 与 tokio 引擎契合、MVCC/`BEGIN CONCURRENT` 为联网版并发留余量。退路：若 turso 出现阻塞性问题，实现 `IndexStore` 的 rusqlite 后备并切换，仅动 pact-store 一个 crate |
| D14 | 语义收口 | 域良构校验、Rule-W 可满足性判定、Rule-T 终止判定均为 pact-core 纯函数；引擎层只编排不判定 | 三者均为范式语义（§1.4.4、Rule-W、Rule-T），若留在引擎层会造成语义分裂；同时供 analysis/compose/测试复用 |
| D15 | ~~Rule-T 语义~~（**已回滚**，2026-08-26） | ~~termination_holds 排除空前置组合（源类型生产）~~ | 论文 rev1（pactv1_rev1.md）以源生产约束原生解决该问题（Rule-P 前提 Precond(T) ≠ ∅ + 声明期拒绝）：含源类型 produce gate 的域直接 ill-formed，不存在"永不终止"的合法域。临时豁免失去存在意义，回滚并由 D17 取代 |
| D16 | parser 宽容度 | 忠于 §3.1 文法骨架（四段固定顺序、段落完整）；宽容：段内尾分号、preconditions/gates/seed_authority 空段（types 至少一项）；扩展："->" 等价 "→"、`//` 行注释、GateExpr 优先级 not > and > or（文法未规定结合性） | 严格拒绝合法语义的空段比接受更糟；扩展均为可用性增益且不改变可表达语义 |
| D17 | 源类型生产约束（论文原生语义，rev2 §1.4.4/§3.1 沿用） | 三层强制：(1) Rule-P 前提 `Precond(T) ≠ ∅`——源类型（无前置规则）不可生产，运行时返回 `RuleError::SourceTypeProductionDenied`；(2) 声明期良构校验 `DomainError::SourceTypeProduceGate`（§3.1 约束 6）；(3) `is_source_type` 查询函数 + A.5 静态分析独立复核。Rule-T 按论文 Rule-P-enabled 三条件定义实现（源类型天然排除）；enabled_productions 不返回源类型 | 论文 §1.4/§1.4.3/§3.1 的规范语义：源类型唯一入口是 SeedAuthority（入口唯一性定理），消除 D15 指出的"永不终止"异常于声明期；良构域上 termination_holds 与"Counter 为空"等价（终止论证）。影响：LIFECYCLE 黄金域更新（analyst 角色改为 seed authority、operator via 子句移除——论文 Part II §1.3） |
| D18 | compose C2 实践语义 | 共享类型前置一致性判定：**双方都声明规则且集合不同 → 冲突**；单侧声明合法（缺席方对 T 的前置无约束，合并取并） | 论文 §1.8 示例的规范语义：D₂ 共享 design 而不重复声明其前置（顺序组合正依赖此机制），若按 C2 字面（一侧 ∅ 一侧 {…} 即冲突）论文自身示例无法组合。C4 安全性不受影响：`produces T` 只可能来自视 T 为非源的良构片段（必有规则），规则并集只增不减 |
| D19 | 阶段 4 范围与接口接入方式 | pact-http **提前至阶段 4** 与 pact-cli、pact-mcp 同批交付；MCP 与 CLI 均不内嵌引擎，而是经 `pact_http::client`（REST over unix socket）接入同一 daemon；原规划的独立 `pact-server` bin 取消，daemon 由 `pact daemon start`（pact-cli 子命令）与 `pactd`（pact-http 裸 daemon bin）提供 | 用户决策。三接口共享单一 daemon 即单一 Counter 权威（D6），REST 成为唯一状态变更通道——接口等价性由构造保证（MCP/CLI 只是 REST 的形态转换），e2e 只需各走一遍 REST 即覆盖契约一致性；联网版（v2）只换传输与认证，接口层结构零改动 |
| D20 | CLI 参数风格与 MCP 传输范围 | CLI 域操作命令一律 `--domain` / `--participant` / `--token` / `--type` flag（不采用设计稿中 `pact enabled <participant>` 式位置参数简写）；连接参数 `--unix` / `--tcp` / `--bearer` 与输出开关 `--json` 为全局 flag；MCP v1 仅 stdio，Streamable HTTP 移交联网演进（v2） | flag 风格消除位置参数歧义、便于脚本化与 `--json` 组合；MCP 经 REST 接入（D19）后 Streamable HTTP 与 REST 同端口价值消失，随 v2 TLS/远程接入一并落地 |
| D21 | rev2 良构约束强制风格与 A.5 重构（用户选定推荐项） | (1) 空 LHS `{ } → T` 解析期语法拒绝（错误指名 Non-Empty LHS 约束），`validate_domain` 另设 `EmptyPrecondLhs` 声明期防御（覆盖程序化构造/组合出的 Domain）；(2) LHS 重复 `{A + A} → B` 在 parser `build_domain` 保留原始 `Vec<TypeName>` 序列时检测（`DuplicatePrecondLhs`）——`&Domain` 的 BTreeSet 会静默去重，此为唯一强制点；RHS 重复由 BTreeMap 键唯一构造性保证；(3) count 字面量 PosNat（`pos_nat()` 拒绝 0 与前导零，`ZeroCountLiteral` 二次防御）；(4) A.5 = `precondition_wf` 三元报告 `{well_formed, violations: Vec<DomainError>, warnings: Vec<WfWarning>}`，违规判定委托 `core::validate_domain`（单一判定源），Seed/Produce 重叠（§1.4.3 Advisory）为 `SeedProduceOverlap` 咨询性警告——**类型级判定**：produced 取全体参与者 produces gate 的并集，非源类型同时被 seed 与被任一人 produce 即告警，`participant` 字段为 seed 持有者；比论文 §1.4.3 的“同一参与者”表述更宽，取假阳性方向不漏报——论文维持参与者级原文，实现登记为参考算法的**保守超集**（2026-10-15 定案；论文曾误改三处、同日经用户裁决回退，规范是否修订由作者另行裁决）；聚合报告字段前置（附录 D：A.5 先于其余分析）；(5) `DomainError::UndeclaredType.referenced_by` 由 `&'static str` 改 `String`，使错误可内嵌进 serde 报告 | 解析期拒绝保持文法忠实；表示层（BTreeSet）无法承载的约束在序列仍在手时检测；A.5 委托避免判定逻辑分裂；前驱互异仅按 TokenId（Rule-P Note），类型级去重由 LHS Uniqueness 良构性免除 |
| D22 | 通用示例与用户手册作为交付资产 | 新增 `examples/`（领域无关通用域 `domains/`：初版四份，2026-10-18 按 D23 增设第五份 `external_materialization` 物化确认示例 + 两份组合片段 `fragments/` + 离线/在线两条上手脚本各带录制轨迹 + `README.md` 场景导航）与 `docs/manual.md`（权威用户手册，12 章）；**不接入 CI**（不新增遍历 `examples/**/*.pact` 的 cargo test）、**不改任何 crate**；脚本运行产物 `examples/online/run-data`、`examples/offline/out` 入 `.gitignore` | 用户决策（FR14）。示例是教学资产而非回归基线，接入 CI 会把示例漂移变成构建失败源，反而抑制示例演进；手册与示例分工为"参考权威 / 可运行路径"，交叉引用不重复维护同一事实。取证过程中发现的三处实现与文档不一致（CLI `reachability` 行把 `seed_types` 打印为"源类型"、MCP `pact_analyze` description 写"五项静态分析"实为六项、compose 语义在 CLI 只打印而 MCP/REST 会装载）**不改代码**，如实登记于 `docs/manual.md` §11 已知偏差与限制 |
| D23 | 消耗提交契约与 Rule-E 幂等（2026-10-18，用户裁决） | (1) **提交点 = JSONL fsync**：`store.append` 追加日志即视为已提交；其后步骤（索引投影、定期快照、Terminated 事件、通知）为**尽力而为的派生动作——失败不回滚、不失败本次调用**，索引滞后由启动 seq 一致性校验触发 `rebuild` 自愈。(2) **Consumed 事件自足**：携带 token 全量快照（`exhausted_at` = `ts`、`exhausted_by` = `by`）与可选 `request_id`，重放与审计不再反查同一域实例其他行。(3) **Rule-E 可选幂等**：同一域实例内重复 `request_id` 返回首次归档结果（不重复消费；跨重启由日志重建幂等表生效），同一 `request_id` 用于另一 token → `IdempotencyConflict`。(4) **原子域含 gate 读集**：`rule_e` 对同一内存 `CounterState` 求 GateHolds 并随即 apply，单写者 actor 消除 TOCTOU 与并发双消费。(5) **边界**：引擎不执行外部副作用，`consume` 成功与外部动作成功是**两个独立承诺**；"产物未就位"不应表现为消费回滚，而应前移为 gate 准入条件（`exists` 型见证 token，见 §8.5） | 此前 Rule-E 仅有单步 SOS 语义、未声明实现级原子性；本决策把它显式化为可验证契约。原实现 `Consumed` 只记 `token_id`、重放反查 Counter，使单行日志不自足、与 NFR2（日志即完整审计）不符；原写路径"索引失败即返回 Err"会造成日志/内存瞬态分歧直至重启。拒绝"库事务 / 2PC / 分布式共识"方案：单写者 actor + 事件日志已满足线性化与崩溃一致性，引入它们只增复杂度 |
| D24 | 生产运行层归属（2026-10-19，用户裁决） | 本仓库新增 crate `pact-runtime`（**已实现，阶段 7 完成**）：参与者驱动循环（订阅/轮询 → `my_gates` / `enabled` → 决策 → 行动）+ 多 agent 编排 + 外部副作用执行器（outbox/saga 消费端）。**引擎保持被动（D5）、MCP 维持 stdio-only（D20）均不变**；依赖方向 `runtime → http（类型化 REST client）/ engine（线上视图）/ core（标识符·token 类型）`，`pact-mcp` 仅作可执行文件由编排 spawn、不作 lib 依赖（详见 §1） | 驱动逻辑与 `Consumed` 事件 schema、`pact_*` 工具面强耦合，须与引擎版本锁定；抽为可复用运行时，避免每个消费方重复实现 P1/P2/P3 自驱策略；engine/store/core 零改动 |
| D25 | 引擎生产化三项（2026-10-19，用户裁决） | (1) 事件日志压缩与 schema 版本化（pact-store）；(2) 运维接口：health/ready、metrics、PID 文件、信号化优雅停机（pact-http + engine）；(3) 域级授权与审计读视图（pact-http 接口层），读审计**可选开启、默认关闭**。**(2) 已实现（2026-10-20，见 §9.5）**：`GET /health`（存活，不依赖引擎）/ `GET /ready`（就绪，探针 = 一次 `list_domains` 往返）/ `GET /metrics`（Prometheus 文本格式：`pact_domains_declared` / `pact_instances_loaded` / `pact_counter_size` / `pact_archive_size` / `pact_terminated` / `pact_uptime_seconds`）；运维端点**不校验令牌**（仅本地/内网）；可选 `pid_file`（缺省不写；启动写入、退出删除；陈旧文件且 PID 存活 → 拒绝启动）；优雅停机 SIGINT/SIGTERM → 停止接受新连接 → 有界（10s）等待在途 → 终态快照 → 清理。**(1) 已实现（2026-10-20，见 §7.5）**：`WireEvent` 顶层 `version` + 日志前缀压缩（快照自足锚 + `logmeta` 水位 + 原子重写）、REST/CLI/MCP 压缩入口、可选启动自动压缩。**(3) 已实现（2026-10-20，见 §9.6）** | 生产部署的可运维性与多租户边界（NFR7）；三者均为存储/接口层增强，不触碰核心语义与 SOS 规则。(3) 修订 D9 默认值表述——D9「审计流扩展位保留」在此落地为配置开关，默认仍不持久化 Rule-R |
| D26 | 分发产物归属（2026-10-19，用户裁决） | 本仓库 `dist/`：Dockerfile、systemd unit、musl 静态二进制打包脚本；不接入 CI 语义测试（同 D22） | 与源码同仓库保证版本一致；registry 推送、k8s 编排模板等外部细节不属本仓库 |
| D27 | 域双键实例化与形式层裁定（2026-10-20，领域专家裁定 + 用户裁决） | **立项**：把「域」拆为**定义键 domain + 实例键 instance**，同一域声明可并发承载多个 Counter 实例；实现走**路径 β**（`pact-store`/索引显式 `(domain, instance)` 双列 + 引擎**声明注册表**，`declare` 与 `instantiate` 分离）。**论文母本 rev2 → rev3**（用户破例授权），5 处改动：§1.3 增列「**Counter instantiation and partitioning**」条款、§1.4.1 TokenId 作用域措辞、Rule-T 增补 scope note、§4.5 与 §B.2 增补 CPN 作用域、§1.8 及 §4.1–4.3 术语 `domain instance` → `domain declaration`——**零文法变化、零语义变化**（`diff` 已核，9 处增补/替换；G3 就地修订 §1.4.1 Rationale 追加 1 处，**累计 10 处**，见 `docs/design-change.md` 第 20 轮）。**术语登记**：定义键 = 论文 `domain declaration`；实例键 = 论文 `Counter instance`（§1.3 描述性短语，非正式原语）；**中文文档/注释用「域实例」指该实例键**（即论文 `Counter instance`，与论文已废弃的英文 `domain instance` 无关，亦不替代论文术语）。**细节定案（2026-10-20，用户裁决）**：存储布局 = **域子目录** `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `<domain>/<instance>.snapshot.json`、审计流 `<domain>/<instance>.read-audit.jsonl` 同目录）；默认实例名 = **`default`**（不指定实例段 ⇒ 落到 `default` 实例）；声明绑定 = **实例创建时快照声明**（`DomainLoaded` 携声明原文，后续 `declare` 覆盖**不影响已运行实例**，重放自封闭、确定性优先，与 NFR2「日志即完整审计」一致）。**派生细节定案（2026-10-20，用户裁决，阶段 10 实施前置）**：旧日志**不兼容**——双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`，顶层遗留单层文件启动恢复时 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移；实例键以**路径段**承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` 等价 `default` 实例）；`DomainLoaded` **不新增 `instance` 字段**（实例由文件路径承载）；声明原文另存 `<data_dir>/<domain>/declaration.pact`（`declare` 时写入、`recover` 回填注册表）；**护栏**：`data_dir` 独占 flock（`<data_dir>/pactd.lock`）+ 非空实例（`last_seq > DomainLoaded.seq`）再加载默认拒绝（`DomainBusy`），须 `force`。**已实现（2026-10-20，阶段 10 路径 β，见 §11 阶段 10）** | 触发：两个并行项目共用同一域声明时，既有替换语义会销毁对方 Counter 与快照。专家裁定 Q1(a) 概念承认（以 §1.3 实现自由形式落地）、Q2(a) 术语消歧、Q3(a+c) 限定重述、Q4 保留「globally unique」但作用域收窄至该 Counter（实现以 ULID 加强为跨实例唯一）、Q5 确认**无任何论文级不变量依赖「一个域名至多一本 Counter」**、Q6 确认 CPN 编码需加单实例作用域、Q7 不新增正式术语。据 §1.3「Physical implementation of the Counter」，实例划分属**实现自由**，不触 §13 边界判据 (4)。评估与执行序、咨询简报与答复为**过程产物**，阶段 10 交付后已按用户指示移除（其结论已并入本条与 §11） |
| D28 | CPN 编码生成器（2026-10-21，用户选定阶段 6） | 新增 `pact-cpn` + CLI `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；输出 CPN Tools XML（`format='6'`）；单实例作用域以实例名命名空间化；颜色 `PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`，Archive 另加 Participant × Timestamp；生产前驱 test arc、读自环、消费归档、种子无输入弧；GateExpr → NNF → DNF，`or` 展开多迁移、`not` 下推为 `count T <= k` 并以互补库所编码（须显式上界）；`not via` 显式拒绝；条件恒假保留迁移 `guard=false`；DNF 上限 1024、count 展开上限 1024；输出经 CPN DTD 校验 + XML 良构/结构测试，CPN Tools GUI 加载人工验收 | 论文 §4.5/§B 只对正片段直出，负条件需已知上界；上界由运行期并发决定、不可从声明推导，故必须由调用方显式提供并承担正确性。`or` 用 DNF 多迁移而非 guard 近似，保持结构等价；`not via` 是实例级依赖关系而非计数，无法用互补库所表达，宁失败不生成错模型。生成器零 IO、零运行时依赖，不进入引擎/存储路径 |
| D29 | WASM 复用与纯类型层（2026-10-21，用户批准 P1） | 新增 `pact-store-model`（`Event`/`WireEvent`/`Snapshot`/`InstanceKey`/`ModelError`，无 IO）与 `pact-views`（查询/分析视图，无 tokio）；`pact-store`/`pact-engine` 通过 re-export 保持既有 API；磁盘格式与线上格式零变化 | 为 WASM 本地 spike 提供可编译到 `wasm32-unknown-unknown` 的共用模型层，事件 schema 与视图定义只保留一份，避免原生与 WASM 漂移。拆分边界：model 仅 `core + serde + serde_json`；views 为 `core + analysis + serde` |
| D30 | WASM 门面 crate（2026-10-21，用户批准 P2） | 新增 `pact-wasm`（`cdylib + rlib`）：可原生测试的 `PactHost` + 仅 wasm32 启用的 `wasm-bindgen` 包装；v0 导出 `new`/`load_domain`/`validate`/`analyze`/`compose`/`list_domains`，JSON 字符串 ABI；新增 `wasm-release` 体积优先 profile；暂不接 CI、分发与持久化 | 以最小门面验证 wasm-bindgen 导出、体积与静态分析对账；宿主核心与 wasm 包装分离，使原生测试无需 wasm 运行时。S0 不承担事件写入与存储适配（P3/P4），浏览器绑定与 OPFS 仍按 Q6 暂不做。**P3/S1 增补（2026-10-21）**：`PactHost` 增 `(domain, instance)` 会话表（每实例独立声明快照、事件序列、`last_seq`/`next_seq`、Rule-E 幂等表、`terminated`）与 `prepare_seed/produce/consume → commit/abort` 两阶段提交（prepare 分配 seq、abort 回收以保持日志连续；commit 应用状态并按原生顺序产出 Rule-T `Terminated`）；新增 `read`/`counter`/`archive`/`gates`/`enabled`/`status`、`export_events`（JSONL）、`export_snapshot`、`restore`（复刻 `recover_instance`）；ABI 升至 `pact-wasm/0.2`，错误码与原生接口层同名。固定 ID/ts 金标语料与原生日志逐行相等，JSONL 双向互通。**P4/S2 增补（2026-10-21）**：新增 Node 宿主适配层 `crates/pact-wasm/js/`（`FileStore` + `PactHost` + demo + `node:test`）：日志追加 `fsync`、末尾半行按字节截断修复、快照 tmp+rename+目录 `fsync`、`.lock` O_EXCL 单写者（含陈旧 pid 回收）；写路径 `prepare → append+fsync → commit`（失败 abort），打开优先日志/快照恢复；原生与 WASM 可交替打开同一日志 |
| D31 | WASM 本地分发形态条件立项（2026-10-21，P6 裁决 A） | 把 WASM 本地运行登记为**实验性可选分发形态**（Node/桌面宿主），权威仍是原生引擎；`pactd`、NFR4 单静态二进制与 FR18 生产分发承诺不变。ABI 固定 `pact-wasm/0.x` + JSON 字符串，`abi_version` 为探测入口，破坏性变更须升版并记入本变更文档。稳定化前置：Q7 CI → 金标扩面（多实例/组合域/条件分支/压缩恢复）→ 抽取 `pact-engine-kernel` → 宿主边界安全审视。不纳入：浏览器 OPFS/SharedWorker、多用户 ACL/审计、DSH `govern`、NFR6 v2 | P0–P4 证据表明语义与持久化行为可与原生对账（事件逐行相等、日志双向互通、崩溃恢复与单写者测试通过）；但双实现重叠约 300 行、金标覆盖窄，若长期保留必须用 CI 与 kernel 抽取防漂移。选项 B 会让 wasm target/bindgen 随工具链腐烂，选项 C 无需求证据且引入浏览器弱持久化与 v2 安全面，故取条件立项。详见 `docs/wasm-local-spike-feasibility.md` §11 |
| D32 | 抽取 `pact-engine-kernel`（2026-10-21，用户选定） | 新增纯内核 crate `pact-engine-kernel`（仅依赖 `pact-core`/`pact-parser`/`pact-analysis`/`pact-store-model`/`pact-views`，无 IO、无 tokio）：`Session` 统一持有 `declaration`/`domain`/`state`/`last_seq`/`next_seq`/`dl_seq`/`terminated`/`consumed_requests`，并提供 `fresh`、`recover`（锚点/快照/幂等表/终止态）、`prepare_seed`/`prepare_produce`/`prepare_consume`/`read`、`mark_prepared`/`abort_prepared`、`commit`/`record_terminated`、视图与 `snapshot`/`summary`，以及 `analysis_report`；`KernelError` 由调用方映射为各自错误类型。原生 `InstanceSession` 与 WASM `InstanceSession` 改为通过 `Deref` 嵌入同一 `Session`，各自仅保留自有字段（原生：`events_since_snapshot`/`notify`；WASM：`events`）。行为契约不变：DomainLoaded seq 从 1 起、失败命令仍推进时钟、Rule-T 终止 `seq = last_seq + 1`、压缩水位 `base_seq`、幂等表跨快照恢复 | P3 已量化双实现在 seq 分配、幂等表、Rule-T 终止、恢复锚点四处重复，长期保留会漂移（R1）。抽取把“语义会话”与“IO/actor/宿主编排”分离：内核保持纯函数式状态机，原生与 WASM 仅在错误映射、持久化时机与通知/审计等外围不同。`ConsumePreparation::Prepared` 以 `Box` 承载完整事件以控制枚举尺寸 |

## 1. 总体结构

Cargo workspace，crate 划分（每个保持小而聚焦）：

```
crates/
  pact-core      # 六原语、Domain、GateExpr、SOS 规则（纯函数，零 IO）
  pact-parser    # .pact 域声明文法解析 -> pact_core::Domain（含良构校验）
  pact-analysis  # 六项静态分析（论文 Appendix A.1–A.6；依赖 pact-core + petgraph）
  pact-compose   # 片段并行组合 + 四条件一致性检查（论文 §1.8）
  pact-store-model # 事件/快照/实例键纯模型（无 IO；原生存储与 WASM 共用，D29）
  pact-store     # 事件溯源：JSONL 追加日志、快照、turso 派生索引、重放
  pact-views     # 查询/分析视图纯类型（无 IO/tokio；原生引擎与 WASM 共用，D29）
  pact-engine-kernel # 实例会话语义纯内核（无 IO/tokio，D32）：Rule-S/P/R/E 准备、Rule-T 终止、恢复、视图；原生引擎与 WASM 共用
  pact-engine    # 引擎 actor：命令处理、订阅总线、Rule-T 检测、多域/多域实例会话；状态语义委托 kernel
  pact-wasm      # WASM 本地分发形态（实验性，D30/D31）：内存会话 + 两阶段提交 + 导出/恢复 + wasm-bindgen 导出 + Node 宿主适配层；未接 CI/生产分发
  pact-mcp       # MCP server（rmcp 实现），工具经 pact-http client 映射到引擎命令
  pact-http      # axum REST + SSE，路由映射到引擎命令；含类型化 client 与 pactd bin（裸 daemon）
  pact-cli       # clap 命令行（daemon 控制 + 领域操作 + 离线分析），单二进制 `pact`
  pact-runtime   # 生产运行层（D24）：参与者驱动循环（P1/P2/P3）+ 多 agent 编排（MCP/REST）+ 外部副作用执行器（outbox/saga 消费端）
  pact-cpn       # CPN 编码生成（阶段 6，D12/D28）：域声明 -> CPN Tools XML，离线、零运行时副作用
```

依赖方向（单向，禁止回指）：

```
parser / analysis / compose ──> core
store-model ──> core
store ──> core, store-model
views ──> core, analysis
engine ──> core, store, views
wasm ──> core, parser, analysis, compose, store-model, views（不依赖 store/engine/tokio）
http ──> engine
mcp ──> http（client，经 REST 接入 daemon）
cli ──> http（在线命令经 client；daemon start 复用其运行时）
cli ──> parser / analysis / compose / cpn / core（离线命令，不经 daemon）
cpn ──> core（Domain/GateExpr；不依赖 store/engine/http/parser）
runtime ──> http（类型化 REST client）、engine（线上视图）、core（标识符/token 类型）
            （生产运行层经 REST 接入 daemon；pact-mcp 仅作可执行文件由编排 spawn，不作 lib 依赖）
```

## 2. pact-core：核心语义（阶段 1，接口定义）

### 2.1 纯度约束（硬性清单）

pact-core 是范式的唯一权威语义实现，必须满足：

1. **无 IO**：不依赖任何 IO/async crate；不使用 `std::fs`、`std::net`、`std::process`、环境变量读取。
2. **无日志**：不使用 tracing/log 宏——日志是 IO 副作用，破坏可重放性。可观测性通过返回值（`Result`、报告结构）传递，由引擎层负责落日志。
3. **无时钟**：不读取系统时间。所有时间戳由调用方作为 `now: Timestamp` 参数传入（引擎侧注入 Clock，见 D8）。
4. **无随机**：不生成 TokenId。所有 id 由调用方作为参数传入（引擎侧注入 IdGen，见 D10）。
5. **无全局可变状态**：无 `static mut`、无 `thread_local!` 可变状态。
6. **依赖白名单**：`serde`（仅 derive，纯数据转换）为唯一非 std 依赖；`proptest`、`ulid`（仅测试辅助）只出现在 dev-dependencies。
7. **规则函数只读状态**：全部规则函数以 `&CounterState` 入参、以返回值（新 token / ArchiveEntry / 报告）出参，状态应用由调用方完成——签名层面强制"检查不改状态"。

审计结论（对照初版设计的修正）：
- Rule-T 终止判定、Rule-W 可满足性判定、域良构校验从引擎/parser 层**移入 core**（D14）；
- 增加显式 `Effect` 模型与 `CounterState::apply`，状态变更路径唯一且可测；
- `rule_s` 不读 CounterState（论文 Rule-S 前提不含 Counter 条件），签名相应省略。

### 2.2 数据类型

```rust
pub struct TokenId(pub String);          // ULID，newtype 防混用
pub struct TypeName(pub String);
pub struct ParticipantName(pub String);
pub struct ThingRef(pub Option<String>); // URI 或 ∅；对引擎不透明
pub type Timestamp = String;             // ISO-8601 UTC

pub struct Token {
    pub id: TokenId,
    pub ty: TypeName,
    pub thing_ref: ThingRef,
    pub produced_by: ParticipantName,
    pub depends_on: Vec<TokenId>,        // 直接前驱实例 id，生产时记录
    pub produced_at: Timestamp,
}

pub struct ArchiveEntry {
    pub token: Token,
    pub exhausted_by: ParticipantName,
    pub exhausted_at: Timestamp,
}

pub enum Action { Read, Consume, Produce }

pub enum CountOp { Ge, Eq }

pub enum GateExpr {
    Via(TypeName),
    Exists(TypeName),
    Count { ty: TypeName, op: CountOp, n: u64 },  // n 受 PosNat 约束（§1.7.3：n ≥ 1）
    And(Box<GateExpr>, Box<GateExpr>),
    Or(Box<GateExpr>, Box<GateExpr>),
    Not(Box<GateExpr>),
}

pub struct Domain {
    pub types: BTreeSet<TypeName>,
    /// rhs -> 前驱集合；良构保证每个 rhs 至多一条规则（§1.4.4）
    pub preconditions: BTreeMap<TypeName, BTreeSet<TypeName>>,
    pub gates: BTreeMap<(ParticipantName, Action, TypeName), Option<GateExpr>>,
    pub seed_authority: BTreeSet<(ParticipantName, TypeName)>,
}

pub struct CounterState {
    pub counter: BTreeMap<TokenId, Token>,
    pub archive: Vec<ArchiveEntry>,
}
```

`GateExpr` derive `PartialEq`——直接复用为组合时的"语法同一性"判定（§1.8 条件 3）。

### 2.3 域良构校验（D14，从 parser 移入）

```rust
pub enum DomainError {
    DuplicatePrecondRule { ty: TypeName },                       // §3.1 约束 1 RHS Uniqueness
    DuplicatePrecondLhs { ty: TypeName, lhs_ty: TypeName },      // §3.1 约束 2 LHS Uniqueness（`{A+A}→B`；仅 parser 可报，D21）
    EmptyPrecondLhs { ty: TypeName },                            // §3.1 约束 3 Non-Empty LHS（`{ }→T`；声明期防御）
    UndeclaredType { referenced_by: String, ty: TypeName },      // §3.1 约束 4 Type Closure
    ZeroCountLiteral { participant, action: Action, ty, count_ty }, // §3.1 约束 5 PosNat（程序化构造防御）
    ProduceWithCondition { participant: ParticipantName, ty: TypeName },
    SourceTypeProduceGate { participant: ParticipantName, ty: TypeName }, // §3.1 约束 6（D17）
    DuplicateGateClause { participant: ParticipantName, action: Action, ty: TypeName },
}

/// 源类型判定（论文 §1.4.3）：无前置规则（Precond = ∅，含规则缺席）即源类型，
/// 唯一入口为 SeedAuthority / Rule-S。
pub fn is_source_type(domain: &Domain, ty: &TypeName) -> bool;

/// 良构性检查（§1.4.4 + §3.1 六项约束 + 引用封闭性）。
/// 解析后、组合后、加载前调用；返回全部错误（非首错即停）。
/// 约束 2（LHS Uniqueness）在 &Domain 表示层由 BTreeSet 去重结构化排除，
/// 其强制点在 parser build_domain（D21）；约束 1（RHS）由 BTreeMap 键唯一保证。
pub fn validate_domain(domain: &Domain) -> Result<(), Vec<DomainError>>;
```

### 2.4 规则函数（纯函数）

```rust
pub enum RuleError {
    StaticGateDenied { participant, action: Action, ty: TypeName },
    GateConditionFailed { participant, action: Action, ty: TypeName, expr: GateExpr },
    PreconditionMismatch { ty, expected: BTreeSet<TypeName>, provided: BTreeSet<TypeName> },
    SourceTypeProductionDenied { participant, ty: TypeName }, // Rule-P 前提 Precond(T) ≠ ∅（D17）
    PredecessorNotOnCounter { token_id: TokenId },
    DuplicatePredecessor { token_id: TokenId },
    TokenNotOnCounter { token_id: TokenId },
    SeedAuthorityDenied { participant, ty: TypeName },
}

/// Rule-S（§1.4.3）：仅检查 (participant, ty) ∈ SeedAuthority；不查前置条件、不读 Counter。
pub fn rule_s(domain: &Domain, participant: &ParticipantName, ty: &TypeName,
              thing_ref: ThingRef, id: TokenId, now: Timestamp)
    -> Result<Token, RuleError>;

/// Rule-P（论文 rev1 §1.4）：检查静态 produce gate + Precond(T) ≠ ∅（源类型排除，D17）；
/// Precond(ty) 集合与提供的前驱类型集合相等（AND 语义）；
/// 前驱均在 Counter 上且互异。前驱不被消费——&CounterState 在类型层面保证只读。
pub fn rule_p(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              ty: &TypeName, predecessors: &[TokenId], thing_ref: ThingRef,
              id: TokenId, now: Timestamp)
    -> Result<Token, RuleError>;

/// Rule-R：token ∈ Counter；静态 read gate；GateHolds。无状态变更，返回 token 克隆。
pub fn rule_r(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              token_id: &TokenId)
    -> Result<Token, RuleError>;

/// Rule-E：检查同 Rule-R（action=Consume）；效果 = Counter 移除 + Archive 追加，无级联。
pub fn rule_e(domain: &Domain, state: &CounterState, participant: &ParticipantName,
              token_id: &TokenId, now: Timestamp)
    -> Result<ArchiveEntry, RuleError>;
```

### 2.5 GateHolds 与 GateExpr 求值（§1.4.0 / §1.7.3）

```rust
pub enum GateFailure {
    StaticDenied,                                    // (p, action, type(t)) ∉ gates
    ConditionFailed { expr: GateExpr },              // cond(t, Counter) = false
}

/// GateHolds：静态 gate + 条件子句合取；仅作用于 Read/Consume。
/// cond 为 None 或无匹配子句 → 空真成立。
pub fn gate_holds(domain: &Domain, state: &CounterState,
                  participant: &ParticipantName, action: Action /* Read|Consume */,
                  token: &Token) -> Result<(), GateFailure>;

impl GateExpr {
    /// 纯函数求值。签名只暴露 (target token, CounterState)——
    /// 接口层面强制 §1.7.1 的搜索空间（当前 Counter + depends_on）与纯度约束。
    pub fn evaluate(&self, target: &Token, state: &CounterState) -> bool;
}
```

`via` 求值（溯源敏感，回归测试锚点，论文 §4.2 State A/B）：

```rust
GateExpr::Via(ta) => state.counter.values()
    .any(|w| w.ty == ta && target.depends_on.contains(&w.id))
// 两条件须同时成立：w 在 Counter 上 ∧ w.id ∈ t.depends_on（实例级，非类型级）
```

### 2.6 效果应用（纯）

```rust
pub enum Effect {
    Seeded(Token),
    Produced(Token),
    Consumed(ArchiveEntry),
}

impl CounterState {
    /// 状态变更唯一路径：apply 规则函数的成功返回值。
    /// 引擎与测试共用，保证重放与在线行为逐字节一致。
    pub fn apply(&mut self, effect: Effect);
}
```

### 2.7 Rule-W / Rule-T 判定（D14，纯函数）

```rust
/// Rule-W 服务：参与者在当前状态下可满足的全部 Rule-P 组合。
/// 对每个 produce gate 类型 T：源类型排除（Rule-P 前提 Precond ≠ ∅，D17），
/// 其余检查每种前驱类型是否各有一枚实例（贪心见证选取）。
/// 不枚举全部组合（组合爆炸），只报可满足性与一组见证。
pub struct ProductionOption {
    pub ty: TypeName,
    pub witness_predecessors: Vec<TokenId>, // 一组满足前置的前驱实例
}
pub fn enabled_productions(domain: &Domain, state: &CounterState,
                           participant: &ParticipantName) -> Vec<ProductionOption>;

/// Rule-T 判定（论文 §1.4）：Counter = ∅ ∧ ¬∃ p: p is Rule-P-enabled。
/// Rule-P-enabled 三条件：(1) Precond(T) ≠ ∅；(2) produce gate；(3) 全部前驱
/// 类型在 Counter 上各有一枚实例。Counter = ∅ 时条件 (3) 必然失败，
/// 故良构域上本判定等价于 Counter 为空（终止论证）。
pub fn termination_holds(domain: &Domain, state: &CounterState) -> bool;
```

## 3. pact-parser：域声明文法（阶段 1，已实现）

- 手写递归下降解析器（文法极小，不引入 parser 生成器依赖；`logos` 做词法）。
- 文法即论文 §3.1：`types` / `preconditions`（含 `{A + B} → C` 多前驱形式）/ `gates`（`reads|consumes T [via T_a | when GateExpr]`、`produces T`）/ `seed_authority`。
- `via` 糖形式解析时脱糖为 `when via T_a`（§1.7.4），AST 唯一。
- 宽容度与扩展遵循 D16（段序严格、types 非空；尾分号/空段宽容；`->`、`//` 注释、优先级 not > and > or 为扩展）。
- rev2 解析期拒绝（D21）：空 LHS `{ } → T`（指名 Non-Empty LHS 约束）、count 字面量非 PosNat（0 / 前导零，指名 §1.7.3 与 Normalization rule）；LHS 类型重复 `{A + A} → B` 在 build_domain 报 `DuplicatePrecondLhs`（良构层）。

```rust
pub struct Span { pub line: usize, pub column: usize }

pub struct SyntaxError { pub span: Span, pub message: String }  // 含期望符号

pub enum Problem {
    Syntax(Vec<SyntaxError>),
    WellFormedness(Vec<pact_core::DomainError>),  // 转调 core::validate_domain（D14）
}

/// 一步到位：文本 -> 良构 Domain。返回全部问题（非首错即停）。
pub fn load_domain(src: &str) -> Result<Domain, Problem>;

/// 仅语法层（供工具链分步报告：先语法后语义）。
pub fn parse_raw(src: &str) -> Result<RawDeclaration, Vec<SyntaxError>>;
```

## 4. pact-analysis：静态分析（阶段 2，已实现，2026-09-09 按 rev2 重构 A.5）

六项算法按论文 Appendix A（rev2 版）直译，输入 `&Domain`，输出结构化报告（非仅布尔）：

| 分析 | 输出 |
|------|------|
| A.1 可达性 | `ReachabilityReport { seed_types, reachable, unreachable }`；单目标查询 `is_reachable` |
| A.2 生产覆盖 | 无任何 producer 的**非源类型**清单（源类型经 SeedAuthority 进入，排除） |
| A.3 消费可达性 | 无任何 consumer 的类型清单（技术债/滞留信号，含源类型） |
| A.4 结构死锁 | 无种子入口且不可外部馈入的 SCC 清单（`DeadlockComponent`） |
| A.5 前置与源生产良构性 | `PreconditionWfReport { well_formed, violations: Vec<DomainError>, warnings: Vec<WfWarning> }` 三元输出（rev2 Appendix A.5）；违规判定委托 `core::validate_domain`（单一判定源，D21）；`SeedProduceOverlap`（§1.4.3 Advisory）为咨询性警告 |
| A.6 搁浅风险 | `StrandingRisk { ty_b, ty_a, via_holder, consumer, potential }` 四元组清单；无环精确判定，有环标记 `potential`（保守） |

A.5 是其余分析的前提（附录 D 顺序）：聚合报告 `AnalysisReport` 将 `precondition_wf` 置于首位（引擎 `analyze_loaded` 经 `load_domain` 已拒绝非良构域，线上报告该字段恒为良构——独立复核语义保留给直接持有 `&Domain` 的调用方）。

实现要点：
- 内部用 `petgraph`（Tarjan/Kosaraju SCC、拓扑排序判环），前置图以 `DiGraphMap<&TypeName, ()>` 构建（边 X → T 当且仅当 X ∈ Pre(T)）；
- A.4 的"外部馈入"判定 = SCC 直接入边的前驱是否在 A.1 种子可达闭包内；
- A.6 提取复合 GateExpr 中全部嵌套 `Via` 原子（非仅顶层）；`potential` 按全局拓扑有环性判定（论文按拓扑性质在精确/保守两算法间选择）；
- 全部输出按 BTreeSet/BTreeMap 序确定排序（可作黄金断言与线上 JSON）。

## 5. pact-compose：片段组合（阶段 2，已实现）

```rust
pub enum Conflict {
    PreconditionMismatch { ty, pre_a, pre_b },   // C1+C2
    GateConditionMismatch { participant, action, ty, cond_a, cond_b }, // C3
    IllFormed(DomainError),                       // C4 终检透传
}

/// 多片段并行组合：从空域（组合单位元）折叠二元组合链；
/// 任一步冲突返回该步全部冲突明细。
pub fn compose(fragments: &[Domain]) -> Result<Domain, Vec<Conflict>>;

/// 二元并行组合 D₁ ∥ D₂（论文 §1.8）：四条件检查 → 四元组集合并 → 良构终检。
pub fn compose_pair(a: &Domain, b: &Domain) -> Result<Domain, Vec<Conflict>>;
```

四条件（论文 rev1 §1.8）：C1 前置唯一性保持、C2 共享类型前置一致（**D18 实践语义**：双侧都声明且不同才冲突，单侧声明合法——论文示例规范语义）、C3 gate 条件语法同一（`GateExpr: PartialEq` 深度相等，`Option<GateExpr>` 的 None 与 None 同一）、C4 源生产封闭（良构片段 + C2 + C3 下自动保持，经 `validate_domain` 终检兜底）。C1 的跨域情形（同 rhs 双规则不同前驱）为 C2 冲突特例（rhs 必属双方 Types，良构引用封闭）。

## 6. 领域数据格式（线上格式，三接口共享）

全部接口（MCP 工具参数/返回、REST 载荷、CLI 的 `--json` 输出、SSE/通知事件）共用以下 JSON 格式。字段名 snake_case；时间戳 ISO-8601 UTC。

### 6.1 `.pact` 域声明文件（注入点载体，论文 §3.1 文法）

```pact
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, consumes implementation, produces test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
```

（论文 rev1 Part II §1：requirement 为源类型，"analyst 引出需求"建模为种子权威而非生产 gate——D17 源生产约束。）

### 6.2 Token 与 ArchiveEntry（JSON）

```json
{
  "id": "01J9ZQ5T8W3N2R4Y6A7B8C9D0E",
  "type": "design",
  "thing_ref": "file://artifacts/design-001.md",
  "produced_by": "architect",
  "depends_on": ["01J9ZQ3M..."],
  "produced_at": "2026-08-25T10:15:30Z"
}
```

ArchiveEntry = Token + `"exhausted_by"` + `"exhausted_at"`。

### 6.3 动作请求/响应

以 Rule-P 为例（其余动作同构）：

```json
// 请求：POST /domains/{d}/tokens  /  MCP pact_produce 参数
{ "participant": "developer",
  "type": "implementation",
  "predecessors": ["01J9ZQ…"],
  "thing_ref": "file://src/impl-001" }

// 成功响应：HTTP 201 / MCP 工具返回
{ "token": { …Token… } }

// 失败响应：HTTP 4xx / MCP 工具 is_error 返回
{ "error": { "code": "GateConditionFailed",
             "participant": "operator",
             "action": "read",
             "type": "test_report",
             "expr": "via implementation",
             "message": "见证 token 不在 Counter 上：依赖的 implementation 实例已被消费" } }
```

`error.code` = `RuleError` / `Problem` 变体名，机器可判；`message` 面向人类。拒绝动作一律整体失败，无部分效果。

Rule-E 的请求体只带参与者，并可选一个幂等键：

```json
// 请求：POST /domains/{d}/tokens/{id}/consume  /  MCP pact_consume 参数
{ "participant": "developer", "request_id": "retry-2f9c" }   // request_id 可省略
```

同一域实例内重复的 `request_id` 返回首次归档结果（`{ "entry": ArchiveEntry }`），不重复消费；同一 `request_id` 用于另一 token 返回 HTTP 409 `IdempotencyConflict`（设计 D23）。

### 6.4 Counter 变更通知事件（Rule-W 唤醒，SSE `data:` / MCP notification）

```json
{ "version": 1,                 // 事件 schema 版本（D25 子项 1；缺省按 v1）
  "event": "Produced",          // Seeded | Produced | Consumed | Terminated
  "domain": "software-lifecycle",
  "instance": "default",
  "seq": 42,
  "ts": "2026-08-25T10:15:31Z",
  "token_id": "01J9ZQ…",        // Terminated 时省略
  "summary": { "counter_size": 3 } }
```

### 6.5 查询响应

- `GET /domains/{d}/counter?type=` → `{"tokens": [Token…], "size": n}`
- `GET /domains/{d}/enabled?participant=` → `{"options": [{"type": "implementation", "witness_predecessors": ["01J…"]}] }`
- `GET /domains/{d}/status` → `{"counter_size": n, "archive_size": m, "terminated": false}`

## 7. pact-store：事件溯源存储（阶段 3）

### 7.1 事件类型（真相源）

事件、线上封装（`WireEvent`）与 `apply_event` 定义在纯模型 crate `pact-store-model`（D29），`pact-store` 原样 re-export；原生存储与 WASM 门面共用该定义。

```rust
pub enum Event {
    DomainLoaded { name: String, declaration: String }, // 原文存档，可复现解析
    Seeded    { seq: u64, ts: Timestamp, token: Token },
    Produced  { seq: u64, ts: Timestamp, token: Token },  // token 内含 depends_on/produced_by
    Consumed  { seq: u64, ts: Timestamp, token: Token, by: ParticipantName,
                request_id: Option<String> }, // token 全量快照（自足）；request_id 为可选幂等键
    Terminated{ seq: u64, ts: Timestamp },                // Rule-T 检测结果（信息性）
}
```

线上/落盘形态统一为 `WireEvent{version: u32, #[serde(flatten)] event: Event}`（D25 子项 1）：
`EVENT_SCHEMA_VERSION = 1`，缺 `version` 视同 v1，更高版本读取 fail-fast（`UnsupportedSchemaVersion`，见 §7.5）。

JSONL：**每域实例**一个 `<domain>/<instance>.events.jsonl`，逐行 JSON 追加，`seq` 在该实例内单调。

### 7.2 存储布局

```
<data_dir>/
  <domain>/                        # 每个域声明一个子目录（D27）
    <instance>.events.jsonl        # 真相源，只追加（前缀压缩后为 `seq > base_seq` 的超集/尾部）
    <instance>.snapshot.json       # 定期 / 压缩时写入的快照（自足锚：version/seq/dl_seq/declaration/state/consumed_requests），原子写（tmp+rename）
    <instance>.logmeta.json        # 压缩水位（version/base_seq/compacted_at），缺省不存在（D25 子项 1）
    declaration.pact               # 声明原文（启动恢复时回填声明注册表）
  index.tursodb                    # 全部实例共用：events 与 tokens 的查询索引（(domain, instance) 双列）
  pactd.lock                       # data_dir 独占 flock（单写者护栏，D27）
```

快照数据结构 `Snapshot` 定义在 `pact-store-model`（D29）；`pact-store::snapshot` 只保留原子读写、损坏兜底与版本 fail-fast。

### 7.3 IndexStore trait（D13 隔离层）

```rust
/// 派生索引抽象。默认实现 TursoIndexStore（turso）；
/// 阻塞性问题出现时可实现 RusqliteIndexStore 替换，仅动本 crate。
#[async_trait]
pub trait IndexStore: Send {
    async fn open(path: &Path) -> Result<Self, StoreError>;
    async fn upsert_event(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError>;
    async fn upsert_token(&self, key: &InstanceKey, t: &Token, status: &TokenStatus) -> Result<(), StoreError>;
    async fn last_event_seq(&self, key: &InstanceKey) -> Result<Option<u64>, StoreError>;
    /// 裁剪实例内 `seq <= base_seq` 的事件行（压缩后的索引同步，D25 子项 1）。
    async fn prune_events(&self, key: &InstanceKey, base_seq: u64) -> Result<(), StoreError>;
    async fn rebuild(&self, entries: &[(InstanceKey, Vec<Event>)]) -> Result<(), StoreError>; // 从日志全量重建
}

索引 schema（turso 兼容 SQLite 方言；events 主键为 `(domain, instance, seq)`，D27）：

```sql
CREATE TABLE events(domain TEXT, instance TEXT, seq INTEGER, kind TEXT, ts TEXT, payload TEXT, PRIMARY KEY(domain, instance, seq));
CREATE TABLE tokens(token_id TEXT PRIMARY KEY, domain TEXT, type TEXT, produced_by TEXT,
                    thing_ref TEXT, depends_on TEXT /*JSON数组*/, produced_at TEXT,
                    status TEXT /*counter|archived*/, exhausted_by TEXT, exhausted_at TEXT);
CREATE INDEX idx_tokens_domain_type ON tokens(domain, type, status);
```

写入路径（引擎临界区内）：追加 JSONL（fsync）为**提交点** → 派生索引投影（**尽力而为**，失败不回滚、不失败本次调用）→ 更新内存态。启动路径：校验索引与日志末尾 seq 一致，不一致则 `rebuild`；加载最近快照 + 重放尾部事件恢复内存态。提交契约见 D23。

### 7.4 崩溃一致性

- **提交点 = 日志 fsync**：日志落盘即视为已提交，调用方收到的成功/失败由它唯一决定；
- 提交点之后的步骤（索引投影、快照、Terminated、通知）均可失败而**不回滚、不失败本次调用**——由此排除"日志已提交但返回错误、内存未 apply"的瞬态分歧（D23）；
- 快照写失败不影响正确性（重放兜底）；
- JSONL 半行写入（崩溃残留）→ 启动时检测并截断至最后一个完整行；
- 索引文件永远可丢弃重建——真相源唯一；索引滞后（投影失败）由启动 seq 一致性校验触发 `rebuild` 自愈；
- Consumed 事件自足（携 token 全量快照），重放与审计不依赖同一域实例其他行。
- **压缩窗口**（§7.5）：快照 → logmeta 水位 → 日志原子重写 → 索引裁剪；崩溃于任一中间点均落到「未截断超集态」或「已截断 + 水位 + 快照」两个一致态之一，恢复侧以 `base_seq > 0` 补 `prune_events` 自愈。

### 7.5 日志压缩与 schema 版本化（D25 子项 1，已实现）

**schema 版本化**：真相源与线上事件统一包裹于 `WireEvent{version, #[serde(flatten)] event}`，顶层 `version` 恒先于事件体出现在 JSON（`{"version":1,"kind":…}`）。`EVENT_SCHEMA_VERSION = 1` 为唯一真源（`pact-store::event`）。`Notification`（§6.4 SSE）同样带顶层 `version`。兼容契约：

| 情形 | 处理 |
|------|------|
| 旧日志/旧快照**缺 `version`** | serde `default` 视同 v1（向后兼容） |
| `version == EVENT_SCHEMA_VERSION` | 正常重放 |
| `version > EVENT_SCHEMA_VERSION` | **fail-fast `StoreError::UnsupportedSchemaVersion`**（不静默按 v1 重放，防误读新格式） |

**前缀压缩**：谓词 =「已入快照的 `seq ≤ base_seq` 前缀」。顺序（崩溃安全）：① 写**全量自足快照**（提交点）② 写 `<instance>.logmeta.json` 水位（`{version, base_seq, compacted_at}`，先于日志重写）③ tmp+fsync+rename **原子重写**日志（保留 `seq > base_seq` 行原始字节）④ 裁剪索引 `events` 行（`seq <= base_seq`）⑤ 重置追加句柄续接点 `last_seq = snap.seq`。水位**单调不回退**（重复压缩时 `base_seq = max(seq-keep_last, prev_base)`）。

**快照自足锚**：压缩后日志可能不含 `DomainLoaded`，`Snapshot` 因之承载 `version` / `seq` / `dl_seq` / `declaration` / `state` / `consumed_requests`（**Rule-E 幂等表随快照持久化**，否则前缀中 `Consumed` 行被丢弃会破坏跨重启幂等）。恢复分支：日志含 `DomainLoaded` ⇒ 与旧行为一致（快照 + 重放尾部）；不含 ⇒ **快照即锚**（`snapshot_is_anchor`），以 `snap.declaration`/`snap.dl_seq`/`snap.state`/`snap.consumed_requests` 为基补后续事件。日志已截断而快照缺失或不覆盖 `max(base_seq, first_seq-1)` ⇒ **`StoreError::SnapshotRequiredForCompactedLog` fail-fast**（数据安全优先于可用性）。

**seq 不变量（截断后）**：由「首行恒为 1」放宽为**连续性**——首行 `seq` 允许 ≤ `base_seq + 1`（未截断的超集态），其余行逐行 +1；`last_seq = max(日志末尾, base_seq, snap_seq)`。启动索引一致性校验相应放宽为「索引不落后于日志」（`idx_last == log_last || (log_last == 0 && idx_last <= base_seq)`），避免压缩后误触发全量 `rebuild`（会连带清空 tokens 投影）；`base_seq > 0` 且索引未裁剪时由 `recover` 补 `prune_events` 自愈。

**触发**：`POST /domains/{d}[/instances/{i}]/compact {keep_last?}`（缺省 0）→ `CompactionOutcome{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events}`；CLI `pact compact --domain D [--instance I] [--keep-last N]`；MCP `pact_compact`。**可选启动自动压缩**：`pact.toml` 的 `compact_on_start`（缺省 false）与 `compact_keep_last`（缺省 0），`EngineConfig` 承载，恢复完成后逐实例压缩，单个失败仅告警不阻断启动。

**固有取舍（须记入文档）**：压缩后被丢弃前缀不再可查——审计视图退化为「快照 + 保留尾部事件」，`events` 索引行被裁剪（`tokens` 投影行保留）。压缩是**运维显式动作**，不做自动周期性轮转（v1）。

## 8. pact-engine：引擎 actor（阶段 3）

查询/分析视图类型（`CounterView`、`ArchiveView`、`StatusView`、`GateClause`、`DomainSummary`、`DomainListing`、`CompactionOutcome`、`AnalysisReport`）定义在纯 crate `pact-views`（D29），`pact_engine::views` 原样 re-export。

### 8.1 执行模型

tokio 任务 + mpsc 命令通道，独占持有全部域的 `CounterState`。每个命令处理 = 查找域 → 调用 pact-core 纯函数 → 成功则 `store.append(事件)`（**提交点**）→ `state.apply(effect)` → 广播通知；提交点之后的步骤（索引/快照/通知）尽力而为（见 §8.5、D23）。检查与应用在同一临界区内原子完成（D6）。引擎**只编排不判定**（D14）：GateHolds、Rule-W、Rule-T 的判定全部转调 core。

```rust
pub enum Command {
    LoadDomain { name, declaration, resp },       // 替换语义：同名额重置会话、清理快照、seq 续接
    Validate   { declaration, resp },             // 只校验不落盘
    Seed     { domain, participant, ty, thing_ref, resp },
    Produce  { domain, participant, ty, predecessors, thing_ref, resp },
    Read     { domain, participant, token_id, resp },
    Consume  { domain, participant, token_id, request_id, resp },  // request_id 可选幂等键
    QueryCounter { domain, filter, resp },
    QueryArchive { domain, filter, resp },
    QueryGates   { domain, participant, resp },
    QueryEnabled { domain, participant, resp },   // 转调 core::enabled_productions
    QueryStatus  { domain, resp },                // 含 core::termination_holds 结果
    ListDomains  { resp },
    Subscribe    { domain, resp },                // 返回 broadcast::Receiver<Notification>
    Analyze      { text, resp },                  // 六项静态分析聚合报告
    ComposeAndLoad { name, fragments, resp },     // 组合 + emit 回写 + 加载
    Compact      { key, keep_last, resp },        // 日志前缀压缩（D25 子项 1）
    Shutdown     { resp },                        // 全部域终态快照后退出
}
```

启动恢复（actor 首要动作，失败则 fail-fast 排空后续命令）：逐域实例取**最后一次** DomainLoaded 解析声明；快照仅在 `snapshot.seq ≥ 该 DomainLoaded.seq` 时有效（防替换后旧快照复活）；重放尾部事件重建 CounterState 与 terminated 标记。**压缩后日志可能无 DomainLoaded**：此时以快照为锚（`snapshot_is_anchor`，声明/状态/`dl_seq`/幂等表均取自快照），日志只补尾部；快照缺失或不覆盖水位 ⇒ 启动 fail-fast（§7.5）。Terminated 事件只在 false→true 转移时追加一次。

Rule-E 处理顺序（D23）：幂等命中检查（同一域实例 `request_id`）→ `rule_e`（对同一内存 `CounterState` 求 GateHolds）→ `store.append(Consumed)`（**提交点**）→ `state.apply` → 记入幂等表 → `post_commit`（快照/Terminated/通知，尽力而为）。幂等表为 `request_id → (token_id, ArchiveEntry)`，日志锚时由最后一次 DomainLoaded 之后的全部 Consumed 事件重建；**快照锚时以快照的 `consumed_requests` 为基**再补尾部 `Consumed`，故压缩不破坏跨重启幂等（§7.5）。命中且 token 相同则返回首次结果，命中但 token 不同则 `IdempotencyConflict`。

### 8.2 通知（Rule-W 唤醒，D4）

每域实例一个 `tokio::sync::broadcast` 通道，状态变更事件（Seeded/Produced/Consumed/Terminated）即时广播（格式见 §6.4）。MCP 端：工具调用返回 + 订阅通知（rmcp 支持 server notification）；REST 端：SSE `GET /domains/{d}/events`。轮询兜底 = `QueryEnabled` / `QueryStatus`。

### 8.3 Rule-T 检测

每次状态变更后惰性求值 `core::termination_holds` → 广播 `Terminated`。信息性事件，不锁定域实例（种子仍可注入）。

### 8.4 时钟与 id 生成（D8/D10）

`trait Clock { fn now(&self) -> Timestamp }` + `trait IdGen { fn next(&self) -> TokenId }`，由引擎构造时注入（生产: SystemClock + UlidGen；测试: LogicalClock + 顺序 id）。

### 8.5 消耗原子性契约与物化边界（D23）

Rule-E 的实现级保证（可验证契约）：

| 保证 | 机制 |
|------|------|
| 线性化 | 单写者 actor 串行处理命令；线性化点 = `Consumed` 事件落盘 |
| 至多一次 | 同一 TokenId 再次消费返回 `TokenNotOnCounter`；带 `request_id` 时重复请求返回首次结果 |
| Gate 原子性 | `rule_e` 与 `apply` 落在同一临界区，GateHolds 读集（`via` / `exists` / `count` / `not`）不会在提交点前被并发修改（无 TOCTOU） |
| 归档原子性 | Counter 移除与 Archive 追加由同一事件导出，二者永不分裂 |
| 持久原子性 | 提交点 = 日志 fsync；此后步骤尽力而为，崩溃后重放等价于已提交前缀 |
| 幂等 | 可选 `request_id`，重复请求返回首次结果（跨重启由日志重建幂等表生效） |

**边界（引擎不承诺的部分）**：引擎不执行外部副作用，`ThingRef` 对引擎不透明、不校验其指向物是否存在。因此"`consume` 成功"与"外部动作（产物真正生成/落地）成功"是两个**独立承诺**，二者之间是**最终一致**而非同一事务：消费成功既不等待、也不回滚外部动作。落地方案有两种，**可并用、不互斥**（同一域可按入口分别选用）：

- **前移为准入条件（witness-token 模式）**：把"外部产物已就位"表示为一枚 token（外部系统经 Rule-S 注入），消费 gate 加 `when exists <readiness>`。于是"就位确认"进入 GateHolds 读集，与消费共享同一次判定——未就位则 `GateConditionFailed` 拦下。示例见 `examples/domains/external_materialization.pact`。
- **后置为显式补偿（saga 式）**：消费照常提交，外部动作失败后由参与者以一次新的 Rule-P 记录"未完成/需重做"事实；历史不可回滚，补偿是追加的新事实（引擎只负责把含全量 token 的 `Consumed` 事件发出，外部动作由 agent/执行器承担，即 outbox 语义）。

两种方案都不引入库事务 / 2PC / 分布式共识——其代价高于本场景收益（D23）。

## 9. 接口层（阶段 4，已实现）

三接口共享 §6 线上格式，能力对等；差异仅在传输与形态。接入方式（D19）：daemon 是唯一权威进程（`pact daemon start` 或 `pactd`，REST + SSE）；pact-mcp 与 pact-cli 在线命令均经 `pact_http::client` 走 REST 接入 daemon，不内嵌引擎。

### 9.1 pact-mcp（AI agent 主接口，已实现）

MCP server（rmcp），v1 传输仅 stdio（D20；Streamable HTTP 随 v2 联网接入落地）。`pact-mcp --unix <sock>`（或 `--tcp`）由 agent 拉起为子进程，工具经 REST 转发至 daemon。工具清单（前缀 `pact_`）：

| 工具 | 语义 |
|------|------|
| `pact_domain_validate` | 校验声明文本，返回良构/分析报告 |
| `pact_domain_load` | 加载/替换命名域（幂等按 name） |
| `pact_list_domains` | 已加载域清单 |
| `pact_analyze` | 六项静态分析聚合报告（A.5 `precondition_wf` 前置） |
| `pact_compose` | 组合多片段并加载 |
| `pact_seed` | Rule-S |
| `pact_produce` | Rule-P（显式传前驱 token id 列表） |
| `pact_read` | Rule-R（返回 token 全量数据） |
| `pact_consume` | Rule-E（可选 `request_id` 幂等去重） |
| `pact_counter_query` / `pact_archive_query` | 状态查询（按类型过滤） |
| `pact_my_gates` | 参与者的全部 gate 子句（含条件），agent 自省 |
| `pact_enabled` | Rule-W 轮询：当前可满足的 Rule-P 组合 |
| `pact_status` | Counter 概要 + terminated |
| `pact_compact` | 日志前缀压缩（D25 子项 1；`keep_last` 保留尾部条数） |
| `pact_subscribe` | 注册 Counter 变更通知（Rule-W 唤醒） |

工具的输入输出 schema 由类型派生（serde + schemars），错误以 §6.3 结构化 JSON 返回，agent 可编程处理（如 gate 被拒后转入等待）。

### 9.2 pact-http（应用程序接口，已实现）

axum，JSON over HTTP，监听 unix socket（默认，单机安全）与/或 127.0.0.1（可配）。载荷格式一律取 §6。实现注记：`{d}:analyze` / `{d}:compose` 因路由库不支持同段"参数+字面量"，在 `/domains/{d}` 统一入口内按段内冒号后缀分发（对外 URL 不变）；另提供独立 `POST /validate`（声明校验，不触域）与类型化 Rust client（pact-mcp / pact-cli 共用）。

```
POST   /domains                       {name, declaration}
GET    /domains                       域清单
GET    /domains/{d}                   域信息 + 状态摘要
DELETE /domains/{d}                   卸载（内存态，日志保留）
POST   /domains/{d}:analyze | :compose
POST   /domains/{d}/seeds             Rule-S
POST   /domains/{d}/tokens            Rule-P {participant, type, predecessors[], thing_ref}
GET    /domains/{d}/counter?type=
GET    /domains/{d}/archive?type=
POST   /domains/{d}/tokens/{id}/read     {participant}   Rule-R
POST   /domains/{d}/tokens/{id}/consume  {participant[, request_id]}   Rule-E
GET    /domains/{d}/gates/{participant}
GET    /domains/{d}/enabled?participant=
GET    /domains/{d}/status
POST   /domains/{d}/compact           {keep_last?}   日志前缀压缩（D25 子项 1）
GET    /domains/{d}/events            SSE 订阅（Rule-W 唤醒）
GET    /domains/{d}/read-audit?limit= 读审计视图（D25 子项 3；default 实例）
```

被拒动作返回 4xx + §6.3 结构化错误体（错误码 = RuleError 变体名）。

**实例路由（D27，阶段 10，已实现）**：上表 `/domains/{d}/…` 语义 = `default` 实例。实例级为 `/domains/{d}/instances/{i}/…` 镜像组（seeds / tokens / tokens/{id}/read / tokens/{id}/consume / counter / archive / gates/{participant} / enabled / status / compact / events / read-audit 全套同形），另加实例管理端点：

```
POST   /domains                       {name, declaration[, instance][, force]}   declare + instantiate（默认 default）
POST   /domains/{d}/instances         {instance}                                 在同一域声明下 instantiate 新实例
GET    /domains                       域清单（Vec<DomainListing{name, instances}>）
```

- `default` 缺省回退：请求未给实例段 ⇒ `default` 实例，既有脚本语义不变。`InstanceKey{domain, instance}` 的 `Display = "domain/instance"`。
- 实例键以**路径段**承载；`DomainLoaded` 事件**不新增 `instance` 字段**（实例由文件路径承载）。
- 键名谓词 `[A-Za-z0-9_-]{1,128}`（`validate_domain_name` / `validate_instance_name`）在 `engine.instantiate` / `load_domain`（唯一触盘入口）强制，防路径穿越；未实例化键的动作端点返回 `InstanceNotFound`(404)，不触盘。
- 错误码扩展：`InstanceNotFound`(404) / `DomainNotDeclared`(404) / `InstanceExists`(409) / `DomainBusy`(409)（非空实例再加载未带 `force`）/ `InvalidDomainName`(400) / `InvalidInstanceName`(400)。
- 认证：实例级与默认级共享同一 `*_inner` handler，`authorize` 统一在 `*_inner` 内完成，实例路由同样受令牌约束。
- **域级 ACL（D25 子项 3，已实现）**：`[auth.acl]` 非空即启用。实例级端点（生命周期、四类动作、查询、compact、读审计视图）按 `(domain, instance)` 校验；定义级 `:analyze` 要求对该域任一授权；`GET /domains` 按授权过滤（空域剔除）。无令牌 `401`，越权 `403`；ACL 空表时全部检查短路（既有行为不变）。

### 9.3 pact-cli（人类接口，已实现）

单二进制 `pact`（D20：域操作一律 `--domain` / `--participant` / `--token` / `--type` flag，不用位置参数简写）。全局 flag：`--unix <sock>`（缺省 ./pact.sock）/ `--tcp <addr>`（优先于 --unix）/ `--bearer <token>` / `--json`（§6 线上格式输出）。

- 注入点：`pact domain validate <file.pact>`；`pact domain load <file.pact> [--name N] [--instance I] [--force]`；`pact seed --domain D --participant P --type T [--thing-ref R]`。
- 领域操作：`pact produce --domain D --participant P --type T [--predecessors id1,id2] [--thing-ref R]`；`pact read --domain D --participant P --token ID`；`pact consume --domain D --participant P --token ID [--request-id R]`；`pact counter ls --domain D [--type T]`；`pact archive ls --domain D [--type T]`；`pact gates show --domain D --participant P`；`pact enabled --domain D --participant P`；`pact status --domain D`；`pact compact --domain D [--instance I] [--keep-last N]`（日志前缀压缩，D25 子项 1）。
- 域管理：`pact domain list|info --domain D|unload --domain D`；**实例（D27）**：`pact domain instantiate --domain D --instance I`、`pact domain instances --domain D`（打印该域实例清单）；域操作命令均接受可选 `--instance I`（缺省 = `default`），`--force` 仅用于非空实例的显式重置。`pact domain list` 打印 `name [instances…]`。
- 离线能力（不经 daemon）：`pact analyze <file.pact>`、`pact compose f1.pact f2.pact ...`、`pact replay <domain>.events.jsonl`。
- daemon 控制：`pact daemon start --config pact.toml`（前台运行，与裸 `pactd` bin 等价）。
- 在线命令经 unix socket 调 REST API（复用 `pact_http::client`）；错误按 §6.3 结构化输出，exit code 非 0。

### 9.4 配置与身份（D11）

`pact.toml`：

```toml
data_dir = "./data"
listen_unix = "./pact.sock"        # 默认；listen_tcp 可选
compact_on_start = false           # 可选：恢复后自动压缩全部实例日志（D25 子项 1）
compact_keep_last = 0              # 可选：自动压缩保留的尾部事件条数
read_audit = false                 # 可选：持久化 Rule-R 读审计（D25 子项 3；fail-closed）
[mcp] enabled = true
[auth]
# 可选：参与者令牌表；缺省 = 信任声明模式
[auth.tokens]
architect = "…"   # Authorization: Bearer
# 可选：域级授权（D25 子项 3）；非空时必须同时配置令牌表
[auth.acl]
architect = ["proj/*", "other/default"]
```

接口层在命令进入引擎前完成参与者身份校验；核心与引擎不感知认证方式（联网版只换此层）。ACL 启用后的身份来源：动作端点 = body/query 的 `participant`（须与 Bearer 令牌归属一致）；生命周期与查询端点 = Bearer 令牌反查的参与者。`Config::validate()` 在 daemon 启动前拒绝「ACL 非空 + 令牌表为空」及非法条目。

### 9.5 运维接口（D25 子项 2，已实现）

daemon 在域端点之外暴露三个**免令牌**运维端点（仅本地/内网，探活与抓取惯例）：

| 端点 | 语义 | 响应 |
|------|------|------|
| `GET /health` | 存活（liveness）：进程在服务即 200，不依赖引擎 | `{"status":"ok"}` |
| `GET /ready` | 就绪（readiness）：探针 = 一次 `list_domains` 往返；引擎可应答（恢复完成且未停机）→ 200，否则 503 | `{"status":"ready","domains":n,"instances":n}` / `{"status":"not_ready","error":…}` |
| `GET /metrics` | 运行指标，Prometheus 文本暴露格式（`text/plain; version=0.0.4`） | 见下 |

指标集（gauge）：`pact_domains_declared`、`pact_instances_loaded`、`pact_counter_size{domain,instance}`、`pact_archive_size{domain,instance}`、`pact_terminated{domain,instance}`（1/0）、`pact_uptime_seconds`。仅上报**已加载（内存）**实例——磁盘存在但未加载者由 `status` 报 `InstanceNotFound`，跳过；引擎不可应答时返回 `503`。

**PID 文件（可选）**：配置 `pid_file` 缺省不写；启动写入自身 PID、退出（正常与信号停机）删除；启动时若发现陈旧文件且其 PID 仍存活 → 拒绝启动（`AlreadyExists`），避免两个 pactd 争同一 pid 文件。存活判定 Linux 下以 `/proc/<pid>` 存在为准（PID 复用可能假阳性 → 保守拒绝），其他平台交由 `pactd.lock` flock 兜底。

**优雅停机**：SIGINT 或 SIGTERM 均走同一路径——广播停机 → 各监听循环停止接受新连接 → **有界（10s）**等待在途连接归零 → `engine.shutdown()`（终态快照）→ 清理 PID 文件与套接字。SSE 长连接会占满宽限窗口，故必须设有界超时（超时后强制进入终态快照，不阻塞退出）。

### 9.6 域级 ACL 与读审计（D25 子项 3，已实现）

**ACL 模型**：`[auth.acl] principal = ["domain/instance", "domain/*"]`。匹配仅限两种形态（拒绝 `proj`、`proj/*/x`、路径穿越等）；启用后所有域作用域端点先鉴权再进引擎：

| 端点类别 | 身份来源 | 授权键 |
|----------|----------|--------|
| load / instantiate / unload / compact / compose | Bearer 令牌反查参与者 | `(domain, instance)`（compose/默认路径 = `default`） |
| seed / produce / read / consume / gates / enabled | body/query `participant`（令牌归属须一致） | `(domain, instance)` |
| counter / archive / status / events / read-audit | Bearer 令牌反查参与者 | `(domain, instance)` |
| `:analyze`（定义级） | Bearer 令牌反查参与者 | 该域任一授权 |
| `GET /domains` | Bearer 令牌反查参与者 | 逐实例过滤，空域剔除 |

无令牌 → `401 Unauthorized`；令牌有效但无该键授权 → `403 Forbidden`（结构化 §6.3 错误体）。ACL 空表时上述检查全部短路，保持既有信任声明行为。daemon 启动前 `Config::validate()` 拒绝「ACL 非空 + 令牌表为空」（无法判定身份）及非法条目。`/health` `/ready` `/metrics` `/validate` 不属于域作用域，保持免令牌。

**读审计流**（FR17，默认关闭）：`read_audit = true` 时，每次 Rule-R 成功读取后由引擎 actor（单写者）向 `<data_dir>/<domain>/<instance>.read-audit.jsonl` 追加一行并 `fsync`：

```json
{"version":1,"ts":"…","domain":"proj","instance":"default","participant":"architect","token_id":"…"}
```

- **fail-closed**：写审计失败 → `EngineError::Store` → REST `500`，本次读取不返回 token；杜绝「读成功但无审计」。
- **与核心语义隔离**：审计流不产生事件、不推进 `seq`、不参与重放/Rule-T/快照/压缩；实例未加载也可查询历史审计。
- **读取**：`Store::read_audit_tail` 按写入顺序取末尾 N 条（缺省 100、上限 1000）；仅当文件末尾无换行（崩溃残留半行）时容忍末行解析失败，其余损坏行 fail-fast（不静默跳过，避免掩盖篡改）。
- **视图**：`GET /domains/{d}[/instances/{i}]/read-audit?limit=N` → `{"records":[…]}`，受 ACL 约束。

## 10. 测试策略（FR13，四层）

| 层次 | 范围 | 位置 |
|------|------|------|
| 单元测试 | core 规则函数/GateExpr 求值/validate_domain；parser 文法与错误报告；analysis/compose；store 读写 | 各 crate `#[cfg(test)]` |
| 属性测试 | Rule-R 后 Counter 逐字节不变；Rule-P 后前驱仍在 Counter；Rule-E 只影响目标 token 与 Archive；GateExpr 求值不改状态（纯度）；apply+重放等价 | pact-core proptest |
| 集成测试 | 引擎命令流（多域、并发命令串行化）、store 崩溃恢复（JSONL 半行截断、索引重建）、订阅通知时序、**Rule-E 幂等（request_id 去重与跨重启生效）** | crates/tests |
| 端到端 + 回归 | 黄金轨迹逐事件比对：软件工程生命周期 t₀–t₁₀、医疗路径 t₁–t₁₃（via 全部通过点）、ReAct 循环（循环拓扑 + Counter O(1)）、**§4.2 区分性场景 State A/B**（via 溯源敏感性回归锚点）；MCP 与 REST 契约等价；无人值守全流程 | 阶段 4/5 的 e2e 测试 |
| WASM 门禁 | `wasm32-unknown-unknown` 构建 + wasm 目标 clippy + ABI 版本断言 + 最小语义冒烟（validate/analyze）+ 体积上限（raw ≤ 600 KiB、gzip ≤ 200 KiB）+ Node 持久化测试 | `.github/workflows/ci.yml` 的 `wasm` job；脚本 `crates/pact-wasm/js/ci-smoke.js` |

## 11. 实现阶段

| 阶段 | 内容 | 验收 | 状态 |
|------|------|------|------|
| 1 | pact-core + pact-parser + 黄金轨迹/属性测试 | 论文全部示例轨迹通过；core 纯度约束清单（§2.1）逐条满足 | **已完成（2026-08-25，2026-08-26 按 rev1 更新）**：27 项测试全绿（11 单元含源类型约束专项 + 2 属性 × 512 用例 + 5 黄金 + 9 parser）；纯度核验（依赖树仅 serde、禁用 API 扫描零命中）；D15 已回滚、D17 生效（论文 rev1 源生产约束）；已知缺口：容器内无 rustfmt/clippy 组件（无 rustup/sudo），待环境补齐后补跑 |
| 2 | pact-analysis + pact-compose | 六项分析对论文域输出正确；组合一致性冲突可检出 | **已完成（2026-08-26）**：20 项测试全绿（analysis 12：四论文域验证 + 死锁/馈入/不可达/循环保守/间接祖先/嵌套 via 用例；compose 8：论文 §1.8 示例逐字段比对 + 四条件冲突各型 + 单位元/交换律/幂等合并）；D18 落定（C2 实践语义） |
| 3 | pact-store + pact-engine | 重放等价、崩溃恢复、多域会话 | **已完成（2026-08-26；2026-10-18 按 D23 补充消耗提交契约）**：新增 31 项测试全绿——store 18（事件 JSON 往返、**Consumed 自足（空 Counter 上重放）**、**幂等键序列化**、seq 逐行 +1、域名校验、索引滞后重建、快照原子写重启存活、日志清洗）；engine 单元 4（ISO-8601 纪元含闰日、逻辑时钟单调、顺序/ULID id）；集成 11（全生命周期 + 通知时序 seq/规模断言、快照路径与纯重放路径状态逐字段等价、崩溃半行截断后可续作、索引丢失重建、中部损坏 fail-fast、多域隔离、8 并发 seed 串行化、域替换恢复、validate/analyze/compose 命令、Clock/IdGen 确定性注入、**Rule-E request_id 幂等与跨重启生效**）。纯度核验：pact-core 依赖树仅 serde，IO/时钟/随机 API 扫描零命中。D23 落定（提交点 = 日志 fsync；索引投影尽力而为；Consumed 自足；Rule-E 可选幂等） |
| 4 | pact-http + pact-cli + pact-mcp（stdio）（D19：pact-http 提前） | 人类与 agent 均可走完生命周期 | **已完成（2026-09-04）**：pact-http（axum daemon：unix socket 默认/TCP 可选、§6.3 类型化错误体、SSE 通知流、类型化 client、pactd bin）+ pact-mcp（rmcp stdio，15 个 `pact_` 工具，经 REST 接入，schemars 派生 schema）+ pact-cli（单二进制 `pact`：daemon start + 在线命令经 client + 离线 analyze/compose/replay + `--json`）。e2e 三绿：HTTP daemon 全生命周期（含 404/错误码映射、SSE、compose/unload）；CLI 真实子进程 daemon 全生命周期 + 错误路径 + 离线命令 + 事件日志回放对照；MCP 真实 stdio 子进程 15 工具全生命周期 + 3 错误路径。workspace 累计 95 项测试全绿。D19/D20 落定 |
| 5 | 无人值守场景验证（pact-http 已提前至阶段 4 交付） | 多 agent 经 MCP 自驱完成 ReAct/软件工程域，全程无介入至 Terminated；含 MCP Streamable HTTP 与否由联网演进决定（D20：v1 不做） | **已完成（2026-09-05；2026-09-09 修正场景 B 卫生断言）**：`pact-mcp/tests/unattended_e2e.rs` 单测试内顺序验证两场景，测试进程只做"外部启动机制"（拉起 daemon + N 个真实 `pact-mcp` stdio 子进程），此后零 Rule-S/P/R/E 介入。场景 A（ReAct）：user 播种 + agent 私有生产预算 7，自驱两轮循环至 Terminated，7 产/8 消/7 读、Counter 恒 ≤2（单 agent 串行，真不变量）、archive 序两轮同构；场景 B（软件工程生命周期）：external_stakeholder 播种 + 5 worker 并发自驱接力至 Terminated，4 产/5 消/4 读、角色分工与溯源链 requirement→…→deployment 逐项核验、Counter ≤ 类型数 5（结构上界；典型调度 ≤2，但 CI 慢调度下下一阶段可在上一阶段 produce→consume 窗口内合法生产，窗口因 produces 门无条件子句而语义上不可消除，2026-09-09 CI 实测 max_counter=3 后断言修正为结构上界）。agent 自驱策略 P1 生产优先/P2 汇消费/P3 等待（订阅推送 + 轮询兜底，Rule-W）。修复两类自驱竞态：①播种前过早终止（域加载与种子注入间 Counter=∅ 使 `termination_holds` 平凡为真，worker 误判退出）→ 仅当 `archive_size>0` 才承认终止（符合 §"Terminated 信息性、不锁定域"，修复落在 agent 驱动层非引擎）；②Terminated 一次性推送与轮询判定竞态 → 终止后 500ms 宽限窗口排空在途推送。合并为单 runtime 顺序执行以规避 tokio 多 runtime 子进程回收陷阱。D20 生效（v1 不做 Streamable HTTP） |
| 6（可选，已实现） | pact-cpn：CPN 编码生成（正片段直出，not 条件走互补库所编码）；**按实例独立编码**（论文 §4.5/§B.2 单实例作用域：每实例各自编码，库所/迁移不跨实例共享，跨实例协同不入 `N(D)`；源域为域声明、实例划分属运行期，见 G1/D27） | CPN Tools 可加载验证；单实例作用域符合 §4.5/§B.2 | **已完成（2026-10-21，D28）**：新增 crate `pact-cpn`（`generate(&Domain, &CpnOptions) -> CpnModel`）+ CLI `pact cpn`；每类型库所 + Archive、`PACT_TOKEN` 五元颜色、生产前驱 test arc、读自环、消费归档、种子无输入弧；`GateExpr → NNF → DNF`（or 多迁移、not 互补库所 + `--bound`、not via 显式拒绝、恒假 guard=false、DNF 上限 1024）；实例命名空间 `i_<instance>__`；输出 CPN Tools XML format 6。验证：11 项 crate 测试 + CLI 离线 e2e（结构/错误路径/实例前缀），全量 **166 项**测试全绿；`examples/domains/*.pact` 生成物全部通过 CPN DTD 校验（xmllint）。**限制**：沙箱无 CPN Tools GUI，实际图形加载需人工验收；`--bound` 正确性由调用方承担（运行期并发上界不可从声明推导） |
| 重构（2026-09-09） | 论文 rev2 对齐：良构约束六项化（新增 LHS Uniqueness / Non-Empty LHS / PosNat 解析期与声明期强制）、A.5 重构为 precondition_wf 三元报告（D21）、DomainError 自有 String | rev2 专项测试（`{A+A}→B` / `{ }→B` / count 0 / 前导零拒绝、自环接受、EmptyPrecondLhs / ZeroCountLiteral / SeedProduceOverlap 用例）+ 三接口 e2e 断言更新全绿 | **已完成**：workspace 103 项测试全绿（净增 6 项）；fmt/clippy 清零 |
| 7 | pact-runtime：参与者驱动运行时（D24，设备生产化） | 可复用自驱循环：给定参与者名 + 门，实现 P1/P2/P3 策略并驱动至 Terminated；多 agent 编排可拉起 N 个真实 MCP/HTTP 客户端；外部副作用执行器消费 `Consumed` 并追加补偿事实 | **已完成（2026-10-19）**：新增 crate `pact-runtime`（lib + bin `pact-runtime`）——传输层抽象（`ParticipantClient` trait / `RestClient` 包装 `pact_http::client` / `McpClient` rmcp stdio 子进程）+ 驱动循环 `drive()`（P1 生产优先/P2 汇消费/P3 等待；订阅推送 + 轮询兜底；`pristine` 守卫 + 500ms 终止宽限，与 `unattended_e2e` 逐条同构）+ 编排 `Orchestrator`（每 agent `tokio::spawn`，REST/MCP 两具体 client 装箱统一、不引入 dyn）+ 执行器 `SideEffectExecutor`（archive 游标增量、失败以 Rule-S 追加补偿事实）+ TOML 配置驱动 bin。集成 1 项（单 `#[tokio::test]` 内 in-process daemon 上顺序跑 REST 编排 SE 生命周期 / ReAct 循环 / 副作用执行器）+ 单元 4 项；workspace **110 项**测试全绿（基线 105 + 本轮 5），fmt/clippy 清零。修复两处（见 design-change 第 17 轮）：①执行器与编排并发启动下域未就绪 → 执行器增域发现轮询；②驱动 P1 需 `read` 产出见证前驱 → 测试域 `worker` 补 `reads task` 门 |
| 8 | 引擎生产化三项（D25） | 事件日志压缩与 schema 版本化；health/ready + metrics + PID 文件 + 信号化优雅停机；域级 ACL + 可选审计读视图（默认关闭）。**验收项与 D27 实例化耦合**：(1) 的压缩部分与 (3) 全部须按双键重述，`:compact` 路由须随实例段重设计 | **子项 (2) 运维接口已完成（2026-10-20，见 §9.5）**：`/health` `/ready` `/metrics`（Prometheus 文本）+ 可选 `pid_file`（陈旧检测）+ SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界 10s 等待在途 → 终态快照）；workspace **131 项**测试全绿（基线 123 + 本轮 8）。**子项 (1) 日志压缩与 schema 版本化已完成（2026-10-20，见 §7.5）**：`WireEvent` 顶层 `version`（+ `Notification` 顶层 `version`）+ 旧日志/快照缺省按 v1 兼容、更高版本 fail-fast；前缀压缩（全量自足快照 → `logmeta` 水位 → 原子重写 → 裁剪索引）、快照承载 `declaration`/`dl_seq`/`consumed_requests`（幂等表随快照存活）、恢复 `snapshot_is_anchor` 分支与 `SnapshotRequiredForCompactedLog` 护栏；REST `POST /domains/{d}[/instances/{i}]/compact`、CLI `pact compact`、MCP `pact_compact`、`pact.toml` 可选 `compact_on_start`/`compact_keep_last`；workspace **145 项**测试全绿（基线 131 + 本轮 14）。**子项 (3) 域级 ACL 与审计读视图 已完成（2026-10-20，见 §9.6）**：`[auth.acl]` 实例级条目（`domain/instance` / `domain/*`）覆盖生命周期、四类动作、查询、compact 与读审计视图，`GET /domains` 按授权过滤；ACL 非空而令牌表为空 ⇒ 拒绝启动；可选 `read_audit = true` 写独立审计流（`<instance>.read-audit.jsonl`，append+fsync，fail-closed）并提供 `GET .../read-audit?limit=` 只读视图；workspace **155 项**测试全绿（基线 145 + 本轮 10） |
| 9 | 分发产物 `dist/`（D26） | Dockerfile、systemd unit、musl 静态二进制打包脚本，可构建可运行 | **已完成（2026-10-20）**：`dist/Dockerfile`（多阶段 `rust:1.98-alpine` 构建 musl 静态四 bin → `alpine:3.22` 运行；非 root uid 10001、`/var/lib/pact` 数据卷、`/health` HEALTHCHECK、入口 `pactd --config /etc/pact/pact.toml`）+ `Dockerfile.dockerignore`（构建上下文排除 `docs/pact*`、examples、target 等）；`dist/pactd.service`（`User=pact`、`StateDirectory`/`RuntimeDirectory`、SIGTERM 优雅停机、`ProtectSystem=strict` 与空 capability 集等加固）；`dist/build-static.sh`（`x86_64-unknown-linux-musl`、`--locked --bins`、strip + `readelf` 静态链接核验、tar + `SHA256SUMS` 复现打包）；配置示例 `dist/pact.toml.example`（systemd）与 `dist/pact.container.toml`（容器）。离线校验：脚本语法、两份 TOML 解析、systemd unit 语义校验；按 D26 不接入 CI 语义测试。**注**：当前开发沙箱无 musl C 工具链与容器运行时，静态打包与镜像构建仅做到脚本/配置层面校验；实际构建需在装有 `musl-gcc` 的主机或 Docker 中执行 |
| 10 | 域双键实例化（D27，路径 β） | 定义键/实例键落地：存储 `(domain, instance)` 双列 + 域子目录布局 + 引擎声明注册表（`declare` / `instantiate` 分离，实例创建时快照声明）+ 接口全链（REST / CLI / MCP / `pact-runtime`）+ 默认实例 `default` + 旧日志 fail-fast + 通用护栏（`data_dir` flock 独占锁、非空实例 load 默认拒绝）；阶段 8 的耦合项随之一并完成 | **已完成（2026-10-20，路径 β）**：`pact-store` `InstanceKey{domain,instance}` + 域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、声明原文 `declaration.pact`）+ 索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键，`open()` 自愈重建）；`pact-engine` 声明注册表（`declarations` / `instances` 两表，`declare` / `instantiate` 私有方法 + 公开命令 `LoadDomain` / `Instantiate`，`InstanceSession.is_occupied = last_seq > dl_seq`）——错误码 `InstanceNotFound` / `DomainNotDeclared` / `InstanceExists` / `DomainBusy`；接口全链：REST 默认族 + 实例镜像族（`/domains/{d}/instances/{i}/…`）+ `InstantiateReq` + `DomainListing`，CLI `--instance`/`--force`/`instantiate`/`instances`，MCP 16 工具（新增 `pact_domain_instantiate`），`pact-runtime` 实例透传（`ParticipantClient::instance` + `bound_ready`）；护栏 `flock`（`fs4`，`<data_dir>/pactd.lock`）+ `DomainBusy`；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**（形式语义不变）。workspace **123 项**测试全绿（基线 110 + β 专项），fmt/clippy 清零 |

## 12. 联网演进路径（NFR6）

v1 → v2 的变化被限制在接口层：TLS 终结、真实参与者凭据（签名密钥替代 bearer token）、MCP Streamable HTTP 远程接入。引擎、存储、核心语义零改动——因为 Counter 单权威不因联网改变（client/server，无共识）。turso 的 MVCC 并发写能力为 v2 多客户端并发查询提供余量。

**MCP 提供形态（2026-10-19 确认，D24）**：v1 维持 stdio-only（D20 不变）到 v2；服务端形态（daemon 内建 MCP Streamable HTTP）随 v2 落地。`pact-mcp` 是**传输适配器/客户端**而非权威，权威恒为 daemon；它随本仓库交付（独立 bin），由 agent 宿主在 MCP 配置中拉起（`command=pact-mcp, args=--unix …`）。**不把 MCP server 拆为独立项目**——它与线上格式/事件 schema 强耦合。

## 13. 生产运行层与生产化边界（D24–D26，2026-10-19 裁决）

生产化被拆为三层归属（详见 `docs/architecture.md` §6）：

- **L1 范式引擎（本仓库 `crates/`）**：领域无关的协调机制，即现有一切（core/parser/analysis/compose/store/engine + 三接口）。生产化增强（日志压缩、运维接口、域 ACL）仍落于此（阶段 8）。
- **L2 领域适配层（独立仓库 `~/projects/skills`）**：RE/SE 技能的 `.pact` 域与内容层（`docs/skill-adaptation.md` §1 已定案），不属本仓库。
- **L3 生产运行层（本仓库 crate `pact-runtime`，阶段 7，**已实现**）**：引擎保持被动（D5），agent 必须自驱。本层提供可复用的参与者驱动循环、多 agent 编排、外部副作用执行器，经 MCP（stdio）或 REST 接入 daemon。驱动逻辑已收于 `pact-runtime`（lib + bin）；`pact-mcp/tests/unattended_e2e.rs` 保留为经真实 MCP 子进程验证引擎自驱场景的回归测试，不再是唯一驱动实现。

**边界判据**（回答"本仓库 or 另立"）：(1) 是否范式通用（不绑定领域/技能）？否 → 另立；(2) 是否须与线上格式/事件 schema 版本锁定？是 → 本仓库（可独立 crate/bin）；(3) 是否只是部署/分发/编排细节？→ 发行仓库或本仓库 `dist/`；(4) 是否新增范式语义？→ 须论文母本先行（超出实现方范畴）。

**构建形态**：保持一个 workspace、多 bin（`pact` = daemon + CLI；`pactd` = 裸 daemon 入口；`pact-mcp` = stdio MCP server；`pact-runtime` = TOML 配置驱动的生产编排入口）。不建议合并——CLI 是离线工具，不应背负 daemon 运行时；`pact-mcp` 生命周期与 daemon 不同，必须可被宿主以子进程拉起。

## 14. 分发产物（D26，阶段 9，已实现）

`dist/` 与源码同仓库，保证版本与线上格式/事件 schema 一致；只承载**部署细节**，不含新增范式语义。全部产物不接入 CI 语义测试（D26，与 D22 同精神）。

| 产物 | 形态 | 要点 |
|------|------|------|
| `dist/Dockerfile` | 多阶段容器镜像 | 构建阶段 `rust:1.98-alpine` + `musl-dev`，`cargo build --release --locked --target x86_64-unknown-linux-musl --bins`；运行阶段 `alpine:3.22`，非 root（uid/gid 10001）、`WORKDIR /var/lib/pact`、`EXPOSE 8787`、`HEALTHCHECK` 探 `/health`、入口 `pactd --config /etc/pact/pact.toml` |
| `dist/Dockerfile.dockerignore` | 构建上下文过滤 | 排除 `.git`、`target`、`docs`（含论文母本）、`examples`、本地数据与标记文件 |
| `dist/pact.container.toml` | 容器默认配置 | `data_dir=/var/lib/pact`、`listen_unix=""`、`listen_tcp="0.0.0.0:8787"`（仅为端口映射；发布须绑回环） |
| `dist/pactd.service` | systemd unit | `pact` 用户、`StateDirectory`/`RuntimeDirectory`、`Restart=on-failure`、`TimeoutStopSec=20s` 容纳 10s 优雅停机；`ProtectSystem=strict`、`ProtectHome`、空 `CapabilityBoundingSet`、`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6` 等加固 |
| `dist/pact.toml.example` | systemd 配置示例 | `data_dir=/var/lib/pact`、`listen_unix=/run/pact/pact.sock`，压缩/读审计/ACL 均为注释项 |
| `dist/build-static.sh` | musl 静态打包脚本 | 目标缺省 `x86_64-unknown-linux-musl`；自动探测 `musl-gcc`（或 musl 系 `cc`）作 linker/C 编译器；四个 bin 逐一带 `readelf -d` 无 `NEEDED` 校验；strip；生成 `bin/` + unit + 配置示例 + `LICENSE` + `SHA256SUMS` 的 tar.gz 与 `.sha256` |

**工具链前置**：`rustup target add x86_64-unknown-linux-musl` 与 musl C 工具链（turso 的 C 依赖需编译；Fedora `musl-gcc`、Debian `musl-tools`、Alpine `musl-dev gcc`）。脚本对缺失前置**显式失败**，不回退到 glibc 构建。

**安全边界**：容器与 systemd 均以非 root 运行且数据目录 0700；容器默认只暴露 8787 且文档要求 `-p 127.0.0.1:8787:8787`；v1 无 TLS，跨主机访问须外部 TLS 终结并启用 `[auth.tokens]` / `[auth.acl]`（§9.4/§9.6）。

**非目标**：不推送镜像仓库、不提供 k8s/Compose 编排模板、不做自动更新或打包签名；这些属外部发行细节。

## 15. pact-cpn：CPN 编码生成（阶段 6，D28）

### 15.1 边界与入口

- `pact-cpn` 只依赖 `pact-core`（`Domain` / `GateExpr`），**不依赖** parser/store/engine/http；纯函数 `generate(&Domain, &CpnOptions) -> Result<CpnModel, CpnError>`，无 IO。
- CLI 离线命令：`pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；`FILE` 由 `pact-parser` 解析并校验，`--json` 输出 `CpnStats`。
- 一次生成只编码**一个 Counter 实例**；库所/迁移名统一前缀 `i_<instance>__`，页面名 `PACT_<instance>`。不同实例分别生成、互不共享（§4.5/§B.2 Scope，G1）。
- `pact-cpn` 不进入 daemon、MCP 或运行时路径；不改变事件 schema 与线上格式。

### 15.2 网结构映射

| PACT 元素 | CPN 结构 |
|---|---|
| 类型 `T` | 库所 `p_T`，颜色 `PACT_TOKEN` |
| Archive | 库所 `p_archive`，颜色 `PACT_ARCHIVE` |
| 种子 `(p,T)` | 迁移 `t_p_T_seed`，无输入弧，输出 `PACT_TOKEN`（空依赖列表） |
| 生产 `(p,T)` | 迁移 `t_p_T_prod`；对每个前驱 `A_i` 一条 test arc（变量 `x_i` 输入 + 同变量输出）；输出 `PACT_TOKEN`，依赖列表 `[#1 x_i]` |
| 读 `(p,T)` | 迁移 `t_p_T_read`：`x` 自环（输入 + 同变量输出） |
| 消费 `(p,T)` | 迁移 `t_p_T_consume`：输入 `x`，输出 Archive `(x,p,pact_now())` |

颜色：

```text
PACT_TOKEN   = product PACT_TOKENID * PACT_PARTICIPANT * PACT_THINGREF * PACT_PRECONDREFS * PACT_TIMESTAMP
PACT_ARCHIVE = product PACT_TOKEN * PACT_PARTICIPANT * PACT_TIMESTAMP
PACT_COMPL   = int          // 互补库所只承载计数不变量
```

`pact_next_id()` 以声明在 globbox 的 `ref` 计数器生成新 TokenId；`pact_now()` 为时间戳占位（v1 编码不引入时间语义）。图形位置由生成器按网格排布。

### 15.3 GateExpr 编码

1. `GateExpr` → NNF（`not` 下推，De Morgan）→ DNF。
2. 每个 DNF 合取项生成**一条**迁移（读/消费；条件为 `or` 时名称加 `_c1/_c2…`），避免用 guard 近似析取导致结构不等价。
3. 正原子：`exists T` 1 条 test arc；`count T >= n` n 条 test arc + 两两 `#1 yi <> #1 yj` guard；`via T` 1 条 test arc + `List.exists (fn z => z = #1 y1) (#4 x)` guard。
4. 目标类型自查：目标 token 自身计入目标类型的 `count`，故 `exists T`/`count T >= 1` 在 `T` 为目标时无需额外弧；`count T >= n` 只需 `n-1` 个额外见证；`via T` 在目标类型上仍需额外见证（目标不能在自身依赖列表中）。
5. 负原子：`count T <= k` 用互补库所。`p_T_not` 初值 `N`，并维持 `M(p_T) + M(p_T_not) = N`：种子/生产输出 `T` 时消耗 1 个互补 token，消费 `T` 时归还 1 个；测试弧以 `N-k` 个 token 输入并原数返回。`N` 由 `--bound T=N` 提供，且必须是该实例中 `T` 的真实并发上界。
6. `not via T` 无法用计数互补表达，返回 `NegatedVia`；缺上界返回 `MissingBound`；DNF 超过 1024 项或 `count T >= n` 的 `n > 1024` 返回 `ConditionTooComplex`（防展开膨胀）。
7. 条件恒假（DNF 无合取项，或目标类型 `count T <= 0`）保留迁移并置 `guard=false`，便于与声明条目逐一对账。

### 15.4 输出与验证

- 序列化为 CPN Tools XML（`generator format='6'`，附 CPN DTD DOCTYPE）；所有文本节点经 XML 转义。
- 测试：结构计数（库所/迁移/弧/互补/禁用的迁移）、test arc、互补初值与不变量、guard 形状、单实例命名空间、错误路径；CLI e2e 覆盖成功/缺上界/JSON 输出。
- 额外人工校验：`examples/domains/*.pact` 生成物以 `xmllint --dtdvalid` 对照 CPN DTD 全部通过；CPN Tools GUI 内的加载/状态空间实验为人工验收项（沙箱无 GUI）。
- 已知限制：TokenId 生成器依赖 CPN ML `ref` 副作用，状态空间工具可能将其视为非纯表达式；上界由用户保证；不编码跨实例协同。

## 16. WASM 本地分发形态（实验性，D31）

### 16.1 定位与权威边界

- `pact-wasm` 是 `pactd` 之外的**可选本地分发形态**，嵌入 Node/桌面宿主进程；**原生 daemon 仍是 Counter 的唯一权威**，两者不并存于同一实例（同一实例由宿主 lockfile 单写者约束）。
- NFR4 的单一静态二进制与 FR18 的 `dist/` 分发产物不变；WASM 不替代 `pactd`、不提供 MCP/HTTP 服务、不承担多用户与远端接入。
- 安全边界为**本地单用户**：WASM 内核不做身份认证、ACL 与读审计；这些能力只在原生 daemon（FR17）。宿主不得把 WASM 形态暴露为多用户服务。

### 16.2 组件与宿主职责

| 层 | 责任 | 已交付 |
|---|---|---|
| `pact-engine-kernel`（Rust，纯） | 实例会话语义：声明快照、Counter 状态、seq 分配、Rule-S/P/R/E 准备、Rule-T 终止判定、Rule-E 幂等表、恢复锚点、查询视图；无 IO/tokio | D32 |
| `pact-wasm`（Rust，wasm32） | 声明注册表、`(domain, instance)` 会话（嵌入 kernel `Session`）、两阶段提交、JSONL/快照导出与 `restore`、ABI 包装 | S2 |
| JS 宿主适配层（`crates/pact-wasm/js/`） | JSONL 追加 + fsync、末尾半行截断、快照 tmp+rename+目录 fsync、`.lock` 单写者、日志优先恢复、`prepare → append → commit` 编排 | S2（Node） |
| 原生 `pact-store` / `pact-engine` | 真相源布局、事件 schema、快照结构、索引、压缩、ACL/审计；`pact-engine` 会话同样嵌入 kernel `Session` | 不变 |

- 磁盘布局 `<root>/<domain>/<instance>.{events.jsonl,snapshot.json,lock}` 与原生 `<data_dir>/<domain>/<instance>.*` 同构，原生 daemon 与 WASM 宿主已验证可**交替打开同一日志**。
- 宿主 lockfile 是**进程级近似**单写者，不等价于原生 `flock`；不承诺多进程/多 tab 的内核级互斥。

### 16.3 ABI 与写协议

- ABI 为 `pact-wasm/0.x`，全部方法以 JSON 字符串收发；`abi_version` 为版本探测入口。破坏性变更须提升版本号并记入 `docs/design-change.md`。
- 唯一写路径为 `prepare_* → 宿主持久化（append + fsync）→ commit`；持久化失败调用 `abort` 回收未占用 seq，保证日志 seq 连续（原生 `Store` 的 `seq == last+1` 校验可直接接受该日志）。
- `commit` 返回的 Rule-T `Terminated` 事件由宿主按原生顺序追加；该追加为派生动作，失败不回滚已提交状态（与 D23 一致）。
- `token_id` 与 `ts` 由宿主注入（ULID/ISO-8601），内核不读系统时钟、不依赖 `getrandom`，保证与原生金标逐行对账可复现。

### 16.4 稳定化前置与不纳入项

- 去除“实验性”标记前须完成：Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）→ 金标扩面（多实例、组合域、条件分支、压缩后恢复）→ 抽取 `pact-engine-kernel` 消除双实现 → 宿主边界安全审视。前三项**已完成（2026-10-21）**；仅剩推送/发布前的宿主边界安全审视。
- 不纳入：浏览器 OPFS/SharedWorker（Q6）、多用户 ACL/读审计、turso 索引与压缩、DSH `govern`（Q5）、NFR6 v2 远端模式。触发条件见 `docs/wasm-local-spike-feasibility.md` §11.2。
