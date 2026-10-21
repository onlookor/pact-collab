# 需求变更记录

## 2026-10-21（第 19 轮，FR21 去除 WASM “实验性”标记）

- **FR21 状态变更**：WASM 本地分发形态由“实验性”转为**受支持**（D33，用户裁决）。稳定化前置 (1) Q7 CI、(2) 金标扩面、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视均已完成。
- **范围不变**：仍为 Node/桌面本地单用户嵌入；权威仍是原生 daemon；不提供 MCP/HTTP、ACL/审计、浏览器 OPFS/SharedWorker、DSH `govern` 与 NFR6 v2；不进入 `dist/` 产物集，不替代 `pactd`。
- **ABI 不变**：继续为 `pact-wasm/0.x` + JSON 字符串；去除标记不改变版本策略。
- **范围外表述同步**：§6 中“实验性 WASM 形态”改为“WASM 本地分发形态”，语义范围未变。

## 2026-10-21（第 18 轮，P6 决策门裁决 A：WASM 本地分发形态条件立项）

- **触发**：P0–P4 已完成（编译探针、纯类型层拆分、WASM 门面、内存引擎与原生对账、Node 持久化），P6 决策门评估（`docs/wasm-local-spike-feasibility.md` §11）建议 A；用户裁决 **A. 条件立项（实验性）**。
- **新增 FR21 WASM 本地分发形态（实验性，D31）**：WASM 为 `pactd` 之外的**可选本地分发形态**，权威仍是原生引擎，不替代 `pactd`；能力含语义核心、内存多实例会话、Rule-S/P/R/E/T、两阶段提交、JSONL/快照导出恢复与查询视图；持久化、崩溃恢复与单写者由宿主适配层承担（Node `crates/pact-wasm/js/` 已交付）。
- **能力边界**：不含浏览器 OPFS/SharedWorker、多用户 ACL/读审计、MCP/HTTP、TLS、turso 索引、压缩与运维接口；DSH `govern` 与 NFR6 v2 无硬依赖；安全边界为**本地单用户**，不得视为多用户安全边界。
- **ABI 策略**：`pact-wasm/0.x` + JSON 字符串；破坏性变更须升版并记入 `docs/design-change.md`；`abi_version` 为探测入口。
- **稳定化前置**（去除“实验性”标记的条件）：Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）→ 扩大金标（多实例、组合域、条件分支、压缩恢复）→ 抽取 `pact-engine-kernel` → 宿主边界安全审视。在此之前 WASM 不进入 FR18 生产分发承诺。
- **§6 范围外新增**：WASM 浏览器本地运行时、多用户 ACL/审计、DSH loop 治理与 NFR6 v2 远端模式；触发条件满足后另立范围。
- **既有条目**：FR1–FR20、NFR1–NFR7 语义不变；NFR4（单一静态二进制）仍是权威部署形态，FR21 不改变其表述。

## 2026-10-21（第 17 轮，阶段 6 CPN 编码生成转正式范围）

