# 模型 C 全面影响评估（临时评估产物，非代码、非定稿）

**状态**：评估文档。评估当时不改任何代码、不修改 `docs/` 定稿、**不修改范式母本** `docs/pactv1_rev2.md`（涉及母本的任何改动事前须用户显式确认）。**后经用户破例授权，母本已切换至 rev3——见 §9.1 后续进展**；评估正文仍按 rev2 原文引用（rev2 保留在 `docs/` 作历史）。
**触发**：用户提问「假设有两个同一个域的项目并行，应如何处置？」+ 指令「暂停实施阶段 8，评估模型 C 的全面影响，包括论文（可能需要领域专家）」。
**本轮两条既有修正仍然有效**：(1) *生产运行层面*「不安装任何包」约束放松，优先成熟 crate；(2) 域级访问控制作为**变化点**，不绑死 ACL。

---

## 0. 评估对象与事实基线

**模型 C**：把 domain 从「唯一会话键」升级为 **domain（定义键）+ instance（实例键）双键**——路由 `/domains/{d}/instances/{i}`（或 query `?instance=`）、存储文件 `<domain>.<instance>.events.jsonl`、CLI/MCP/ACL/审计/压缩全链加实例维度。

**对照组**：
- **A｜命名实例化**：会话名自带实例（`pact domain load f.pact --name projA_se`）。代码零改，靠约定与护栏。
- **B｜每项目一 daemon**：一 `data_dir` + 一 socket。架构零改，运维变重，需 `data_dir` 独占锁。

**三档判定口径**：`确定不变` = 契约/语义不受影响，改 C 也不必动；`确定要改` = 若立项 C 则必改，且改动点已定位；`待领域专家裁定` = 属范式层表述/概念，超出实现方权限。

### 0.1 触发本次评估的工程事实（已逐条核对）

| # | 事实 | 证据 |
|---|------|------|
| F1 | **域名 = 会话标识**（含声明与运行态） | design.md D7、§8.1；`engine.rs:281` 附近的 `BTreeMap<String, DomainSession>` |
| F2 | `LoadDomain` 是**替换语义**：同名额重置会话、清快照、seq 续接 | `engine.rs:476-518`（`remove_snapshot` 于 `:493-495`，`CounterState::new()` 于 `:509`） |
| F3 | 域名**直接作文件名**，校验仅允许 `[A-Za-z0-9_-]`、长度 ≤128 | `pact-store/src/log.rs:107-114`、`pact-store/src/error.rs:92-107` |
| F4 | 域名同时是 REST 路由键 | `pact-http/src/server.rs:218-235`；`{d}:analyze`/`{d}:compose` 经段内冒号分发（`:239-291`），**`:suffix` 命名空间已被占用** |
| F5 | 域名是索引归属列；索引主键 `(domain, seq)`（相对设计稿已修正，见 `index.rs:4-5`） | `pact-store/src/index.rs:75-93` |
| F6 | 不存在任何 `data_dir` 锁 | 全仓无 `flock`；`pact-http/src/config.rs:12-41` 无锁/PID 相关键 |
| F7 | CLI 已支持覆盖会话名：`pact domain load <file> --name N` | design.md §9.3；`pact-cli/tests/cli_e2e.rs:115` |
| F8 | MCP/CLI/runtime 逐工具逐命令均只带 `domain`（无实例维度） | `pact-mcp/src/main.rs:58-140`（`LoadIn.name`/`DomainIn.domain`）、`pact-cli/src/main.rs:53-206`、`pact-runtime/src/rest.rs:16,70`、`mcp.rs:51`、`orchestrator.rs:102`、`bin/pact-runtime.rs:25` |
| F9 | 事件日志与状态事件**不含域键**（域由文件名承载） | `pact-store/src/event.rs:8-41`（`Event` 各变体无 domain 字段） |

> **F2 是本次问题的病灶**：两个并行项目若共用同一域名，第二次 `load` 会**销毁**第一本的 Counter 与快照（`engine.rs:493-516`）。Model C 的直接动机就是消除这个破坏性耦合。

---

## 1. 层一：形式模型 / 论文层（`docs/pactv1_rev2.md`）

