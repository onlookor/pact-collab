# PACT examples —— 通用域、组合片段与可复现轨迹

[English](README.md) | **简体中文**

这套 examples 与任何具体技能或方法论**无关**，只演示 PACT 范式系统本身：
它适合描述什么样的协作过程、怎么把一个场景写成 `.pact` 域声明、六个静态分析
各自在说什么、daemon 跑起来之后四类动作如何被 Gate 与前置图拦截、以及事件
日志如何离线重建状态。

与两个技能（requirements-engineering / software-engineering）强绑定的适配产物
在另一个仓库，说明见 [`docs/skill-adaptation.md`](../docs/skill-adaptation.md)；
本目录是**通用**的那一套。

约定：下文所有 `pact ...` 都是 `./target/debug/pact`（或你安装的同名二进制）；
`§x.y` 引用范式规范 `docs/pactv1_rev2.md`（论文手稿族不入公开仓库，仅本地保留，
见 `.gitignore` 的 `/docs/pact*`）。

---

## 1. 覆盖范围

### 1.1 四份通用域（`domains/`）

计数由 `pact domain validate` 实测得出（该命令需 daemon，见 §2）。

| 文件 | 场景 | types | participants | gates | seed_authority | analyze 退出码 |
|------|------|-------|--------------|-------|----------------|----------------|
| [`domains/se_lifecycle.pact`](domains/se_lifecycle.pact) | 软件工程通用流水线（中等粒度：开发主链 + 只读评审 + 带 `via` 的部署把关） | 5 | 8 | 17 | 1 | 0 |
| [`domains/re_pipeline.pact`](domains/re_pipeline.pact) | 需求工程通用流水线（含合取前置与变更控制回环） | 7 | 7 | 19 | 2 | 0 |
| [`domains/react_loop.pact`](domains/react_loop.pact) | 单智能体 ReAct 循环（前置图带环，仍自然终止） | 4 | 2 | 12 | 1 | 0 |
| [`domains/cross_harness.pact`](domains/cross_harness.pact) | 多智能体协作，无中心编排器 | 5 | 4 | 13 | 1 | 0 |

四份域的六项分析实测结果（逐字来自 `examples/offline/trace.txt`）：

| 域 | 良构（违规/警告） | seed_types | 可达/不可达 | gate_coverage | consumption_reachability | structural_deadlock | stranding_risk |
|----|-------------------|------------|-------------|---------------|--------------------------|---------------------|----------------|
| se_lifecycle | 0 / 0 | 1 | 5 / 0 | 0 | 0 | 0 | **1** |
| re_pipeline | 0 / 0 | 2 | 7 / 0 | 0 | **2** | 0 | 0 |
| react_loop | 0 / **1** | 1 | 4 / 0 | 0 | 0 | 0 | 0 |
| cross_harness | 0 / 0 | 1 | 5 / 0 | 0 | 0 | 0 | 0 |

三份非零项都不是缺陷，都是**咨询性**结果，分类理由见 §4。

### 1.2 两份组合片段（`fragments/`）

片段是「半个域」：只声明自己关心的类型、前置与 gate，靠 `pact compose` 与宿主
域按 §1.8 的 C1–C4 合并。

| 文件 | 用途 | types | participants | gates | seed_authority |
|------|------|-------|--------------|-------|----------------|
| [`fragments/security_review.pact`](fragments/security_review.pact) | **可组合**：给任意含 `implementation` 的域挂一条安全扫描支线 | 2 | 2 | 4 | 0 |
| [`fragments/security_blocking.pact`](fragments/security_blocking.pact) | **被拒**：与宿主对 `deployment` 的前置理解不一致，触发 C2 冲突 | 4 | 1 | 2 | 0 |

片段单独 `analyze` 会出现大片 unreachable（security_review 2 个、security_blocking
4 个），这是**预期**的：片段没有 seed_authority，A.1 的 BFS 没有起点。片段只在
组合后才有意义，所以 §1.1 的表不列它们。

### 1.3 两个脚本与两份录制轨迹

