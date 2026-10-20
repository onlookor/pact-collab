# PACT 用户手册

本手册面向 **PACT 范式系统的使用者**：要用它建模自己的协作流程、跑起来、看分析结果、做审计的人。
内容与任何具体技能、行业、方法论无关——需求工程、软件工程、agent 循环、跨工具编排都只是它的实例。

引用约定：

| 记号 | 指向 |
|------|------|
| `§x.y` | 范式规范 `docs/pactv1_rev3.md`（论文母本，本手册不复述其推导） |
| `设计 §x.y` | `docs/design.md`（实现决策与线上格式） |
| `架构 §x.y` | `docs/architecture.md`（进程与数据流视图） |
| `示例 §x` | `examples/README.md`（五份通用域 + 两条可复现轨迹） |
| `pact` | CLI 二进制，下文假定 `alias pact=./target/debug/pact` |

---

## 1. 这套系统给你什么

### 1.1 一句话

PACT 把你的协作流程写成一份 **域声明**（`.pact` 文件），然后由一个 **daemon** 持有唯一权威的
`Counter`（在制品集合），按六条规则裁决每一步动作：**谁**（Participant）**在什么前提下**
（Precondition）**能对什么**（Token 的 Type）**做什么**（Gate：read / consume / produce），
被消费的东西进 `Archive` 且不可逆。

它不替你干活，也不调度你。它只做三件事：**拒绝非法动作**、**记录每一步**、**告诉你现在能做什么**。

### 1.2 六原语与六规则

| 原语 | 是什么 | 在 `.pact` 里怎么写 |
|------|--------|---------------------|
| Token | 不可变的产出实例，有 ULID id、类型、前驱、生产者 | 不写——运行时产生 |
| Participant | 动作主体（人、agent、工具） | `gates` / `seed_authority` 段的主语 |
| Precondition | 类型级前置图，`Precond(T)` = 生产 T 所需的前驱类型集合 | `preconditions { A → B; {A + C} → D }` |
| Gate | 静态授权矩阵 `(参与者, 动作, 类型) ↦ 可选条件` | `gates { dev: reads design, consumes design, produces impl }` |
| Counter | 当前在场 token 集合 | 不写——运行时状态 |
| Archive | 已消费 token 的只增账本 | 不写——运行时状态 |

| 规则 | 效果 | 命令 | 关键约束 |
|------|------|------|----------|
| Rule-S 种子注入 | 无中生有放入一个 token | `pact seed` | 豁免前置条件，但必须 `(参与者, 类型) ∈ SeedAuthority` |
| Rule-P 生产 | 由前驱产出新 token | `pact produce` | **前驱必须仍在 Counter 上**，且类型集合恰等于 `Precond(T)`；前驱不被消费 |
| Rule-R 读取 | 取回 token 全量数据 | `pact read` | 非破坏；只对 Counter 上的 token 成立 |
| Rule-E 消费 | token 移出 Counter 进 Archive | `pact consume` | 显式、无级联、**不可逆** |
| Rule-W 自驱 | 问「我现在能生产什么」 | `pact enabled` / `pact_subscribe` | 引擎被动，不替你决定 |
| Rule-T 终止 | `Counter = ∅` 且无人可 Rule-P | `pact status` 的 `terminated` | 信息性判定，**不锁定域实例** |

三条最容易踩的性质：

1. **Rule-P 不消费前驱**。`design → implementation` 之后 `design` 仍在 Counter 上，
   可以被别人再读、再作为前驱。想让它消失必须显式 `consume`。
2. **Rule-E 无级联、不可逆**。没有撤销命令。消费顺序错了只能重载域重来。
3. **Rule-T 是信息，不是闸门**。域终止后不禁止动作，只是 Counter 空了让 Rule-P 拿不到前驱；
   再 `seed` 一次，`terminated` 就翻回 `false`（示例 §3 第 11 步实测）。

### 1.3 三个接口，一个权威

```
agent ── MCP (stdio) ─┐
应用  ── REST + SSE ──┼── 同一个 daemon 进程 ── Counter 唯一权威 ── JSONL 事件日志
人    ── CLI ─────────┘        （pact-mcp / pact-cli 的在线命令都是 REST 客户端）
```

三接口能力对等，共享同一套线上格式（设计 §6）。差别只在传输与形态；
唯一例外是 `compose` 的语义差异，见 §6.3。

### 1.4 什么时候该用（§2.1 五条判据）

五条**全部**满足才划算：

| # | 判据 | 含义 | 反例信号 |
|---|------|------|----------|
| 1 | Discrete Artifacts | 流程产物是可数的离散件（文档、报告、构建、工单） | 产物是连续量（温度、进度百分比） |
| 2 | Precedence Relations | 「B 必须在 A 之后」是流程的**本质**约束而非偏好 | 各步骤本来就能任意乱序 |
| 3 | Transferable Outputs | 产出要在参与者之间传递 | 单人自娱自乐、无交接 |
| 4 | Participant Count ≥ 1 | 至少一个参与者；**单参与者是合法退化情形** | — |
| 5 | Identifiable Entry Points | 每个源类型都能指认一个种子权威 | 说不清「第一份东西谁给」 |

第 5 条常被误读为「必须有源类型」。不是的：`examples/domains/react_loop.pact` 的四个类型各有
唯一前驱（纯循环，§1.4 意义下没有源类型），第 5 条**空满足**，但仍然需要
`seed_authority { user: seeds observation }` 点火——**SeedAuthority 不只服务于源类型，
它也是给纯循环域提供启动推力的手段**（seed 豁免前置条件，§1.4.3）。

### 1.5 什么时候别用（§2.3 九条边界）