### 1.1 关键形式事实

| # | 事实 | 证据（行号） |
|---|------|------|
| P1 | **Domain 文法不含名字**：`Domain ::= TypeSchema PreconditionGraph GateMatrix SeedAuthorityDecl` | `pactv1_rev2.md:436` |
| P2 | **Counter 是独立文法项**（状态，非声明的组成部分） | `:498-502`（`Counter ::= …`）；Archive 同（`:504-511`） |
| P3 | 静态分析**显式声明为声明级**："performed on the static domain declaration, without executing the system" | `:684`；A.1 取 `REACHABILITY(D, T)` `:1213`；A.5 同 |
| P4 | §1.8 组合作用于**声明**，四条件全为声明级 | `:347-365` |
| P5 | Rule-T 以 `Counter = ∅ ∧ ¬∃ p: Rule-P-enabled` 定义 → **per-Counter** | `:165-179` |
| P6 | §1.4.3 源类型入口唯一性定理：所有 T 型 token 均由 Rule-S 引入 → **per-Counter** | `:202-204` |
| P7 | §1.6 非保证表（含 stranding）→ **per-system** | `:261-273` |
| P8 | §1.3 明确「范式不规定：…**Physical implementation of the Counter**」 | `:56` |
| P9 | §2.2 域独立性 = 四个注入点；范式本身不变 | `:409-413` |
| P10 | Part III §2.1「Multiple harnesses coordinate through a **shared Counter**」 | `:1096-1105` |
| P11 | §1.8 定理原文用词 "a well-formed PACT **domain instance**" | `:358` |

### 1.2 判定

- **确定不变**：§1.4.4 / §3.1 良构六项；A.1–A.6 与第 5 章静态分析；§1.8 组合四条件；GateHolds/§1.7 GateExpr；Rule-P/R/E/S/W 的形式定义。理由：这些全部以「**声明**」或「**对 the Counter**」为论域，实例维度只是回答「哪一本 Counter」，不改变任何判定式。
- **确定要改（表述层，仍须专家点头）**：
  - P11 的 "domain instance" 用词与运行时「实例」同名 → **术语冲突**，须消歧。
  - §1.3（`:48-60`）与 §2.2（`:409-413`）未列「实例化 / 会话划分」；若承认实例概念，需在此二处增补一句。
- **待领域专家裁定**：见 §8 的 Q1–Q4、Q7。

**核心结论**：Model C **不构成形式冲突**。论文的 Domain 是声明（无名字，P1），Counter 是状态（P2），二者本就是不同层；Model C 只是给「一次 Counter 运行」命名。按 P8，Counter 的实例化细节本属「范式不规定」的实现自由——因此 C 可先在实现方立项，论文改动仅限**措辞与概念的显式化**，非语义修订。

---

## 2. 层二：存储层（`pact-store`）

### 2.1 判定

- **确定要改**
  - `log::events_path` / `snapshot_path`（`log.rs:107-114`）：文件名须容纳实例维度。
  - `Store::recover`（`store.rs:49-63`）：从 `*.events.jsonl` **反解**域名的逻辑要改为反解 `(domain, instance)`。
  - `Store::list_domains`（`store.rs:182-190`）：同样按文件名反解。
  - `validate_domain_name`（`error.rs:92-107`）：实例名需同等校验；若用复合键须**禁止分隔符注入与 `..`**。
  - 索引归属（`index.rs:75-93`，`PRIMARY KEY(domain, seq)`）：若显式引入实例列，为一处小 schema 演进（索引可重建，D13 已对冲）。
- **确定不变**
  - 日志行格式（`event.rs:8-41`，**事件不承载域键**，F9）——与实例维度正交。
  - `seq` 逐行 +1 不变量、fsync 提交点、快照原子写、崩溃一致性（`store.rs:141-160`、design §7.4）。
  - Consumed 自足（D23）、Rule-E 幂等表：**作用域由「域」改为「实例」**，机制不变。
- **待领域专家裁定**：无（纯实现）。
- **需登记（非专家问题）**：**seq 空间天然 per-instance**（seq 附于日志，现状 per-domain 已是 per-日志，故等价）；跨实例比较 seq 无意义 → 写入手册。