| 文件 | 说明 |
|------|------|
| [`online/run.sh`](online/run.sh) → [`online/trace.txt`](online/trace.txt) | daemon 全生命周期 14 步（238 行）：启动 → 装载 → 四类动作 → Gate 拦截 → Rule-T 终止 → 事件重放 → 卸载 |
| [`offline/run.sh`](offline/run.sh) → [`offline/trace.txt`](offline/trace.txt) | 离线三件套 5 步（187 行）：analyze × 4 域、三类良构违例、compose 成功与 C2 拒绝、replay |
| [`offline/fixtures/se_lifecycle.events.jsonl`](offline/fixtures/se_lifecycle.events.jsonl) | 12 行事件日志，让离线演示在 fresh clone 上自足（不依赖先跑在线脚本） |

`trace.txt` 是**真实录制的输出**，不是手写样例：token id（ULID）、时间戳、
daemon pid、绝对路径每次运行都不同，路径已脱敏为 `<REPO>`；步骤结构、错误码、
分析数字与退出码应当与你的运行完全一致。

---

## 2. 前置与运行

```bash
cargo build -p pact-cli                     # 产出 ./target/debug/pact

examples/offline/run.sh                     # 离线三件套，不需要 daemon
examples/online/run.sh                      # 在线全生命周期，自起自停 daemon
```

两个脚本都从任意工作目录可运行（内部用 `BASH_SOURCE` 定位仓库根），也可用
`PACT=/path/to/pact` 指定二进制。录制方式：

```bash
examples/offline/run.sh > my-trace.txt 2>&1
```

**命令的在线/离线归属**——这是最容易踩的一点：

| 归属 | 命令 | 说明 |
|------|------|------|
| 离线（不经 daemon） | `analyze`、`compose`、`replay` | 只依赖纯函数层，不起进程、不碰存储 |
| 在线（必须连 daemon） | `daemon start`、`domain validate\|load\|list\|info\|unload`、`seed`、`produce`、`read`、`consume`、`counter ls`、`archive ls`、`gates show`、`enabled`、`status` | 无 daemon 时统一报 `error [Transport]: client error (Connect)` |

注意 `domain validate` 属于**在线**：它要做的是「解析 + 良构校验 + 装载预演」，
走的是 REST 通道。想纯离线校验域声明，用 `analyze`（它的 `precondition_wf`
项就是同一套良构判定）。

在线脚本的副作用只落在 `examples/online/run-data/`（开头会清空该目录），离线的
落在 `examples/offline/out/`。两个目录都已 gitignore。

---

## 3. 在线轨迹：daemon 全生命周期 14 步

完整输出见 [`online/trace.txt`](online/trace.txt)。下表是逐步导读；「规则」列
指向范式规范的 SOS 规则（Rule-P 生产 / Rule-R 读取 / Rule-E 消费 / Rule-S 种子
注入 / Rule-W 见证 / Rule-T 终止）。