| 你的需求 | 为什么不适用 | 绕法 |
|----------|--------------|------|
| 连续过程（信号、流量、进度） | 范式是离散 token 的 | 离散化为里程碑 token |
| 实时约束（超时、截止、时延） | 无时钟原语，`Timestamp` 只是记录 | 超时逻辑放参与者侧，用 `thing_ref` 挂外部工单 |
| 概率分支（按 likelihood 走不同路） | 规则是确定性的 | 概率决策留在参与者内部，Counter 只承接其结果 |
| OR 分支（A **或** B 都能触发 C） | 前置图只支持合取（§1.4.4） | 拆成两个类型 `C_from_A` / `C_from_B`，或统一到一个前驱类型 |
| 产物的内部结构（章节、字段、AST） | Token 不可变、内部不透明 | 用 `ThingRef` 委派给外部系统 |
| 自动生命周期排序（自动推进到下一阶段） | 设计如此（§1.6，零隐式动作） | 由参与者显式 produce/consume；agent 用 Rule-W 自驱 |
| 生产时的额外 gate 条件 | 设计如此（§1.5.2，`produces` 子句不带条件） | 把约束表达进前置图（增加前驱类型） |
| 两次消费之间的时序（先消费 A 才能消费 B） | `GateExpr` 只评估当前状态（§1.7.3） | 把时序编码进类型与前置图 |
| 内部生产源类型 | 设计如此（§1.4，源类型只经 Rule-S 进入） | 给它指定 seed 权威，或改成有前驱的非源类型 |

「设计如此」的四行不是缺陷清单，是范式的边界声明：PACT 刻意不做隐式动作，
否则 Counter 就不再是唯一可信的协调介质。

---

## 2. 装起来、跑起来

### 2.1 构建

```bash
cargo build -p pact-cli        # → target/debug/pact（含 daemon start）
cargo build -p pact-http       # → target/debug/pactd（裸 daemon，等价于 pact daemon start）
cargo build -p pact-mcp        # → target/debug/pact-mcp（stdio MCP server，由 agent 拉起）
```

### 2.2 配置文件 `pact.toml`

```toml
data_dir    = "./data"            # 事件日志 / 快照 / 索引目录
listen_unix = "./pact.sock"       # 默认监听；单机安全
# listen_tcp  = "127.0.0.1:8787"  # 可选，与 listen_unix 可同时启用

[mcp]
enabled = true

[auth]                            # 可选；整段缺省 = 信任声明模式
[auth.tokens]
architect = "sekret"              # 参与者 → 令牌，请求头 Authorization: Bearer
```

缺省语义（文件不存在或字段缺省时）：`data_dir = "./data"`、`listen_unix = "./pact.sock"`、
无 TCP、无令牌表。两个易踩点：

- 配置用 `deny_unknown_fields`，**写错键名会直接报错**，不会静默忽略。
- `listen_unix = ""`（空串）= 禁用 unix socket；只写 `listen_tcp` 而不写 `listen_unix`，
  unix socket 也一并禁用。

### 2.3 启动与停止 daemon

```bash
pact daemon start --config pact.toml      # 前台运行；Ctrl-C 停止
```

**没有 `daemon stop` 子命令。** 停止就是终止进程（`kill <pid>`）；
`examples/online/run.sh` 的 `cleanup()` 就是这么做的，并用 `trap ... EXIT` 保证兜底。
状态不会丢：JSONL 是真相源，下次启动自动做「快照 + 其后事件重放」恢复（架构 §2.3）。

### 2.4 身份与令牌

- **信任声明模式**（默认）：`[auth.tokens]` 缺省或为空表，任何 `--participant` 值都被接受。
  身份校验退化为「你说你是谁就是谁」，权限裁决仍由 Gate 与 SeedAuthority 完成。
- **令牌模式**：令牌表非空时，参与者必须在表内且 `Authorization: Bearer <token>` 匹配，
  否则 `401 Unauthorized`。比较用恒时算法（防 CWE-208 时序侧信道）。

身份在**命令进入引擎之前**由接口层校验；核心与引擎不感知认证方式（设计 §9.4，NFR5）。

---

## 3. 写一份域声明

### 3.1 四段文法（§3.1，段序严格）

```pact
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    {test_report + security_report} → deployment   // 多前驱合取
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    reviewer:  reads implementation,
               consumes test_report via implementation,   // 糖形式，脱糖为 when via implementation
               produces test_report when count implementation >= 1;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
```

宽容度与扩展（设计 §3，D16）：段内尾分号可省、`preconditions`/`gates`/`seed_authority` 可为空段
（`types` 至少一项）、`->` 等价 `→`、`//` 行注释、`GateExpr` 优先级 `not > and > or`。
`via T_a` 在解析期脱糖为 `when via T_a`，AST 唯一。

### 3.2 四个注入点（§2.2）

领域知识**只**从这四个地方进入范式，其余部分是领域无关的固定机制：

| 注入点 | 你注入什么 | 对应段 |
|--------|-----------|--------|
| TypeSchema | 有哪些产物类型 | `types` |
| PreconditionGraph | 类型之间的先后依赖 | `preconditions` |
| GateMatrix | 谁能对什么做什么，附加什么条件 | `gates` |
| SeedAuthorityDecl | 谁能无中生有 | `seed_authority` |

可选的第五点是 **Archival Policy**（`AP: Counter → 2^Counter`）：它经正常 Rule-E 执行，
不引入新原语、不引入隐式系统动作。**本实现不内置**（`requirements.md` §6 范围外）；
需要归档策略时，把它实现为一个持有相应 `consumes` gate 的参与者。

### 3.3 良构约束（装载前就会被拒的八类）

