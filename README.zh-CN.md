# PACT

**面向多智能体与人机协作的声明式协同层。**

[![CI](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml/badge.svg)](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) | **简体中文**

PACT 是一个很小的协同引擎，面向多智能体与人机协作：它判断**此刻哪些协作动作合法**，执行这个判定，并把每一次动作记录在案。它不是 agent 框架，也不是工作流执行器——任务由参与者完成。

本仓库是它的参考实现：Rust 引擎，外加 CLI、REST 与 MCP 接口、静态分析、CPN 导出，以及可选的参与者运行时。

你把一条协作流程写成一份 **域声明**（`.pact`）：哪些产物可以存在、什么产物必须先就位才能产出下一件、谁能创建 / 读取 / 消费哪类产物、谁有权注入第一件产物。单权威 daemon 持有当前产物集合（`Counter`），对来自人、agent harness 或普通应用的每一次调用应用这些规则，并把被接受的动作追加进不可变事件日志；被消费的产物进入 `Archive`。

因此，流程是**数据，不是代码**：它可以被评审、静态分析、换版交接、重放，也可以被运行在不同 harness 里的 agent 共同使用。任务仍由参与者完成；PACT 负责准入判定并留下记录。

## PACT 是什么？

PACT（Precondition-governed Autonomous Counter-based Token）是这一层背后的形式化范式。四个概念撑起了大部分内容：

- **Token**——不可变、有类型的产物实例。类型、生产者、前驱与外部引用都在记录里。
- **Precondition**——类型级规则：产出 `B` 需要一枚在场且可用的 `A`（也可以是 `{A + C}` 这样的合取）。前驱是见证，生产从不消费它们。
- **Gate**——谁可以对哪类产物 `read`、`consume`、`produce`。read 与 consume 门可附带状态条件，例如 `via T`（必须是那一枚特定的前驱实例仍在场）或 `count T >= n`；production 门不带条件，只由前置图约束。
- **Counter + Archive**——当前产物集合与已消费产物的只增账本。读取非破坏；消费显式、无级联、不可逆。

由此得到三条性质：

1. **零隐式动作。** 只有参与者显式 seed / produce / consume，`Counter` 才会变化；没有自动推进、隐藏清理或隐式重试。
2. **溯源敏感的准入。** gate 可以要求**特定的前驱实例**仍在场，而不只是“存在某枚 T 类型产物”；审批链因此可以被机械检查。
3. **单权威、可重放。** 每一个被接受的动作都是事件；快照与索引只是派生物，日志既能重建状态，也构成审计记录。

## 归入哪一类

**类别：** 面向离散产物型协作的、**声明式、状态驱动的协同层**。如果需要一个熟悉的标签，可以叫它协同引擎或工作流引擎——但要注意，它不执行任务本身。

更准确地说，PACT 是一种**声明式流程编排**：流程定义在域声明里，而不是画成图或写成命令式代码；图是可导出的视图，具体任务仍由参与者执行。

PACT 位于工作流编排、多智能体协调、授权与审计三者的交界处。它与三者都重叠，但不能直接替代其中任何一个。最接近的常见参照是：