| 步 | 做什么 | 观察到什么 | 规则/机制 |
|----|--------|-----------|-----------|
| 0 | 生成 `pact.toml` | `data_dir` + `listen_unix` 两个键；缺省是 `./data` 与 `./pact.sock` | 单机单进程部署 |
| 1 | `daemon start` | 打印 pid 与 socket 路径 | daemon 是 Counter 的**唯一权威**；CLI/REST/MCP 三接口都汇聚到它 |
| 2 | 装载前先 `analyze` | 六项分析，唯一非零是 `stranding_risk: 1` | 静态分析是纯函数，装载前就能跑（第 12 步把它变成真实拒绝） |
| 3 | `domain validate/load/list/info/status` | `ok: types=5 participants=8 gates=17 seed_authority=1`；装载后 `counter=0 archive=0 terminated=true` | **空域就是终止态**（Rule-T 的两条都满足），seed 后翻回 `false` |
| 4 | `gates show` + `enabled` | deployer 的两条子句带 `when via implementation`；Counter 空 ⇒ `enabled productions for architect: 0` | GateMatrix 静态授权 + Rule-P 动态可用性 |
| 5 | 空 Counter 上强行 `produce`/`seed` | `PredecessorNotOnCounter`、`SeedAuthorityDenied` | Rule-P 要求前驱**仍在 Counter 上**；Rule-S 受 SeedAuthority 约束 |
| 6 | 合法 `seed` | 得到 token；`status` 变 `terminated=false`；`enabled` 变 `1` 并给出 `design witnesses=[<ULID>]` | Rule-S 豁免前置条件；引擎直接告诉你该用哪个前驱 |
| 7 | 越权 `consume` / 幽灵参与者 `produce` | 两次 `StaticGateDenied` | reviewer 只有 `reads` 没有 `consumes`；`ghost` 不在参与者集合里 |
| 8 | 主链路 `read → produce → consume` × 3 | 每个 `produce` 都带 `--predecessors`；`consume` 后 token 出现在 `archive ls` | **关键顺序**：先 produce 后 consume（见 §5 坑 3）；消费显式、无级联、不可逆 |
| 9 | 只读参与者 + 带 `via` 的消费 | 读已消费的 design → `TokenNotOnCounter`；deployer 读 test_report 成功（implementation 仍在 Counter 且是直接前驱） | Rule-R 只对 Counter 上的 token 成立；`via` 是**实例级**见证 |
| 10 | 清空 Counter | `counter: 0 tokens` | — |
| 11 | Rule-T 终止判定 | `counter=0 archive=5 terminated=true`；Archive 五条各带 `exhausted_by`/`at`；再 `produce` 得 `PredecessorNotOnCounter` | 引擎里**没有**独立的「已终止」规则；终止态是 Counter=∅ 且无 Rule-P-enabled 的推论 |
| 12 | **搁浅陷阱**：同一域装载为 `se_lifecycle_trap`，把第 8 步的顺序换一下 | tester 提前消费 implementation ⇒ deployer 连**读** test_report 都被拒：`GateConditionFailed: ... Via(TypeName("implementation"))`；域卡在 `counter=1 archive=3 terminated=false` | 第 2 步 `stranding_risk` 的实点。修法三选一，都是改**域声明**而非改引擎 |
| 13 | `ls run-data` + `replay` | `replayed 10 events (domain se_lifecycle): counter=0 archive=5`，与第 11 步 `status` 一致 | JSONL 是真相源，SQLite 索引与快照都是派生物；崩溃恢复 = 快照 + 其后事件重放 |
| 14 | `domain unload` × 2 | `domain list` 打印 `(无域)` | 卸载不删事件日志，事后审计仍可用 |

第 12 步值得单独强调：它演示的是 PACT 的一个核心取舍——`via` 把「顺序」变成了
可机械检查的硬约束，代价是见证的生命周期必须比依赖它的动作长。分析器在装载前
就报出了这个风险（`potential=false` 表示无环拓扑下的**确定性**报告），运行期
果然按预测拒绝了。修法（任选其一）：

- a) 让 tester 不 `consume implementation`（见证活得比 test_report 久）；
- b) 把 deployer 的 `via` 换成生命周期更长的见证类型；
- c) 去掉 `via`，接受更弱的溯源保证。

---

## 4. 静态分析怎么读：六项输出

`pact analyze` 一次给出六项（论文 Appendix A.1–A.6）。**A.5 良构性是其余五项的
前提**：违规会破坏它们的假设，所以聚合报告把 `precondition_wf` 放在首位，须先
修违规再解读其余结果。

| 输出行 | 分析问题 | 非零意味着 | 是否阻断装载 |
|--------|----------|-----------|--------------|
| `precondition_wf: 良构（N 项违规，M 项警告）` | §3.1 六项良构约束 + Seed/Produce 重叠 | 违规 N>0 ⇒ 域声明不可用（退出码非 0）；警告 M>0 ⇒ 需复核建模意图 | 违规阻断；警告不阻断 |
| `reachability: N 源类型，X 可达，Y 不可达` | 从 SeedAuthority 类型沿前置图 BFS | Y>0 ⇒ 有类型永远做不出来 | 咨询性 |
| `gate_coverage: N 个无 producer 的非源类型` | 每个非源类型是否至少有一个 producer | N>0 ⇒ 有类型无人能生产 | 咨询性 |
| `consumption_reachability: N 个无 consumer 的类型` | 每个类型（含源类型）是否至少有一个 consumer | N>0 ⇒ 潜在技术债：token 会一直留在 Counter | 咨询性 |
| `structural_deadlock: N 个强连通分量` | 前置图 SCC 中「无种子入口且不可外部馈入」的分量 | N>0 ⇒ 环里的类型永远启动不了 | 咨询性 |
| `stranding_risk: N 处滞留风险` | `via T_a` 的见证能否先于依赖动作被消费 | N>0 ⇒ 存在一条真实的顺序约束 | 咨询性 |