| 错误码 | 触发条件 |
|--------|----------|
| `DuplicatePrecondRule` | 同一类型出现两条前置规则（`Precond` 必须是单值函数） |
| `DuplicatePrecondLhs` | 同一规则 LHS 里类型重复，如 `{A + A} → B` |
| `EmptyPrecondLhs` | `{ } → B`，空 LHS |
| `UndeclaredType` | `preconditions` / `gates` / `seed_authority` 引用了 `types` 未声明的类型 |
| `ZeroCountLiteral` | `count T >= 0` 之类非 PosNat 字面量（0 或前导零） |
| `ProduceWithCondition` | `produces T when …`——生产时刻不允许附加条件（§1.5.2） |
| `SourceTypeProduceGate` | 给源类型写 `produces`（源类型只能经 Rule-S 进入，§1.4） |
| `DuplicateGateClause` | 同一 `(参与者, 动作, 类型)` 重复声明 |

解析期就拒绝的两类：空 LHS、非 PosNat 的 count 字面量。
其余在良构层（`validate_domain`）拒绝。**解析返回全部问题，不是首错即停。**

### 3.4 装载前自检

```bash
pact analyze my_domain.pact        # 离线：六项静态分析（§5），不需要 daemon
pact domain validate my_domain.pact # 在线：解析 + 良构 + 装载预演，不留下状态
pact domain load my_domain.pact [--name N]   # 缺省域名 = 文件 stem
```

> `domain validate` 是**在线**命令（走 REST）。没有 daemon 时它会报
> `error [Transport]: client error (Connect)`。想纯离线校验，用 `pact analyze`
> 看首行 `precondition_wf`。

---

## 4. 跑一次流程

### 4.1 唯一正确的动作顺序

```
read（看清楚） → produce --predecessors <前驱 id>（产出新件） → consume（把用尽的移走）
```

颠倒 produce 与 consume 的后果是确定性的：前驱一旦离席，`produce` 立刻报
`PredecessorNotOnCounter`，而且**没有撤销命令**——已经消费的 token 回不来。
示例 §3 第 12 步专门演示了这个陷阱：把顺序换一下，域卡在
`counter=1 archive=3 terminated=false`，既没终止也走不下去。

### 4.2 六条规则对应的命令

```bash
# Rule-S：种子注入（豁免前置，受 SeedAuthority 约束）
pact seed    --domain D --participant external_stakeholder --type requirement \
             [--thing-ref JIRA-123]

# Rule-P：生产（--predecessors 逗号分隔，可多值；类型集合须恰等于 Precond(T)）
pact produce --domain D --participant architect --type design \
             --predecessors 01M500J902XFZ23Q1CWWA3H3S2 [--thing-ref doc://design]

# Rule-R：读取（非破坏）
pact read    --domain D --participant reviewer --token 01M500J9…

# Rule-E：消费（不可逆，入 Archive；可选 --request-id 幂等去重）
pact consume --domain D --participant architect --token 01M500J9… [--request-id RETRY-2F9C]
```

`--thing-ref` 是产物内部结构的**唯一**出口：Token 本身不可变、不透明，
把外部系统的引用（文件路径、工单号、URL）挂在这里。

### 4.3 看清当前局面

```bash
pact counter ls --domain D [--type T]        # 在场 token
pact archive ls --domain D [--type T]        # 已消费 token
pact gates show --domain D --participant P   # 我的全部授权（含条件）
pact enabled    --domain D --participant P   # Rule-W：现在能生产什么 + 见证前驱
pact status     --domain D                   # counter / archive / terminated
```

`enabled` 是 agent 自驱的核心：它返回 `{type, witness_predecessors[]}` 列表，
拿到 witness 就能直接喂给 `produce --predecessors`，不必自己推前置图。

### 4.4 Rule-W：等待与唤醒

引擎被动（D5），不轮询你、不催你。两种自驱方式：

- **订阅推送**：`GET /domains/{d}/events`（SSE）或 MCP `pact_subscribe`；
  Counter 每次变更推一条通知（设计 §6.4）：

  ```json
  { "event": "Produced",        // Seeded | Produced | Consumed | Terminated
    "domain": "se_lifecycle", "seq": 42, "ts": "2026-10-15T14:48:23Z",
    "token_id": "01M500J9…",   // Terminated 时省略
    "summary": { "counter_size": 3 } }
  ```

  MCP 侧经 `notifications/pact/event` 转发。
- **轮询兜底**：`pact enabled` / `pact status`。

`Terminated` 是**一次性**推送。写自驱 agent 时要注意两个已知竞态（设计 §11 阶段 5 的修复记录）：
播种前 Counter 为空会让终止判定平凡为真（应要求 `archive_size > 0` 才承认终止）；
终止推送与轮询判定之间需要宽限窗口排空在途事件。

### 4.5 消费的原子性与幂等

引擎对每一次 Rule-E 的承诺（设计 §8.5、决策 D23）：

- **提交点 = 事件日志落盘（fsync）**。成功返回即意味着该次消费已进入不可变日志；
  其后的索引投影、快照、通知等派生步骤即使失败，也不会回滚、不会改变返回结果。
- **原子域包含 GateHolds 的读集**。`via` / `exists` / `count` / `not` 读到的 Counter 子集，
  与「目标 token 移出 Counter + 写入 Archive」在同一次判定里完成，不存在"检查通过后被人抢先消费"的 TOCTOU。
- **至多一次**。同一 token 二次消费报 `TokenNotOnCounter`；要安全重试请带 `--request-id`：

  ```bash
  pact consume --domain D --participant architect --token ID --request-id RETRY-2F9C
  ```

  同一域内重复的 `request-id` 直接返回**首次**的归档结果（不重复消费），
  因此响应丢失后可以原样重发。该幂等表随事件日志持久化，**跨重启仍生效**。
  同一 `request-id` 却换了 token，会返回 `IdempotencyConflict`（HTTP 409）——那是键复用错误，不是消费失败。

### 4.6 消费与外部动作：物化确认