| 你可能熟悉的工具/类别 | 它主要决定什么 | PACT 的不同 | 能否配合使用 |
|---|---|---|---|
| **图式 agent 编排**——LangGraph | agent 流程如何沿节点和边移动；图和状态由代码定义。 | PACT 声明的是产物类型与当前状态上的规则，下一步合法动作由 `Counter` 计算。图是**可导出的视图**（CPN），不是运行时对象；声明可以在运行前被静态分析。 | 可以——图里的节点或 agent 可以作为参与者调用 PACT。 |
| **持久化执行 / workflow-as-code**——Temporal、Step Functions | 如何持久、可靠地运行业务代码，含重试、定时器、信号与副作用。 | 核心 PACT 引擎不执行业务代码，没有时钟、重试或定时器，也不自动落地副作用；每次迁移都是参与者的显式动作。可选 runtime 只能在准入后执行副作用，并显式补偿。 | 可以——Temporal 可以在参与者内部执行实际工作。 |
| **BPMN 流程引擎**——Camunda / Zeebe | 完整的业务流程控制流：网关、事件、定时器、人工任务、补偿。 | PACT 的核心小得多：只支持 AND 前置，没有 OR 网关与定时器；读与消费分离，gate 作用于动作级，并内置静态分析。 | 需要丰富控制流选 BPMN；需要小而可分析的协同内核选 PACT。两者可以在参与者边界汇合。 |
| **DAG 调度 / 自动化**——Airflow、n8n | 何时运行哪个任务：按时间表或触发器调度，含重试与集成。 | PACT 没有调度器、时钟或自动触发。只有参与者动作且规则允许时，任务才发生；协调由产物状态驱动，而不是时间表驱动。 | 可以——调度器负责唤醒参与者，PACT 判断动作是否合法。 |
| **Agent 框架**——CrewAI、AutoGen、OpenAI Agents SDK | agent 如何推理、对话、调用工具与交接任务。 | PACT 没有模型调用、消息、角色、任务派发或 agent 循环。它是跨 agent / 跨 harness 的共享规则书；决策仍由 agent 自己作出。 | 可以——任何 agent 框架都可以运行在参与者内部。 |
| **Harness 内团队 / 任务板插件**——如 DSH Agent Teams | 队长建队员、派任务、共享看板与消息，协调发生在宿主内部。 | PACT 不建队、不派活、不传消息；它跨 harness 共享产物与准入规则。 | 可以——团队插件负责队内分工，PACT 负责跨团队 / 跨 harness 的产物准入。 |
| **策略引擎**——OPA、Cedar | 依据请求上下文判断某个请求是否被策略授权。 | PACT 的 gate 绑定在流程状态与产物溯源上，还治理生产与消费并保留归档；它不是通用策略语言。 | 可以——OPA/Cedar 可以为参与者或网关提供属性级判定。 |
| **Petri 网 / CPN**——CPN Tools | 并发系统的形式建模与状态空间分析。 | PACT 是可执行的、基于声明的模型，内置参与者 gate、token 身份、溯源与归档。它与 P/T 网的原始表达力**互不包含**（论文 §4.2–§4.3）；可以编码为着色 Petri 网（CPN）用于分析与可视化。 | 可以——`pact cpn` 导出 CPN 模型；CPN Tools 提供形式化探索。 |

一句话差异：多数工具回答**“下一步该跑什么？”**或**“谁可以调这个 API？”**；PACT 回答**“当前存在什么、它从哪来、谁在什么条件下可以对哪枚产物做什么？”**，并让这个答案可重放。

### PACT 不是什么

- **不是 agent 框架。** 不做模型调用、提示词、对话、工具执行或 agent 生命周期管理。
- **不是任务调度器或工作流执行器。** 核心引擎没有时钟、定时器、自动推进或重试；副作用由参与者或可选 runtime 的显式执行器承担。
- **不是通用工作流引擎。** 只支持 AND 前置；没有 OR 网关、引擎级取消/补偿语义或动态参与者集合。
- **不是通用策略引擎。** gate 是绑定在类型化产物上的领域准入规则，不是属性级授权语言。
- **不是分布式系统。** 每个 Counter 实例只有一个权威；无共识、复制或跨实例协同。
- **不是图编辑器。** 流程由规则定义；图是可导出的第二表示，不是真相源。

完整的“刻意不建模什么”清单见 [`docs/expressiveness-boundaries.md`](docs/expressiveness-boundaries.md)。

### 什么时候该用它

- 流程产出的是**离散产物**（文档、报告、构建、工单），而且“B 必须在 A 之后”是本质约束而不是偏好。
- 不止一个参与者——人、agent 或普通服务——需要就**谁能创建、读取、消费**哪件产物达成一致。
- 你需要**跨 harness 协调**或可审计的记录，但想保留现有的 agent 框架或执行器。
- 你想**在运行前检查流程**：死锁、搁浅、不可达状态、gate 覆盖。
- 你需要**溯源敏感的审批**（必须是这一次评审、一直存活到部署），而不只是角色级权限。

如果流程是连续的、由定时器驱动、以 OR 分支为主，或需要自动重试与副作用执行，PACT 很可能不是合适的层。

## 这套系统给你什么

**建模与检查**

