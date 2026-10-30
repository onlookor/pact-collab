# PACT 实现设计

对应需求：`docs/requirements.md`。范式规范：`paper/pactv1_rev3.md`（章节引用均指该论文最新版）。本文件只保留当前设计，变更历史记入 `docs/design-change.md`。

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
| D12 | CPN 生成器 | 延后为可选阶段 6（**实现须遵论文 §4.5/§B.2 单实例作用域**：每个 Counter 实例独立编码、库所与迁移不跨实例共享、跨实例协同不入 `N(D)`；见 D27 与「CPN 单实例作用域」） | 不阻塞核心价值交付 |
| D13 | SQLite 索引引擎 | 采用 turso，经 pact-store 内部 `IndexStore` trait 隔离 | turso 现状（2026-08）：BETA，官方自述未达 SQLite 级可靠性，但已在多家生产环境运行；本架构中 SQLite 仅是**可随时重建的派生索引**（真相源为 JSONL），beta 风险被完全对冲——最坏情况是删库重放重建。收益：纯 Rust 工具链（无 C 编译依赖，musl 静态编译顺滑）、async-first API 与 tokio 引擎契合、MVCC/`BEGIN CONCURRENT` 为联网版并发留余量。退路：若 turso 出现阻塞性问题，实现 `IndexStore` 的 rusqlite 后备并切换，仅动 pact-store 一个 crate |
| D14 | 语义收口 | 域良构校验、Rule-W 可满足性判定、Rule-T 终止判定均为 pact-core 纯函数；引擎层只编排不判定 | 三者均为范式语义（§1.4.4、Rule-W、Rule-T），若留在引擎层会造成语义分裂；同时供 analysis/compose/测试复用 |
| D15 | ~~Rule-T 语义~~（**已回滚**，2026-08-26） | ~~termination_holds 排除空前置组合（源类型生产）~~ | 论文 rev1（已归档，不入仓库）以源生产约束原生解决该问题（Rule-P 前提 Precond(T) ≠ ∅ + 声明期拒绝）：含源类型 produce gate 的域直接 ill-formed，不存在"永不终止"的合法域。临时豁免失去存在意义，回滚并由 D17 取代 |
| D16 | parser 宽容度 | 忠于 §3.1 文法骨架（四段固定顺序、段落完整）；宽容：段内尾分号、preconditions/gates/seed_authority 空段（types 至少一项）；扩展："->" 等价 "→"、`//` 行注释、GateExpr 优先级 not > and > or（文法未规定结合性） | 严格拒绝合法语义的空段比接受更糟；扩展均为可用性增益且不改变可表达语义 |
| D17 | 源类型生产约束（论文原生语义，rev2 §1.4.4/§3.1 沿用） | 三层强制：(1) Rule-P 前提 `Precond(T) ≠ ∅`——源类型（无前置规则）不可生产，运行时返回 `RuleError::SourceTypeProductionDenied`；(2) 声明期良构校验 `DomainError::SourceTypeProduceGate`（§3.1 约束 6）；(3) `is_source_type` 查询函数 + A.5 静态分析独立复核。Rule-T 按论文 Rule-P-enabled 三条件定义实现（源类型天然排除）；enabled_productions 不返回源类型 | 论文 §1.4/§1.4.3/§3.1 的规范语义：源类型唯一入口是 SeedAuthority（入口唯一性定理），消除 D15 指出的"永不终止"异常于声明期；良构域上 termination_holds 与"Counter 为空"等价（终止论证）。影响：LIFECYCLE 黄金域更新（analyst 角色改为 seed authority、operator via 子句移除——论文 Part II §1.3） |
| D18 | compose C2 实践语义 | 共享类型前置一致性判定：**双方都声明规则且集合不同 → 冲突**；单侧声明合法（缺席方对 T 的前置无约束，合并取并） | 论文 §1.8 示例的规范语义：D₂ 共享 design 而不重复声明其前置（顺序组合正依赖此机制），若按 C2 字面（一侧 ∅ 一侧 {…} 即冲突）论文自身示例无法组合。C4 安全性不受影响：`produces T` 只可能来自视 T 为非源的良构片段（必有规则），规则并集只增不减 |
| D19 | 阶段 4 范围与接口接入方式 | pact-http **提前至阶段 4** 与 pact-cli、pact-mcp 同批交付；MCP 与 CLI 均不内嵌引擎，而是经 `pact_http::client`（REST over unix socket）接入同一 daemon；原规划的独立 `pact-server` bin 取消，daemon 由 `pact daemon start`（pact-cli 子命令）与 `pactd`（pact-http 裸 daemon bin）提供 | 用户决策。三接口共享单一 daemon 即单一 Counter 权威（D6），REST 成为唯一状态变更通道——接口等价性由构造保证（MCP/CLI 只是 REST 的形态转换），e2e 只需各走一遍 REST 即覆盖契约一致性；联网版（v2）只换传输与认证，接口层结构零改动 |
| D20 | CLI 参数风格与 MCP 传输范围 | CLI 域操作命令一律 `--domain` / `--participant` / `--token` / `--type` flag（不采用设计稿中 `pact enabled <participant>` 式位置参数简写）；连接参数 `--unix` / `--tcp` / `--bearer` 与输出开关 `--json` 为全局 flag；MCP v1 仅 stdio，Streamable HTTP 移交联网演进（v2） | flag 风格消除位置参数歧义、便于脚本化与 `--json` 组合；MCP 经 REST 接入（D19）后 Streamable HTTP 与 REST 同端口价值消失，随 v2 TLS/远程接入一并落地。**v2 已落地（2026-10-22，D35）**：stdio 保留，daemon 另挂 `/mcp` |
| D21 | rev2 良构约束强制风格与 A.5 重构（用户选定推荐项） | (1) 空 LHS `{ } → T` 解析期语法拒绝（错误指名 Non-Empty LHS 约束），`validate_domain` 另设 `EmptyPrecondLhs` 声明期防御（覆盖程序化构造/组合出的 Domain）；(2) LHS 重复 `{A + A} → B` 在 parser `build_domain` 保留原始 `Vec<TypeName>` 序列时检测（`DuplicatePrecondLhs`）——`&Domain` 的 BTreeSet 会静默去重，此为唯一强制点；RHS 重复由 BTreeMap 键唯一构造性保证；(3) count 字面量 PosNat（`pos_nat()` 拒绝 0 与前导零，`ZeroCountLiteral` 二次防御）；(4) A.5 = `precondition_wf` 三元报告 `{well_formed, violations: Vec<DomainError>, warnings: Vec<WfWarning>}`，违规判定委托 `core::validate_domain`（单一判定源），Seed/Produce 重叠（§1.4.3 Advisory）为 `SeedProduceOverlap` 咨询性警告——**类型级判定**：produced 取全体参与者 produces gate 的并集，非源类型同时被 seed 与被任一人 produce 即告警，`participant` 字段为 seed 持有者；比论文 §1.4.3 的“同一参与者”表述更宽，取假阳性方向不漏报——论文维持参与者级原文，实现登记为参考算法的**保守超集**（2026-10-15 定案；论文曾误改三处、同日经用户裁决回退，规范是否修订由作者另行裁决）；聚合报告字段前置（附录 D：A.5 先于其余分析）；(5) `DomainError::UndeclaredType.referenced_by` 由 `&'static str` 改 `String`，使错误可内嵌进 serde 报告 | 解析期拒绝保持文法忠实；表示层（BTreeSet）无法承载的约束在序列仍在手时检测；A.5 委托避免判定逻辑分裂；前驱互异仅按 TokenId（Rule-P Note），类型级去重由 LHS Uniqueness 良构性免除 |
| D22 | 通用示例与用户手册作为交付资产 | 新增 `examples/`（领域无关通用域 `domains/`：初版四份，2026-10-18 按 D23 增设第五份 `external_materialization` 物化确认示例 + 两份组合片段 `fragments/` + 离线/在线两条上手脚本各带录制轨迹 + `README.md` 场景导航）与 `docs/manual.md`（权威用户手册，12 章）；**不接入 CI**（不新增遍历 `examples/**/*.pact` 的 cargo test）、**不改任何 crate**；脚本运行产物 `examples/online/run-data`、`examples/offline/out` 入 `.gitignore` | 用户决策（FR14）。示例是教学资产而非回归基线，接入 CI 会把示例漂移变成构建失败源，反而抑制示例演进；手册与示例分工为"参考权威 / 可运行路径"，交叉引用不重复维护同一事实。取证过程中发现的三处实现与文档不一致（CLI `reachability` 行把 `seed_types` 打印为"源类型"、MCP `pact_analyze` description 写"五项静态分析"实为六项、compose 语义在 CLI 只打印而 MCP/REST 会装载）**不改代码**，如实登记于 `docs/manual.md` §11 已知偏差与限制 |
| D23 | 消耗提交契约与 Rule-E 幂等（2026-10-18，用户裁决） | (1) **提交点 = JSONL fsync**：`store.append` 追加日志即视为已提交；其后步骤（索引投影、定期快照、Terminated 事件、通知）为**尽力而为的派生动作——失败不回滚、不失败本次调用**，索引滞后由启动 seq 一致性校验触发 `rebuild` 自愈。(2) **Consumed 事件自足**：携带 token 全量快照（`exhausted_at` = `ts`、`exhausted_by` = `by`）与可选 `request_id`，重放与审计不再反查同一域实例其他行。(3) **Rule-E 可选幂等**：同一域实例内重复 `request_id` 返回首次归档结果（不重复消费；跨重启由日志重建幂等表生效），同一 `request_id` 用于另一 token → `IdempotencyConflict`。(4) **原子域含 gate 读集**：`rule_e` 对同一内存 `CounterState` 求 GateHolds 并随即 apply，单写者 actor 消除 TOCTOU 与并发双消费。(5) **边界**：引擎不执行外部副作用，`consume` 成功与外部动作成功是**两个独立承诺**；"产物未就位"不应表现为消费回滚，而应前移为 gate 准入条件（`exists` 型见证 token，见 §8.5） | 此前 Rule-E 仅有单步 SOS 语义、未声明实现级原子性；本决策把它显式化为可验证契约。原实现 `Consumed` 只记 `token_id`、重放反查 Counter，使单行日志不自足、与 NFR2（日志即完整审计）不符；原写路径"索引失败即返回 Err"会造成日志/内存瞬态分歧直至重启。拒绝"库事务 / 2PC / 分布式共识"方案：单写者 actor + 事件日志已满足线性化与崩溃一致性，引入它们只增复杂度 |
| D24 | 生产运行层归属（2026-10-19，用户裁决） | 本仓库新增 crate `pact-runtime`（**已实现，阶段 7 完成**）：参与者驱动循环（订阅/轮询 → `my_gates` / `enabled` → 决策 → 行动）+ 多 agent 编排 + 外部副作用执行器（outbox/saga 消费端）。**引擎保持被动（D5）不变；MCP 传输范围由 D20 的 stdio-only 经 D35 扩展为 stdio + daemon 内建 `/mcp`（v2），stdio 仍保留**；依赖方向 `runtime → http（类型化 REST client）/ engine（线上视图）/ core（标识符·token 类型）`，`pact-mcp` 仅作可执行文件由编排 spawn、不作 lib 依赖（详见 §1） | 驱动逻辑与 `Consumed` 事件 schema、`pact_*` 工具面强耦合，须与引擎版本锁定；抽为可复用运行时，避免每个消费方重复实现 P1/P2/P3 自驱策略；engine/store/core 零改动 |
| D25 | 引擎生产化三项（2026-10-19，用户裁决） | (1) 事件日志压缩与 schema 版本化（pact-store）；(2) 运维接口：health/ready、metrics、PID 文件、信号化优雅停机（pact-http + engine）；(3) 域级授权与审计读视图（pact-http 接口层），读审计**可选开启、默认关闭**。**(2) 已实现（2026-10-20，见 §9.9）**：`GET /health`（存活，不依赖引擎）/ `GET /ready`（就绪，探针 = 一次 `list_domains` 往返）/ `GET /metrics`（Prometheus 文本格式：`pact_domains_declared` / `pact_instances_loaded` / `pact_counter_size` / `pact_archive_size` / `pact_terminated` / `pact_uptime_seconds`）；运维端点**不校验令牌**（仅本地/内网）；可选 `pid_file`（缺省不写；启动写入、退出删除；陈旧文件且 PID 存活 → 拒绝启动）；优雅停机 SIGINT/SIGTERM → 停止接受新连接 → 有界（10s）等待在途 → 终态快照 → 清理。**(1) 已实现（2026-10-20，见 §7.5）**：`WireEvent` 顶层 `version` + 日志前缀压缩（快照自足锚 + `logmeta` 水位 + 原子重写）、REST/CLI/MCP 压缩入口、可选启动自动压缩。**(3) 已实现（2026-10-20，见 §9.10）** | 生产部署的可运维性与多租户边界（NFR7）；三者均为存储/接口层增强，不触碰核心语义与 SOS 规则。(3) 修订 D9 默认值表述——D9「审计流扩展位保留」在此落地为配置开关，默认仍不持久化 Rule-R |
| D26 | 分发产物归属（2026-10-19，用户裁决） | 本仓库 `dist/`：Dockerfile、systemd unit、musl 静态二进制打包脚本；不接入 CI 语义测试（同 D22） | 与源码同仓库保证版本一致；registry 推送、k8s 编排模板等外部细节不属本仓库 |
| D27 | 域双键实例化与形式层裁定（2026-10-20，领域专家裁定 + 用户裁决） | **立项**：把「域」拆为**定义键 domain + 实例键 instance**，同一域声明可并发承载多个 Counter 实例；实现走**路径 β**（`pact-store`/索引显式 `(domain, instance)` 双列 + 引擎**声明注册表**，`declare` 与 `instantiate` 分离）。**论文母本 rev2 → rev3**（用户破例授权），5 处改动：§1.3 增列「**Counter instantiation and partitioning**」条款、§1.4.1 TokenId 作用域措辞、Rule-T 增补 scope note、§4.5 与 §B.2 增补 CPN 作用域、§1.8 及 §4.1–4.3 术语 `domain instance` → `domain declaration`——**零文法变化、零语义变化**（`diff` 已核，9 处增补/替换；G3 就地修订 §1.4.1 Rationale 追加 1 处，**累计 10 处**，见 `docs/design-change.md` 第 20 轮）。**术语登记**：定义键 = 论文 `domain declaration`；实例键 = 论文 `Counter instance`（§1.3 描述性短语，非正式原语）；**中文文档/注释用「域实例」指该实例键**（即论文 `Counter instance`，与论文已废弃的英文 `domain instance` 无关，亦不替代论文术语）。**细节定案（2026-10-20，用户裁决）**：存储布局 = **域子目录** `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `<domain>/<instance>.snapshot.json`、审计流 `<domain>/<instance>.read-audit.jsonl` 同目录）；默认实例名 = **`default`**（不指定实例段 ⇒ 落到 `default` 实例）；声明绑定 = **实例创建时快照声明**（`DomainLoaded` 携声明原文，后续 `declare` 覆盖**不影响已运行实例**，重放自封闭、确定性优先，与 NFR2「日志即完整审计」一致）。**派生细节定案（2026-10-20，用户裁决，阶段 10 实施前置）**：旧日志**不兼容**——双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`，顶层遗留单层文件启动恢复时 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移；实例键以**路径段**承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` 等价 `default` 实例）；`DomainLoaded` **不新增 `instance` 字段**（实例由文件路径承载）；声明原文另存 `<data_dir>/<domain>/declaration.pact`（`declare` 时写入、`recover` 回填注册表）；**护栏**：`data_dir` 独占 flock（`<data_dir>/pactd.lock`）+ 非空实例（`last_seq > DomainLoaded.seq`）再加载默认拒绝（`DomainBusy`），须 `force`。**已实现（2026-10-20，阶段 10 路径 β，见 §11 阶段 10）** | 触发：两个并行项目共用同一域声明时，既有替换语义会销毁对方 Counter 与快照。专家裁定 Q1(a) 概念承认（以 §1.3 实现自由形式落地）、Q2(a) 术语消歧、Q3(a+c) 限定重述、Q4 保留「globally unique」但作用域收窄至该 Counter（实现以 ULID 加强为跨实例唯一）、Q5 确认**无任何论文级不变量依赖「一个域名至多一本 Counter」**、Q6 确认 CPN 编码需加单实例作用域、Q7 不新增正式术语。据 §1.3「Physical implementation of the Counter」，实例划分属**实现自由**，不触 §13 边界判据 (4)。评估与执行序、咨询简报与答复为**过程产物**，阶段 10 交付后已按用户指示移除（其结论已并入本条与 §11） |
| D28 | CPN 编码生成器（2026-10-21，用户选定阶段 6） | 新增 `pact-cpn` + CLI `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；输出 CPN Tools XML（`format='6'`）；单实例作用域以实例名命名空间化；颜色 `PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`，Archive 另加 Participant × Timestamp；生产前驱 test arc、读自环、消费归档、种子无输入弧；GateExpr → NNF → DNF，`or` 展开多迁移、`not` 下推为 `count T <= k` 并以互补库所编码（须显式上界）；`not via` 显式拒绝；条件恒假保留迁移 `guard=false`；DNF 上限 1024、count 展开上限 1024；输出经 CPN DTD 校验 + XML 良构/结构测试，CPN Tools GUI 加载人工验收 | 论文 §4.5/§B 只对正片段直出，负条件需已知上界；上界由运行期并发决定、不可从声明推导，故必须由调用方显式提供并承担正确性。`or` 用 DNF 多迁移而非 guard 近似，保持结构等价；`not via` 是实例级依赖关系而非计数，无法用互补库所表达，宁失败不生成错模型。生成器零 IO、零运行时依赖，不进入引擎/存储路径 |
| D29 | WASM 复用与纯类型层（2026-10-21，用户批准 W-P1） | 新增 `pact-store-model`（`Event`/`WireEvent`/`Snapshot`/`InstanceKey`/`ModelError`，无 IO）与 `pact-views`（查询/分析视图，无 tokio）；`pact-store`/`pact-engine` 通过 re-export 保持既有 API；磁盘格式与线上格式零变化 | 为 WASM 本地 spike 提供可编译到 `wasm32-unknown-unknown` 的共用模型层，事件 schema 与视图定义只保留一份，避免原生与 WASM 漂移。拆分边界：model 仅 `core + serde + serde_json`；views 为 `core + analysis + serde` |
| D30 | WASM 门面 crate（2026-10-21，用户批准 W-P2） | 新增 `pact-wasm`（`cdylib + rlib`）：可原生测试的 `PactHost` + 仅 wasm32 启用的 `wasm-bindgen` 包装；v0 导出 `new`/`load_domain`/`validate`/`analyze`/`compose`/`list_domains`，JSON 字符串 ABI；新增 `wasm-release` 体积优先 profile；暂不接 CI、分发与持久化 | 以最小门面验证 wasm-bindgen 导出、体积与静态分析对账；宿主核心与 wasm 包装分离，使原生测试无需 wasm 运行时。W-S0 不承担事件写入与存储适配（W-P3/W-P4），浏览器绑定与 OPFS 仍按 W-Q6 暂不做。**W-P3/W-S1 增补（2026-10-21）**：`PactHost` 增 `(domain, instance)` 会话表（每实例独立声明快照、事件序列、`last_seq`/`next_seq`、Rule-E 幂等表、`terminated`）与 `prepare_seed/produce/consume → commit/abort` 两阶段提交（prepare 分配 seq、abort 回收以保持日志连续；commit 应用状态并按原生顺序产出 Rule-T `Terminated`）；新增 `read`/`counter`/`archive`/`gates`/`enabled`/`status`、`export_events`（JSONL）、`export_snapshot`、`restore`（复刻 `recover_instance`）；ABI 升至 `pact-wasm/0.2`，错误码与原生接口层同名。固定 ID/ts 金标语料与原生日志逐行相等，JSONL 双向互通。**W-P4/W-S2 增补（2026-10-21）**：新增 Node 宿主适配层 `crates/pact-wasm/js/`（`FileStore` + `PactHost` + demo + `node:test`）：日志追加 `fsync`、末尾半行按字节截断修复、快照 tmp+rename+目录 `fsync`、`.lock` O_EXCL 单写者（含陈旧 pid 回收）；写路径 `prepare → append+fsync → commit`（失败 abort），打开优先日志/快照恢复；原生与 WASM 可交替打开同一日志 |
| D31 | WASM 本地分发形态条件立项（2026-10-21，W-P6 裁决 A） | 把 WASM 本地运行登记为**实验性可选分发形态**（Node/桌面宿主），权威仍是原生引擎；`pactd`、NFR4 单静态二进制与 FR18 生产分发承诺不变。ABI 固定 `pact-wasm/0.x` + JSON 字符串，`abi_version` 为探测入口，破坏性变更须升版并记入本变更文档。稳定化前置：W-Q7 CI → 金标扩面（多实例/组合域/条件分支/压缩恢复）→ 抽取 `pact-engine-kernel` → 宿主边界安全审视。不纳入：浏览器 OPFS/SharedWorker、多用户 ACL/审计、DSH `govern`、NFR6 v2 | W-P0–W-P4 证据表明语义与持久化行为可与原生对账（事件逐行相等、日志双向互通、崩溃恢复与单写者测试通过）；但双实现重叠约 300 行、金标覆盖窄，若长期保留必须用 CI 与 kernel 抽取防漂移。选项 B 会让 wasm target/bindgen 随工具链腐烂，选项 C 无需求证据且引入浏览器弱持久化与 v2 安全面，故取条件立项。详见 `docs/wasm-local-spike-feasibility.md` §11 |
| D32 | 抽取 `pact-engine-kernel`（2026-10-21，用户选定） | 新增纯内核 crate `pact-engine-kernel`（仅依赖 `pact-core`/`pact-parser`/`pact-analysis`/`pact-store-model`/`pact-views`，无 IO、无 tokio）：`Session` 统一持有 `declaration`/`domain`/`state`/`last_seq`/`next_seq`/`dl_seq`/`terminated`/`consumed_requests`，并提供 `fresh`、`recover`（锚点/快照/幂等表/终止态）、`prepare_seed`/`prepare_produce`/`prepare_consume`/`read`、`mark_prepared`/`abort_prepared`、`commit`/`record_terminated`、视图与 `snapshot`/`summary`，以及 `analysis_report`；`KernelError` 由调用方映射为各自错误类型。原生 `InstanceSession` 与 WASM `InstanceSession` 改为通过 `Deref` 嵌入同一 `Session`，各自仅保留自有字段（原生：`events_since_snapshot`/`notify`；WASM：`events`）。行为契约不变：DomainLoaded seq 从 1 起、失败命令仍推进时钟、Rule-T 终止 `seq = last_seq + 1`、压缩水位 `base_seq`、幂等表跨快照恢复 | W-P3 已量化双实现在 seq 分配、幂等表、Rule-T 终止、恢复锚点四处重复，长期保留会漂移（W-R1）。抽取把“语义会话”与“IO/actor/宿主编排”分离：内核保持纯函数式状态机，原生与 WASM 仅在错误映射、持久化时机与通知/审计等外围不同。`ConsumePreparation::Prepared` 以 `Box` 承载完整事件以控制枚举尺寸 |
| D33 | 去除 WASM “实验性”标记（2026-10-21，用户裁决） | `pact-wasm` 由“实验性可选分发形态”转为**受支持的本地分发形态**：稳定化前置 (1) W-Q7 CI、(2) 金标扩面、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视均已完成。定位与边界不变：权威仍是原生 daemon；仅 Node/桌面本地单用户嵌入；不提供 MCP/HTTP、ACL/审计、浏览器 OPFS/SharedWorker、DSH `govern` 与 NFR6 v2；不进入 `dist/` 产物集，不替代 `pactd`。ABI 仍为 `pact-wasm/0.x` + JSON 字符串，破坏性变更照旧升版记录 | 四项稳定化验收全部完成且 CI 持续约束（wasm32 构建/clippy、Node 冒烟、体积/ABI 断言、7 项金标对账）；继续保留“实验性”会与实际维护承诺不符。去除标记不扩大能力范围，只确认既有 Node/桌面形态可被依赖 |
| D34 | TLS 与 Ed25519 请求签名身份（NFR6 v2，2026-10-22，用户裁决） | (1) **TLS**：`[tls] certificate/private_key[/client_ca]` 存在即启用，仅对 `listen_tcp` 生效；rustls 终止于 pactd 进程，证书链 PEM + 私钥 PEM（PKCS#8/PKCS#1/SEC1），支持 TLS 1.2/1.3；`client_ca` 存在时以 `WebPkiClientVerifier` 要求客户端证书（mTLS）。启动日志标注 `tls`/`mtls`；握手失败仅记录并继续接受。内置客户端 `Client::https(addr, ca)` 与 `--https/--tls-ca` 仅做服务端认证，不提供客户端证书（mTLS 面向外部客户端）。(2) **签名身份**：`[auth.keys] 参与者 = base64(32 字节 Ed25519 公钥)` 非空即启用；客户端从 `--sign-key`（base64 32 字节 Ed25519 私钥种子）构造 `SigningIdentity`，请求携带 `x-pact-participant/x-pact-timestamp/x-pact-nonce/x-pact-signature`；规范化串 `PACT-SIGNATURE-V1`（方法、路径+查询、SHA-256(body) 小写 hex、时间戳、nonce、参与者，`\n` 分隔无尾换行）；服务端顺序 = 四头齐全 → 时间窗 ±300s → 公钥查表与 Ed25519 验签 → nonce 去重（TTL 300s/上限 8192；验签失败不消耗缓存）→ 写入可信头；body 上限 8 MiB（413），重放 429；先剥离客户端伪造的 `x-pact-verified-principal`。(3) **绑定**：签名与 Bearer 可同时配置；启用签名后 REST 非豁免请求必须签名（Bearer 不能替代，两者同时出现时以签名身份为准）；解析身份必须等于请求 `participant`，否则 401；ACL 照常（`[auth.acl]` 非空时 tokens 或 keys 至少其一）。签名仅护 REST，`/health` `/ready` `/metrics` `/mcp` 与 RFC 9728 元数据豁免；`/validate` 仍需签名。(4) **信任模式**：REST 无令牌且无密钥、MCP 另需无 OIDC 时保持信任声明；签名模式仅在 keys 非空时启用。 | TLS 免除外部反向代理与明文链路；签名把「参与者」从自报字符串升级为可验证身份，且复用现有 Bearer 身份位与 ACL，不触碰引擎。nonce 缓存与时间窗防重放；8 MiB 上限防验签前内存放大；可信头由中间件唯一写入，防止外部伪造。保留 Bearer 兼容既有部署与 `pact-mcp` 远端子进程。内置客户端不做 mTLS 是显式取舍：证书签发/托管不在本仓库（非目标），外部客户端可自行出示证书 |
| D35 | MCP Streamable HTTP（NFR6 v2，2026-10-22，用户裁决） | 工具层从 `pact-mcp` 下沉到 `pact-http::mcp`（`PactMcp`，17 个工具与输入结构原样迁移），daemon 与 stdio 二进制共用同一实现。daemon `[mcp].enabled = true` 时 `router_with_mcp` 以 `nest_service("/mcp", …)` 挂载 `StreamableHttpService<PactMcp, LocalSessionManager>`（rmcp 配置 builder，`legacy_session_mode(true)`、`allowed_hosts = 回环 + public_url authority`）。鉴权中间件独立于签名：OIDC（FR24）或静态 Bearer；令牌表为空且无 OIDC 时为信任模式（仅单机/回环，启动警告）；`[auth.keys]` 非空而 tokens 为空且无 OIDC ⇒ 拒绝启动。工具内以整个 `Client` 克隆（本地模式带 `x-pact-internal-auth`）调用进程内 tower Router，不发起 TCP 自连接；外部 JWT 不转发给 REST，参与者经 `x-pact-verified-principal` 按请求注入。`pact-mcp` 仅是 stdio 薄封装（`--unix/--tcp/--https/--tls-ca/--sign-key/--sign-as/--bearer|--bearer-file`），仍经 REST 接 daemon。 | MCP 远程接入与 REST 同端口，无需新增进程/端口；工具实现只保留一份，stdio 与 HTTP 行为一致。本地 tower 路由避免自连接与鉴权递归，内部能力令牌使本地调用免签名又不暴露给外部。信任模式只用于单机/回环，打印警告避免误部署。禁止 JWT 转发，防止把 MCP 身份扩展到 REST |
| D36 | MCP OIDC 资源服务器（NFR6 v2，2026-10-22，用户裁决） | daemon 只验证 IdP 签发的 JWT，不内置授权服务器。`[mcp.oidc]`：`issuer/audience/principal_claim/principal_map/jwks_uri/discovery/allowed_algs/ca/allow_static_tokens/allow_insecure_http`；启用时必须 `[mcp].enabled` 且 `public_url` 为绝对 URL。发现默认 `issuer /.well-known/openid-configuration` → `jwks_uri`；JWKS 缓存 TTL 300s、最多 64 键，未知 `kid` 限频 30s 重取一次。校验：alg 白名单 RS256/ES256/EdDSA 且与 JWK 类型一致，拒绝 `none`/HS*/非空 `crit`；`iss` 精确、`aud` 包含、`exp` 必需、`nbf` 校验、60s leeway；`sub` 必需，`principal_claim` 字符串（或唯一字符串元素数组），映射后 `[A-Za-z0-9_-]{1,128}`。OIDC 仅 `/mcp`；REST 仍为静态 Bearer/签名；令牌不缓存、不转发。`GET /.well-known/oauth-protected-resource` 返回 `{resource, authorization_servers, bearer_methods_supported}`；401 带 `WWW-Authenticate`，IdP 不可达 503。 | 复用企业 IdP 的身份与授权码流程，daemon 不承担签发/登录页/令牌端点（非目标）。仅 `/mcp` 收窄攻击面并保持 REST 既有身份路径；JWKS 有界缓存 + 限频重取兼顾密钥轮换与 IdP 压力；令牌不落盘、不转发，避免把 OIDC 权限扩散到 REST 与日志 |
| D37 | DSH Agent Loop 集成（阶段 11，2026-10-22，用户裁决） | 新增 JS 包 `dsh-plugin-pact`（独立仓库 `onlookor/pact-dsh`，ESM、零运行时依赖；2026-10-23 自本仓库 `plugins/dsh/` 迁出）：(1) **选择性工厂接管**——`cordis.patch.yml` 禁用原始 `agent-loop` 行并插入 `pact-agent-loop`（同一 `agentLoop` 服务，`dsh-plugin-pact/loop`）；DSH patch 只按 `id` 覆盖/禁用、`name` 仅作校验，不能改名；`PactAgentLoop extends AgentLoop` 只覆写 `createAgent`/`resume`，`agentPreset` 以 `pact-` 开头才在 agent scope 安装钩子，其余原样 `super`；(2) **两档**——`observe` 镜像 `session/event`（有界缓冲 + 可选 JSONL），`govern` 在 `agent/pre-step` / `tools/pre-execute` 查询门，拒绝返回 `{kind:'reject'}` / `{kind:'deny'}`，`tools/post-execute` 与 `agent/turn-stopping` 只审计（终止 steer 可选）；(3) **后端注入**——`PactBackend` 契约 + `FakePactBackend`（测试）+ 惰性 `WasmPactBackend`（`openPactHost`），不改 Rust、不拉守护进程；(4) **审计最小化**——只落 `type/seq/工具名/call id`，实例键由 session id 消毒为 `[A-Za-z0-9_-]{1,128}`；(5) **失败策略**——`govern` fail-closed，`observe` fail-open 并留痕；配置由 `PACT_DSH_CONFIG`（文件或内联 JSON）注入，不改 `AgentLoop.Config`。`@deepseek-ai/dsh-agent-loop` 声明为 peer，由 DSH 运行时解析。 | 共享讨论结论：PACT 可接管控制流但不能接管模型调用；全量重写 `ReactLoopAgent`（约 2.5k 行、DSH developer preview API）收益低、漂移风险高，故复用官方状态机、只替换 factory 注册者并按 preset 路由。插件与 Rust workspace 隔离，`pnpm`/wasm 产物缺失不影响本地单测；已在 `@deepseek-ai/dsh@0.2.1-rc.2` 完成真实装配（`ctx.agentLoop` 为 `PactAgentLoop`）；已知限制（resume 首步、无 token 级工具映射）显式登记，阶段 12 再评估；2026-10-23 已在真实模型与 `backend=wasm` 下完成允许/工具拒绝/预检拒绝三条路径验证 |
| D38 | 浏览器入手路径与 Pages 发布（2026-10-25，用户裁决） | 新增仓库内 `web/`：`authoring/`（浏览器工作台）、`sandbox/`（运行沙盒）、`connect/`（接入向导）三个**单文件页面** + 静态入口 `web/index.html` + 共享配方 `web/shared/recipes.js` + 统一构建 `web/build.sh`（一次 wasm 构建按页面表注入占位符，校验残留占位符与 `</script`）。三页与 `docs/authoring-contract.md`、`skills/pact-domain-authoring/` 构成完整入手链。页面全部离线：无外部资源、无 `fetch`、无 `eval`；`pact-wasm` 以 base64 字节实例化；动态 DOM 文本经 `esc()`；`?embed=1` 供第三方 iframe 嵌入；`≤760px` / `≤480px` 两档断点。为支撑沙盒，`pact-wasm` 增只读视图 `facets`（参与者、种子授权、前置图），ABI 不升版（向后兼容的新增导出）。发布走 `.github/workflows/pages.yml`：只组装已提交的 `index.html` 到 Pages，不在 CI 重跑 wasm 构建。 | 降低上手门槛的最后一公里：不装工具也能写、跑、接。页面只复用 `pact-wasm` 的既有语义（无新引擎能力），避免双实现漂移；构建产物入库以支持双击离线使用，Pages 只是同一文件的分发副本。发布用 Actions 而非 `gh-pages` 分支，避免仓库里维护一份二进制分支；CI 不重跑 wasm 构建，防止发布产物与仓库内容不一致。 |
| D39 | 显式状态交接与活动实例别名（2026-10-25，用户要求实施状态交接 + 活动实例别名） | (1) **交接**：`POST /domains/{d}/handoff` 校验源/目标键、目标不存在、新声明良构与类型闭包（源 Counter 全类型仍被声明）→ 持久冻结源（`<instance>.sealed`）→ 追加 `Event::Imported{state,consumed_requests,source_domain,source_instance,source_seq,declaration}`（**提交点**）→ 建目标会话 → 更新声明注册表 → best-effort 激活。(2) **`Imported`**：自足基线（完整 `CounterState` + 幂等表 + 声明原文），schema `version = 2`；恢复取最后一个基线（`DomainLoaded`/`Imported`），`is_baseline`/`baseline_declaration` 统一；`SUPPORTED_EVENT_SCHEMA_VERSION = 2`，更高 fail-fast。(3) **冻结**：拒绝 Seed/Produce/新 Consume，Rule-R 读与幂等重放放行；标记跨重启保持，可显式解冻。(4) **别名**：`<domain>/active.json`（`{version,instance,ts,by}`，原子写、损坏 fail-fast）；短路由解析活动实例，无指针回退 `default`；实例路由始终显式；卸载活动实例清除指针；启动悬空指针 fail-fast。(5) **接口**：REST handoff/active/freeze，CLI `domain handoff|activate|active|freeze`，MCP 四工具（17→21）；交接 ACL 要求源/目标实例键与 `domain/*`，激活要求 `domain/*`。(6) **原子性**：追加失败回滚冻结且不建目标；激活失败只置 `activated=false`，不回滚已提交交接；不提供原位保留状态的声明换版、双活或自动回滚。 | 长周期协作流换版不能丢失已积累状态，也不应迫使客户端改路由；外部 watcher 只更新声明、原位换版改动面过大，故取显式交接 + 域级别名。`Imported` 自足使新实例日志可独立审计与恢复（NFR2），事件级版本避免普通事件被迫升版；类型闭包只看 Counter，Archive 允许历史类型只读。冻结持久化保证交接后源不会被并发写入；别名指针与事件日志分离，避免控制面进入数据面 `seq`。 |
| D40 | 物引规范层抽取与消费（2026-10-26，用户要求抽取 `pact-ref-spec` 并切换消费版本） | 在 `pact` workspace 新增纯 crate `crates/pact-ref-spec`：`ReferenceForm`（URI/Identifier/`∅`）、URI profile 校验、`RefBindings` 继承匹配、`ContractCard` 校验；新增 `spec/ref/v1/uri-profile.md`、`ref-bindings.schema.json`、`contract-card.schema.json`、`reference-plane.openapi.yaml`。`pact-ref` 的 `ref-core` 改为 re-export 规范类型，并以 `version = "0.1.0"` + `vendor/pact-ref-spec` 只读快照消费；`scripts/sync-spec.sh` 负责同步与 `--check`。规范 crate 不依赖 engine/store，可编译到 wasm32。 | 把“票面/绑定/契约卡”的规范从治理实现中抽出，避免两仓库各写一套校验；引导阶段用 vendored 快照避免 CI/容器构建跨仓库检出与网络依赖，注册表发布后删除快照、只保留 version 依赖。静态检查由 D41 补齐。 |
| D41 | 物引静态检查命令（FR27，2026-10-27，用户要求实现 `pact ref lint`） | `pact-cli` 新增离线子命令 `ref lint`（`crates/pact-cli/src/ref_lint.rs`）：读取 `--bindings` / `--contracts`（文件或目录，展开 `.yaml/.yml/.json`）与可选 `--declaration`，用 `pact-ref-spec` 做解析与规范校验，并按规则交叉检查：重复域/契约、绑定类型键与声明类型、契约引用可解析、绑定接受面与契约 scheme 一致（收窄仅告警）、`required` 无契约、Identifier 放行、未注册 scheme 放行、`declarationDigest` 缺失、契约 retired/deprecated、未使用契约。输出 `{bindings,contracts,errors,warnings,findings[]}`；有 error 退出 1，`--strict` 下 warning 也退出 1。声明内容摘要不做内容校验（规范未定义算法），仅检查格式与存在性。 | 把物引治理的静态阶段做成可进 CI 的离线命令，复用已抽取的规范 crate，避免 CLI 再写一套解析/校验；分层保持在纯函数层，不经 daemon、不读运行期状态。scheme 收窄可能是刻意策略，因此只告警；scheme 越界会让绑定接受契约不覆盖的引用，因此报错。未使用契约在传入整库卡片时可能偏噪声，故只告警。 |
| D42 | 物引可见性过滤（FR28，2026-10-27，继续推进物引治理路线） | `pact-http` 新增 `visibility` 模块：daemon `[ref_bindings].paths` 指向的 RefBindings 在启动时读取、规范校验并按域索引；`GET /counter` 与 `GET /archive` 逐 token 计算 `RefExposure`，默认 `read-gate`，`domain` 直接可见，`hidden` 永不返回，仅清空 `thing_ref`（保留 token 存在性）。`read-gate` 判定使用 `engine.my_gates` 的无条件 `reads`/`consumes` 子句，`produces` 不授予可见性。视图调用者身份走 `authenticated_principal`：令牌/签名模式下匿名 `401`，信任模式为 `None`（read-gate 隐藏）。profile 解析按 RefBindings 继承链，URI 匹配失败时回退 `defaults → type → instance/type`（忽略 scheme 覆盖）。加载失败/重复域拒绝启动（fail-closed）。 | 票面可能含私有 URI、版本库位置与访问令牌，视图默认暴露会形成旁路；`read-gate` 复用声明中的读取意图，不引入新权限模型。`domain`/`hidden` 用已有 RefBindings 字段表达，避免 daemon 配置与治理文档两套真源。`produces` 只证明写入权，不应推导读取权。条件子句不做逐 token 求值，保守不可见，避免静态存在性高估。视图级读审计由 D43 补齐。 |
| D43 | 视图级读审计（FR17/FR28，2026-10-27，继续推进物引治理路线） | `read_audit = true` 同时覆盖 Counter/Archive 视图读取：同一 `.read-audit.jsonl` 新增 `kind=view` 记录，字段含 `view`（`counter`/`archive`）、`type_filter` 与按类型聚合的 `types[{type,tokens,refs_visible,refs_filtered}]`；只记暴露计数，**不记 `thing_ref` 值**。`ReadAuditRecord` 的 `participant`/`token_id` 改为可选（匿名视图）；`kind` 缺省 `read`，旧审计流无 `kind` 仍按 `read` 解析。视图审计由引擎 actor 单写者落盘（`Engine::record_view_audit`），写失败上抛 → 接口层 `500` 拒绝视图（fail-closed）。 | 可见性过滤若无审计只形成「表面安全」，无法回答谁在何时看到了哪类票面；复用既有独立审计流与 `read_audit` 开关，避免第二套真源。只记计数不记票面值，守住 ThingRef「无秘密」约束；旧记录兼容避免审计流迁移。审计仍不进入事件日志 / `seq` / 重放 / Rule-T，也不改变可见性判定本身。 |
| D44 | 引用保留、tombstone 与可发现性（FR29，2026-10-27，保留阶段） | ContractCard `lifecycle` 收紧：`retention` 只接受 `permanent` 或正 ISO 8601 duration；`deletionPolicy` 枚举 `retain`/`delete`/`tombstone-then-delete`；`tombstone-then-delete` 强制 URI `tombstoneRef`，其余策略禁止声明；归档钉版（`pinning.requiredForArchive`）禁止 `delete`。新增 `pact-ref-spec::Tombstone` 与 `spec/ref/v1/tombstone.schema.json`：`metadata.id` 为 URI，`spec` 含 `contract`/`ref`/`reason`/`deletedAt`、可选 `retainUntil`（须晚于 `deletedAt`）、`replacedBy`（`reason=replaced` 必填）与 `note`。`pact ref lint --tombstones` 增加契约存在性、策略一致、引用 scheme/长度、重复 id 与重复 (contract, ref) 检查；归档钉版但无 `lifecycle` 的契约给出警告。`pact ref lint` 另对 `required=true` 做可发现性检查：`required_hidden`、`view_read_path_missing`、`no_reader`、`retention_missing`、`deletion_without_tombstone`。 | 保留期与删除证据必须能在进入运行前验收，否则 Archive 中的 token 可能指向已消失且无迹可查的制品；可发现性检查回答“下游行动时是否真的能拿到票面与保留证据”。复用既有 `lifecycle` 字段并收紧语义，不新增引擎能力；tombstone 作为普通域制品/token 产生，保持引擎不 fetch/delete。只做静态一致性，不做计时与自动删除，避免把治理平面塞进引擎；周与日期分量互斥、时间戳必须可比，减少「看起来合法」的保留声明。 |
| D45 | ThingRef 入口校验边界与策略（FR30，2026-10-28，入口阶段） | 校验放在 **REST 写入边界**：`pact-http` 在 `seed_inner`/`produce_inner` 调引擎前执行 `VisibilityProfiles::validate_thing_ref`；三态由 `ThingRefField` 保留（缺失/显式 `null`/字符串），结构规则与 RefBindings 策略（`required`/`allowIdentifier`/`schemes`/`maxLength`）在写入时强制，成功写规范化值。错误码 `ThingRefMissing`/`ThingRefRequired`/`InvalidThingRef` 均为 `400`。 | 引擎与 `pact-core` 必须保持不透明、不得依赖治理 crate，否则「引擎不解析票面」的隔离被打破；放 REST 边界使 CLI、MCP、`pact-runtime` 经同一 client 自动继承，避免多套真源。三态必须在反序列化层保留，serde 的 `Option` 会折叠「没说」与「确认没有」。策略校验复用规范层匹配，保证 lint 与运行期同一套语义；WASM 本地宿主不在 REST 边界内，单列事项。 |
| D46 | 形态、未知 scheme 与钉版策略的执行点（FR31，2026-10-29，入口/归档阶段） | `value`、`onUnknownScheme`、`pinning` 在 REST 边界执行：写入时（seed/produce）与归档前（consume 调引擎前）。`onUnknownScheme` 的“未知”= 有效 `schemes` 非空且不含 `*` 时不在白名单；`treat-as-identifier` 重分类后仍须 `allowIdentifier`。`pinning: required` 写入即拦，`required-at-archive` 仅在 consume 拦；钉版=URI 含 `sha256:<64 小写 hex>`（fragment 或 OCI 形式），`∅` 豁免。consume 只读 `merged_rule` 的 `pinning`，不重跑写入期规则。失败 `ThingRefUnpinned`（`400`）。 | 规范层早已声明三类策略，但只在 lint 生效会让“检查通过”与“运行期行为”脱节；执行点继续放 REST 边界以保持引擎不透明。`required-at-archive` 必须在进入 Archive 的唯一转换（consume）上拦，否则归档后无法补救；归档检查只读钉版字段，避免规则收紧后历史 token 永久无法归档（fail-open 于写入规则、fail-closed 于钉版）。ContractCard 的等价字段需要先解决“卡片从哪加载”，故不并入本轮。 |
| D47 | 物引契约卡运行期加载与版本不可变（FR32，2026-10-29 定案；2026-10-30 已实现） | daemon 新增可选 `[ref_contracts].paths`（文件或目录，可重复；目录展开 `.yaml`/`.yml`/`.json`，规则同 `[ref_bindings]`）；启动时用 `pact-ref-spec::ContractCard::validate` 校验，并以 `contract_id()`（`metadata.id@version`）建索引。**按数据触发**：任一 RefBindings 有效规则声明 `contract` ⇒ 必须配置且每个引用可解析，否则启动失败；无 `contract` 引用 ⇒ 不要求配置。**同版本不可变**：同一 `contract_id()` 出现两次（无论内容是否相同）⇒ 启动失败。**只加严**：consume 的有效钉版要求 = 绑定 `pinning` ∪ 卡片 `reference.pinning.requiredForArchive` / `artifact.digestPolicy: required-at-archive`，卡片不得放宽绑定；判定与错误码沿用 D46（`ThingRefUnpinned`）。首版不读 `artifact.kind`/`mediaType`、不做 `lifecycle` 运行期检查，也不做热更新（D48）。 | 卡片已被绑定声明、被 lint 检查，运行期不读会让“检查通过”与“归档行为”脱节；索引键必须与绑定 `contract` 的 `id@version` 一致，否则同一引用在不同层指向不同版本。按数据触发让不使用契约的部署零迁移，同时保证一旦使用就 fail-closed。同版本不可变比“重启/重载”更底层：否则同一部署在不同时刻对同一 token 给出不同判定且不留痕迹。卡片只加严，避免“卡片与绑定谁优先”的歧义；artifact/mediaType 需要票面携带 media type，当前不推断，避免把不可判定的事做成检查。 |
| D48 | 物引契约卡与绑定显式重载（FR33，2026-10-29 定案；2026-10-30 已实现） | 新增 `POST /ref/reload`（body `{"dryRun": bool}`，缺省 false）与 CLI `pact ref reload [--dry-run] [--json]`；不提供 MCP 工具。重载把 `[ref_contracts]` 与 `[ref_bindings]` 作为**同一事务**：读取配置源 → 解析/规范校验 → 校验所有 `contract` 引用可解析 → 计算影响报告 → 写控制面审计 → 原子替换内存索引与规则；任一步失败保持旧状态，不做部分生效。并发重载单飞，冲突 `409 RefReloadBusy`；校验失败 `400 RefReloadInvalid`；审计写失败 `500`。`dryRun=true` 完成校验与影响报告但不替换；报告按 `(domain,instance,type)` 统计换绑后因缺摘要而无法归档的存量 Counter token 数，不输出票面值。审计写 `<data_dir>/ref-reload.jsonl`（append+fsync）：`ts`/`by`（无身份为 null）/`dryRun`/`outcome`/`files[{path,sha256,kind}]`/卡片与绑定计数/可选 `error`。认证：配置任何身份后必须认证；ACL 启用时调用者须对当前与新的绑定索引中每个域持有 `{domain}/*`，索引为空时只需认证；信任声明模式放行匿名。实现上 daemon 启动注入 `RefReloadConfig`（绑定/卡片路径与审计路径），每次重载**重新读取**配置源，`VisibilityProfiles` 记录本次实际读取文件的 `sha256`（审计摘要与生效内容同源）；索引以 `RwLock<Arc<VisibilityProfiles>>` 原子换指针，请求处理期间取同一快照。未注入配置 → `500 RefReloadUnavailable`；影响预检引擎错误 → `500 RefReloadFailed`；审计写失败统一 `500 RefReloadAuditFailed`（覆盖原错误）；认证失败与并发冲突不写审计。不做文件监听、SIGHUP、自动回滚与按域灰度。 | 卡片不含状态、不进事件日志，不能照搬 D39 的交接；版本化 + 显式激活即可获得“新版本服务新工作、旧版本仍可解析”的效果，而无需迁移状态。卡片与绑定必须同事务，否则中间态会出现“绑定引用了尚未加载的新版本”或反之。原子换指针（在途请求持旧快照）避免读到半套规则。默认不监听文件，避免半写文件、跨文件依赖竞态与不可审计的版本漂移；`dryRun` 让“收紧钉版会阻断存量归档”在生效前可见。审计先于替换写盘，保证每次行为变化都有可追溯记录。 |

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
  pact-wasm      # WASM 本地分发形态（受支持，D30/D31/D33）：内存会话 + 两阶段提交 + 导出/恢复 + wasm-bindgen 导出 + Node 宿主适配层；wasm CI 门禁，非 dist/ 产物
  pact-mcp       # MCP stdio 薄封装（rmcp 实现）；工具实现已下沉 pact-http::mcp，二者共用
  pact-http      # axum REST + SSE + 内建 `/mcp`；TLS、Ed25519 签名身份、OIDC 资源服务器；类型化 client 与 pactd bin（裸 daemon）
  pact-cli       # clap 命令行（daemon 控制 + 领域操作 + 离线分析），单二进制 `pact`
  pact-runtime   # 生产运行层（D24）：参与者驱动循环（生产优先 / 汇消费 / 等待三策略）+ 多 agent 编排（MCP/REST）+ 外部副作用执行器（outbox/saga 消费端）
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
- **TokenId 唯一性守卫**：`Session` 在 Rule-S/Rule-P 校验通过后，用内存 `used_ids`（Counter + Archive 并集，恢复时重建、不进快照）拒绝重复 id，返回 `RuleError::DuplicateTokenId`；守卫位于 kernel 而非规则函数，保持 `rule_s` 不读状态。

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
    DuplicateTokenId { token_id: TokenId }, // 目标 id 已在 Counter 或 Archive 中使用
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