引擎**不执行**外部副作用：`ThingRef` 对它不透明，它也不校验指向物是否存在。
所以"`consume` 成功"与"外部动作（产物真正生成/落地）成功"是**两个独立承诺**，
二者最终一致而非同一事务——消费成功既不等待、也不回滚外部动作。

要让"产物就位"成为消费的**准入条件**，用 **witness-token 模式**：
外部系统确认就位后经 Rule-S 注入一枚 token（如 `readiness`），
消费方 gate 写成 `consumes work_item when exists readiness`。
于是"就位确认"进入 GateHolds 读集，与消费共享同一次判定；未就位则 `GateConditionFailed` 拦下。
示例见 `examples/domains/external_materialization.pact`。

若选择**先消费、后补做**（saga 式），则由参与者以一次新的 Rule-P 记录"未完成/需重做"事实——
历史不可回滚，补偿是追加的新事实，而不是反向操作。

两种方案**可并用、不互斥**：同一域可对不同入口分别选用（就位可确认的入口用 witness-token，
无法预先确认的入口用 saga 补偿）。

---

## 5. 六项静态分析

`pact analyze <file>`（离线）或 `POST /domains/{d}:analyze`（在线，分析已装载域）或
MCP `pact_analyze`。聚合报告里 **A.5 排首位**，因为良构性是其余五项的前提（Appendix D）：
先修违规，再解读其余结果。

| 分析 | 回答的问题 | 输出 | 阻断装载？ |
|------|-----------|------|-----------|
| A.5 `precondition_wf` | §3.1 良构约束满足吗 | `{well_formed, violations[], warnings[]}` | **是**（violations 非空即拒） |
| A.1 `reachability` | 每个类型能从种子权威类型沿前置图到达吗 | `{seed_types[], reachable[], unreachable[]}` | 否（咨询） |
| A.2 `gate_coverage` | 每个**非源类型**至少有一个 producer 吗 | 无 producer 的类型清单 | 否（咨询） |
| A.3 `consumption_reachability` | 每个类型（**含源类型**）至少有一个 consumer 吗 | 无 consumer 的类型清单 | 否（咨询） |
| A.4 `structural_deadlock` | 前置图里有「无种子入口的强连通分量」吗 | `DeadlockComponent[]` | 否（咨询） |
| A.6 `stranding_risk` | `via` 见证会不会先于依赖它的动作被消费 | `StrandingRisk[]`（带 `potential` 标志） | 否（咨询） |

**退出码**：语法错误或良构违规 → `1`；其余情况（含 unreachable / stranding 等咨询性发现）→ `0`。
咨询性发现不改退出码，是否接受由建模者裁决——CI 里要拦就自己解析 JSON。

三项咨询分析怎么读：

- **A.3 未消费类型**常是「本来就不该有消费者」。示例 `re_pipeline` 的 `baseline` 是常驻配置资产，
  被反复读取而不被耗尽；A.3 只看 GateMatrix 有没有 `consumes` 子句，无法区分「忘了写」和
  「本就不该写」，所以如实报告。也可能是真的技术债（Part II §1.6）。
- **A.4 结构死锁**的两个豁免：SCC 里有成员出现在 `SeedAuthority`（可点火），
  或存在来自 SCC 外部且种子可达的入边（可馈入）。两者都不满足才报。
- **A.6 搁浅风险**的 `potential` 字段：前置图**有环**时为 `true`（保守报告
  `POTENTIAL_STRANDING`），**无环**时为 `false`（确定性报告 `STRANDING_RISK`）。
  只有 `via` 引用的类型是**直接**前驱时才有风险；间接祖先的 `via` 永不可满足，不算风险。

### 5.1 术语陷阱：两个「源类型」

同一份 `analyze` 输出里，`reachability` 与 `gate_coverage` 说的**不是同一个东西**：

| 出处 | 实际含义 | JSON 键名 |
|------|----------|-----------|
| `reachability: N 源类型` | **SeedAuthority 里的类型**（`{T' : ∃p, (p,T') ∈ SeedAuthority}`） | `seed_types` ✓ 准确 |
| `gate_coverage: N 个无 producer 的非源类型` | **§1.4 源类型**（`Precond(T) = ∅`）被排除 | 按 `is_source_type` 判定 ✓ |

CLI 的人类可读输出把前者打印成「源类型」，是措辞不准。`security_review.pact` 单独分析是活证据：

```
reachability: 0 源类型，0 可达，2 不可达
gate_coverage: 0 个无 producer 的非源类型
```

`gate_coverage` 报 0，正因为 `implementation` 的 `Precond` 是空集、按 §1.4 **是**源类型而被排除；
`reachability` 报「0 源类型」，则因为该片段没有 `SeedAuthority`。**机读一律以 JSON 键名为准。**

### 5.2 一个警告：`SEED_PRODUCE_OVERLAP`

A.5 的 warnings 报「某参与者对非源类型 T 持 seed 权威，且 T 存在 produces gate」。
这在规范上是**安全**的（seed 实例可作前驱被消费，produce 路径产出新实例），
但通常是建模混淆或需要显式部署裁决的信号，应复核意图。

实现是**类型级**判定（produces gate 取全体参与者的并集，不要求与 seed 持有者同一参与者），
比 §1.4.3「同一参与者」的表述更宽——跨参与者重叠也告警。偏差方向是假阳性、不漏报，
登记为规范的安全超集（设计 D21）。

---

## 6. 片段组合

大域可以拆成片段并行开发，再组合。`examples/fragments/` 下有一对演示片段
（`security_review.pact` + `security_blocking.pact`）。

### 6.1 四条组合条件（§1.8）