- **触发**：阶段 1–5、7–10 均已完成；用户在剩余候选（阶段 6 CPN / NFR6 v2 联网演进 / 加固收尾）中选定**阶段 6 pact-cpn**，将其从「范围外/可选」转为正式交付。
- **新增 FR20 CPN 编码生成**：新 crate `pact-cpn` + CLI 离线命令 `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；只依赖 `pact-core`，不进入 daemon/存储/引擎/运行时路径。
  - 单实例作用域（G1/D27）：一次生成只编码一个 `Counter instance`，库所/迁移以 `i_<instance>__` 命名空间化，不跨实例共享。
  - 结构映射：每类型库所 + Archive；`PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`；生产前驱 test arc、读自环、消费归档、种子无输入弧。
  - 条件：`and`/`or`/`not` → NNF → DNF；`or` 展开多迁移；`not` 下推为 `count T <= k` 并以互补库所编码（须显式 `--bound T=N` 真实上界）；`not via` 显式拒绝；恒假条件保留 `guard=false` 迁移；DNF 上限 1024。
  - 输出 CPN Tools XML（`format='6'`），通过 CPN DTD 校验与 XML 良构/结构测试；CPN Tools GUI 加载与状态空间实验为人工验收（沙箱无 GUI）。
- **§6 范围外收窄**：删除「CPN 编码生成器」整条，改为只排除「CPN 图形布局美化与 CPN Tools GUI 内交互式状态空间实验的自动化」；编码生成本身已属 FR20。
- **实现状态**：`pact-cpn` 11 项测试 + CLI 离线 e2e（成功/缺上界/JSON/实例前缀）；全量 **166 项**测试全绿；示例域生成物全部通过 CPN DTD 校验。
- 既有 FR1–FR19、NFR1–NFR7 无实质变更。

## 2026-10-20（第 16 轮，D25 子项 3 域级 ACL 与审计读视图落地）

- **触发**：用户裁决实施 D25 子项 (3)，四项取舍均选推荐项（实例级 ACL + 通配；覆盖生命周期 / 四类动作 / 查询；独立读审计流；fail-closed + REST 视图）。
- **FR17 细化（语义未变，仅把概括落为可实现口径）**：
  - ACL 条目 `domain/instance` 或 `domain/*`；非空时覆盖全部域作用域端点（生命周期、动作、查询、compact、审计视图），`GET /domains` 过滤；无令牌 `401`、越权 `403`；未配置 ACL 保持既有行为。
  - ACL 非空而令牌表为空 ⇒ daemon 拒绝启动；条目格式非法同样拒绝启动。
  - 读审计默认关闭；开启后每次成功 Rule-R 追加 `<instance>.read-audit.jsonl`（append + fsync），失败则本次读取 `500`（fail-closed）；审计流不进入事件日志 / `seq` / 重放 / Rule-T / 压缩。
  - 只读视图 `GET /domains/{d}[/instances/{i}]/read-audit?limit=`（缺省 100、上限 1000、时间正序）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；NFR2 的「日志即完整审计」不因独立审计流改变（审计流是附加可选项，不替代事件日志）。

## 2026-10-20（第 15 轮，D25 子项 1 日志压缩与 schema 版本化落地）

- **触发**：用户裁决「开始实现日志压缩与 schema 版本化」，并就四项设计取舍选定推荐项：压缩触发 = **显式运维接口 + 可选启动自动**；截断机制 = **前缀删除 + 旁路 base_seq 清单**；幂等表 = **纳入快照**；version 形态 = **每事件顶层 `version` 字段**。
- **FR17 第 1 条细化（语义未变，仅把概括落为可实现口径）**：
  - 截断机制：快照为提交点 → `<instance>.logmeta.json` 记 `base_seq` 水位（先于日志重写）→ 日志 tmp+fsync+rename 原子重写（保留 `seq > base_seq` 行）；水位单调不回退。
  - 快照自足：快照承载 `version`/`seq`/`dl_seq`/`declaration`/`state`/`consumed_requests`，**Rule-E 幂等表随快照持久化**；恢复以快照为锚补尾部事件。
  - 数据安全：日志已截断而快照缺失/不覆盖水位 ⇒ fail-fast（`SnapshotRequiredForCompactedLog`）。
  - 版本兼容：`EVENT_SCHEMA_VERSION = 1`；缺 `version` 视为 v1；更高版本 fail-fast（`UnsupportedSchemaVersion`）。
  - 触发：REST / CLI / MCP 显式入口 + `pact.toml` 可选 `compact_on_start` / `compact_keep_last`。
  - 固有取舍：压缩后审计视图退化为「快照 + 保留尾部事件」，须在 design/manual 文档化。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；NFR2 的「日志即完整审计」在压缩后由「快照 + 保留尾部」共同承担（已写入 FR17）。子项 (3) 域级 ACL 与审计读视图仍未实现。

## 2026-10-20（第 14 轮，D25 子项 2 运维接口落地）

- **触发**：用户裁决「继续开发」，并选定阶段 8 先做**子项 (2) 运维接口**（另三项取舍：Prometheus 文本格式、运维端点免令牌、PID 文件可选+陈旧检测、系统化优雅停机）。
- **NFR7 条目具体化**（语义未变，仅把原先一句概括落为可实现口径）：`GET /health` / `GET /ready` / `GET /metrics`（Prometheus 文本）、可选 `pid_file`（缺省不写、含陈旧检测）、SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界等待在途 → 终态快照 → 清理）；明确**运维端点不校验令牌**（仅本地/内网）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；子项 (1) 压缩与 schema 版本化、(3) 域级 ACL 与审计读视图仍未实现。

## 2026-10-20（第 13 轮，术语精化：幂等/自足作用域改用「域实例」）

- **触发**：用户指出第 12 轮「同域→同一实例」的措辞会与工程上的「实例」混淆，且须为将来类型-实例分离（如分布式 counter）留余地，要求统一写「域实例」。
- **措辞精化**：第 12 轮 G2 的「同一实例」→「**同一域实例**」，覆盖 FR9（事件自足）、FR10（Rule-T 信息性）、FR12、FR15、NFR2。术语映射见 `docs/design.md` D27（中文「域实例」= 论文 `Counter instance`，与论文已废弃的英文 `domain instance` 无关）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 语义不变（仅中文措辞精化）。

## 2026-10-20（第 12 轮，多实例约束覆盖核对后的文档补齐 G1/G2）

- **触发**：用户要求核对「论文多实例并发约束的实现覆盖」。核对基准 = rev3 与多实例直接相关的条款（§1.3 两半、§1.4.1 唯一性、Rule-T scope note、§4.5/§B.2 CPN scope、§1.8 及 §4.1–4.3 术语）。核对结论：C1（按实例成立 / 跨实例不协同）、C2（TokenId 作用域 + 可加强）、C3（按实例终止）、C6（术语）**全覆盖**；**C4/C5（CPN 作用域）未登记（G1）**、**幂等作用域措辞滞后（G2）**。用户裁定「G1、G2 补文档；G3（母本 §1.4.1 Rationale 措辞不一致）特批改论文但不产生修订版本」。
- **G1（补登记）**：把论文 §4.5/§B.2 的 **CPN 单实例作用域**登记为阶段 6（`pact-cpn`，未开始）的**实现约束**。见 §6 范围外 CPN 条目。要求：每个 Counter 实例独立编码，库所与迁移不跨实例共享，跨实例协同不入 `N(D)`；CPN 编码的「域」为域声明，实例划分属运行期（D27）。
- **G2（措辞收窄）**：幂等/自足作用域由「同域」统一改为「**同一实例**」——D27 将「以域名为唯一会话键」收窄为「以实例为会话键」（FR19），代码先已按每实例实现，仅需求/设计文本滞后。涉及 FR9（事件自足）、FR12、FR15、NFR2。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 语义不变（仅措辞随 D27 收窄）。CPN 约束为**既有范围外项的实现前置约束**，不改变其「后期可选阶段」定位。

## 2026-10-20（第 11 轮，阶段 10 实施：FR19 落地与细化）

- **触发**：用户「按计划实施」——实施阶段 10 域双键实例化（D27，路径 β，`docs/design.md` §11 行 10）。**本轮仅 D27 双键本体**，阶段 8 仍暂停。
- **FR19 细化（实施前置的四项裁决 + 通用护栏）**：
  - **默认回退**：不指定实例 ⇒ `default` 实例；既有 `--domain` 调用与脚本语义不变（落到 `default`）。
  - **旧日志不兼容（用户裁决）**：双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**，启动恢复 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移，不静默丢数据。
  - **实例键承载形态（用户裁决）**：实例名作为**路径段**（`/domains/{d}/instances/{i}/…`），`/domains/{d}` ≡ `default` 实例；`DomainLoaded` **不新增 `instance` 字段**（实例由文件路径承载）。
  - **声明原文另存**：`declare` 时写 `<data_dir>/<domain>/declaration.pact`，`recover` 用它回填声明注册表（支持重启后新增实例）。
  - **通用护栏（用户裁决）**：同一 `data_dir` 上 daemon 启动须取独占 flock（`<data_dir>/pactd.lock`），第二实例启动即拒；对**非空实例**（`last_seq > DomainLoaded.seq`）再加载默认拒绝（`DomainBusy`），须显式 `force`。
- **FR19 实现状态行**：由「立项（未实现）」更新为「**已实现（阶段 10，路径 β）**」——存储 `(domain, instance)` 双列 + 域子目录；引擎声明注册表（`declare` / `instantiate` 分离）；接口全链（REST 默认族 + 实例镜像族、CLI `--instance`/`--force`/`instantiate`/`instances`、MCP 16 工具含 `pact_domain_instantiate`、`pact-runtime` 实例透传）；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。
- **FR10 / NFR7 无删改**：多域会话行与多实例边界行维持第 10 轮表述（FR19 已涵盖实例维度）。
- **FR17 维持暂停**：引擎生产化三项仍与实例化耦合，未在本轮实现。
- **无其他需求增删**：FR1–FR18、NFR1–NFR7 语义不变。

## 2026-10-20（第 10 轮，域双键实例化：FR19 + FR10/NFR7 补充，论文 rev2 → rev3）

- **触发**：用户提问「两个同一个域的项目并行如何处置？」→ 指示暂停阶段 8 并评估模型 C 的全面影响。领域专家裁定（要点见 `docs/design-change.md` 第 18 轮）后，用户裁决**立项**、实现走**路径 β**、并破例授权把论文修订为 rev3。
- **范式来源切换**：`docs/pactv1_rev2.md` → `docs/pactv1_rev3.md`（rev3 仅含术语与作用域增补：§1.3 增列「Counter instantiation and partitioning」、§1.4.1 TokenId 作用域、Rule-T scope note、§4.5/§B.2 CPN 作用域、`domain instance` → `domain declaration`）——**零文法变化、零语义变化**，故 FR4–FR9 语义条目无需重述。
- **新增 FR19 域双键实例化**：定义键 `domain` 与实例键 `instance` 分离；同一域声明可并发承载多个 **Counter 实例**，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表；实例为接口层一等键（REST 路由、CLI、MCP、ACL、审计、压缩、运维指标）；不指定实例 ⇒ 默认实例，既有调用与旧日志语义不变。归属 `pact-store` + `pact-engine` + 接口层；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **FR10 补充**：多域会话行增「域内可并发承载多个 Counter 实例」。
- **NFR7 补充（多实例边界）**：同一 `data_dir` 并发承载多实例须以独占锁（flock）保证单写者，禁止交错写同一日志；实例间不共享 Counter/Archive，跨实例协同不在范式保障范围内。
- **FR17 无删改**：引擎生产化三项维持有效，但验收与实例化耦合——(1) 的压缩部分与 (3) 全部须按双键重述，`:compact` 路由须随实例段重设计。
- **无删改**：FR1–FR16、FR18、NFR1–NFR6 语义不变。

## 2026-10-19（第 9 轮，生产化边界裁决：FR16–FR18 + NFR7）

- **触发**：用户就"作为生产系统的功能边界"提问——还需补足什么、留在本项目还是另立、同一二进制还是分开构建、MCP server 由谁提供。经四问裁决：(1) agent 驱动运行时 → 本仓库新增 crate；(2) 引擎生产化三项全部推进；(3) 分发产物 → 本仓库 `dist/`；(4) MCP 维持 stdio-only 到 v2。
- **新增 FR16 生产运行层**：可复用的参与者驱动运行时（自驱循环 + 多 agent 编排）+ 外部副作用执行器；归属本仓库新增 crate（决策 D24）。引擎保持被动（FR11/D5）、核心语义零改动。
- **新增 FR17 引擎生产化**：事件日志压缩与 schema 版本化（pact-store）、运维接口（health/ready、metrics、PID、信号化优雅停机）、域级授权与审计读视图（读审计可选开启、默认关闭，修订 D9 默认值表述）。
- **新增 FR18 部署与分发产物**：本仓库 `dist/`（Dockerfile、systemd unit、musl 静态二进制打包脚本），属交付资产，不接入 CI 语义测试（同 D22）。
- **新增 NFR7 可运维性**：可被外部进程管理器托管（可探活、可优雅停机）、运行状态可观测、数据可备份恢复、事件日志长期增长有可控压缩/保留手段。
- **无删改**：FR1–FR15、NFR1–NFR6 语义不变；§6 范围外三条维持（CPN 生成器、Archival Policy、分布式/OR/时间/概率）；原"引擎不执行外部副作用"边界不变——副作用执行端移入 FR16。

## 2026-10-19（第 8 轮，开发环境描述更正）

- **触发**：用户指出开发环境描述过时——`pact-dev` 容器已不存在，现开发环境为 Trae 沙箱环境。
- **§2 总体约束「开发环境」行更正**："所有开发工作在 distrobox 容器 `pact-dev` 中进行" → "所有开发工作在 Trae 沙箱环境中进行"；同步更正 `AGENTS.md` 同名规则条与 `docs/skill-adaptation.md` §9.1/§9.2 的环境表述（技能侧记录见 `docs/skill-adaptation-change.md` 第 5 轮）。
- **无功能需求变更**：FR1–FR15、NFR1–NFR6 均不变，仅环境约束的文字表述更新。

## 2026-10-18（第 7 轮，消耗原子性、幂等与物化边界）

- **触发**：对"消耗原子性"组织评审后，用户裁决采纳"文档化实现契约 + 补幂等 + witness-token 示例（2a）与 outbox 语义（2b）并行"组合，要求契约表述自足、不依赖外部讨论语境。
- **新增 FR15 消耗原子性、幂等与物化边界**：明确引擎对 Rule-E（消费）在并发、崩溃、重试与外部副作用四个维度上的承诺与不作为——提交点（JSONL fsync）前无部分效果、提交点后派生步骤尽力而为、Rule-E 支持可选幂等键、引擎不执行外部副作用。
- **FR9 补两条**：事件自足（Consumed 事件携带 token 全量快照，单行即可审计与重放）、提交契约（JSONL fsync 为提交点，其后派生动作不回滚）。
- **FR10 补一条外部副作用边界**：`ThingRef` 不透明、引擎不执行外部动作，"consume 成功" 不等于 "外部动作成功"，物化确认由 witness-token 或 saga 语义在域外承载。
- **FR12 补一条**：Rule-E 消费请求支持可选 `request_id` 幂等键；同键同 token 重试返回首次归档结果，同键异 token 报 `IdempotencyConflict`（HTTP 409）。
- **FR13 集成测试补幂等**：新增 request_id 去重、跨重启幂等表重建、同键异 token 冲突三类断言。
- **FR14 域数四 → 五**：通用域新增 `external_materialization.pact`（witness-token 物化确认模式）。
- **NFR2 强化**："每条事件自足"——单行日志即可独立审计与重放，不依赖同域其他行。
- **§6 范围外新增一条**：分布式事务 / 2PC / 分布式共识、以及由引擎执行外部副作用，均不在实现范围内（单写者 actor 已满足线性化需求，见 design.md D23）。

## 2026-10-15（第 6 轮，通用示例与用户手册）

- **触发**：用户指出既有 `docs/skill-adaptation.md` 一族的适配结构与 requirements-engineering / software-engineering 两技能强绑定，需补一套与技能无关的需求工程/软件工程通用结构，并配 examples 向用户说明整个 PACT 范式系统的使用场景与方法。
- **新增 FR14 通用示例与用户手册**：`examples/domains/` 四份通用域、`examples/fragments/` 两份组合片段、`examples/offline/run.sh` 与 `examples/online/run.sh` 两条上手路径（各带录制轨迹）、`examples/README.md` 场景导航；`docs/manual.md` 为权威用户手册。明确二者分工（可运行教学路径 / 参考权威）与交叉引用义务。
- **§6 范围外新增一条**：示例不接入 CI 回归——`examples/**/*.pact` 不由 cargo test 遍历校验，示例正确性由人工执行与录制轨迹对照保证（决策 D22）。
- **用户裁定的范围边界**：通用域含 RE / SE / ReAct-agent 三类，**不含**医疗路径域；SE 通用域取中等粒度；examples 深度为离线三件套 + 在线 daemon 全生命周期轨迹；用户向文档落点为 `examples/README.md` 与 `docs/manual.md` 两者。
- 既有 FR1–FR13、NFR1–NFR6 无实质变更。

## 2026-09-09（第 5 轮，论文 rev2 对齐）

- 范式规范来源切换：`pactv1_rev1.md` → `pactv1_rev2.md`。
- FR4：Rule-P 前驱互异补充"只按 TokenId 判定"（rev2 Rule-P Note）。
- FR5：新增 count 字面量 PosNat 约束（`[1-9][0-9]*`，拒绝 0 与前导零；缺席语义用 `not exists T` 表达，Normalization rule）。
- FR6：良构校验由"四条零散规则"重述为 §3.1 六项约束（新增 LHS Uniqueness、Non-Empty LHS 两条；自环 `A → A` 显式合法）；明确解析期语法拒绝 + 声明期 validate_domain 防御两层。
- FR7：A.5 由"源生产合法性"重定义为"前置与源生产良构性"（rev2 A.5）——§3.1 六项约束违规清单 + Seed/Produce 重叠咨询性警告，三元输出 Violations/Warnings/WF(D)，按附录 D 为其余分析的前提、置于报告首位。

## 2026-08-26（第 4 轮）

- 范式规范来源切换：`pactvfinal.md` → `pactv1_rev1.md`（论文最新版解决了 D15 指出的 Rule-T 终止问题，以 rev1 为准）。
- FR4：Rule-P 补充前提 `Precond(T) ≠ ∅`（源类型不可生产）；Rule-T 按 rev1 Rule-P-enabled 三条件定义。
- FR6：良构校验补充 WF-3——源类型（无前置规则）不得声明 produce gate。
- FR7：静态分析由五项扩为六项（新增 rev1 A.5 源生产合法性）。
- FR8：片段组合一致性由三条件扩为四条件（新增 C4 源生产封闭）。

## 2026-08-25（第 2 轮）

- 文档标题去掉"（当前有效）"后缀（三文档统一）。
- 新增 FR12 领域数据格式：`.pact` 声明文件为注入点唯一载体；定义 Token/ArchiveEntry/动作请求响应/结构化错误/通知事件的 JSON 线上格式；三接口共享同一套格式；时间戳 ISO-8601 UTC、字段 snake_case。
- 新增 FR13 测试组织：随变更组织单元/集成/端到端/回归四类测试（对应 AGENTS.md 新增规则）。

## 2026-08-25（第 1 轮）

- 初始需求建档（FR1–FR11，NFR1–NFR6，范围外清单）。