### 2.2 一个决定性简化（见 §7.2）

若会话键取复合字符串 `<domain>.<instance>`，则 `store`/`index`/`engine` 的「domain」列**直接存复合键**：**存储层零 schema 变更**，仅接口层增加参数与校验。这是把 C 的爆炸半径压到「接口层 + 校验」的关键选项。

---

## 3. 层三：引擎层（`pact-engine`）

### 3.1 判定

- **确定要改**
  - 会话表键：`BTreeMap<String, DomainSession>`（`engine.rs:281` 附近）由「域名字面量」改为「会话键」。
  - 全部命令的 `domain: String` 字段语义变化（`engine.rs:117-205`，16 个命令变体，其中 11 个带域键）。
  - `load_domain` 的**替换语义**（`engine.rs:476-518`）：替换粒度由「域」降为「实例」。
  - 启动恢复 `recover_domain`（`engine.rs:314-383`）逐会话执行。
  - 每会话一个 `broadcast` 通道（`engine.rs:496-497`）与通知载荷（`notify.rs:21` 的 `domain` 字段）。
- **确定不变**
  - 单权威 actor 串行化（D6）、检查+应用同临界区（§8.1）、Rule-E 原子性契约（§8.5）、`Shutdown` 终态快照、Clock/IdGen 注入（D8/D10）、引擎「只编排不判定」（D14）。
- **待领域专家裁定**：无。
- **待用户/架构裁决（模型 C 的真正取舍）**：`load` 的语义在实例化后是否仍为「同名即重置」？Model C 下更自然的是**声明与实例分离**（`declare` 绑定定义键、`start` 实例化），但这是**实现级设计决策**，不属论文层（建议作为新决策 D-N 定夺）。

---

## 4. 层四：接口层（`pact-http` / `pact-cli` / `pact-mcp` / `pact-runtime`）

### 4.1 判定

- **确定要改**
  - REST 路由表（`server.rs:218-235`）：11 条 `/domains/{d}` 路由全部需实例维度；**`{d}:analyze`/`{d}:compose` 的段内冒号分发（`server.rs:239-291`）在新增实例段后必须整体重做**——且阶段 8 的 `:compact` 正要落在这个已占用的 `:suffix` 命名空间里。
  - `client.rs` 全部域方法（`load_domain`/`domain_info`/`unload_domain`/`analyze_domain` …）。
  - CLI：所有 `--domain` 命令（`pact-cli/src/main.rs:53-206`）需 `--instance`（或复合 `--name`）；`pact domain load --name`（F7）语义变化。
  - MCP：`LoadIn.name` / `DomainIn.domain`（`pact-mcp/src/main.rs:58-140`）需增实例字段。
  - `pact-runtime`：`RestClient.domain`（`rest.rs:16,70`）、`McpClient`（`mcp.rs:51`）、`Orchestrator.domain`（`orchestrator.rs:102`）、bin 配置 `domain`（`bin/pact-runtime.rs:25`）——每个 runtime 进程**绑定单域**，需增实例。
  - SSE 订阅（`GET /domains/{d}/events`）按会话键分通道。
- **确定不变**
  - 线上 JSON 格式 §6（token/archive/action DTO 不含域键）。
  - 三接口等价性由构造保证（D19：MCP/CLI 均经 REST）；MCP stdio-only（D20）；信任声明 + 可选 bearer（D11）。
  - `pact.toml` 结构（`config.rs:12-41`）——复合键路径下**无需新增实例相关键**。
- **待领域专家裁定**：无。
- **注**：域级访问控制作为变化点（seam `IdentityProvider` + `AccessPolicy`，`domain: Option<&str>` 入参）**天然可扩展**到 `(domain, instance)`，不需要绑死 ACL——满足用户修正 (2)。

---

## 5. 层五：迁移与兼容

- **确定要改**
  - 默认实例约定（建议：**不加实例段 = 默认实例**，则旧文件 `<domain>.events.jsonl` 天然落入默认实例，**磁盘零迁移**）。
  - 路由兼容：`/domains/{d}` → 默认实例；`/domains/{d}/instances/{i}` 为新增路径。
  - CLI/MCP 兼容：`--instance` / 新字段**可选**，缺省默认实例 ⇒ 旧脚本与旧 agent 调用零改动。
  - 文档面：manual.md / README / examples 的域命名说明。