| 条件 | 要求 | 冲突码 |
|------|------|--------|
| C1 | 前置唯一性保持：合并后每个类型仍只有一条前置规则 | `PreconditionMismatch` |
| C2 | 共享类型的前置一致 | `PreconditionMismatch` |
| C3 | 共享 `(参与者, 动作, 类型)` 的 gate 条件必须**语法同一** | `GateConditionMismatch` |
| C4 | 合并结果仍须通过 `validate_domain` | `IllFormed`（内含 §3.4 的 `DomainError` 变体） |

C1+C2 的判定要点：**只有双方都为同一 rhs 声明了前置规则且集合不同**才报冲突。
**单侧声明是合法的**（D18）——缺席方不构成约束，合并取并集，这是 §1.8 示例的语义。

### 6.2 用法

片段是**挂在宿主域上的增量**，不是彼此独立拼装的两半——组合命令的第一个参数通常是完整宿主域。

成功合并（给 `se_lifecycle` 挂一条安全扫描支线；下为实测输出，`gates` 段有删节，退出码 0）：

```bash
pact compose examples/domains/se_lifecycle.pact examples/fragments/security_review.pact
```

```
types { deployment; design; implementation; requirement; security_report; test_report }

preconditions {
    test_report → deployment;
    requirement → design;
    design → implementation;
    implementation → security_report;
    implementation → test_report;
}

gates {
    …（八个参与者的 gate 子句按参与者名排序合并）
    security_scanner: reads implementation, produces security_report;
    …
}

seed_authority {
    external_stakeholder: seeds requirement;
}
```

C2 冲突被机械拦下（同一宿主域，换一个对 `deployment` 前置理解不同的片段，退出码 1）：

```bash
pact compose examples/domains/se_lifecycle.pact examples/fragments/security_blocking.pact
```

```
error [ComposeConflict]: 片段组合冲突（1 处）
```

`--json` 给出可机读的冲突明细（`detail.conflicts[]` 是 `Conflict` 变体的 Debug 串）：

```json
{"error":{"code":"ComposeConflict","detail":{"conflicts":["PreconditionMismatch { ty: TypeName(\"deployment\"), pre_a: {TypeName(\"test_report\")}, pre_b: {TypeName(\"security_report\"), TypeName(\"test_report\")} }"]},"message":"片段组合冲突（1 处）"}}
```

读法：宿主认为 `deployment` 只需 `test_report`，片段认为还需 `security_report`——这就是 C2 要拦的分歧。

每个输入片段会被**单独**跑一遍良构校验（含类型闭包），所以**片段必须自足声明它引用的全部类型**。
组合是离线操作，不需要 daemon。

### 6.3 CLI 与 MCP/REST 的语义差异（重要）

| 接口 | 行为 |
|------|------|
| CLI `pact compose f1 f2 …` | 只把合并后的域声明文本**打印到 stdout**，不装载 |
| REST `POST /domains/{name}:compose` | 调 `compose_and_load`，**装载为命名域**，返回 `DomainSummary` + `201` |
| MCP `pact_compose` | 同 REST，**会装载** |

也就是说 MCP 的 `pact_compose` 是有副作用的。想只看不装，用 CLI。

---

## 7. 审计、事件溯源与重放

### 7.1 存储布局（设计 §7.2）

```
<data_dir>/
  <domain>/                        # 每个域声明一个子目录
    <instance>.events.jsonl        # 实例级真相源，只追加
    <instance>.snapshot.json       # 定期快照（seq + CounterState），原子写（tmp + rename）
    declaration.pact               # 声明原文（启动恢复时回填声明注册表）
  index.tursodb                    # 全部实例共用的查询索引（派生物，(domain, instance) 双列）
  index.tursodb-wal
  pactd.lock                       # data_dir 独占 flock（单写者护栏）
```

**JSONL 是真相源，SQLite 索引与快照都是派生物。** 索引丢了会重建；JSONL 中部损坏则 fail-fast。

**实例布局（D27）**：实例名作为**路径段**承载，默认实例名为 `default`——`<domain>/default.events.jsonl` 即域的默认实例。顶层遗留的单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**：启动恢复时直接 fail-fast（`LegacyLayoutDetected`），须人工迁移，不静默丢数据。同一 `data_dir` 上 daemon 以 `<data_dir>/pactd.lock` 独占 flock 保证单写者，第二个实例启动即被拒。

### 7.2 事件类型（设计 §7.1）

```rust
DomainLoaded { name, declaration }   // 声明原文存档，可复现解析
Seeded     { seq, ts, token }
Produced   { seq, ts, token }        // token 内含 depends_on / produced_by
Consumed   { seq, ts, token, by, request_id? }  // token 全量自足；request_id 可选幂等键（D23）
Terminated { seq, ts }               // Rule-T 检测结果，信息性
```

`seq` 全局单调。**Rule-R 不持久化**（D9：读取无状态变更，重放不需要）——
所以「谁读过什么」不在事件日志里，需要读审计得自己在外层记。

### 7.3 离线重放

```bash
pact replay <data_dir>/se_lifecycle.events.jsonl
# → replayed 10 events (domain se_lifecycle): counter=0 archive=5
```

不需要 daemon。重放结果与线上状态逐字段等价（集成测试锚定）。
崩溃恢复走同一条路径：最新快照 + 其后的事件重放；JSONL 末行半截（写入中途崩溃）可被安全跳过续作。

---

## 8. 接口参考

### 8.1 CLI

全局 flag（对所有子命令生效）：

| flag | 说明 |
|------|------|
| `--unix <PATH>` | unix socket 路径，缺省 `./pact.sock` |
| `--tcp <ADDR>` | TCP 地址（如 `127.0.0.1:8787`）；**设置时优先于 `--unix`** |
| `--bearer <TOKEN>` | 参与者令牌（`Authorization: Bearer`）；信任声明模式下可省 |
| `--json` | 以 §6 线上格式输出 JSON |