状态交接与活动实例（D39）：

```json
// POST /domains/{d}/handoff
{ "source_instance": "default", "target_instance": "v2",
  "declaration": "types { … } …", "activate": true }   // activate 缺省 true

// 成功响应 201
{ "domain": "proj", "source_instance": "default", "target_instance": "v2",
  "source_seq": 42, "target_seq": 1,
  "counter": 7, "archive": 12, "consumed_requests": 3,
  "active_instance": "v2", "activated": true }

// GET /domains/{d}/active → 200
{ "domain": "proj", "instance": "v2" }

// PUT /domains/{d}/active → 200
{ "instance": "default" }                               // 请求体

// PUT /domains/{d}/instances/{i}/freeze → 204
{ "frozen": true }                                       // 请求体；false 即解冻
```

失败码（HTTP 409）：`InstanceFrozen`、`HandoffIncompatible`、`InstanceExists`；目标/源不存在为 `InstanceNotFound`(404)，越权为 `Forbidden`(403)。

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
    /// 状态交接基线（D39）：整段状态搬迁，单行自足；写 schema v2。
    Imported  { seq: u64, ts: Timestamp, name: String, declaration: String,
                source_domain: String, source_instance: String, source_seq: u64,
                state: CounterState,
                consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)> },
    Terminated{ seq: u64, ts: Timestamp },                // Rule-T 检测结果（信息性）
}
```

线上/落盘形态统一为 `WireEvent{version: u32, #[serde(flatten)] event: Event}`（D25 子项 1）：
普通事件 `EVENT_SCHEMA_VERSION = 1`；`Imported` 写 `IMPORTED_EVENT_SCHEMA_VERSION = 2`（按事件自身版本输出）；
读取上限 `SUPPORTED_EVENT_SCHEMA_VERSION = 2`，缺 `version` 视同 v1，更高版本读取 fail-fast（`UnsupportedSchemaVersion`，见 §7.5）。
`is_baseline()` 同时识别 `DomainLoaded` 与 `Imported`，`baseline_declaration()` 对二者返回 `(name, declaration)`。

JSONL：**每域实例**一个 `<domain>/<instance>.events.jsonl`，逐行 JSON 追加，`seq` 在该实例内单调。

### 7.2 存储布局

```
<data_dir>/
  <domain>/                        # 每个域声明一个子目录（D27）
    <instance>.events.jsonl        # 真相源，只追加（前缀压缩后为 `seq > base_seq` 的超集/尾部）
    <instance>.snapshot.json       # 定期 / 压缩时写入的快照（自足锚：version/seq/dl_seq/declaration/state/consumed_requests），原子写（tmp+rename）
    <instance>.logmeta.json        # 压缩水位（version/base_seq/compacted_at），缺省不存在（D25 子项 1）
    <instance>.sealed              # 冻结标记 JSON {version, reason, ts, by}（D39）；缺省不存在
    declaration.pact               # 声明原文（启动恢复时回填声明注册表）
    active.json                    # 活动实例指针 {version, instance, ts, by}（D39）；缺省不存在 ⇒ 回退 default
  index.tursodb                    # 全部实例共用：events 与 tokens 的查询索引（(domain, instance) 双列）
  pactd.lock                       # data_dir 独占 flock（单写者护栏，D27）
```

`active.json` 与 `<instance>.sealed` 均为小体积控制面文件：原子写（tmp → fsync → rename → 目录 fsync），不进入事件日志、不参与 `seq` / 快照 / 压缩。两者自版本 2 起记录控制面审计字段 `ts`（注入 `Clock`：生产 ISO-8601，测试逻辑时钟）与 `by`（认证身份；信任声明模式为 `None`）；旧版纯文本 `.sealed` 与无审计字段的 `active.json` 仍可读（字段缺失视为 `None`）。`active.json` 解析失败或指向未加载实例 ⇒ 启动 fail-fast（避免静默误路由）；卸载活动实例时删除指针。快照 schema 未变；`Imported` 后的实例以「状态非空」判定已占用，而非仅凭 `last_seq > dl_seq`。

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
- Consumed 事件自足（携 token 全量快照），重放与审计不依赖同一域实例其他行；目标必须仍在 Counter（缺失 ⇒ `ConsumeTargetNotOnCounter` fail-closed，禁止重复归档）。
- **压缩窗口**（§7.5）：快照 → logmeta 水位 → 日志原子重写 → 索引裁剪；崩溃于任一中间点均落到「未截断超集态」或「已截断 + 水位 + 快照」两个一致态之一，恢复侧以 `base_seq > 0` 补 `prune_events` 自愈。

### 7.5 日志压缩与 schema 版本化（D25 子项 1，已实现）

**schema 版本化**：真相源与线上事件统一包裹于 `WireEvent{version, #[serde(flatten)] event}`，顶层 `version` 恒先于事件体出现在 JSON（`{"version":1,"kind":…}`）。普通事件 `EVENT_SCHEMA_VERSION = 1`；状态交接事件 `Imported` 写 `IMPORTED_EVENT_SCHEMA_VERSION = 2`（D39）；读取上限 `SUPPORTED_EVENT_SCHEMA_VERSION = 2` 为唯一真源（`pact-store-model::event`）。`Notification`（§6.4 SSE）同样带顶层 `version`。兼容契约：

| 情形 | 处理 |
|------|------|
| 旧日志/旧快照**缺 `version`** | serde `default` 视同 v1（向后兼容） |
| `version` 在 `1..=SUPPORTED_EVENT_SCHEMA_VERSION` | 正常重放；`Imported` 仅接受 v2 |
| `version > SUPPORTED_EVENT_SCHEMA_VERSION` | **fail-fast `StoreError::UnsupportedSchemaVersion`**（不静默按 v1 重放，防误读新格式） |

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
| 归档原子性 | Counter 移除与 Archive 追加由同一事件导出，二者永不分裂；目标不在 Counter 时 `apply_event` 以 `ConsumeTargetNotOnCounter` fail-closed（防宿主违反串行契约造成重复归档） |
| 持久原子性 | 提交点 = 日志 fsync；此后步骤尽力而为，崩溃后重放等价于已提交前缀 |
| 幂等 | 可选 `request_id`，重复请求返回首次结果（跨重启由日志重建幂等表生效） |

**边界（引擎不承诺的部分）**：引擎不执行外部副作用，`ThingRef` 对引擎不透明、不校验其指向物是否存在。因此"`consume` 成功"与"外部动作（产物真正生成/落地）成功"是两个**独立承诺**，二者之间是**最终一致**而非同一事务：消费成功既不等待、也不回滚外部动作。落地方案有两种，**可并用、不互斥**（同一域可按入口分别选用）：

- **前移为准入条件（witness-token 模式）**：把"外部产物已就位"表示为一枚 token（外部系统经 Rule-S 注入），消费 gate 加 `when exists <readiness>`。于是"就位确认"进入 GateHolds 读集，与消费共享同一次判定——未就位则 `GateConditionFailed` 拦下。示例见 `examples/domains/external_materialization.pact`。
- **后置为显式补偿（saga 式）**：消费照常提交，外部动作失败后由参与者以一次新的 Rule-P 记录"未完成/需重做"事实；历史不可回滚，补偿是追加的新事实（引擎只负责把含全量 token 的 `Consumed` 事件发出，外部动作由 agent/执行器承担，即 outbox 语义）。

两种方案都不引入库事务 / 2PC / 分布式共识——其代价高于本场景收益（D23）。

### 8.6 显式状态交接与活动实例别名（D39，已实现）

**目标**：声明换版不丢失已积累的 Counter/Archive/幂等表，且客户端可继续使用短路由。

**交接时序（引擎临界区内，单写者）**：

1. 校验域名/源实例名/目标实例名（`[A-Za-z0-9_-]{1,128}`）；源 ≠ 目标；目标在内存、事件日志与残留 `.sealed` 中均不存在（否则 `InstanceExists`）。
2. 解析新声明 `load_domain`（良构 + 组合无关）；源实例存在且未冻结（否则 `InstanceNotFound` / `InstanceFrozen`）。
3. **类型闭包**：源 Counter 中每个 `TypeName` 必须仍出现在新声明 `types`；Archive 不参与（允许历史类型只读）。缺失 ⇒ `HandoffIncompatible`，**无任何副作用**。
4. **冻结源**：写 `<domain>/<source>.sealed`（原子写；含 `reason` 与 `ts`/`by`）。失败 ⇒ 直接返回，目标未创建。
5. **提交点**：向目标日志追加 `Event::Imported{seq: 1, state: 源 Counter/Archive, consumed_requests, declaration: 新声明原文, source_domain/source_instance/source_seq}`（fsync）。失败 ⇒ 删除 `.sealed`、恢复内存 `frozen=false`，目标不留日志。
6. 以 `Session::recover(&[imported], None, seq)` 建目标会话（`dl_seq = 1`，状态来自事件，不重复 apply 基线）。
7. 登记新声明到域注册表（后续 `instantiate` 使用新声明）。
8. `activate = true` 时写 `active.json`（含 `ts`/`by`，与 `Imported` 共用同一时刻）。此步为 **best-effort**：失败只令 `activated=false`，不回滚已提交交接。

**冻结语义**（源实例）：

| 动作 | 冻结时行为 |
|------|------------|
| `seed` / `produce` / 新 `consume`（`Prepared`） | 拒绝：`InstanceFrozen`(409) |
| 幂等重放（`request_id` + 同 token） | 放行：返回首次归档结果（`Replayed`） |
| `read` / `counter` / `archive` / `gates` / `enabled` / `status` / SSE | 可用 |
| 重载/实例化（无 `force`） | 拒绝：`InstanceFrozen`(409)；卸载不解除冻结 |
| `force` 重置加载 | 清除 `.sealed` 与内存 `frozen`（显式重置即解冻） |

**别名解析**：`ServerState::resolve_key(domain)` = `engine.active_instance(domain)` → `InstanceKey`；短路由（`/domains/{d}/…`）用它，实例路由（`/domains/{d}/instances/{i}/…`）不解析。无指针 ⇒ `default`。客户端 `base_path(None)` 走短路由，`base_path(Some(i))` 始终显式（含 `default`），保证别名启用后仍可访问源。

**恢复**：取最后基线（`DomainLoaded` 或 `Imported`）；快照可用条件 `baseline_idx.is_none() || (snap.seq >= dl_seq && snap.seq <= last_seq)`；`Imported` 基线直接采用事件 `state`/`consumed_requests`。启动时读取每个域的 `active.json`，指针悬空 ⇒ `EngineError::Recovery`（进程启动失败，命令以该错误应答）。`unload_domain` 若卸载活动实例则 `clear_active`。冻结标记在卸载后仍生效：重载/实例化先查 `.sealed`，无 `force` 即 `InstanceFrozen`，`force` 重置才清除标记。

**审计**：`Imported` 单行包含完整状态、幂等表、源坐标与目标声明，可独立重放；冻结/解冻与活动切换不写事件日志（控制面），其操作者与时间记录在 `.sealed` / `active.json` 的 `by`/`ts` 字段（`GET /domains/{d}/active` 返回 `ts`/`by`）。`by` 取已认证身份；信任声明模式下无身份，记为 `None`（不虚构操作者）。

## 9. 接口层（阶段 4，已实现）

三类接口（REST / MCP / CLI）共享 §6 线上格式，能力对等；MCP 有 stdio 与 Streamable HTTP 两种传输，差异仅在传输与形态。接入方式（D19）：daemon 是唯一权威进程（`pact daemon start` 或 `pactd`，REST + SSE + 可选 `/mcp`）；pact-mcp 与 pact-cli 在线命令均经 `pact_http::client` 走 REST 接入 daemon，不内嵌引擎。

### 9.1 pact-mcp（AI agent 主接口，已实现）

工具实现位于 `pact-http::mcp`（`PactMcp`），stdio 与 daemon 内建 `/mcp` 共用同一份 21 个工具。`pact-mcp` 仅保留 stdio 薄封装（D35）：`--unix <sock>` / `--tcp` / `--https`（配 `--tcp`）/ `--tls-ca` / `--sign-key` / `--sign-as` / `--bearer|--bearer-file`，由 agent 拉起为子进程，工具经 REST 转发至 daemon。工具清单（前缀 `pact_`）：

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
| `pact_domain_handoff` | 状态交接：冻结源、以新声明创建目标并迁移状态（D39） |
| `pact_domain_active` | 查询域的活动实例（无指针回退 `default`） |
| `pact_domain_activate` | 切换域的活动实例指针 |
| `pact_domain_freeze` | 冻结/解冻实例（`unfreeze` 布尔，默认冻结） |

工具的输入输出 schema 由类型派生（serde + schemars），错误以 §6.3 结构化 JSON 返回，agent 可编程处理（如 gate 被拒后转入等待）。

### 9.2 pact-http（应用程序接口，已实现）

axum，JSON over HTTP，监听 unix socket（默认，单机安全）与/或 TCP（可配；TLS 见 §9.5）。载荷格式一律取 §6。实现注记：`{d}:analyze` / `{d}:compose` 因路由库不支持同段"参数+字面量"，在 `/domains/{d}` 统一入口内按段内冒号后缀分发（对外 URL 不变）；另提供独立 `POST /validate`（声明校验，不触域）、`GET /health` / `GET /ready` / `GET /metrics`、按需挂载的 `/mcp` 与 `GET /.well-known/oauth-protected-resource`，以及类型化 Rust client（pact-mcp / pact-cli / pact-runtime 共用）。

MCP 线上端点（D35/D36，`[mcp].enabled = true` 时）：

```
POST/GET/DELETE /mcp                                      MCP Streamable HTTP（21 工具）
GET             /.well-known/oauth-protected-resource    RFC 9728 元数据（仅 OIDC 启用时）
```

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
GET    /domains/{d}/read-audit?limit= 读审计视图（D25 子项 3；短路由按活动实例解析）
POST   /domains/{d}/handoff            {source_instance, target_instance, declaration[, activate=true]}  状态交接（D39）
GET    /domains/{d}/active             活动实例别名（响应含 ts/by）
PUT    /domains/{d}/active             {instance}   切换活动实例（域级 ACL `domain/*`；响应含 ts/by）
PUT    /domains/{d}/instances/{i}/freeze  {frozen}  冻结/解冻（D39）
```

被拒动作返回 4xx + §6.3 结构化错误体（错误码 = RuleError 变体名）。

**实例路由（D27/D39，阶段 10，已实现）**：上表 `/domains/{d}/…` 语义 = **活动实例**（`active.json`；无指针回退 `default`）。实例级为 `/domains/{d}/instances/{i}/…` 镜像组（seeds / tokens / tokens/{id}/read / tokens/{id}/consume / counter / archive / gates/{participant} / enabled / status / compact / events / read-audit 全套同形），另加实例管理端点：

```
POST   /domains                       {name, declaration[, instance][, force]}   declare + instantiate（默认 default）
POST   /domains/{d}/instances         {instance}                                 在同一域声明下 instantiate 新实例
GET    /domains                       域清单（Vec<DomainListing{name, instances, active?}>）
```

- `default` 缺省回退：短路由未给实例段且无活动指针 ⇒ `default` 实例，既有脚本语义不变；活动指针非空 ⇒ 解析到活动实例。`InstanceKey{domain, instance}` 的 `Display = "domain/instance"`。
- 实例键以**路径段**承载；`DomainLoaded` 事件**不新增 `instance` 字段**（实例由文件路径承载）。
- 键名谓词 `[A-Za-z0-9_-]{1,128}`（`validate_domain_name` / `validate_instance_name`）在 `engine.instantiate` / `load_domain`（唯一触盘入口）强制，防路径穿越；未实例化键的动作端点返回 `InstanceNotFound`(404)，不触盘。
- 错误码扩展：`InstanceNotFound`(404) / `DomainNotDeclared`(404) / `InstanceExists`(409) / `DomainBusy`(409) / `InstanceFrozen`(409) / `HandoffIncompatible`(409) / `InvalidDomainName`(400) / `InvalidInstanceName`(400)；交接源已冻结亦为 `InstanceFrozen`。
- 认证：实例级与默认级共享同一 `*_inner` handler，`authorize` 统一在 `*_inner` 内完成，实例路由同样受令牌/签名身份约束。
- **状态交接 ACL 与认证（D39）**：配置了任何身份（`[auth.tokens]` 或 `[auth.keys]`）后，`handoff` / `freeze` / `GET|PUT /active` 均要求认证（不沿用生命周期端点的信任行为）；信任声明模式（两者皆空）放行匿名。ACL 启用时：`POST /domains/{d}/handoff` 要求源实例键、目标实例键与域通配键 `{d}/*`（交接会改写域级 `declaration.pact`）；`PUT /domains/{d}/active` 要求 `{d}/*`；`GET /domains/{d}/active` 与 `freeze` 按实例键授权。
- **域级 ACL（D25 子项 3，已实现）**：`[auth.acl]` 非空即启用。实例级端点（生命周期、四类动作、查询、compact、读审计视图）按 `(domain, instance)` 校验；定义级 `:analyze` 要求对该域任一授权；`GET /domains` 按授权过滤（空域剔除，`active` 无权访问时置空）。无有效身份 401，身份不匹配 401，越权 403；ACL 空表时全部域级检查短路（启用签名时验签仍生效；静态 Bearer 仅用于四类动作与新增控制面端点）。

### 9.3 pact-cli（人类接口，已实现）

单二进制 `pact`（D20：域操作一律 `--domain` / `--participant` / `--token` / `--type` flag，不用位置参数简写）。全局 flag：`--unix <sock>`（缺省 ./pact.sock）/ `--tcp <addr>`（优先于 --unix）/ `--https`（须配 `--tcp`，TLS 连接）/ `--tls-ca <pem>` / `--sign-key <file>` + `--sign-as <participant>`（成对；base64 32 字节 Ed25519 种子）/ `--bearer <token>` / `--bearer-file <path>`（与 `--bearer` 互斥，便于 secret 挂载）/ `--json`（§6 线上格式输出）。

- 注入点：`pact domain validate <file.pact>`；`pact domain load <file.pact> [--name N] [--instance I] [--force]`；`pact seed --domain D --participant P --type T [--thing-ref R]`。
- 领域操作：`pact produce --domain D --participant P --type T [--predecessors id1,id2] [--thing-ref R]`；`pact read --domain D --participant P --token ID`；`pact consume --domain D --participant P --token ID [--request-id R]`；`pact counter ls --domain D [--type T]`；`pact archive ls --domain D [--type T]`；`pact gates show --domain D --participant P`；`pact enabled --domain D --participant P`；`pact status --domain D`；`pact compact --domain D [--instance I] [--keep-last N]`（日志前缀压缩，D25 子项 1）。
- 域管理：`pact domain list|info --domain D|unload --domain D`；**实例（D27）**：`pact domain instantiate --domain D --instance I`、`pact domain instances --domain D`（打印该域实例清单）；域操作命令均接受可选 `--instance I`（缺省 = 活动实例，无指针回退 `default`），`--force` 仅用于非空实例的显式重置。`pact domain list` 打印 `name [instances…]`。
- **状态交接（D39）**：`pact domain handoff --domain D --declaration <file.pact> --source I --target I [--no-activate]`（`--source`/`--target` 必填）；`pact domain active --domain D`；`pact domain activate --domain D --instance I`；`pact domain freeze --domain D --instance I [--unfreeze]`。
- 离线能力（不经 daemon）：`pact analyze <file.pact>`、`pact compose f1.pact f2.pact ...`、`pact replay <domain>.events.jsonl`。
- daemon 控制：`pact daemon start --config pact.toml`（前台运行，与裸 `pactd` bin 等价）。
- 在线命令经 unix socket 调 REST API（复用 `pact_http::client`）；错误按 §6.3 结构化输出，exit code 非 0。

### 9.4 配置与身份（D11，NFR6 v2 扩展）

`pact.toml`（全部字段可选；`deny_unknown_fields`）：

```toml
data_dir = "./data"
listen_unix = "./pact.sock"        # 默认；listen_tcp 可选
listen_tcp = "0.0.0.0:7654"        # TLS 仅作用于 TCP 监听
compact_on_start = false           # 可选：恢复后自动压缩全部实例日志（D25 子项 1）
compact_keep_last = 0              # 可选：自动压缩保留的尾部事件条数
read_audit = false                 # 可选：持久化读审计（Rule-R + 视图记录，D25 子项 3 / D43；fail-closed）
pid_file = "/run/pactd.pid"        # 可选

# D34：TLS 终止（缺省关闭）；client_ca 存在即要求客户端证书（mTLS）
[tls]
certificate = "/etc/pact/server-chain.pem"
private_key = "/etc/pact/server.key"
# client_ca = "/etc/pact/client-ca.pem"

# D34：Ed25519 公钥表；非空即启用请求签名（值 = base64 的 32 字节公钥）
[auth.keys]
architect = "<base64-32B-pubkey>"   # 32 字节公钥 base64

[auth]
# 可选：按参与者静态令牌；与 [auth.keys] 可并存；启用签名后 REST 请求必须签名
[auth.tokens]
architect = "<token>"             # Authorization: Bearer
# 可选：域级授权（D25 子项 3）；非空时 tokens 或 keys 至少其一
[auth.acl]
architect = ["proj/*", "other/default"]

# D35：MCP Streamable HTTP
[mcp]
enabled = true
public_url = "https://pact.example.com"   # OIDC 启用时必填；亦用于 allowed_hosts

# D36：MCP OIDC 资源服务器（仅 /mcp）
[mcp.oidc]
issuer = "https://idp.example.com"
audience = "pact-mcp"
principal_claim = "sub"            # 缺省 sub
principal_map = { "auth0|alice" = "alice" }  # 可选：claim 值 -> 参与者名
# jwks_uri = "https://idp.example.com/.well-known/jwks.json"
discovery = true                   # 缺省 true；false 时必须显式 jwks_uri
allowed_algs = ["RS256", "ES256", "EdDSA"]   # 缺省即这三项
# ca = "/etc/pact/idp-ca.pem"      # IdP TLS 自定义 CA
allow_static_tokens = true         # 允许非 JWT 静态令牌并存
allow_insecure_http = false        # 仅本地/测试可设 true
```

接口层在命令进入引擎前完成参与者身份校验；核心与引擎不感知认证方式。身份来源（v2）：

- **REST 动作端点**：启用 `[auth.keys]` 时取 Ed25519 签名身份（除豁免端点外必须签名，Bearer 不能替代）；未启用签名时取 Bearer 静态令牌。请求 `participant` 必须等于解析身份，否则 `401`。
- **REST 生命周期/查询端点**：启用签名时取签名身份；否则取 Bearer 令牌反查的参与者。
- **`/mcp`**：OIDC JWT（D36）或静态 Bearer；OIDC 与静态令牌都不存在时为信任模式。Ed25519 签名**不用于** `/mcp`；OIDC 令牌**不用于** REST。

`Config::validate()` 在 daemon 启动前拒绝：ACL 非空但 tokens 与 keys 均为空；签名密钥非 32 字节/base64 非法；TLS 配置不完整或缺少 `listen_tcp`；`[mcp].enabled` 且 keys 非空但 tokens 为空且无 OIDC；OIDC 启用但未 `enabled` 或缺 `public_url`；issuer/JWKS 非 https（除非 `allow_insecure_http`）、带查询或片段；空 audience/issuer；非法 `principal_claim`/映射目标；未知 alg。

### 9.5 TLS（D34，已实现）

- **启用条件**：`[tls] certificate` 与 `private_key` 同时存在，且必须配置 `listen_tcp`；否则启动失败。`listen_unix` 始终明文（本地 socket）。
- **实现**：rustls + axum-server，TLS 1.2/1.3；证书链 PEM、私钥 PEM（PKCS#8/PKCS#1/SEC1）。`client_ca` 存在时以 `WebPkiClientVerifier` 校验客户端证书（mTLS）；握手失败记录后继续接受，不影响已有连接。
- **客户端**：`pact_http::client::Client::https(addr, ca)`（返回 `Result`）；CLI/MCP/runtime 的 `--https` 与 `--tls-ca` 均落到该构造。内置客户端**不发送客户端证书**；需要 mTLS 的外部客户端须自备证书，否则握手失败。
- **观测**：启动日志标注监听方式（`unix` / `tcp` / `tls` / `mtls`），不记录私钥与令牌。

### 9.6 Ed25519 请求签名身份（D34，已实现）

- **配置**：`[auth.keys] 参与者 = base64(32B Ed25519 公钥)`；非空即启用，空表即关闭。ACL 可仅依赖 keys（tokens 与 keys 至少其一）。**启用后除豁免端点外的 REST 请求必须签名；Bearer 不能替代签名**（两者同时配置时，带签名的请求以签名身份为准）。
- **请求头**：`x-pact-participant` / `x-pact-timestamp`（Unix 秒）/ `x-pact-nonce` / `x-pact-signature`（Ed25519 原始签名 base64）。客户端每请求生成新 ULID nonce；`Client::with_signing` 对所有非豁免 REST 请求（含 GET）生效。
- **规范化串**：`PACT-SIGNATURE-V1\n<METHOD>\n<path?query>\n<sha256(body) 小写 hex>\n<timestamp>\n<nonce>\n<participant>`，无尾换行；body 为空串时 hash 为空体 SHA-256。
- **服务端顺序**：中间件先移除客户端 `x-pact-verified-principal` → 四头齐全 → 时间窗 `±300s`（绝对差）→ 公钥查表与 Ed25519 验签 → nonce 去重（TTL 300s、容量 8192；满则以 429 拒绝新 nonce；验签失败不写入缓存，避免未认证请求污染防重放缓存）→ 注入可信头。`body` 超过 8 MiB 返回 `413`（验签前拦截）。
- **失败语义**：缺头/未知参与者/时间戳越界/签名错误 → `401`；nonce 重放或缓存满 → `429`；body 超限或读取失败 → `413`；ACL 越权 → `403`；请求 `participant` 与身份不一致 → `401`。
- **豁免**：`/health`、`/ready`、`/metrics`、`/mcp` 与 `GET /.well-known/oauth-protected-resource` 不验签；`/validate` 仍需签名。REST 信任模式（无 keys 且无 tokens；OIDC 仅影响 `/mcp`，不影响 REST）保持自报参与者并允许无身份请求。
- **内部能力**：daemon 内建 `/mcp` 调本地 REST 时使用 `x-pact-internal-auth`（随机 32 字节 hex，进程内生成、只驻留内存），验签/令牌中间件识别后仅接受进程内注入的可信参与者；外部请求即使伪造该头也需猜中随机值。该头与令牌均不写入日志。

### 9.7 MCP Streamable HTTP（D35，已实现）

- **挂载**：`pact_http::server::router_with_mcp(state, mcp_enabled)`；`[mcp].enabled = true` 时在基路由上 `nest_service("/mcp", StreamableHttpService<PactMcp, LocalSessionManager>)`，同一监听端口/同一 TLS 终止。`public_url` 提供 authority，与回环名单一起传给 rmcp `allowed_hosts`（Host 头保护）；未配置 `public_url` 时仅回环 Host 可通过。
- **会话**：rmcp 配置以 builder 构造，启用 `legacy_session_mode(true)`（兼容旧 `mcp-session-id` 客户端），其余取默认。
- **工具与本地调用**：`PactMcp` 持有 `Client` 克隆；本地模式走 `tower::ServiceExt::oneshot` 调进程内 Router，不发起网络自连接、不重复 TLS/签名。工具身份由当前请求的 `x-pact-verified-principal` 每调用注入，禁止跨会话缓存。
- **stdio 保留**：`pact-mcp` 仍是 stdio 薄封装，经 REST 接 daemon；签名/TLS/Bearer 参数与 CLI 同名。`pact-runtime` 的 MCP 子进程仅透传 TLS 与 Bearer。
- **信任模式**：`enabled` 且无 OIDC 且 tokens 为空 ⇒ 放行无 Authorization 的请求（仅单机/回环），启动打印 `WARN`；若请求携带任意令牌则 `401`（避免误把令牌当有效身份）。

### 9.8 MCP OIDC 资源服务器（D36，已实现）

- **配置与发现**：`[mcp.oidc]` 见 §9.4。`discovery = true`（缺省）时拉取 `<issuer>/.well-known/openid-configuration` 取 `jwks_uri`；`discovery = false` 时必须显式 `jwks_uri`。发现与 JWKS 均缓存 300s；JWKS 最多保留 64 键；未知 `kid` 触发限频重取（同一 30s 窗口最多一次）。
- **JWT 校验**：仅接受 `allowed_algs`（缺省 RS256/ES256/EdDSA）且 JWK alg/类型匹配；拒绝 `none`、HS*、非空 `crit`；`iss` 精确匹配、`aud` 包含 `audience`、`exp` 必需、`nbf` 存在时校验，leeway 60s；`sub` 必需。`principal_claim` 可为字符串或唯一字符串元素数组，经 `principal_map` 映射后须匹配 `[A-Za-z0-9_-]{1,128}`，否则 `401`。
- **线上契约**：`GET /.well-known/oauth-protected-resource` → `{resource: public_url, authorization_servers: [issuer], bearer_methods_supported: ["header"]}`；OIDC 未启用时该路径 `404`。`/mcp` 失败返回 `401` + `WWW-Authenticate: Bearer resource_metadata="<public_url>/.well-known/oauth-protected-resource"`（`public_url` 存在时）。
- **错误与边界**：令牌缺失/格式非法/签名或 claim 不符 → `401`；IdP 发现或 JWKS 拉取不可达 → `503`；REST 不调用 IdP，故 IdP 故障不影响 REST。OIDC 令牌仅在 `/mcp` 请求内校验，不缓存、不落盘、不转发给 REST；`allow_static_tokens = false` 时 `/mcp` 只接受 JWT。
- **并发**：JWKS/发现缓存按需刷新，失败时不提供陈旧密钥（fail-closed），返回 `503`。

### 9.9 运维接口（D25 子项 2，已实现）

daemon 在域端点之外暴露三个**免身份**运维端点（仅本地/内网，探活与抓取惯例）：

| 端点 | 语义 | 响应 |
|------|------|------|
| `GET /health` | 存活（liveness）：进程在服务即 200，不依赖引擎 | `{"status":"ok"}` |
| `GET /ready` | 就绪（readiness）：探针 = 一次 `list_domains` 往返；引擎可应答（恢复完成且未停机）→ 200，否则 503 | `{"status":"ready","domains":n,"instances":n}` / `{"status":"not_ready","error":…}` |
| `GET /metrics` | 运行指标，Prometheus 文本暴露格式（`text/plain; version=0.0.4`） | 见下 |

指标集（gauge）：`pact_domains_declared`、`pact_instances_loaded`、`pact_counter_size{domain,instance}`、`pact_archive_size{domain,instance}`、`pact_terminated{domain,instance}`（1/0）、`pact_uptime_seconds`。仅上报**已加载（内存）**实例——磁盘存在但未加载者由 `status` 报 `InstanceNotFound`，跳过；引擎不可应答时返回 `503`。

**PID 文件（可选）**：配置 `pid_file` 缺省不写；启动写入自身 PID、退出（正常与信号停机）删除；启动时若发现陈旧文件且其 PID 仍存活 → 拒绝启动（`AlreadyExists`），避免两个 pactd 争同一 pid 文件。存活判定 Linux 下以 `/proc/<pid>` 存在为准（PID 复用可能假阳性 → 保守拒绝），其他平台交由 `pactd.lock` flock 兜底。

**优雅停机**：SIGINT 或 SIGTERM 均走同一路径——广播停机 → 各监听循环停止接受新连接 → **有界（10s）**等待在途连接归零 → `engine.shutdown()`（终态快照）→ 清理 PID 文件与套接字。SSE 长连接会占满宽限窗口，故必须设有界超时（超时后强制进入终态快照，不阻塞退出）。

### 9.10 域级 ACL 与读审计（D25 子项 3 / D43，已实现）

**ACL 模型**：`[auth.acl] principal = ["domain/instance", "domain/*"]`。匹配仅限两种形态（拒绝 `proj`、`proj/*/x`、路径穿越等）；启用后所有域作用域端点先鉴权再进引擎：

| 端点类别 | 身份来源 | 授权键 |
|----------|----------|--------|
| load / instantiate / unload / compact / compose | 启用签名时取签名身份；否则 Bearer 令牌反查参与者 | `(domain, instance)`（compose/默认路径 = `default`） |
| seed / produce / read / consume / gates / enabled | body/query `participant`（须与令牌/签名身份一致） | `(domain, instance)` |
| counter / archive / status / events / read-audit | 启用签名时取签名身份；否则 Bearer 令牌反查参与者 | `(domain, instance)` |
| `:analyze`（定义级） | 启用签名时取签名身份；否则 Bearer 令牌反查参与者 | 该域任一授权 |
| `GET /domains` | 启用签名时取签名身份；否则 Bearer 令牌反查参与者 | 逐实例过滤，空域剔除 |

无身份 → `401 Unauthorized`；身份有效但无该键授权 → `403 Forbidden`（结构化 §6.3 错误体）。ACL 空表时上述检查全部短路，保持既有信任声明行为。daemon 启动前 `Config::validate()` 拒绝「ACL 非空 + 令牌表与密钥表均为空」（无法判定身份）及非法条目。`/health` `/ready` `/metrics` `/validate` 不属于域作用域、不做 ACL；其中 `/validate` 在签名启用时仍需签名（仅运维三端点与 `/mcp`/元数据豁免验签）。

**读审计流**（FR17，默认关闭）：`read_audit = true` 时，Rule-R 成功读取与 Counter/Archive 成功视图读取都向 `<data_dir>/<domain>/<instance>.read-audit.jsonl` 追加一行并 `fsync`。两类记录以 `kind` 区分，旧记录无 `kind` 按 `read` 解析：

```json
{"version":1,"kind":"read","ts":"…","domain":"proj","instance":"default","participant":"architect","token_id":"…"}
{"version":1,"kind":"view","ts":"…","domain":"proj","instance":"default","participant":"architect","view":"counter","type_filter":"design","types":[{"type":"requirement","tokens":1,"refs_visible":0,"refs_filtered":1},{"type":"design","tokens":1,"refs_visible":1,"refs_filtered":0}]}
```

- **视图记录**（D43）：`participant` 为调用者主体；信任声明模式无主体时省略（匿名）。`type_filter` 为请求的 `?type=` 过滤器；`types[]` 按类型聚合 `tokens`（视图内 token 总数）、`refs_visible`（实际返回票面数）、`refs_filtered`（被可见性策略清空票面数）。**不记录 `thing_ref` 值**，审计流同样受「无秘密」约束。
- **fail-closed**：写审计失败 → `EngineError::Store` → REST `500`，本次读取不返回 token、视图不返回结果；杜绝「读成功但无审计」。视图审计由引擎 actor 单写者落盘（`Engine::record_view_audit`），接口层不直接写文件。
- **与核心语义隔离**：审计流不产生事件、不推进 `seq`、不参与重放/Rule-T/快照/压缩；实例未加载也可查询历史审计。
- **读取**：`Store::read_audit_tail` 按写入顺序取末尾 N 条（缺省 100、上限 1000）；仅当文件末尾无换行（崩溃残留半行）时容忍末行解析失败，其余损坏行 fail-fast（不静默跳过，避免掩盖篡改）。
- **视图**：`GET /domains/{d}[/instances/{i}]/read-audit?limit=N` → `{"records":[…]}`，受 ACL 约束。

### 9.11 保留与 tombstone（D44）

**契约声明**：`ContractCard.spec.lifecycle` 现在是可机检的生命周期声明：

```yaml
lifecycle:
  retention: P365D                  # permanent 或正 ISO 8601 duration
  deletionPolicy: tombstone-then-delete
  tombstoneRef: urn:pact:tombstone:se/design-review
```

- `retention`：`permanent` 或 `P[nY][nM][nW][nD][T[nH][nM][nS]]`；分量按序、非负整数、总时长 > 0；`W` 不得与其它日期分量组合（Rust 校验，JSON Schema 做结构约束）。
- `deletionPolicy`：`retain`（不自动删除）、`delete`（到期删除且无 tombstone）、`tombstone-then-delete`（先发布 tombstone 再删除）。
- `tombstone-then-delete` 必须有 `tombstoneRef`，且必须是 URI（不得含 userinfo）；`retain`/`delete` 不得声明 `tombstoneRef`。
- 归档钉版契约（`reference.pinning.requiredForArchive = true`）禁止 `delete`：进入 Archive 的 ref 删除时必须留下 tombstone。保留期长度本身由域作者按下游最晚行动时间决定，规范不猜测。

**Tombstone 记录**：`pact.ref/v1` 的独立对象，由域作者以正常 `seed`/`produce` 产生，`ThingRef` 指向它；引擎不直接写、不解析、不自动发布。

```yaml
apiVersion: pact.ref/v1
kind: Tombstone
metadata:
  id: urn:pact:tombstone:se/design-review:req-001
  owner: architecture-guild
spec:
  contract: se/design-review@2
  ref: https://artifacts.example.com/design/req-001
  reason: expired                    # expired/replaced/revoked/compromised/legal/manual
  deletedAt: 2026-10-27T00:00:00Z
  retainUntil: 2027-10-27T00:00:00Z  # 可选，必须晚于 deletedAt
  replacedBy: https://artifacts.example.com/design/req-002  # reason=replaced 必填
  note: 保留期结束，按契约删除
```

- `metadata.id`：全局唯一 URI；`spec.ref`：失效的具体引用（URI 或 legacy Identifier）。
- 时间戳只接受 RFC3339 UTC（大写 `Z`），允许 1–9 位小数秒与闰秒；`retainUntil > deletedAt`。
- `note` ≤ 2048 字节、无控制字符，不得含秘密；`replacedBy` 必须与 `ref` 不同。
- 规范文件：`spec/ref/v1/tombstone.schema.json`；Rust 类型：`pact_ref_spec::{Tombstone, TombstoneReason, ...}`。

**静态检查**（`pact ref lint --tombstones`）：Tombstone 解析/规范；重复 `metadata.id`（error）与重复 `(contract, ref)`（warning）；契约存在性（无契约文件时 warning，有契约但查不到 error）；删除策略必须为 `tombstone-then-delete`；`ref`/`replacedBy` 的 scheme 必须在契约允许集合内、长度不超过契约上限；归档钉版契约未声明 `lifecycle` → warning `ref.contract.retention_missing`。

**可发现性检查**（FR29 收尾，同一命令，只对 `required=true`；实例级只在实例显式声明该类型时检查，避免与类型级重复）：

- `ref.discoverability.required_hidden`：effective `visibility=hidden` → Counter/Archive 永不返回票面，下游只能依赖 Rule-R/consume。
- `ref.discoverability.view_read_path_missing`：effective `visibility=read-gate`，但 `.pact` 声明中没有任何参与者对该类型有无条件 `reads`/`consumes` → 视图无法解锁票面。
- `ref.discoverability.no_reader`：声明中没有任何参与者对该类型具备 `reads`/`consumes`（条件子句静态视为“可能成立”）→ 即使视图可见，票面也无法取回。
- `ref.discoverability.retention_missing`：被要求的契约没有 `lifecycle` 且非归档钉版（归档钉版已由 `ref.contract.retention_missing` 覆盖）→ 下游无法判断保留期。
- `ref.discoverability.deletion_without_tombstone`：被要求的契约 `deletionPolicy=delete` → 到期后下游无法区分删除与故障。

`visibility=domain` 时视图直接暴露票面，抑制上述读取路径告警。全部为 warning，`--strict` 视为失败；检查只证明静态路径存在，不保证运行期一定可读。

**边界**：不校验内容是否已删除、tombstone 是否已发布或解析可达；不做保留计时、自动删除、自动 tombstone；不改变引擎、存储或 Archive 语义。这些仍是治理平面/制品域的职责。

### 9.12 ThingRef 入口校验（D45，已实现）

**位置**：`pact-http` 的 `seed_inner` / `produce_inner`，在调用 `engine.seed` / `engine.produce` 之前完成。`pact-engine`、`pact-store`、`pact-core` 均不依赖 `pact-ref-spec`；票面写入后仍是不透明 `ThingRef`，GateExpr / Rule-P / Rule-T 不读取其值。

**请求模型（三态）**：`SeedReq` / `ProduceReq` 的 `thing_ref` 是 `ThingRefField(Option<Option<String>>)`：

| 线上形态 | 内部表示 | 语义 |
|---|---|---|
| 字段缺失 | `ThingRefField(None)` | 未表态 → `400 ThingRefMissing` |
| `"thing_ref": null` | `ThingRefField(Some(None))` | 确认没有外部制品 → `∅` |
| `"thing_ref": "…"` | `ThingRefField(Some(Some(v)))` | 有引用 → 校验后写入 |

`Serialize` 保持 `Some(None)` → `null`，客户端构造请求时总是显式发送该字段，因此 CLI/MCP 的“未提供引用”等价于 `∅` 而非缺失。

**校验顺序**（`VisibilityProfiles::validate_thing_ref`）：

1. 三态判定（上表）。
2. `ReferenceForm::classify_with_limit(value, 65536)`：空/空白、首尾空白、控制字符、裸路径、密码型 userinfo；scheme 合法则规范化为小写，否则按 Identifier 文法校验。
3. 若该域已加载 RefBindings，`RefBindings::resolve(instance, type, form)` 合并 `defaults → scheme → type → instance/type` 并强制 `required` / `allowIdentifier` / `schemes`；`∅` 也走同一解析（`required=true` → `ReferenceRequired`）。
4. 长度：`maxLength`（绑定）缺省 2048 字节；超过 → `TooLong`。硬上限 65536 由 `validate_rule` 保证声明不可越界。
5. 返回规范化字符串（scheme 小写）；`∅` 返回 `None`。

**错误映射**（`server.rs::validate_entry_thing_ref`）：`RefEntryError::Missing` → `ThingRefMissing`；`SpecError::ReferenceRequired` → `ThingRefRequired`；其余规范错误 → `InvalidThingRef`。全部 `400`，走 §6.3 结构化错误体。

**行为变化**：一旦某域加载了 RefBindings，该域默认只接受 URI——旧 Identifier 需要在绑定里显式 `allowIdentifier: true`；未加载绑定的域只做结构规则，Identifier 仍可写入（回归锚点）。

**未覆盖路径**：`pact-wasm` 的 `prepare_seed`/`prepare_produce` 直接构造 `ThingRef`，未接本校验；`pact-runtime` 的进程内 REST 客户端继承本校验。归档钉版、`onUnknownScheme`、`RefRule.value` 见 §9.13（D46）；`declarationDigest` 与 ContractCard 运行期加载仍未生效（方案见 `docs/thingref-governance.md` §13.4）。

**测试**：`pact-ref-spec` 单测（userinfo、Identifier 文法、长度、规范化）；`pact-http` 单测（三态、绑定策略）；`crates/pact-http/tests/visibility_e2e.rs` 端到端（原始 HTTP 缺失字段、显式 `null`、userinfo、裸路径、空格 Identifier、超长、大写 scheme、`required` 拒绝 `∅`、Identifier/scheme/maxLength 拒绝）。

### 9.13 形态、未知 scheme 与钉版（D46，已实现）

**执行点**：

| 策略 | 写入（seed/produce） | 归档（consume） |
|---|---|---|
| `value` | 判定 | 不重跑 |
| `onUnknownScheme` | 判定 | 不重跑 |
| `pinning: required` | 判定 | 兜底判定 |
| `pinning: required-at-archive` | 放行 | 判定 |

**`onUnknownScheme`**：`schemes` 非空且不含 `*` 时，scheme 不在白名单即“未知”。`reject`（缺省）由 `RefBindings::resolve` 直接拒绝；`allow` 跳过白名单；`treat-as-identifier` 用 `classify_identifier` 重新分类，并要求 `allowIdentifier`。白名单为空/含 `*` 表示不限，不触发策略。

**`value`**：对非空票面判定 `none` / `uri` / `identifier`；`∅` 只由 `required` 决定。`required=true` 与 `value=none` 在 `RefBindings::validate` 阶段即报错（配置矛盾不进入运行期）。

**钉版判定**：`pact-ref-spec::pinned_digest` 接受 `sha256:<64 位小写 hex>` 与 `sha256=<64 位小写 hex>`，即 OCI `@sha256:…` 与 URL `#sha256=…` 两种形态；大写十六进制、长度不足或后接额外十六进制字符均不算钉版。`∅` 豁免，Identifier 不算钉版。

**归档检查**：`consume_inner` 在 `authorize_action` 之后、引擎调用之前执行 `enforce_archive_pinning`：若该域加载了 RefBindings，则从 `engine.counter` 找到待消费 token，取 `thing_ref` 与类型，调用 `validate_archive_thing_ref`。该方法先用 `pinned_digest` 快速放行，再只用 `RefBindings::merged_rule` 读取继承出的 `pinning`，**不**重跑 `required`/`schemes`/`value`。token 不存在时不拦截，交给引擎返回原有的 not-found / 幂等结果。

**错误码**：`pinning` 失败 → `400 ThingRefUnpinned`（`message` 区分写入/归档）；`value`/`onUnknownScheme` 失败 → `400 InvalidThingRef`；配置矛盾 → daemon 启动失败（`RefBindings` 校验）。

**边界**：ContractCard 的运行期加载与归档钉版并集由 §9.14（D47）承接，显式重载由 §9.15（D48）承接，均已在 REST 边界实现；`artifact.kind`/`mediaType`、`lifecycle` 运行期检查不在本层。`pact-wasm` 本地宿主不在 REST 边界内。

**测试**：`pact-ref-spec`（`pinned_digest`、未知 scheme 三分支、`merged_rule` 跳过入口校验、`required+value=none` 拒绝）；`pact-http` 单测（形态、未知 scheme、写入/归档钉版）；`visibility_e2e` 端到端（`ThingRefUnpinned` 写入与 consume、钉版引用成功归档）。

### 9.14 契约卡加载与归档钉版并集（D47，已实现）

**配置与加载**：daemon 新增可选 `[ref_contracts].paths`（文件或目录，可重复；目录展开 `.yaml`/`.yml`/`.json`，规则同 `[ref_bindings]`）。`VisibilityProfiles::load_with_contracts` 在启动时读取卡片、`ContractCard::validate` 后按 `contract_id()`（`metadata.id@version`）建索引；同一 `contract_id()` 出现两次即启动失败，即使内容相同——卡片变更必须提升 `metadata.version`。

**按数据触发闭环**：加载完成绑定后，扫描原始规则（`defaults` / `types` / `schemes` / `instances`）中出现的 `contract` 引用。引用非空但未配置 `[ref_contracts]` ⇒ `VisibilityError::ContractsNotConfigured`；配置了但引用的契约缺失 ⇒ `VisibilityError::MissingContract`；两者都拒绝启动。没有 `contract` 引用的部署无需配置，行为不变。

**归档判定**：`validate_archive_thing_ref` 在绑定继承出的 `pinning` 之外，读取 `merged_rule.contract` 指向的卡片：`reference.pinning.requiredForArchive=true` 或 `artifact.digestPolicy=required-at-archive` 时要求钉版。有效要求是“绑定 ∪ 卡片”，卡片不能放宽绑定；判定仍只在 consume、仍只读 `thing_ref`，不重跑入口策略。`∅` 豁免，已钉版快速放行。若绑定引用的契约在索引中缺失（启动/重载校验被绕过），返回 `InvalidThingRef`，fail-closed。

**错误面**：启动阶段使用 `VisibilityError` 的 `ContractIo` / `ContractParse` / `ContractInvalid` / `DuplicateContract` / `ContractsNotConfigured` / `MissingContract`，信息指名文件与契约；运行期仍映射为 `400 ThingRefUnpinned` / `400 InvalidThingRef`。

**未生效**：`artifact.kind`/`mediaType`（票面不携带 media type）、`lifecycle` 运行期计时/删除。显式重载见 §9.15（D48）。

**测试**：`pact-http` 单测（卡片加载与引用闭环、无引用时不要求配置、缺失/重复拒绝、卡片钉版阻断归档、卡片不放宽绑定、索引缺失 fail-closed）；`visibility_e2e`（卡片要求归档钉版时 `ThingRefUnpinned`、钉版可归档、重复版本拒绝启动）；`config` 单测（`[ref_contracts]` 空 paths 拒绝）。

### 9.15 契约卡与绑定的显式重载（D48，已实现）

**入口与配置注入**：daemon 启动时在 `ServerState` 注入 `RefReloadConfig { binding_paths, contract_paths, audit_path }`（`audit_path = <data_dir>/ref-reload.jsonl`）；未注入则 `POST /ref/reload` 返回 `500 RefReloadUnavailable`，不静默成功。验证入口是 `POST /ref/reload`（`{"dryRun": bool}`）与 CLI `pact ref reload [--dry-run] [--json]`；不提供 MCP 工具。

**事务顺序**（固定，任一步失败保持旧快照）：认证（`control_principal`）→ 当前索引域 ACL（每域 `{domain}/*`）→ 重新读取并校验全部绑定与卡片（含 `contract_id()` 索引与引用闭环）→ 新索引域 ACL → 影响预检 → 控制面审计 → 原子替换。`dryRun=true` 在审计后停止，`applied=false`，响应仍返回完整计数与阻断报告。

**原子快照**：索引持有 `RwLock<Arc<VisibilityProfiles>>`；请求入口取一次 `Arc` 快照并在整个处理期间复用，替换只换 `Arc` 指针。在途请求持旧快照完成，新请求读新索引；绑定与卡片永远同批生效，不出现“绑定引用未加载版本”的中间态。

**审计摘要同源**：`VisibilityProfiles` 在加载时记录每个实际读取文件的 `path`、`sha256` 与 `kind`（`bindings`/`contract`），审计直接写该清单，避免重载后二次读取导致“审计摘要 ≠ 生效内容”的 TOCTOU。审计为 append+fsync；写失败一律 `500 RefReloadAuditFailed` 且不替换索引（包括 ACL/校验拒绝路径）。认证失败（`401`）与并发冲突（`409`）不写审计，因为前者无可信操作者、后者未进入事务。

**影响预检**：遍历全部域实例的 Counter token，只有“旧规则归档判定通过、新规则因 `PinningRequired { at_archive: true }` 阻断”的 token 计入新增影响；按 `(domain, instance, type)` 聚合计数，不输出票面值。旧规则已阻断的存量不计入本次变更。预检引擎错误 → `500 RefReloadFailed` 并写 `rejected` 审计。

**错误面**：`400 RefReloadInvalid`（解析/校验/重复 `contract_id()`/引用不可解析，信息含文件）、`409 RefReloadBusy`（`try_lock` 失败）、`403`（ACL 未覆盖当前或新域）、`401`（配置身份后未认证）、`500 RefReloadUnavailable` / `RefReloadFailed` / `RefReloadAuditFailed`。审计 `outcome` 只有 `applied`（含 dryRun 的“将会生效”）与 `rejected`。

**测试**：`pact-http` 单测（审计 JSONL 与 fsync、审计目标为目录时失败、锁被占时 `409`）；`ref_reload_e2e`（dryRun 报告与确认后生效、非法绑定保持旧快照、审计失败保持旧快照、令牌模式匿名 `401`、信任模式匿名放行、ACL 覆盖当前与新域、未注入配置 `500`）；CLI e2e（真实二进制 `pact ref reload --dry-run` → 应用 → 新规则阻断写入）。

## 10. 测试策略（FR13，四层）

| 层次 | 范围 | 位置 |
|------|------|------|
| 单元测试 | core 规则函数/GateExpr 求值/validate_domain；parser 文法与错误报告；analysis/compose；store 读写；**读审计 schema（D43 旧/新记录兼容、视图计数往返）**；**D44 retention 语法、生命周期策略组合、Tombstone 字段与时间戳校验、可发现性五类 warning code**；**D45 URI profile / Identifier 文法 / 三态 / 绑定策略**；**D46 未知 scheme 三分支 / value 形态 / 钉版判定（写入与归档）**；**D47 卡片加载/索引/闭环；D48 审计 JSONL、审计失败与并发单飞** | 各 crate `#[cfg(test)]` |
| 属性测试 | Rule-R 后 Counter 逐字节不变；Rule-P 后前驱仍在 Counter；Rule-E 只影响目标 token 与 Archive；GateExpr 求值不改状态（纯度）；apply+重放等价 | pact-core proptest |
| 集成测试 | 引擎命令流（多域、并发命令串行化）、store 崩溃恢复（JSONL 半行截断、索引重建）、订阅通知时序、**Rule-E 幂等（request_id 去重与跨重启生效）**、**D43 视图审计（开关、按类型计数、写失败 fail-closed）**、**D45 入口校验（原始 HTTP 缺失字段、显式 `null`、绑定策略拒绝）** | crates/tests |
| 端到端 + 回归 | 黄金轨迹逐事件比对：软件工程生命周期 t₀–t₁₀、医疗路径 t₁–t₁₃（via 全部通过点）、ReAct 循环（循环拓扑 + Counter O(1)）、**§4.2 区分性场景 State A/B**（via 溯源敏感性回归锚点）；MCP 与 REST 契约等价；无人值守全流程；**Counter/Archive 视图审计（D43：按类型暴露计数、匿名、写失败 500）**；**`pact ref lint --tombstones` 目录展开、JSON/人类输出与退出码（D44）**；**可发现性告警在真实二进制上的 JSON 输出与 `--strict` 退出码**；**D47 卡片钉版归档与重复版本拒绝**；**D48 显式重载（dryRun 报告、失败保持旧状态、审计/ACL/认证）与真实二进制 `pact ref reload`** | 阶段 4/5 的 e2e 测试 |
| v2 接口安全 | TLS/mTLS 握手与自定义 CA、Ed25519 签名正/负（请求体篡改、nonce 重放、参与者不匹配）、`/mcp` 静态 Bearer/信任模式、OIDC 正/负（iss/aud/exp/nbf/alg/crit/claim 映射/未知 kid）、RFC 9728 元数据、IdP 不可达 503、内部能力令牌不泄露 | `crates/pact-http/tests/nfr6_v2.rs`（含本地 TLS/mTLS 与 mock IdP） |
| WASM 门禁 | `wasm32-unknown-unknown` 构建 + wasm 目标 clippy + ABI 版本断言 + 最小语义冒烟（validate/analyze）+ 体积上限（raw ≤ 600 KiB、gzip ≤ 200 KiB）+ Node 持久化测试 | `.github/workflows/ci.yml` 的 `wasm` job；脚本 `crates/pact-wasm/js/ci-smoke.js` |
| 依赖安全审计 | `cargo audit --file Cargo.lock`：RustSec 漏洞（vulnerability）即失败；unmaintained/unsound 仅告警，保持可见不阻断；`cargo-audit` 版本固定并缓存 | `.github/workflows/ci.yml` 的 `audit` job |

## 11. 实现阶段

| 阶段 | 内容 | 验收 | 状态 |
|------|------|------|------|
| 1 | pact-core + pact-parser + 黄金轨迹/属性测试 | 论文全部示例轨迹通过；core 纯度约束清单（§2.1）逐条满足 | **已完成（2026-08-25，2026-08-26 按 rev1 更新）**：27 项测试全绿（11 单元含源类型约束专项 + 2 属性 × 512 用例 + 5 黄金 + 9 parser）；纯度核验（依赖树仅 serde、禁用 API 扫描零命中）；D15 已回滚、D17 生效（论文 rev1 源生产约束）；已知缺口：容器内无 rustfmt/clippy 组件（无 rustup/sudo），待环境补齐后补跑 |
| 2 | pact-analysis + pact-compose | 六项分析对论文域输出正确；组合一致性冲突可检出 | **已完成（2026-08-26）**：20 项测试全绿（analysis 12：四论文域验证 + 死锁/馈入/不可达/循环保守/间接祖先/嵌套 via 用例；compose 8：论文 §1.8 示例逐字段比对 + 四条件冲突各型 + 单位元/交换律/幂等合并）；D18 落定（C2 实践语义） |
| 3 | pact-store + pact-engine | 重放等价、崩溃恢复、多域会话 | **已完成（2026-08-26；2026-10-18 按 D23 补充消耗提交契约）**：新增 31 项测试全绿——store 18（事件 JSON 往返、**Consumed 自足（归档内容随事件携带；目标缺失 fail-closed）**、**幂等键序列化**、seq 逐行 +1、域名校验、索引滞后重建、快照原子写重启存活、日志清洗）；engine 单元 4（ISO-8601 纪元含闰日、逻辑时钟单调、顺序/ULID id）；集成 11（全生命周期 + 通知时序 seq/规模断言、快照路径与纯重放路径状态逐字段等价、崩溃半行截断后可续作、索引丢失重建、中部损坏 fail-fast、多域隔离、8 并发 seed 串行化、域替换恢复、validate/analyze/compose 命令、Clock/IdGen 确定性注入、**Rule-E request_id 幂等与跨重启生效**）。纯度核验：pact-core 依赖树仅 serde，IO/时钟/随机 API 扫描零命中。D23 落定（提交点 = 日志 fsync；索引投影尽力而为；Consumed 自足；Rule-E 可选幂等） |
| 4 | pact-http + pact-cli + pact-mcp（stdio）（D19：pact-http 提前） | 人类与 agent 均可走完生命周期 | **已完成（2026-09-04）**：pact-http（axum daemon：unix socket 默认/TCP 可选、§6.3 类型化错误体、SSE 通知流、类型化 client、pactd bin）+ pact-mcp（rmcp stdio，17 个 `pact_` 工具，经 REST 接入，schemars 派生 schema）+ pact-cli（单二进制 `pact`：daemon start + 在线命令经 client + 离线 analyze/compose/replay + `--json`）。e2e 三绿：HTTP daemon 全生命周期（含 404/错误码映射、SSE、compose/unload）；CLI 真实子进程 daemon 全生命周期 + 错误路径 + 离线命令 + 事件日志回放对照；MCP 真实 stdio 子进程 15 工具全生命周期 + 3 错误路径。workspace 累计 95 项测试全绿。D19/D20 落定 |
| 5 | 无人值守场景验证（pact-http 已提前至阶段 4 交付） | 多 agent 经 MCP 自驱完成 ReAct/软件工程域，全程无介入至 Terminated；含 MCP Streamable HTTP 与否由联网演进决定（D20：v1 不做） | **已完成（2026-09-05；2026-09-09 修正场景 B 卫生断言）**：`pact-mcp/tests/unattended_e2e.rs` 单测试内顺序验证两场景，测试进程只做"外部启动机制"（拉起 daemon + N 个真实 `pact-mcp` stdio 子进程），此后零 Rule-S/P/R/E 介入。场景 A（ReAct）：user 播种 + agent 私有生产预算 7，自驱两轮循环至 Terminated，7 产/8 消/7 读、Counter 恒 ≤2（单 agent 串行，真不变量）、archive 序两轮同构；场景 B（软件工程生命周期）：external_stakeholder 播种 + 5 worker 并发自驱接力至 Terminated，4 产/5 消/4 读、角色分工与溯源链 requirement→…→deployment 逐项核验、Counter ≤ 类型数 5（结构上界；典型调度 ≤2，但 CI 慢调度下下一阶段可在上一阶段 produce→consume 窗口内合法生产，窗口因 produces 门无条件子句而语义上不可消除，2026-09-09 CI 实测 max_counter=3 后断言修正为结构上界）。agent 自驱策略为生产优先 / 汇消费 / 等待三档（订阅推送 + 轮询兜底，Rule-W）。修复两类自驱竞态：①播种前过早终止（域加载与种子注入间 Counter=∅ 使 `termination_holds` 平凡为真，worker 误判退出）→ 仅当 `archive_size>0` 才承认终止（符合 §"Terminated 信息性、不锁定域"，修复落在 agent 驱动层非引擎）；②Terminated 一次性推送与轮询判定竞态 → 终止后 500ms 宽限窗口排空在途推送。合并为单 runtime 顺序执行以规避 tokio 多 runtime 子进程回收陷阱。D20 生效（v1 不做 Streamable HTTP；v2 已由 D35 增加 `/mcp`，stdio 薄封装保留） |
| 6（可选，已实现） | pact-cpn：CPN 编码生成（正片段直出，not 条件走互补库所编码）；**按实例独立编码**（论文 §4.5/§B.2 单实例作用域：每实例各自编码，库所/迁移不跨实例共享，跨实例协同不入 `N(D)`；源域为域声明、实例划分属运行期，见 D12/D27） | CPN Tools 可加载验证；单实例作用域符合 §4.5/§B.2 | **已完成（2026-10-21，D28）**：新增 crate `pact-cpn`（`generate(&Domain, &CpnOptions) -> CpnModel`）+ CLI `pact cpn`；每类型库所 + Archive、`PACT_TOKEN` 五元颜色、生产前驱 test arc、读自环、消费归档、种子无输入弧；`GateExpr → NNF → DNF`（or 多迁移、not 互补库所 + `--bound`、not via 显式拒绝、恒假 guard=false、DNF 上限 1024）；实例命名空间 `i_<instance>__`；输出 CPN Tools XML format 6。验证：11 项 crate 测试 + CLI 离线 e2e（结构/错误路径/实例前缀），全量 **166 项**测试全绿；`examples/domains/*.pact` 生成物全部通过 CPN DTD 校验（xmllint）。**限制**：沙箱无 CPN Tools GUI，实际图形加载需人工验收；`--bound` 正确性由调用方承担（运行期并发上界不可从声明推导） |
| 重构（2026-09-09） | 论文 rev2 对齐：良构约束六项化（新增 LHS Uniqueness / Non-Empty LHS / PosNat 解析期与声明期强制）、A.5 重构为 precondition_wf 三元报告（D21）、DomainError 自有 String | rev2 专项测试（`{A+A}→B` / `{ }→B` / count 0 / 前导零拒绝、自环接受、EmptyPrecondLhs / ZeroCountLiteral / SeedProduceOverlap 用例）+ 三接口 e2e 断言更新全绿 | **已完成**：workspace 103 项测试全绿（净增 6 项）；fmt/clippy 清零 |
| 7 | pact-runtime：参与者驱动运行时（D24，设备生产化） | 可复用自驱循环：给定参与者名 + 门，实现生产优先 / 汇消费 / 等待三档策略并驱动至 Terminated；多 agent 编排可拉起 N 个真实 MCP/HTTP 客户端；外部副作用执行器消费 `Consumed` 并追加补偿事实 | **已完成（2026-10-19）**：新增 crate `pact-runtime`（lib + bin `pact-runtime`）——传输层抽象（`ParticipantClient` trait / `RestClient` 包装 `pact_http::client` / `McpClient` rmcp stdio 子进程）+ 驱动循环 `drive()`（生产优先 / 汇消费 / 等待三档；订阅推送 + 轮询兜底；`pristine` 守卫 + 500ms 终止宽限，与 `unattended_e2e` 逐条同构）+ 编排 `Orchestrator`（每 agent `tokio::spawn`，REST/MCP 两具体 client 装箱统一、不引入 dyn）+ 执行器 `SideEffectExecutor`（archive 游标增量、失败以 Rule-S 追加补偿事实）+ TOML 配置驱动 bin。集成 1 项（单 `#[tokio::test]` 内 in-process daemon 上顺序跑 REST 编排 SE 生命周期 / ReAct 循环 / 副作用执行器）+ 单元 4 项；workspace **110 项**测试全绿（基线 105 + 本轮 5），fmt/clippy 清零。修复两处（见 design-change 第 17 轮）：①执行器与编排并发启动下域未就绪 → 执行器增域发现轮询；②驱动层的生产优先策略需 `read` 产出见证前驱 → 测试域 `worker` 补 `reads task` 门 |
| 8 | 引擎生产化三项（D25） | 事件日志压缩与 schema 版本化；health/ready + metrics + PID 文件 + 信号化优雅停机；域级 ACL + 可选审计读视图（默认关闭）。**验收项与 D27 实例化耦合**：(1) 的压缩部分与 (3) 全部须按双键重述，`:compact` 路由须随实例段重设计 | **子项 (2) 运维接口已完成（2026-10-20，见 §9.9）**：`/health` `/ready` `/metrics`（Prometheus 文本）+ 可选 `pid_file`（陈旧检测）+ SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界 10s 等待在途 → 终态快照）；workspace **131 项**测试全绿（基线 123 + 本轮 8）。**子项 (1) 日志压缩与 schema 版本化已完成（2026-10-20，见 §7.5）**：`WireEvent` 顶层 `version`（+ `Notification` 顶层 `version`）+ 旧日志/快照缺省按 v1 兼容、更高版本 fail-fast；前缀压缩（全量自足快照 → `logmeta` 水位 → 原子重写 → 裁剪索引）、快照承载 `declaration`/`dl_seq`/`consumed_requests`（幂等表随快照存活）、恢复 `snapshot_is_anchor` 分支与 `SnapshotRequiredForCompactedLog` 护栏；REST `POST /domains/{d}[/instances/{i}]/compact`、CLI `pact compact`、MCP `pact_compact`、`pact.toml` 可选 `compact_on_start`/`compact_keep_last`；workspace **145 项**测试全绿（基线 131 + 本轮 14）。**子项 (3) 域级 ACL 与审计读视图 已完成（2026-10-20，见 §9.10）**：`[auth.acl]` 实例级条目（`domain/instance` / `domain/*`）覆盖生命周期、四类动作、查询、compact 与读审计视图，`GET /domains` 按授权过滤；ACL 非空而令牌表与密钥表均为空 ⇒ 拒绝启动；可选 `read_audit = true` 写独立审计流（`<instance>.read-audit.jsonl`，append+fsync，fail-closed）并提供 `GET .../read-audit?limit=` 只读视图；workspace **155 项**测试全绿（基线 145 + 本轮 10） |
| 9 | 分发产物 `dist/`（D26） | Dockerfile、systemd unit、musl 静态二进制打包脚本，可构建可运行 | **已完成（2026-10-20）**：`dist/Dockerfile`（多阶段 `rust:1.98-alpine` 构建 musl 静态四 bin → `alpine:3.22` 运行；非 root uid 10001、`/var/lib/pact` 数据卷、`/health` HEALTHCHECK、入口 `pactd --config /etc/pact/pact.toml`）+ `Dockerfile.dockerignore`（构建上下文排除 `docs/pact*`、examples、target 等）；`dist/pactd.service`（`User=pact`、`StateDirectory`/`RuntimeDirectory`、SIGTERM 优雅停机、`ProtectSystem=strict` 与空 capability 集等加固）；`dist/build-static.sh`（`x86_64-unknown-linux-musl`、`--locked --bins`、strip + `readelf` 静态链接核验、tar + `SHA256SUMS` 复现打包）；配置示例 `dist/pact.toml.example`（systemd）与 `dist/pact.container.toml`（容器）。离线校验：脚本语法、两份 TOML 解析、systemd unit 语义校验；按 D26 不接入 CI 语义测试。**注**：当前开发沙箱无 musl C 工具链与容器运行时，静态打包与镜像构建仅做到脚本/配置层面校验；实际构建需在装有 `musl-gcc` 的主机或 Docker 中执行 |
| 10 | 域双键实例化（D27，路径 β） | 定义键/实例键落地：存储 `(domain, instance)` 双列 + 域子目录布局 + 引擎声明注册表（`declare` / `instantiate` 分离，实例创建时快照声明）+ 接口全链（REST / CLI / MCP / `pact-runtime`）+ 默认实例 `default` + 旧日志 fail-fast + 通用护栏（`data_dir` flock 独占锁、非空实例 load 默认拒绝）；阶段 8 的耦合项随之一并完成 | **已完成（2026-10-20，路径 β）**：`pact-store` `InstanceKey{domain,instance}` + 域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、声明原文 `declaration.pact`）+ 索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键，`open()` 自愈重建）；`pact-engine` 声明注册表（`declarations` / `instances` 两表，`declare` / `instantiate` 私有方法 + 公开命令 `LoadDomain` / `Instantiate`，`InstanceSession.is_occupied = last_seq > dl_seq`）——错误码 `InstanceNotFound` / `DomainNotDeclared` / `InstanceExists` / `DomainBusy`；接口全链：REST 默认族 + 实例镜像族（`/domains/{d}/instances/{i}/…`）+ `InstantiateReq` + `DomainListing`，CLI `--instance`/`--force`/`instantiate`/`instances`，MCP 16 工具（新增 `pact_domain_instantiate`），`pact-runtime` 实例透传（`ParticipantClient::instance` + `bound_ready`）；护栏 `flock`（`fs4`，`<data_dir>/pactd.lock`）+ `DomainBusy`；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**（形式语义不变）。workspace **123 项**测试全绿（基线 110 + β 专项），fmt/clippy 清零 |
| 11 | DSH Agent Loop 集成（D37，独立仓库 `pact-dsh`，不改 Rust） | 交付 `dsh-plugin-pact`：observe 镜像、govern 门控、`pact-*` preset 选择性工厂接管、可注入后端与假实现；非 pact agent 行为与官方一致；Node 单测全绿 | **已完成（2026-10-22/23）**：独立仓库 11 个运行时文件 + 7 个测试文件；`node --test` **47 项**通过、`node --check` 全部通过；7 类测试覆盖配置/命名/假后端/WASM 适配/观察者/治理/路由。**真实装配已验证**（`@deepseek-ai/dsh@0.2.1-rc.2`）：profile 安装、`--dump-config` 组合、运行时 `ctx.agentLoop` 为 `PactAgentLoop`。**真实模型门控已验证**（`deepseek-official/deepseek-flash`）：允许 / `denyTools` 拒绝 / `fakeEnabled=false` 预检拒绝。**WASM 后端已 live（2026-10-23）**：`wasm-bindgen 0.2.127` 产物 + `open-host.js` 组装，在 headless profile 中复跑三条路径均通过 |


## 12. 联网演进路径（NFR6）

v2 的变化被限制在接口层：TLS 终结、真实参与者凭据（Ed25519 签名替代/补充 Bearer）、MCP Streamable HTTP 远程接入、MCP OIDC 资源服务器。引擎、存储、核心语义零改动——因为 Counter 单权威不因联网改变（client/server，无共识）。turso 的 MVCC 并发写能力为 v2 多客户端并发查询提供余量。

**v2 已实现（2026-10-22，D34–D36）**：

- **传输**：daemon 在同一 `listen_tcp` 上可选 TLS 1.2/1.3 与 mTLS（§9.5）；unix socket 仍是本地默认，不受 TLS 配置影响。
- **REST 身份**：静态 Bearer 保留；新增 Ed25519 请求签名（§9.6）；启用签名后非豁免 REST 请求必须签名（Bearer 不能替代），并与 ACL 共用同一参与者身份位。失败语义 401/403/413/429。
- **MCP**：daemon 内建 `/mcp`（§9.7），OIDC 资源服务器（§9.8）或静态 Bearer；`pact-mcp` 的 stdio 形态保留为薄封装，面向本地宿主与 `pact-runtime`。
- **外部依赖**：仅在启用 OIDC 时新增 IdP 与 JWKS 依赖；REST、CLI、stdio MCP 不访问 IdP，IdP 故障不影响它们。
- **不在 v2**：daemon 内授权服务器/授权码 + PKCE、OIDC 令牌向 REST 透传、多租户配额/WAF、内置客户端 mTLS 证书管理。

**MCP 提供形态（2026-10-19 确认，D24；2026-10-22 按 D35 更新）**：stdio 与 Streamable HTTP 并存。`pact-mcp` 是**传输适配器/客户端**而非权威，权威恒为 daemon；它随本仓库交付（独立 bin），由 agent 宿主在 MCP 配置中拉起（`command=pact-mcp, args=--unix …`）。**不把 MCP server 拆为独立项目**——它与线上格式/事件 schema 强耦合；HTTP 形态直接复用 `pact-http::mcp` 的实现。

## 13. 生产运行层与生产化边界（D24–D26，2026-10-19 裁决）

生产化被拆为三层归属（详见 `docs/architecture.md` §6）：

- **范式引擎层（本仓库 `crates/`）**：领域无关的协调机制，即现有一切（core/parser/analysis/compose/store/engine + 三接口）。生产化增强（日志压缩、运维接口、域 ACL）仍落于此（阶段 8）。
- **领域适配层（独立仓库 `~/projects/skills`）**：RE/SE 技能的 `.pact` 域与内容层（`docs/skill-adaptation.md` §1 已定案），不属本仓库。
- **生产运行层（本仓库 crate `pact-runtime`，阶段 7，**已实现**）**：引擎保持被动（D5），agent 必须自驱。本层提供可复用的参与者驱动循环、多 agent 编排、外部副作用执行器，经 MCP（stdio）或 REST 接入 daemon。驱动逻辑已收于 `pact-runtime`（lib + bin）；`pact-mcp/tests/unattended_e2e.rs` 保留为经真实 MCP 子进程验证引擎自驱场景的回归测试，不再是唯一驱动实现。

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

**安全边界**：容器与 systemd 均以非 root 运行且数据目录 0700；容器默认只暴露 8787 且文档要求 `-p 127.0.0.1:8787:8787`。v2 起可选用 daemon 内 TLS/mTLS（§9.5）与 Ed25519 签名/静态令牌（§9.6）；若仍由外部反向代理终结 TLS，则后端应只监听回环并启用 `[auth.tokens]` / `[auth.keys]` / `[auth.acl]`（§9.4/§9.10）。内置客户端不支持 mTLS 客户端证书，需要 mTLS 的部署须由外部客户端出示证书。

**非目标**：不推送镜像仓库、不提供 k8s/Compose 编排模板、不做自动更新或打包签名；这些属外部发行细节。

## 15. pact-cpn：CPN 编码生成（阶段 6，D28）

### 15.1 边界与入口

- `pact-cpn` 只依赖 `pact-core`（`Domain` / `GateExpr`），**不依赖** parser/store/engine/http；纯函数 `generate(&Domain, &CpnOptions) -> Result<CpnModel, CpnError>`，无 IO。
- CLI 离线命令：`pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；`FILE` 由 `pact-parser` 解析并校验，`--json` 输出 `CpnStats`。
- 一次生成只编码**一个 Counter 实例**；库所/迁移名统一前缀 `i_<instance>__`，页面名 `PACT_<instance>`。不同实例分别生成、互不共享（§4.5/§B.2 Scope，D12/D27）。
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

## 16. WASM 本地分发形态（D31/D33）

### 16.1 定位与权威边界

- `pact-wasm` 是 `pactd` 之外的**受支持本地分发形态**，嵌入 Node/桌面宿主进程；**原生 daemon 仍是 Counter 的唯一权威**，两者不并存于同一实例（同一实例由宿主 lockfile 单写者约束）。
- 2026-10-21（D33）起去除“实验性”标记：四项稳定化验收（W-Q7 CI、金标扩面、kernel 抽取、宿主边界安全审视）均已通过，WASM 形态可被 Node/桌面宿主依赖；该项变更不扩大能力范围。
- NFR4 的单一静态二进制与 FR18 的 `dist/` 分发产物不变；WASM 不替代 `pactd`、不提供 MCP/HTTP 服务、不承担多用户与远端接入，也不进入 `dist/`。
- 安全边界为**本地单用户**：WASM 内核不做身份认证、ACL 与读审计；这些能力只在原生 daemon（FR17）。宿主不得把 WASM 形态暴露为多用户服务。

### 16.2 组件与宿主职责

| 层 | 责任 | 已交付 |
|---|---|---|
| `pact-engine-kernel`（Rust，纯） | 实例会话语义：声明快照、Counter 状态、seq 分配、Rule-S/P/R/E 准备、Rule-T 终止判定、Rule-E 幂等表、恢复锚点、查询视图；无 IO/tokio | D32 |
| `pact-wasm`（Rust，wasm32） | 声明注册表、`(domain, instance)` 会话（嵌入 kernel `Session`）、两阶段提交、JSONL/快照导出与 `restore`、ABI 包装 | W-S2 |
| JS 宿主适配层（`crates/pact-wasm/js/`） | JSONL 追加 + fsync、末尾半行截断、快照 tmp+rename+目录 fsync、`.lock` 单写者、日志优先恢复、`prepare → append → commit` 编排 | W-S2（Node） |
| 原生 `pact-store` / `pact-engine` | 真相源布局、事件 schema、快照结构、索引、压缩、ACL/审计；`pact-engine` 会话同样嵌入 kernel `Session` | 不变 |

- 磁盘布局 `<root>/<domain>/<instance>.{events.jsonl,snapshot.json,lock}` 与原生 `<data_dir>/<domain>/<instance>.*` 同构，原生 daemon 与 WASM 宿主已验证可**交替打开同一日志**。
- 宿主 lockfile 是**进程级近似**单写者，不等价于原生 `flock`；不承诺多进程/多 tab 的内核级互斥。

### 16.3 ABI 与写协议

- ABI 为 `pact-wasm/0.x`，全部方法以 JSON 字符串收发；`abi_version` 为版本探测入口。破坏性变更须提升版本号并记入 `docs/design-change.md`。
- 唯一写路径为 `prepare_* → 宿主持久化（append + fsync）→ commit`；持久化失败调用 `abort` 回收未占用 seq，保证日志 seq 连续（原生 `Store` 的 `seq == last+1` 校验可直接接受该日志）。
- `commit` 返回的 Rule-T `Terminated` 事件由宿主按原生顺序追加；该追加为派生动作，失败不回滚已提交状态（与 D23 一致）。
- `token_id` 与 `ts` 由宿主注入（ULID/ISO-8601），内核不读系统时钟、不依赖 `getrandom`，保证与原生金标逐行对账可复现。

### 16.4 稳定化验收与不纳入项

- 稳定化验收（**全部完成，2026-10-21**）：W-Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）→ 金标扩面（多实例、组合域、条件分支、压缩后恢复）→ 抽取 `pact-engine-kernel` 消除双实现 → 宿主边界安全审视。四项通过后按 D33 去除“实验性”标记。
- 维护约束：`wasm` CI job 持续执行 wasm32 clippy/build、bindgen、`ci-smoke` 与 Node 7 项测试；任何语义改动必须同时通过原生与 WASM 金标对账。
- 不纳入：浏览器 OPFS/SharedWorker（W-Q6；兼容性评估已完成，未实现）、多用户 ACL/读审计、turso 索引与压缩、NFR6 v2 远端模式。DSH `govern` 已由阶段 11 的仓库内插件实现（FR25/D37），但不由 WASM 内核承担，也不改变本节边界。触发条件见 `docs/wasm-local-spike-feasibility.md` §11.2。

## 17. DSH 插件（阶段 11，D37）

### 17.1 定位与边界

- 插件包 `dsh-plugin-pact` 是**独立仓库**（`onlookor/pact-dsh`）中的 ESM 包，不进入 Cargo workspace、不进入 `dist/`、不修改任何 Rust crate；2026-10-23 自本仓库迁出。
- PACT 只接管**控制流**：决定每一步是否允许开始、每次工具调用是否允许执行；不接管模型调用、流式输出、工具执行、压缩、重试与会话持久化。
- 仅 `agentPreset` 以 `pact-` 开头的 agent 受治理；其余 agent 原样委托官方 loop，行为不变。
- 权威边界：插件是宿主侧执行器，Counter 的权威仍是本地 PACT 实现（WASM 或原生 daemon）；插件不复制状态机。

### 17.2 包布局

以下文件均位于独立仓库 `onlookor/pact-dsh` 的仓库根目录。

| 文件 | 责任 |
|---|---|
| `package.json` | `dsh-plugin-pact`、ESM、零运行时依赖、`dsh.bundle.patch` 指向 patch；`@deepseek-ai/dsh-agent-loop` 为 peer |
| `cordis.patch.yml` | 禁用原始 `agent-loop` 行；插入 `pact-agent-loop`（同一 `agentLoop` 服务）与 `dsh-pact-observe` 行 |
| `loop.js` | 顶层入口：读取 `PACT_DSH_CONFIG`、构造后端、`createPactLoop(AgentLoop)` 默认导出 |
| `index.js` | host 插件 `apply(ctx)`：注册 `session/event` 审计镜像 |
| `src/config.js` | 默认值、`PACT_DSH_CONFIG`（文件或内联 JSON）、校验 |
| `src/names.js` | preset 判定、session→实例键消毒、participant 解析 |
| `src/pact-backend.js` | `PactBackend` 契约、`PactBackendError`、`FakePactBackend` |
| `src/wasm-backend.js` | 惰性 `openPactHost(options)` 适配，缺失时 `BackendUnavailable`；`enabled` 收窄为布尔 |
| `src/observer.js` | 有界环形缓冲、字段裁剪、JSONL `FileSink`（`0700`/`0600`） |
| `src/govern.js` | 四个钩子处理器与失败策略 |
| `src/loop.js` | `createPactLoop(Base, deps)`：路由、setup 组合、resume 事后安装 |

宿主侧组装入口位于 `crates/pact-wasm/js/open-host.js`（CommonJS）：把 wasm-bindgen 产物、`FileStore` 与 `(domain, instance)` 组装为 `PactHost`，插件按文件路径动态 import 后调用 `openPactHost`。

### 17.3 配置

- 唯一入口 `PACT_DSH_CONFIG`：文件路径或内联 JSON；缺省 `{mode:'observe', backend:'fake', domain:'dsh', defaultParticipant:'agent', presetPrefix:'pact-', mirrorLimit:256, denyTools:[], stopOnTermination:false, failClosed:true, fakeEnabled:true}`。
- `mode=govern` 时 `failClosed` 强制 `true`；`backend=wasm` 时 `wasmModule` 必填。
- `wasmOptions`（可选纯对象）原样透传给 `openPactHost`，用于提供 `module`（wasm-bindgen out-dir）、`storeRoot`、`declaration`、`ts` 等宿主参数；插件只校验其为对象，键由宿主模块自校验。
- `fakeEnabled=false` 让 `FakePactBackend` 对所有参与者返回未启用，用于在无 PACT 引擎时预演 `agent/pre-step` 拒绝；仅对 `backend=fake` 生效。
- `participants` 把 preset 映射为 PACT 参与者；缺省用 `defaultParticipant`。
- 配置错误在插件加载期 fail-fast（`PactConfigError`），不带病运行。

### 17.4 路由与钩子映射

| DSH 事件 | 处理器 | 非 pact | pact + 允许 | pact + 拒绝 | 后端异常 |
|---|---|---|---|---|---|
| `agent/pre-step` | `onPreStep` | `next()` | `next()` | `{kind:'reject'}` | govern 拒绝 / observe 放行 |
| `tools/pre-execute` | `onPreTool` | `next()` | `next()` | `{kind:'deny', reason}` | govern 拒绝 / observe 放行 |
| `tools/post-execute` | `onPostTool` | `next()` | 先 `next()` 再审计 | 同左 | 不改变结果 |
| `agent/turn-stopping` | `onTurnStopping` | 无动作 | `terminated && stopOnTermination` → `steer` | 同左 | 吞掉异常，不阻断收尾 |
| `session/event` | `index.js` 观察者 | 记录裁剪字段 | 同左 | 同左 | sink 失败不影响 harness |

- 钩子安装在 **agent scope**（`agentCtx.effect`），不安装在 root，天然只覆盖目标 agent。
- `createAgent` 只在 `mode=govern` 且 preset 命中时组合 `setup`；调用方原始 `setup` 先执行，其 `commit` 原样返回。
- `resume` 在会话发布后按 `session.header.meta.agentPreset` 安装钩子；**恢复后的首步不受门控**（已知限制）。

### 17.5 后端与失败策略

- 契约方法：`enabled/gates/status/seed/produce/consume/close`，键为 `domain/instance`。
- `FakePactBackend` 只模拟插件消费的表面（参与者启用、token 存在性、request_id 幂等），不是 PACT 引擎，不解释门表达式。
- `WasmPactBackend` 惰性导入模块并要求 `openPactHost(options)`；WASM 产物缺失时在首次治理调用显式失败，不影响进程启动。
- WASM 宿主由 `crates/pact-wasm/js/open-host.js` 组装：动态 `require` wasm-bindgen 产物 → `PactWasm` 工厂，`FileStore(storeRoot)` 提供事件日志与实例级 flock，`PactHost` 负责 `prepare → append+fsync → commit`。宿主返回的 `enabled` 是 Rule-W 可生产选项数组，适配层收窄为布尔（非空即“当前可执行”），与插件契约一致。
- 实例键与 session 一一映射：首个治理调用打开实例并获取锁；进程退出不显式 `close` 时锁文件残留，下次打开按 pid 存活检测回收（已验证同一实例跨进程复跑可用）。
- 失败策略：`govern` fail-closed（拒绝），`observe` fail-open（放行并写审计）；`turn-stopping` 永不因 PACT 读失败而阻断收尾。

### 17.6 验证与限制

- 验证：Node 22 内置 `node:test` 7 个文件（含 `wasm-backend.test.js`）**47 项**通过；`node --check` 全部通过；无 Rust 改动，故不跑 `cargo test`；`pact-wasm` 侧新增 `ci-smoke.js` 的 `openPactHost` 持久化往返（新建 → seed → 重开恢复）随 CI wasm job 执行。
- 真实装配已验证（`@deepseek-ai/dsh@0.2.1-rc.2`，2026-10-22）：`dsh plugin --profile pact-web add` 安装 bundle；`--dump-config` 显示原 `agent-loop` 行 `disabled: true` 且新增 `pact-agent-loop`/`dsh-pact-observe`；启动 web profile 后探针确认 `ctx.agentLoop instanceof PactAgentLoop`，且子类继承官方 `AgentLoop`、`Config` 同一。DSH 运行时按 peer 解析 `@deepseek-ai/dsh-agent-loop`。
- **真实会话门控验证（2026-10-22，本地 mock 模型）**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 的 headless profile 中注册 `pact-demo` preset、确定性 mock LLM 与临时驱动插件，通过 `ctx.agents.create({meta:{agentPreset:'pact-demo'}})` 驱动真实 agent turn：
  - `govern` + 允许：`bash` 正常执行，工具结果 `PACT_TOOL_OK`，`turn/end` 正常；
  - `govern` + `denyTools:["bash"]`：`tools/pre-execute` 返回拒绝，工具结果为 `Error: pact: tool denied by policy`，turn 继续到模型收尾；
  - `govern` + `fakeEnabled:false`：`agent/pre-step` 返回 `{kind:'reject'}`，会话在 `turn/start` 后直接 `turn/end`，未发起模型请求。
- **真实模型复跑（2026-10-23，`deepseek-official/deepseek-flash`，凭据仅经环境变量注入、不落盘）**：
  - 允许：bash 执行成功，工具结果 `PACT_TOOL_OK`，模型最终回复 `PACT_TOOL_OK`；
  - `denyTools:["bash"]`：工具结果 `Error: pact: tool denied by policy`（`isError:true`），模型据此收尾；
  - `fakeEnabled:false`：会话事件仅 `turn/start → turn/end`，无 `request/header`、无 step、无工具调用，模型请求未发起。
  - 结论：真实模型供应商下三条门控路径与 mock 结果一致。
- **WASM 后端 live（2026-10-23）**：以 `cargo build -p pact-wasm --target wasm32-unknown-unknown --profile wasm-release` + 官方 `wasm-bindgen 0.2.127`（`--target nodejs`）产出 Node 包；`PACT_DSH_CONFIG` 设 `backend:"wasm"`、`wasmModule:".../crates/pact-wasm/js/open-host.js"`、`wasmOptions:{module:<out-dir>, storeRoot:<dir>, declaration, ts}`。在 headless profile 中复跑三条路径：
  - 允许（实例预置 observation seed）：`agent/pre-step` 与 `tools/pre-execute` 通过，bash 实际执行，工具结果 `isError:false`、输出 `PACT_TOOL_OK`；
  - `denyTools:["bash"]`：工具结果 `Error: pact: tool denied by policy`，turn 正常收尾；
  - 预检拒绝（全新实例、Counter 为空）：`enabled('agent') → []`，会话仅 `turn/start → turn/end`，无 `request/header`、无工具调用；该实例的 `DomainLoaded` 日志由插件运行时通过 WASM 宿主创建，证明路径真实经过 WASM 内核。
  - 结论：`backend=wasm` 与 `backend=fake` 在相同 DSH 装配下行为一致，WASM 内核、`FileStore` 与插件门控全链贯通。验证使用确定性 mock provider（临时 profile 文件，不入仓库）。
- 禁用 `agent-loop` 行会丢弃其 config（当前 `agents: []`，与 DSH base patch 一致）；profile 若配置了 `agents`，须在 `pact-agent-loop` 行重写，否则会被清空。
- 后续（阶段 12 候选）：token 级工具映射（工具成功/失败对应 produce/consume 与补偿）、`resume` 首步门控、PACT 原生工具、真实 DSH 冒烟。

## 18. 表达力边界与编码方案（索引）

规范源是 `paper/pactv1_rev3.md` §1.3、§1.4 Rule-P、§1.4.4、§2.3；实现侧的集中清单、编码模式与“不可编码”候选见 [`docs/expressiveness-boundaries.md`](expressiveness-boundaries.md)。

本设计文档只登记落点，不复制清单：

| 边界编号 | 落点 |
|---|---|
| B4/B10（OR 分支、同类型多实例生产） | `pact-core` 的 `DuplicatePrecondRule` / `DuplicatePrecondLhs`（parser 报出）；域声明按固定角色拆分 |
| B7/B13（生产时条件/否定条件） | `pact-core` 的 `ProduceWithCondition` 拒绝；用前置类型或见证 token 编码 |
| B8/B21（消费时序、见证存活） | `pact-analysis` 的 `stranding_risk`（A.6）；`via` 语义 |
| B11/B12（referent 敏感的生产/读消费） | 版本编成 `TypeName` + SeedAuthority 分类；或外部校验者产生见证 token |
| B15（原子多输出） | 无原子编码；两段式 + 中间类型/复合 token/外部事务，见 `docs/expressiveness-boundaries.md` §3.6 |
| B23/B24（CPN 工具边界） | `pact-cpn` 的 `NegatedVia` / `MissingBound`；见 §15 与 D28 |
| 消费时机（多下游共享 token） | [`docs/token-consumption-timing.md`](token-consumption-timing.md)（TC1–TC13）：判定准则 TC-D1–TC-D6、模式 TC-P1–TC-P9、反模式 TC-A1–TC-A9；母本 §1.6/§1.7.3 已补写消费时机与 ack 屏障；与 B8/B17/B21/B22 交叉引用 |

新增或修订边界/消费时机时，先改母本，再同步 `docs/expressiveness-boundaries.md`、`docs/token-consumption-timing.md` 与本表。