退出码：**良构即 0**，与咨询项是否为 0 无关。`--json` 给出同一份数据的结构化
形式（键序固定为附录 A 顺序），机器读它、人读非 JSON 输出。

### 三份非零咨询结果的分类理由

**se_lifecycle 的 `stranding_risk: 1`** —— 真约束，不是缺陷。

```json
{ "ty_b": "test_report", "ty_a": "implementation",
  "via_holder": "deployer", "consumer": "tester", "potential": false }
```

deployer 用 `via implementation` 守护 test_report，而 tester 有权消费
implementation。分析器如实报告「见证可能先消失」，第 12 步证明它确实会消失。
`potential: false` = 无环拓扑下的确定性报告（STRANDING_RISK）；`true` = 有环
拓扑下的保守报告（POTENTIAL_STRANDING）。

**re_pipeline 的 `consumption_reachability: 2`（`baseline`、`revised_baseline`）**
—— 刻意设计，建模者裁决。

基线是**常驻配置资产**：它被反复读取（change_analyst 做变更影响评估要读、
configuration_manager 做修订基线要读），但不被任何角色「耗尽」。PACT 的 A.3
只看 GateMatrix 里有没有 `consumes` 子句，它无法区分「忘了写消费者」和「本来
就不该有消费者」，所以如实报告，把裁决权留给建模者。

想让这个数字归零有两条路，取舍不同：

- 给 `baseline` 加一个消费者（例如 configuration_manager 在发布 `revised_baseline`
  时消费旧基线）——语义更强：旧基线作废，但也就不能再被读取；
- 接受非零——语义更贴合「基线是长期资产」，代价是 Counter 不会自然归空，
  Rule-T 在基线仍在册时不可达。

本域选后者，因为需求工程的基线确实需要长期可查。**A.3 的非零值本身不携带
好坏判断**，这是 PACT「分析咨询、引擎不替你决定」的一贯立场。

**react_loop 的 `SEED_PRODUCE_OVERLAP` 警告** —— 双入口歧义，需显式裁决。

```json
{ "SeedProduceOverlap": { "participant": "user", "ty": "observation" } }
```

`observation` 不是源类型（它有前置 `tool_result`），但 user 持 seed 权威、agent
又能 produce 它。规范上这是安全的（seed 实例可作前驱被消费，produce 路径产出
新实例），但同一个类型有两个入口通常意味着建模混淆，或需要显式的部署裁决，
所以告警而非静默通过。本域是**故意**这么建的：循环需要一个启动推力，user 注入
首个 observation，之后 observation 由 agent 从 tool_result 生产。

判定粒度注意：实现取**类型级**（produces gate 取全体参与者的并集，不要求与
seed 持有者同一人），比论文 §1.4.3 的「同一参与者」表述更宽，是假阳性方向的
保守超集（不漏报）。详见 [`docs/design.md`](../docs/design.md) D21。

---

## 5. 术语与输出对照的五个坑

这五处都实测踩过，逐条说明以免重蹈。

**坑 1：`reachability: N 源类型` 里的「源类型」≠ §1.4 的源类型。**

同一份 `analyze` 输出里，两个「源类型」含义不同：

- `reachability` 行的 N 是 **`seed_types`** 的数量，即 `{T' : ∃p, (p,T') ∈ SeedAuthority}`——持有种子权威的类型；
- `gate_coverage` 行排除的「非源类型」用的是 **§1.4 定义**：`Precond(T) = ∅`。