子命令（**在线** = 必须有 daemon；**离线** = 纯函数层，不经 daemon）：

| 命令 | 归属 | 说明 |
|------|------|------|
| `daemon start [--config PATH]` | — | 前台启动 daemon，缺省配置 `pact.toml` |
| `domain validate <FILE>` | 在线 | 解析 + 良构 + 装载预演，不留状态 |
| `domain load <FILE> [--name N] [--instance I] [--force]` | 在线 | 装载/替换命名域（默认实例 `default`），缺省名 = 文件 stem |
| `domain list` | 在线 | 已装载域清单（打印 `name [instances…]`） |
| `domain info --domain D [--instance I]` | 在线 | 域信息 + 状态摘要 |
| `domain unload --domain D [--instance I]` | 在线 | 卸载（内存态；事件日志保留） |
| `domain instantiate --domain D --instance I` | 在线 | 在同一域声明下**新建实例**（D27） |
| `domain instances --domain D` | 在线 | 打印该域的实例清单 |
| `seed --domain D --participant P --type T [--thing-ref R] [--instance I]` | 在线 | Rule-S |
| `produce --domain D --participant P --type T [--predecessors a,b] [--thing-ref R] [--instance I]` | 在线 | Rule-P |
| `read --domain D --participant P --token ID [--instance I]` | 在线 | Rule-R |
| `consume --domain D --participant P --token ID [--instance I]` | 在线 | Rule-E |
| `counter ls --domain D [--type T] [--instance I]` | 在线 | Counter 查询 |
| `archive ls --domain D [--type T] [--instance I]` | 在线 | Archive 查询 |
| `gates show --domain D --participant P [--instance I]` | 在线 | 我的 gate 子句（含条件） |
| `enabled --domain D --participant P [--instance I]` | 在线 | Rule-W：当前可执行的生产 |
| `status --domain D [--instance I]` | 在线 | Counter 概要 + `terminated` |
| `analyze <FILE>` | **离线** | 六项静态分析 |
| `compose <FILES…>` | **离线** | 片段组合（≥2 个），打印合并文本 |
| `replay <FILE>` | **离线** | 重放 `<domain>.events.jsonl` |

**实例 flag（D27）**：所有域操作命令的 `--instance I` 可选，**缺省 = 默认实例 `default`**——既有不带该 flag 的调用与脚本语义不变。`--force` 仅用于对**非空实例**（`last_seq > DomainLoaded.seq`）的再加载：默认拒绝（`DomainBusy`），加 `--force` 才重置其 Counter。

输出与退出码：成功 → `0`；失败 → `1`，人类模式把 `error [<code>]: <message>` 写 **stderr**，
`--json` 模式把 §6.3 错误体写 **stdout**。

### 8.2 REST

监听 unix socket（默认）与/或 TCP。全部载荷取 §6 线上格式。

| 方法与路径 | 请求体 | 成功响应 |
|-----------|--------|----------|
| `POST /validate` | `{declaration}` | `200` `{ok, types[], participants[], gates, seed_authority}` |
| `POST /domains` | `{name, declaration[, instance][, force]}` | `201` `DomainSummary{name, instance, types, gates, seed_authority, last_seq}`（declare + instantiate，缺省实例 `default`） |
| `GET /domains` | — | `200` `{domains: [{name, instances[]}]}` |
| `POST /domains/{d}/instances` | `{instance}` | `201` `DomainSummary`（在同一域声明下新建实例，D27） |
| `GET /domains/{d}` | — | `200` `{name, counter_size, archive_size, terminated}`（`default` 实例） |
| `DELETE /domains/{d}` | — | `204` |
| `POST /domains/{d}:analyze` | — | `200` `AnalysisReport`（六项，A.5 首位） |
| `POST /domains/{d}:compose` | `{fragments: [...]}` | `201` `DomainSummary`（**会装载**到 `default` 实例） |
| `POST /domains/{d}/seeds` | `{participant, type, thing_ref?}` | `201` `{token}` |
| `POST /domains/{d}/tokens` | `{participant, type, predecessors[], thing_ref?}` | `201` `{token}` |
| `POST /domains/{d}/tokens/{id}/read` | `{participant}` | `200` `{token}` |
| `POST /domains/{d}/tokens/{id}/consume` | `{participant}` | `200` `{entry}` |
| `GET /domains/{d}/counter?type=` | — | `200` `{tokens: [...], size}` |
| `GET /domains/{d}/archive?type=` | — | `200` |
| `GET /domains/{d}/gates/{participant}` | — | `200` `{gates: [...]}` |
| `GET /domains/{d}/enabled?participant=` | — | `200` `{options: [{type, witness_predecessors[]}]}` |
| `GET /domains/{d}/status` | — | `200` `{counter_size, archive_size, terminated}` |
| `GET /domains/{d}/events` | — | SSE 流（含 keep-alive），每条 `data:` 是 §6.4 通知 |

**实例路由（D27）**：上表 `/domains/{d}/…` 的语义 = **`default` 实例**。实例级为 `/domains/{d}/instances/{i}/…` 镜像组——seeds / tokens / tokens/{id}/read / tokens/{id}/consume / counter / archive / gates/{participant} / enabled / status / events 全套同形。实例名谓词 `[A-Za-z0-9_-]{1,128}`（同域名），非法名返回 `400`。

实现注记：`{d}:analyze` / `{d}:compose` 因路由库不支持同段「参数 + 字面量」，
在 `/domains/{d}` 统一入口内按段内冒号后缀分发，**对外 URL 不变**。

### 8.3 MCP（stdio）

```bash
pact-mcp --unix <sock> [--bearer <token>]     # 或 --tcp <addr>（优先）
```

由 agent 拉起为子进程，工具经 REST 转发至 daemon；schema 由 serde + schemars 派生；
错误以 §6.3 结构化 JSON 返回（`CallToolResult.is_error = true`），agent 可编程处理
（例如 gate 被拒后转入等待）。