- **确定不变**：事件 schema 行格式；日志一切不变量；索引可重建（D13）⇒ 无迁移硬需求。
- **待领域专家裁定**：无。
- **同批处置项（与 C 正交但建议并入）**：`data_dir` 独占锁（F6，Model B 的必要条件，也是多实例同目录的安全前提）；非空域 `load` 默认拒绝（409，须显式 `--force`）——这条护栏对 A/B/C 都适用。

---

## 6. 阶段 8 交叉标注（本次指令的落点）

阶段 8 三项 = design.md §11 行 8 / D25：**(1) 日志压缩 + schema 版本化；(2) health/ready + metrics + PID + 信号化停机；(3) 域级访问控制 + 可选审计读视图**。

| 阶段 8 子项 | 与模型 C 的耦合 | 受影响的具体落点 |
|---|---|---|
| (1) **schema 版本化**（行级 `{"version":1,…}`） | **无耦合** | 行格式与实例维度正交，可**独立先行**，不受 C 影响 |
| (1) **日志压缩** | **强耦合** | 文件名 → 会话键；压缩重写的首行 `DomainLoaded { name }`（design §7.1 / `event.rs:12-17`）**取值语义需重新定义**（存域键还是会话键）；索引 `prune_events` 的域参数同变 |
| (2) **health/ready/metrics** | **中耦合** | `domains_total` / `terminated_domains` 等指标需增实例维度或改名；`/ready` 的 `ReadyView.domains` 同理；控制面计数按会话 |
| (2) **PID 文件 + 信号化停机** | **弱耦合** | 与技术正交；但 `data_dir` 锁（F6）是**多实例同目录的安全前提**，建议同批落地 |
| (2) **路由扩展**（`:compact`、`/audit`） | **强耦合** | `:compact` 落在已被 `:analyze`/`:compose` 占用的 `:suffix` 命名空间（`server.rs:239-291`）；C 改路由后该分发逻辑整体重做 ⇒ **先做 C 可免二次改造** |
| (3) **域级访问控制** | **强耦合** | ACL 作用域由 `domain` 升为 `(domain, instance)`；seam 的 `domain: Option<&str>` 入参需改双参或会话键（seam 本身可保留，**不绑死 ACL**） |
| (3) **审计读视图** | **强耦合** | 审计流文件名 `<domain>.read-audit.jsonl` → 会话键；`AuditRecord` 是否携带实例维度需定契约；审计与主日志的「一一对应」关系不变 |

**结论**：阶段 8 三项中，**2 项强耦合（压缩、ACL/审计）、1 项中耦合（运维指标）**，其中路由扩展与 `:compact` 存在**直接设计冲突**。排序建议：
- 若 C 立项 → **先定会话键契约，再回做阶段 8**（阶段 8 的 (3) 与 (1) 的压缩部分按新键重述；schema 版本化可先独立落地）。
- 若 C 不立项 → 阶段 8 按既有 `spec.md` 实施，但应保留 ACL seam 的域键抽象度，避免将来 C 立项时二次改造。

---

## 7. 三条模型的代价对比与 C 的两条实现路径

### 7.1 对比

| 维度 | A 命名实例化 | B 每项目一 daemon | C 双键 |
|---|---|---|---|
| 代码改动 | **零** | 零（架构不变） | 接口层全链（+可选存储/引擎） |
| 语义承载 | 藏在字符串约定里 | 靠进程/目录隔离 | **结构化、自描述** |
| 分组/聚合查询 | 需前缀解析 | 天然隔离 | 天然（域 → 多实例） |
| 运维负担 | 无 | 重（每项目一套 daemon/socket/data_dir） | 中 |
| ACL/审计/运维分层 | 逐名（粒度=名） | 逐实例（粒度=实例） | 天然两级 |
| 必备护栏 | 文档约定 + 非空域 load 拒绝 | + `data_dir` flock 独占锁 | + 会话键校验（禁分隔符注入/`..`） |