在无 seed_authority 的纯循环域或片段上两者会分道扬镳。实测 `security_review.pact`
单独分析：

```
reachability: 0 源类型，0 可达，2 不可达
gate_coverage: 0 个无 producer 的非源类型
```

按 §1.4，`implementation` 的 `Precond` 是空集，它**是**源类型——这正是
`gate_coverage` 报 0 的原因（源类型不经 Rule-P，被排除）。但 `reachability`
报「0 源类型」，因为该片段没有 SeedAuthority。**机器处理请以 JSON 键名
`seed_types` 为准**，它是准确的；纯循环域（如 `react_loop`）上人读输出会产生
「明明没有 §1.4 源类型却说有 1 个」的错觉。

**坑 2：源文件里的 `via X` 序列化后打印成 `when via X`。**

`compose` 的输出把 `deployer: ... reads test_report via implementation ...`
规范化为 `reads test_report when via implementation`。两种写法等价，`when` 是
序列化器补出来的显式关键字。别以为 compose 改了语义。

**坑 3：必须 `read → produce(--predecessors) → consume`，不能颠倒后两步。**

Rule-P 要求前驱**仍在 Counter 上**。先 consume 再 produce 会得到
`PredecessorNotOnCounter`，而消费是不可逆的（token 已入 Archive），没有撤销
命令。第 8 步与第 12 步就是同一域的正反两种顺序。

顺带：`produce` 必须用 `--predecessors` 显式给出见证 id（逗号分隔多个），引擎
不会替你猜。省略时报错文本里的 token id 是空的——那不是 bug，是没传。

**坑 4：刚装载的域 `terminated=true`。**

Rule-T 的条件是 Counter=∅ 且无参与者 Rule-P-enabled，空域两条都满足。这不是
「域坏了」，注入 seed 后立刻翻回 `false`。同理，终止后 `produce` 报的是
`PredecessorNotOnCounter` 而不是某个「已终止」错误码——引擎里没有独立的终止
规则。

**坑 5：`compose` 对每个片段单独跑良构校验（含 Type Closure）。**

片段必须**自足**声明它引用到的全部类型，不能指望宿主域补齐。
`security_review.pact` 因此显式写了 `types { implementation; security_report }`，
尽管 `implementation` 在宿主里已有。合并检查（C1–C4）在逐片段校验之后才做。

---

## 6. 离线三件套：analyze / compose / replay

三个命令都不经 daemon，是 PACT 工具链里可以放进任何脚本、任何 CI 阶段的纯函数
入口。完整输出见 [`offline/trace.txt`](offline/trace.txt)。

### 6.1 analyze —— 装载前的静态体检

```bash
pact analyze domains/se_lifecycle.pact          # 人读，退出码 0 = 良构
pact --json analyze domains/se_lifecycle.pact   # 机读，六项结构化报告
```

三类良构违例的真实长相（脚本用 heredoc 临时生成坏域，不入库）：

| 违反的约束 | 违例名（`--json` 的 `detail`） |
|-----------|-------------------------------|
| §3.1 约束 6 Source-Production Constraint | `{"SourceTypeProduceGate":{"participant":"p","ty":"a"}}` |
| RHS Uniqueness（同一类型两条前置规则） | `{"DuplicatePrecondRule":{"ty":"c"}}` |
| Type Closure（引用未声明类型） | `{"UndeclaredType":{"referenced_by":"preconditions","ty":"b"}}` + gates 处逐处再报 |

Type Closure 是**逐处**报告的：同一个未声明类型 `b` 被引用 3 次就报 3 条
（preconditions 1 处 + gates 2 处），方便定位而不是只给个计数。

### 6.2 compose —— 片段并行组合（§1.8 C1–C4）

```bash
pact compose domains/se_lifecycle.pact fragments/security_review.pact > out/merged.pact
pact analyze out/merged.pact      # 合并结果本身仍须是良构域
```