| 工具 | 语义 |
|------|------|
| `pact_domain_validate` | 校验声明文本良构，返回结构概要（不加载） |
| `pact_domain_load` | 加载/替换命名域（幂等按 name；可选 `instance` / `force`） |
| `pact_domain_instantiate` | 在同一域声明下**新建实例**（D27，可选 `instance`） |
| `pact_list_domains` | 已加载域清单（每域含 `instances[]`） |
| `pact_analyze` | 六项静态分析聚合报告（A.5 前置） |
| `pact_compose` | 组合多片段**并加载为命名域**（有副作用，见 §6.3；作用于 `default` 实例） |
| `pact_seed` | Rule-S |
| `pact_produce` | Rule-P（显式传前驱 token id 列表） |
| `pact_read` | Rule-R（返回 token 全量数据，不改 Counter） |
| `pact_consume` | Rule-E（移出 Counter，归档） |
| `pact_counter_query` / `pact_archive_query` | 状态查询（可按类型过滤） |
| `pact_my_gates` | 参与者的全部 gate 子句（含条件），agent 自省 |
| `pact_enabled` | Rule-W 轮询：当前可满足的 Rule-P 组合 |
| `pact_status` | Counter 概要 + `terminated` 判定 |
| `pact_subscribe` | 注册 Counter 变更通知；事件经 `notifications/pact/event` 推送 |

共 **16** 个工具。除 `pact_domain_validate` / `pact_analyze` / `pact_compose` / `pact_list_domains` 外，全部工具接受可选 `instance` 参数（缺省 = `default` 实例，D27）。

---

## 9. 错误码表

线上错误体统一为（§6.3）：

```json
{ "error": { "code": "SeedAuthorityDenied",
             "detail": { "participant": "architect", "ty": "requirement" },
             "message": "(architect, requirement) ∉ SeedAuthority：无种子注入权" } }
```

`code` = 枚举变体名，**机器可判**；`detail` 携带结构化字段；`message` 面向人类。
被拒动作一律整体失败，**无部分效果**。

### 9.1 规则层（HTTP 409 Conflict）

| code | 含义 |
|------|------|
| `StaticGateDenied` | `(参与者, 动作, 类型)` 不在 GateMatrix 里 |
| `GateConditionFailed` | gate 存在但其 `GateExpr` 在当前 Counter 上不成立（如 `via` 见证已被消费） |
| `PreconditionMismatch` | 提供的前驱类型集合 ≠ `Precond(T)` |
| `SourceTypeProductionDenied` | 试图 produce 源类型（只能经 Rule-S） |
| `PredecessorNotOnCounter` | 前驱 token 不在 Counter 上（多半已被消费） |
| `DuplicatePredecessor` | 前驱列表里有重复 token id |
| `TokenNotOnCounter` | read/consume 的目标 token 不在 Counter 上 |
| `SeedAuthorityDenied` | `(参与者, 类型) ∉ SeedAuthority` |

### 9.2 声明层（HTTP 400 Bad Request）

`code = "Syntax"`（`detail.errors[]`，含行列位置与期望符号）或
`code = "WellFormedness"`（`detail[]` 是 §3.4 八类 `DomainError` 变体的结构化形式）。
两者都**一次返回全部问题**。

### 9.3 组合层（HTTP 409 Conflict）

`code = "ComposeConflict"`，`detail.conflicts[]` 是冲突的 Debug 串，形如：

```
PreconditionMismatch { ty: TypeName("deployment"),
  pre_a: {TypeName("test_report")},
  pre_b: {TypeName("security_report"), TypeName("test_report")} }
```

### 9.4 接口与运行时层

| code | HTTP | 触发 |
|------|------|------|
| `IdempotencyConflict` | 409 | 同一 `request_id` 已用于另一 token 的消费（复用幂等键，D23） |
| `InstanceNotFound` | 404 | 实例未加载（`domain/instance` 不存在或未 `instantiate`，D27） |
| `DomainNotDeclared` | 404 | 目标域无声明登记（实例化前须先 declare/load） |
| `InstanceExists` | 409 | 实例已存在（重复 `instantiate`） |
| `DomainBusy` | 409 | 对**非空实例**（`last_seq > DomainLoaded.seq`）再加载；须显式 `force` 才重置 |
| `InvalidDomainName` | 400 | 域名不匹配 `[A-Za-z0-9_-]{1,128}`（防路径穿越） |
| `InvalidInstanceName` | 400 | 实例名不匹配 `[A-Za-z0-9_-]{1,128}`（防路径穿越） |
| `Unauthorized` | 401 | 令牌模式下列表无此参与者，或 Bearer 缺失/不匹配 |
| `NotFound` | 404 | 路径不存在 |
| `MethodNotAllowed` | 405 | 方法与路径不匹配（如 `GET /domains/{d}:analyze`） |
| `BadRequest` | 400 | JSON 请求体解析失败 |
| `Store` | 500 | 存储写入失败；**含顶层遗留单层文件的启动 fail-fast**（`LegacyLayoutDetected`，须人工迁移） |
| `Recovery` | 500 | 启动恢复失败 |
| `EngineStopped` | 503 | 引擎已停止 |
| `Transport` | —（仅 CLI/MCP） | 连不上 daemon：`client error (Connect)` |
| `Io` | —（仅 CLI） | 读取 `.pact` / `.jsonl` 文件失败，`detail.path` 是路径 |
| `Usage` | —（仅 CLI） | 命令用法错误，如 `compose` 少于两个片段 |
| daemon 启动拒绝 | —（进程退出） | `data_dir` 已被另一 daemon 独占（`pactd.lock` flock 失败，D27） |

---

## 10. 故障排查