- **域声明**——四段式 `.pact` 文法（`types` / `preconditions` / `gates` / `seed_authority`），论文六项良构约束全部强制。
- **六项静态分析**——可达性、gate 覆盖、消费可达性、结构死锁（SCC）、前置与源生产良构性、搁浅风险。
- **片段组合**——领域并行组合与四条件一致性检查（C1–C4），冲突显式报告。
- **CPN 导出**——把声明离线编码为 CPN Tools 模型（`pact cpn`），用于形式验证与可视化。
- **物引静态检查**——离线 `pact ref lint` 在引用使用前检查 RefBindings、ContractCard、Tombstone 与 `.pact` 声明的类型、契约引用、scheme、保留策略与删除证据是否自洽。
- **物引可见性**——Counter/Archive 默认只在调用者具备无条件 `reads`/`consumes` 时返回 `thing_ref`，也可由 RefBindings 的 `domain`/`hidden` 显式覆盖；token 存在性不受影响。

**运行与治理**

- **完整操作语义**——Rule-P / R / E / S / W / T 与统一准入谓词 `GateHolds`（`via` / `exists` / `count`，支持 `and` / `or` / `not` 组合）。
- **事件溯源引擎**——只追加 JSONL 日志、快照、崩溃恢复、确定性重放、可选日志压缩，以及消费级幂等键。
- **多 Counter 实例与声明交接**——同一域声明可承载多个独立实例；换版时可显式走“冻结 → 迁移 → 激活”，不丢失已积累状态。
- **可选传输与身份**——TLS 1.2/1.3 与 mTLS、静态 Bearer、Ed25519 请求签名、域级 ACL、可选读审计，以及面向 `/mcp` 的 OIDC 资源服务器。

**接入与运维**