**评估**：若目标仅为「两个并行项目不互相踩」，**A + 护栏**已充分（现状 F7 已支持），C 属过度设计；若目标含「同一域多实例的运维/审计/ACL 分层」，C 是唯一正解。

### 7.2 C 的两条实现路径

- **路径 α（低代价，推荐先评估）**：会话键 = `<domain>.<instance>`（两段各自校验，整体禁 `..` 与分隔符注入）。`store`/`index`/`engine` 的 `domain` 列**直接存复合键** ⇒ **存储层零 schema 变更、引擎层零类型变更**，仅接口层增参数 + 校验 + 默认实例。
  - 代价：无法结构化查询「某域的全部实例」（退化为前缀扫描/全列后过滤，仍可实现）。
- **路径 β（高保真）**：`store`/`index` 显式 `(domain, instance)` 两列 + 声明注册表（`declare` 与 `instantiate` 分离，见 §3.1）。
  - 收益：语义最清晰、聚合查询天然；代价：改造面最大（触及 store/engine 类型与索引 schema）。

---

## 8. 领域专家待裁定问题清单

> 说明：本节全部为**范式层表述/概念**问题，按 `docs/design.md` §13 边界判据 (4)——「是否新增范式语义？→ 须论文母本先行」——须专家（论文作者）裁定。**母本改动须经用户显式确认后方可执行。**

- **Q1（概念承认）**：论文是否应显式承认「同一 `Domain` 声明可并发存在多个 Counter（运行实例）」？若承认，§1.3（`:48-60`）与 §2.2（`:409-413`）是否增补「实例化/会话划分」为**不属于范式的实现自由**？（P8 已含「Physical implementation of the Counter」一条，可能已覆盖——取保守解释待确认。）
- **Q2（术语消歧）**：§1.8 定理（`:358`）的 "a well-formed PACT **domain instance**" 与其他章节的「实例」是否冲突？是否改称 "domain declaration" / "composed domain"？（P11）
- **Q3（限定重述）**：Rule-T（`:165-179`）、§1.4.3 入口唯一性定理（`:202-204`）、§1.6 非保证表（`:261-273`）是否需一律加限定「针对单一 Counter 实例」？评估认为它们**已是 per-Counter 语义**，仅措辞可加限定。
- **Q4（全局唯一性）**：§1.4.1（`:118-120`）「TokenId 全局唯一 ⇒ Counter 实为集合」在多实例并发下是否被破坏？实现结论：ULID 全局唯一 ⇒ **不破坏**；是否需要论文澄清？
- **Q5（不变量依赖）**：是否存在任何论文级不变量依赖「一个域名至多一本 Counter」？**评估结论：未见**（已逐项核查 §1.4–§1.8、第 2–5 章、附录 A）。
- **Q6（CPN 编码）**：§4.5 / 附录 B 以**单个** Counter 库所建模；多实例是否需要注明「每实例独立编码一次」？（`:619-658`、`:1432-1580`）
- **Q7（术语完备性）**：论文是否需要新增「实例」术语以支撑工程描述？（Part III §1.1 已有 "Single-Agent ReAct Loop as PACT **Instance**" 用法，`:1036-1061`——与 Q2 同源。）
- **Q8（越界判定）**：若论文维持现状（不显式承认实例概念），实现方以「实现自由」名义落地 C 是否越界？评估结论：按 P8 **不越界**，但需确认。

---

## 9. 结论与建议

1. **是否需要领域专家**：**需要，但不阻塞实施**。形式层「确定不变」占绝对多数（§1.2）；需要专家裁定的仅 Q1–Q4、Q7 五项**表述/概念**问题，其中 Q5 已由本次核查给出「未见依赖」结论。这些都**不影响 Model C 能否落地**——按 P8，Counter 的实例化本属范式不规定的实现自由。
2. **是否立项 C**：§9.1 记录用户裁决。
3. **阶段 8 如何调整**（详见 §6）：
   - (1) schema 版本化**解耦**，可独立先行；
   - (1) 压缩部分与 (3) ACL/审计**强耦合**，按会话键重述后实施；
   - (2) 运维接口的路由扩展与 `:compact` 存在**设计冲突**，宜随 C 一并重设计；PID/`data_dir` 锁建议同批落地（A/B/C 通用护栏）。