| 症状 | 原因 | 处置 |
|------|------|------|
| `error [Transport]: client error (Connect)` | daemon 没起，或 `--unix`/`--tcp` 指错 | 起 daemon；确认 socket 路径与配置一致 |
| 在线命令都失败，`analyze` 却正常 | 你在用离线命令的预期跑在线命令 | 见 §8.1 归属列；`domain validate` 是**在线**的 |
| `PredecessorNotOnCounter` | 前驱已被消费（顺序错了） | 无撤销；`domain unload` + `load` 重来，或重新 seed |
| 装载后 `terminated=true` | Counter 为空即平凡满足 Rule-T | 正常。`seed` 之后翻回 `false` |
| `GateConditionFailed: … Via(TypeName("x"))` | `via x` 的见证 token 已被消费 | 调整消费顺序，或改用不带 `via` 的 gate |
| `IdempotencyConflict` | 复用了别的消费已经用过的 `request_id` | 换一个新的幂等键重发；同一键只对应一次消费（D23） |
| `SourceTypeProduceGate` | 给 `Precond = ∅` 的类型写了 `produces` | 改成 `seed_authority`，或给它加前驱 |
| `pact compose` 报 `UndeclaredType` | 片段引用了未在自己 `types` 段声明的类型 | 片段必须自足（§6.2） |
| 停不掉 daemon | 没有 `daemon stop` | `kill <pid>`（§2.3） |
| 索引文件损坏/丢失 | `index.tursodb` 是派生物 | 删掉重启，会按 JSONL 重建 |
| `analyze` 说 `unreachable` 但域能跑 | 片段单独分析时缺少 `SeedAuthority` | 组合后再分析（示例 §1.2） |
| 启动报 `Store` / `LegacyLayoutDetected` | `data_dir` 顶层有旧版单层文件（`<domain>.events.jsonl`） | 手工迁移到 `<domain>/default.events.jsonl` 后重启（§7.1，不自动迁移） |
| `DomainBusy` | 对非空实例再 `load`（会重置 Counter） | 确认要替换则加 `--force`（CLI）/ `force: true`（REST）；否则换实例名 |
| daemon 启动即退出（锁冲突） | 同一 `data_dir` 已有另一 daemon | 停掉旧实例，或换 `data_dir`（`pactd.lock` 独占，§7.1） |

---

## 11. 已知偏差与限制

如实登记，均不影响范式语义：

1. **CLI 措辞**：`reachability` 那行把 `seed_types` 打印成「源类型」，与同一份输出里
   `gate_coverage` 用的 §1.4 源类型定义冲突（§5.1）。机读以 JSON 为准。
2. **MCP 工具描述**：`pact_analyze` 的 description 写「五项静态分析」，实际是六项 A.1–A.6
   （`docs/design.md` §9.1 的表已是六项）。
3. **`compose` 语义不对称**：CLI 只打印，MCP/REST 会装载（§6.3）。
4. **`SEED_PRODUCE_OVERLAP` 是类型级判定**，比 §1.4.3 参与者级更宽（假阳性方向，不漏报）；
   作为规范的安全超集登记于设计 D21，不回灌论文母本。
5. **Rule-R 不入事件日志**（D9），读审计需在外层自行记录。
6. **Archival Policy 未内置**（§3.2），需要时用一个持 `consumes` gate 的参与者表达。
7. **无 `daemon stop`**，无进程管理（PID 文件、systemd unit 均未提供）。
8. **v1 传输范围**：MCP 仅 stdio，无 Streamable HTTP；daemon 单机单进程，无联网协作（D20，架构 §4）。
9. **旧日志不自动迁移**（D27）：双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件启动即 `LegacyLayoutDetected` fail-fast，须人工迁移（§7.1），不静默丢数据。
10. **跨实例不协同**（D27）：同一域声明的多个 Counter 实例彼此隔离（各自 Counter/Archive/订阅）；跨实例协同不在范式保障范围内（论文 rev3 §1.3 实现自由条款，见 §1.3）。

---

## 12. 上手路径

1. 读 `examples/README.md` §1–§2，跑两条脚本：

   ```bash
   bash examples/offline/run.sh      # 离线三件套：analyze / compose / replay
   bash examples/online/run.sh       # 在线：daemon 全生命周期 15 步（Rule-T 在第 11 步）
   ```

   两者都会把输出录进各自的 `trace.txt`，可与仓库内的录制结果逐行对照。
2. 挑一份与你场景最像的通用域改起：
   `se_lifecycle`（SE 生命周期）、`re_pipeline`（RE 流水线 + 变更回路）、
   `react_loop`（agent 循环）、`cross_harness`（跨工具编排）。
3. 用 §1.4/§1.5 的判据与边界复核选型，用 §5 的六项分析自检。
4. 需要行业/方法论特定的映射（如某个技能体系的产物清单与验收清单）时，
   看 `docs/skill-adaptation.md`——那是本手册之外的**领域适配层**，不属于范式本身。

### 参考

| 文档 | 内容 |
|------|------|
| [`docs/pactv1_rev3.md`](pactv1_rev3.md) | 范式规范母本：六原语、六规则、§2 适用性、§3 文法、Appendix A 分析算法 |
| [`docs/requirements.md`](requirements.md) | 需求与范围（含明确的范围外项） |
| [`docs/design.md`](design.md) | 实现设计：D1–D27 决策记录、§6 线上格式、§9 三接口 |
| [`docs/architecture.md`](architecture.md) | 架构视图：进程与部署、关键数据流、启动恢复 |
| [`docs/skill-adaptation.md`](skill-adaptation.md) | 领域适配层示例（RE / SE 技能），与本手册正交 |
| [`examples/README.md`](../examples/README.md) | 五份通用域 + 两条可复现轨迹的逐例讲解 |