- **三种接口、一个权威**——REST（unix socket 或 TCP、SSE 通知）、MCP（stdio 与 Streamable HTTP `/mcp`，21 个工具）、单一 `pact` CLI。
- **参与者运行时**——`pact-runtime` 提供可复用的参与者自驱循环、多 agent 编排，以及订阅消费事件的外部副作用执行器（失败按 saga 式追加补偿）。
- **运维面**——health / readiness / metrics、优雅停机、PID 文件、systemd 与容器产物。
- **本地嵌入**——面向 Node/桌面宿主的 WASM 构建，事件与快照格式与原生引擎共用（按单用户边界设计）。
- **Harness 适配器**——[`dsh-plugin-pact`](https://github.com/onlookor/pact-dsh) 把 DeepSeek Harness 的 agent 接入引擎。缺省的 `observe` 档位只镜像事件；切到 `govern` 档位后，agent 这一步或这个工具能否执行由 PACT 域声明裁定，DSH 仍负责 agent 循环、模型调用与工具执行。插件不是协同模型本身——引擎独立于 harness，可以同时服务来自不同 harness 的 agent。

## 从哪里开始

PACT 只有一条上手阶梯：域作者契约 → [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) → 浏览器工作台 → 运行沙盒 → 接入向导。三个浏览器步骤也可直接在线打开 **<https://onlookor.github.io/pact-collab/>**；每条路径都以 `pact analyze` 与[用户手册](docs/manual.md)收口。

| 你的处境 | 先看 | 然后 |
|---|---|---|
| 想知道 PACT 是什么、归入哪一类 | [PACT 是什么？](#pact-是什么)与[归入哪一类](#归入哪一类) | [手册 §1](docs/manual.md) 的归类对比，再读[论文母本](paper/pactv1_rev3.md) |
| 判断 PACT 是否适用，或读范式本身 | [PACT 是什么？](#pact-是什么)与[论文母本](paper/pactv1_rev3.md) | [`expressiveness-boundaries.md`](docs/expressiveness-boundaries.md)——PACT 刻意不建模什么 |
| 在 coding agent 里写第一份域声明 | [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) | 跑 `pact analyze <file>` 直到 exit 0；再看[示例域](examples/README.md) |
| 没装工具，想写第一份域声明 | [作者工作台](https://onlookor.github.io/pact-collab/authoring/)（[离线文件](web/authoring/index.html)） | 用[运行沙盒](https://onlookor.github.io/pact-collab/sandbox/)在内存里跑一遍 |
| 手写域声明，或需要完整生成协议 | [《PACT 域作者契约》](docs/authoring-contract.md) | [手册 §3](docs/manual.md)看语法、§5 看六项分析 |
| 跑一个域，并让 agent 驱动它 | [手册 §2](docs/manual.md)（daemon、`pact.toml`、身份）与 §4（CLI / REST / MCP） | [`examples/online/run.sh`](examples/online/run.sh)，再用[接入向导](https://onlookor.github.io/pact-collab/connect/) |
| 在 DeepSeek Harness 里使用 PACT | [`dsh-plugin-pact`](https://github.com/onlookor/pact-dsh#readme) | [手册 §13](docs/manual.md) 的 `observe` / `govern`、配置与限制 |
| 部署与运维 | [手册 §2.2–§2.4](docs/manual.md)（TLS、令牌、签名）、§7（审计与重放）、§10（故障排查） | [`architecture.md`](docs/architecture.md) §3–§4、[`design.md`](docs/design.md) §9 |
| 裁决共享 token 何时可以消费 | [`token-consumption-timing.md`](docs/token-consumption-timing.md) | [契约 §5](docs/authoring-contract.md) |
| 修改引擎或参与贡献 | [`architecture.md`](docs/architecture.md)与[`design.md`](docs/design.md) | [`requirements.md`](docs/requirements.md)、[测试](#测试) |

同样的路径也从手册一侧汇总在 [manual §12](docs/manual.md)；[`examples/README.md`](examples/README.md) 内置五份可直接运行的域声明、两份组合片段与两个可复现演示（附真实录制轨迹），并把每个示例映射到它演示的建模问题。

## 快速开始

需要较新的 stable Rust 工具链。

```bash
cargo build --release

examples/offline/run.sh    # 六项分析 × 5 域、三类良构违例、
                           # compose 接受 + C2 拒绝、事件重放——无需 daemon
examples/online/run.sh     # daemon 全生命周期：装载 → 播种 → 生产/读取/消费
                           # → gate 拦截 → 终止 → 重放
```

或直接离线运行分析：

```bash
pact analyze examples/domains/se_lifecycle.pact          # 人类可读报告
pact analyze --json examples/domains/react_loop.pact     # 机器可读报告
```

动手走一遍 daemon 的完整流程见下一节末尾的"试一试"。连接参数（`--unix` / `--tcp` / `--https` / `--tls-ca` / `--bearer` / `--bearer-file` / `--sign-key` + `--sign-as`）与 `--json` 是所有命令的全局 flag；完整命令集见 `pact --help`。

## 领域声明语法示例

论文中的软件生命周期域——线性流水线，每个产物类型依赖其前驱：

```text
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    architect:  reads requirement, consumes requirement, produces design;
    developer:  reads design, consumes design, produces implementation;
    tester:     reads implementation, consumes implementation, produces test_report;
    operator:   reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
```

- **types**——领域中的 token 类型。**没有**前置规则的类型是*源类型*：不可生产（Rule-P 要求 `Precond(T) ≠ ∅`），只能经 seed 权威进入 Counter。
- **preconditions**——`lhs → rhs` 表示生产 `rhs` token 需要 `lhs` 中每种类型各一枚 token 在 Counter 上（作为见证——Rule-P 从不消费它们）。多前驱规则写作 `{ lab_result + imaging_result } → treatment_plan`；自环（`a → a`）合法。
- **gates**——参与者级能力：`reads`（Rule-R，非破坏）、`consumes`（Rule-E，显式、归档）、`produces`（Rule-P，不接受条件子句）。read/consume 门可带条件：`via T`（溯源见证）、`exists T`、`count T >= n`（仅正整数）。
- **seed_authority**——谁可以注入初始 token（Rule-S）。

本域的可运行版本与另外四份域（含扇入前置与 `via` 溯源的医疗路径、ReAct 循环、跨 harness 多智能体协作，以及以见证 token 作为准入条件的外部物化确认）都在 [`examples/domains/`](examples/README.md)。

**试一试**——把上文声明保存为 `lifecycle.pact`，然后：

```bash
# 终端 1——启动 daemon（无 pact.toml 时使用默认配置）
pact daemon start

# 终端 2——加载域、播种入口类型、开始协作
pact domain load lifecycle.pact --name proj
pact seed --domain proj --participant external_stakeholder --type requirement
pact enabled --domain proj --participant architect
pact produce --domain proj --participant architect --type design --predecessors <requirement-token-id>
pact status --domain proj
```

`pact seed` 会打印 token id，用作 `--predecessors` 的参数。含 gate 拦截与 Rule-T 终止的完整 14 步实录见 [`examples/online/trace.txt`](examples/online/trace.txt)。

## 与论文的对应关系

| 论文章节 | 实现模块 |
|---|---|
| §1.4 SOS 规则（Rule-P/R/E/S/W/T） | `crates/pact-core/src/rules.rs` |
| §1.4.0 / §1.7 Gate 条件（`GateHolds`、`via`/`exists`/`count`） | `crates/pact-core/src/gate.rs` |
| §1.4.3–§1.4.4 源类型与良构约束 | `crates/pact-core/src/domain.rs`（声明期校验）、`crates/pact-parser/src/convert.rs`（解析期 LHS 唯一性） |
| §1.7.3 正整数 count 字面量 | `crates/pact-parser/src/parser.rs` |
| §1.8 并行组合（C1–C4） | `crates/pact-compose` |
| §3.1 声明文法 | `crates/pact-parser` |
| 附录 A 静态分析（A.1–A.6） | `crates/pact-analysis` |
| A.5 / 附录 D 报告顺序 | `crates/pact-engine/src/engine.rs` |
| §4.5 / 附录 B CPN 编码（单实例作用域） | `crates/pact-cpn`（离线 `pact cpn`） |

## 项目结构

| Crate | 职责 |
|---|---|
| `pact-core` | 领域模型、`GateExpr`、SOS 规则、良构校验（纯函数，零 I/O） |
| `pact-parser` | 声明文法的词法 + 递归下降解析器、`via` 脱糖 |
| `pact-analysis` | 六项静态分析 |
| `pact-compose` | 并行组合与一致性检查 |
| `pact-store` | 只追加事件日志、快照、崩溃恢复 |
| `pact-engine` | 会话运行时、通知、确定性时钟/id |
| `pact-http` | Axum REST daemon（unix socket / TCP + TLS/mTLS、Bearer 或 Ed25519 签名、SSE）、内建 `/mcp`、OIDC 资源服务器与类型化 client |
| `pact-mcp` | MCP stdio 薄封装（21 个 `pact_*` 工具，经 REST 接入） |
| `pact-runtime` | 多 agent 编排：参与者自驱循环、gate、outbox/saga 执行器 |
| `pact-cpn` | 域声明的 CPN Tools XML 编码（正向片段 + 互补库所；离线、无 I/O） |
| `pact-cli` | 单一 `pact` 二进制：daemon 控制、在线操作、离线 validate/analyze/compose/replay/cpn/ref lint |

## 测试

```bash
cargo test --workspace
```

workspace 内 245 项测试，覆盖：

- **黄金轨迹**——论文四个示例域：软件生命周期（上文）、医疗路径（扇入前置、`via` 溯源）、ReAct 循环（环形拓扑）、区分性死锁场景。
- **属性测试**——声明往返同一性与执行不变量（各 512 用例）。
- **各 crate 单元测试**——含良构拒绝用例（LHS 重复、空 LHS、零/前导零 count 字面量）与源生产合法性三层强制。
- **三接口端到端**——HTTP daemon 全生命周期（错误映射、SSE、compose/unload）、CLI 对真实子进程 daemon、MCP stdio，以及 Streamable HTTP `/mcp` 全部 21 个工具。
- **NFR6 v2 安全**——自定义 CA 的 TLS/mTLS 握手、Ed25519 签名正反用例（篡改 body、nonce 重放、参与者不匹配）、OIDC 正反用例（iss/aud/exp/nbf/alg/crit/claim 映射/未知 kid）、RFC 9728 元数据，以及 IdP 不可达 503。
- **无人值守多智能体场景**——真实 `pact-mcp` 子进程零介入自驱至 `Terminated`：ReAct 循环与 5 个并发 worker 接力生命周期域。
- **CPN 编码**——test arc、互补库所、DNF 展开、实例命名空间与错误路径的结构断言；生成模型通过 CPN DTD 校验。

CI 在每次 push / PR 时运行 `cargo fmt --check`、`cargo clippy -D warnings` 与全量测试。

## 许可证

- 软件与代码：[MIT](LICENSE)
- 论文手稿 [`paper/pactv1_rev3.md`](paper/pactv1_rev3.md)：[CC BY 4.0](paper/LICENSE)