### 9.1 用户裁决（2026-10-20）

| 议题 | 裁决 |
|---|---|
| 模型 C 是否立项 | **立项** |
| 实现路径 | **路径 β（双列高保真）**——`store`/`index` 显式 `(domain, instance)` 两列 + **声明注册表**（`declare` 与 `instantiate` 分离，见 §3.1） |
| 领域专家 | **现在就请专家裁定** Q1–Q4、Q7（Q5/Q6 一并请确认） |

**§9.1 后续进展（2026-10-20，同日闭环）**：
- 专家已逐条答复（Q1 (a) / Q2 (a) / Q3 (a)+(c) / Q4 保留但作用域收窄至该 Counter / Q5 确认无依赖 / Q6 确认加作用域 / Q7 不新增正式术语），并给出「建议的修订清单」；总括口径确认**不涉任何语义修改**、**实例划分属实现自由不越界**。
- 用户**破例授权改论文为 rev3**：`docs/pactv1_rev3.md` 已落地；相对 rev2 共 **9 个 diff 块**（§1.3 增列条款、§1.4.1 作用域重写、Rule-T scope note、§4.5 与 §B.2 CPN 作用域、§1.8 及 §4.1–4.3 四处术语替换），`diff -u` 核对**零文法变化、零语义变化、零行丢失**。
- 母本引用已同步（design / requirements / manual / skill-adaptation / examples 两份 README）；`docs/design.md` 落 **D27** 与 §11 阶段 10 行，`docs/requirements.md` 落 **FR19** 与 FR10 / NFR7 补充；三份 change 文档各追加一轮。
- **三项细节已定案（2026-10-20，用户裁决）**：① 存储布局 = **域子目录** `<data_dir>/<domain>/<instance>.events.jsonl`（快照/审计流同目录）；② 默认实例名 = **`default`**；③ 声明绑定 = **实例创建时快照声明**（重放自封闭）。
- **派生待定（实施前须定）**：既有单层日志 `<domain>.events.jsonl` 的兼容策略（启动一次性迁移 vs 读取回退）——注意用户选定「显式子目录 + 显式 `default` 名」后，旧日志**不再自然可见**，须显式给出兼容路径；域子目录下是否另存声明原文（`<domain>/declaration.pact`）。

**路径 β 的连带后果（相对路径 α 需额外处理）**：
1. **存储层不再是零改动**：`index.rs` 的 `PRIMARY KEY(domain, seq)` → `(domain, instance, seq)`；`events`/`tokens` 两表增 `instance` 列；索引为派生物可重建（D13），无磁盘迁移硬需求。
2. **存储文件名（已定案）**：**域子目录**布局 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `<domain>/<instance>.snapshot.json`、审计流 `<domain>/<instance>.read-audit.jsonl` 同目录）；`recover` 的扫描逻辑须由单层改为两层递归，且**既有单层旧日志不再自然可见**，须显式给兼容路径（启动一次性迁移 vs 读取回退）。
3. **引擎需要声明注册表**：`DomainSession` 拆为「声明条目（定义键 → 已解析 `Domain` + 原文）」与「实例条目（实例键 → `CounterState` + `last_seq` + 订阅 + 幂等表）」。`load_domain` 的替换语义随之降为"替换**声明**"，实例替换改为独立的 `instantiate`/`reset` 动作。
4. **恢复路径**：`recover_domain` 由"逐域"改为"逐实例"，且需在重放前先按定义键取声明。**声明绑定已定案：实例创建时快照声明**（`DomainLoaded` 携声明原文）——故重放自封闭，后续 `declare` 覆盖定义键不影响已运行实例；代价是实例不随声明热更新（需重建实例）。
5. **`DomainLoaded` 事件的字段语义**（`event.rs:12-17`）：`name` 现在指域名；β 需增 `instance` 字段（或复用 `name` 存实例键）。**该事件是重放路径的唯一声明载体，字段契约一旦改动必须与 schema 版本化（阶段 8 项 (1)）合并处理**。
6. **回归锚点风险**：现有 110 项测试全部以"域 = 会话键"编写，β 会波及 `pact-store`/`pact-engine` 的多数集成断言 ⇒ 需要一次"默认实例"兼容层（无实例段的旧调用映射到默认实例）以保住既有测试与旧脚本。