四元组按**集合并**：types / preconditions / gates / seed_authority 各自取并。
同一参与者的宿主条款与片段条款合成完整 gate 集（实测 deployer 从 3 条子句变成
5 条）。合并后 `analyze` 得 6 可达（多出 `security_report`）、stranding_risk
仍是 1（继承宿主的那条）。

四条一致性条件中，C2（共享类型的前置须一致）最容易撞：

```bash
pact compose domains/se_lifecycle.pact fragments/security_blocking.pact
# error [ComposeConflict]: 片段组合冲突（1 处）   -> exit=1
```

```json
{"error":{"code":"ComposeConflict","detail":{"conflicts":[
  "PreconditionMismatch { ty: TypeName(\"deployment\"), pre_a: {TypeName(\"test_report\")}, pre_b: {TypeName(\"security_report\"), TypeName(\"test_report\")} }"]},
  "message":"片段组合冲突（1 处）"}}
```

宿主说 `deployment` 的前置是 `{test_report}`，片段说是 `{test_report,
security_report}`。**这不是工具缺陷而是范式意图**：前置图是生产合法性的唯一
来源，两个片段对「什么算完成」理解不一致时必须显式解决，不能静默合并。正确
做法是改宿主声明本身（把 `deployment` 的规则直接写成合取），而不是叠加矛盾
片段。

C1 前置唯一性保持 / C2 共享类型前置一致 / C3 共享 `(p, action, T)` 的 GateCond
须语法同一 / C4 `validate_domain` 终检兜底。

### 6.3 replay —— 从事件日志离线重建状态

```bash
pact replay offline/fixtures/se_lifecycle.events.jsonl
# replayed 10 events (domain se_lifecycle): counter=0 archive=5
```

fixture 共 12 行：1 条 `DomainLoaded`（重置状态）、1 条 `Terminated`（信息性）、
10 条会改变状态的动作事件（1 seed + 4 produce + 5 consume）。重建结果
`counter=0 archive=5` 与在线轨迹第 11 步的 `status` 完全一致。

事件溯源的含义：**JSONL 是真相源**，SQLite（turso）索引与快照都只是派生物，
删了也能重建。崩溃恢复走同一条路径——读最近快照，再重放其后的事件。想重放
新鲜日志：`pact replay examples/online/run-data/se_lifecycle.events.jsonl`。

---

## 7. 从场景到 `.pact`：建模步骤与适用边界

### 7.1 五个建模步骤

1. **列离散产物** → `types { ... }`。判据：这个产物在协作中是否以「存在/不存在」
   的离散状态参与？连续量（进度百分比、质量分数）不是 PACT 的类型。
2. **写「X 就位才能做出 Y」** → `preconditions { X → Y; {X + Z} → W; }`。
   **只支持合取**（AND），没有 OR 分支。右部唯一（一个类型只能有一条前置规则）。
3. **列谁能读/消费/产出什么** → `gates { p: reads T, consumes T, produces T'; }`。
   需要顺序约束时用 `via`：`reads T_b when via T_a` 要求读 T_b 时 T_a 的**那个
   实例**仍在 Counter 上且是 T_b 的直接前驱。`produces` 子句**不能带条件**
   （生产合法性完全由前置图决定）。
4. **找入口** → `seed_authority { p: seeds T; }`。§1.4 的源类型（`Precond(T)=∅`）
   **只能**经 Rule-S 进入，给它配 `produces` gate 是良构违例
   （`SourceTypeProduceGate`）。循环域也需要一个 seed 提供启动推力。
5. **`analyze` 六项，逐条裁决**，然后 `domain load` 试跑一条真实链路。
   咨询项非零不等于要改——像 §4 的 baseline 那样，有些非零值是设计意图。

### 7.2 适用判据（§2.1，五条须全部满足）

| # | 判据 | 含义 |
|---|------|------|
| 1 | Discrete Artifacts | 产物以离散的存在/不存在状态参与协作 |
| 2 | Precedence Relations | 存在「某些产出必须先于另一些」的关系 |
| 3 | Transferable Outputs | 存在可被其他参与者使用的离散产出 |
| 4 | Participant Count ≥ 1 | 单参与者系统也是合法的退化情形 |
| 5 | Identifiable Entry Points | 每个源类型都能关联到一个种子权威 |

四份域对这五条的落点各有侧重：

- **se_lifecycle / re_pipeline** 是 1–5 全中的典型流水线；
- **react_loop** 逼近第 4 条的退化情形——agent 独占全部 12 条 gate，user 只持
  seed 权威、不持任何 gate，实质是「单工作者 + 一个注入者」。它的第 5 条还是
  **空满足**的：四个类型各有唯一前驱，§1.4 意义下没有源类型。但循环仍然需要
  `seed_authority { user: seeds observation }` 提供启动推力——**SeedAuthority
  不只服务于源类型**，也是给纯循环域点火的手段（seed 豁免前置条件，§1.4.3）；
- **cross_harness** 突出第 3 条：三个 harness 互不通信，全靠 Counter 上的 token
  传递产出，谁先谁后由 Rule-P 可用性决定，不需要中心调度。

### 7.3 不适用边界（§2.3，九条）

| 不能表达 | 原因 | 通常的绕法 |
|----------|------|-----------|
| 连续过程 | 范式是离散的 | 离散化为里程碑产物 |
| 实时约束 | 范式没有时钟 | 时限交给外部系统，PACT 只管产物就位 |
| 概率分支 | 范式是确定性的 | 把随机决策放在参与者内部 |
| OR 分支 | 只支持合取（§1.4.4） | 拆成多个域，或把「或」上移为参与者的内部选择 |
| 产物内部结构 | 经 ThingRef 委派 | `--thing-ref` 挂外部标识（文件路径、issue 号、URL） |
| 自动生命周期排序 | 设计如此（§1.6） | 顺序由参与者显式动作决定，引擎不代劳 |
| 前置图之外的生产期 gate 条件 | 设计如此（§1.5.2） | 改用前置图表达 |
| 消费之间的时序 | GateExpr 只评估当前状态（§1.7.3） | 用 `via` 把时序编码为「见证是否在场」 |
| 源类型的内部生产 | 设计如此（§1.4） | 源类型只经 Rule-S 进入 |

第 8 行是 `via` 存在的理由，也是 §3 第 12 步搁浅陷阱的根源：GateExpr 无法表达
「A 在 B 之后」，只能表达「A 现在在不在场」，所以时序约束被编码成见证的存活期。

### 7.4 领域无关性（§2.2）

范式本身不变，换域就是换四个注入点：TypeSchema、PreconditionGraph、GateMatrix、
SeedAuthorityDecl。本目录四份域就是四次不同的填充——软件工程、需求工程、
智能体循环、多智能体协作，用的是同一套原语和同一套 SOS 规则。

论文还提到一个**可选**的第五注入点 Archival Policy（`AP: Counter → 2^Counter`，
指定某个系统参与者在条件成立时该消费哪些 token）。它经正常 Rule-E 执行，不
引入新原语、不引入隐式系统动作。**本实现不内置它**（见
[`docs/requirements.md`](../docs/requirements.md) §6 范围外）；需要自动清理的
域可以把清理者建模成一个普通参与者，由外部驱动它调用 `consume`。

---

## 8. 参考

| 文档 | 内容 |
|------|------|
| [`docs/manual.md`](../docs/manual.md) | 权威用户手册：三接口（MCP/REST/CLI）完整命令面、错误码表、部署与运维 |
| [`docs/requirements.md`](../docs/requirements.md) | 需求（FR1–FR14、NFR1–NFR6、范围外） |
| [`docs/design.md`](../docs/design.md) | 设计决策（含 D21 SeedProduceOverlap 的类型级判定、D22 示例与手册作为交付资产） |
| [`docs/architecture.md`](../docs/architecture.md) | 架构：crate 依赖、数据流、部署视图、联网演进 |
| [`docs/skill-adaptation.md`](../docs/skill-adaptation.md) | 与两个技能强绑定的适配产物（另一个仓库），与本目录互补 |
| `docs/pactv1_rev2.md` | 范式规范母本（论文手稿族，不入公开仓库）；本文 §x.y 引用均指该文档 |