## 10. 裁决后的执行序（路径 β）

1. **【已完成 2026-10-20】专家裁定**：简报见根目录 `model-c-expert-brief.md`；Q1–Q7 已逐条答复，Q1 答 (a) **不推翻立项**。
2. **【已完成 2026-10-20】专家答复落地**：经**用户破例授权**，论文已修订为 **rev3**（`docs/pactv1_rev3.md`；9 处增补/术语替换，`diff -u` 核对零文法变化、零语义变化）；母本引用全仓同步至 rev3；裁定要点与术语登记已落 `docs/design-change.md` 第 18 轮（设计侧 D27、需求侧 FR19）。
3. **落定新决策 D-N**（即将写入 `docs/design.md` §0）——**前置：用户裁决 §9.1 的三项细节**（存储布局：单层文件名 vs 域子目录；默认实例名取值；实例与声明版本的绑定：创建时快照声明 vs 引用最新）。D-N 至少含：定义键/实例键的命名、存储布局、默认实例、`declare`/`instantiate` 与 `load_domain` 的关系、声明版本与实例的绑定、路由形态、ACL/审计作用域、`DomainLoaded` 字段契约。
4. **契约先行**：把 `DomainLoaded` 字段契约与 schema 版本化（原阶段 8 项 (1) 的行级 `{"version":…}`）**合并**设计，避免两次破坏性演进。
5. **实施**：`pact-store`（双列 + 布局）→ `pact-engine`（声明注册表 + 实例会话）→ 接口层（路由/CLI/MCP/runtime）→ 通用护栏（`data_dir` flock、非空域 `load` 拒绝、实例命名约定入文档）。
6. **阶段 8 重排**：schema 版本化随第 4 步一并落地；压缩与 ACL/审计按新键重述后实施；运维接口路由随实例段一并重设计。
7. **回归**：默认实例兼容层需保证既有 110 项测试与三接口 e2e 全绿；新增 β 专项测试（同域两实例并行互不干扰、声明替换不影响运行实例、按域聚合查询）。
8. **文档**：`docs/design.md` + `design-change`（新轮次）、`architecture.md` + `architecture-change`（存储布局与引擎会话表变更）、`manual.md`（实例概念、默认实例、路由与 CLI 用法）、`requirements.md`（如 NFR7 多租户边界措辞需澄清）。

**遗留待用户裁决（D-N 范围内，专家答复不影响）**：存储布局取"单层文件名"还是"域子目录"；默认实例名取值；实例创建时声明是"快照"还是"引用最新"。

---

## 附：本评估的事实来源

- 范式母本：`docs/pactv1_rev2.md`（`:56`、`:118-120`、`:165-179`、`:202-204`、`:261-273`、`:344-393`、`:409-413`、`:434-483`、`:498-511`、`:619-658`、`:684`、`:1036-1061`、`:1096-1105`、`:1206-1228`、`:1288+`、`:1432-1580`）
- 设计/架构：`docs/design.md`（D7/D9/D13/D19/D20/D23/D25、§7.1–§7.4、§8.1、§8.5、§9.2–§9.4、§11、§13）、`docs/architecture.md`（§1、§2.3、§5、§6）
- 代码：`crates/pact-store/src/{log.rs:107-114,error.rs:92-107,store.rs:36-63,141-190,index.rs:4-5,75-93,event.rs:8-41}`、`crates/pact-engine/src/{engine.rs:117-200,281,314-383,476-518,notify.rs:21}`、`crates/pact-http/src/{server.rs:218-291,config.rs:12-41,client.rs:128-175}`、`crates/pact-cli/src/main.rs:53-206`、`crates/pact-mcp/src/main.rs:58-140`、`crates/pact-runtime/src/{rest.rs:16,70,mcp.rs:51,orchestrator.rs:102,bin/pact-runtime.rs:25}`
- 阶段 8 规格（未获批）：根目录 `spec.md` / `tasks.md`
