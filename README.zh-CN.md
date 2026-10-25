# PACT 参考实现

[![CI](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml/badge.svg)](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) | **简体中文**

**PACT** 协同范式的 Rust 实现——领域声明、执行语义、静态分析与多智能体运行时，通过 REST、MCP、CLI 三种接口对外提供。

## PACT 是什么？

PACT（Precondition-governed Autonomous Counter-based Token）是一种面向多智能体系统的协同范式：工作项是领域 *Counter* 上的类型化 token，每次状态变更都是一次显式规则应用（生产 / 读取 / 消费 / 播种），谁能做什么由类型化前置条件与参与者级 gate 条件共同治理。协同因此完全显式、可审计、可静态分析——没有隐藏迁移，没有隐式副作用。

## 功能特性

- **领域声明**——极小的四段式 `.pact` 文法（`types` / `preconditions` / `gates` / `seed_authority`），论文 §3.1 六项良构约束全部强制：RHS/LHS 唯一性、非空 LHS、类型封闭、正整数字面量、源生产合法性。
- **执行语义**——完整 SOS 规则（Rule-P / R / E / S / W / T）与统一准入谓词 `GateHolds`（`via` / `exists` / `count` 条件）。事件溯源引擎：只追加日志、快照、崩溃恢复、重放等价。
- **源生产合法性**——三层强制（Rule-P 运行时前提、声明期良构校验、静态分析独立复核）。
- **静态分析**——论文附录六项分析：可达性、gate 覆盖、消费可达性、结构死锁（SCC）、前置与源生产良构性（A.5）、搁浅风险。
- **片段组合**——领域并行组合，含四条件一致性检查（C1–C4）。
- **三种接口**——REST daemon（unix socket / TCP、bearer 认证、SSE 通知）、MCP stdio 服务器（15 个 `pact_*` 工具）、单一 `pact` CLI。

## 从哪里开始

PACT 只有一条上手阶梯：域作者契约 → [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) → 浏览器工作台 → 运行沙盒 → 接入向导。三个浏览器步骤也可直接在线打开 **<https://onlookor.github.io/pact-collab/>**；每条路径都以 `pact analyze` 与[用户手册](docs/manual.md)收口。

| 你的处境 | 先看 | 然后 |
|---|---|---|
| 判断 PACT 是否适用，或读范式本身 | [PACT 是什么？](#pact-是什么)与[论文母本](paper/pactv1_rev3.md) | [`expressiveness-boundaries.md`](docs/expressiveness-boundaries.md)——PACT 刻意不建模什么 |
| 在 coding agent 里写第一份域声明 | [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) | 跑 `pact analyze <file>` 直到 exit 0；再看[示例域](examples/README.md) |
| 没装工具，想写第一份域声明 | [作者工作台](https://onlookor.github.io/pact-collab/authoring/)（[离线文件](web/authoring/index.html)） | 用[运行沙盒](https://onlookor.github.io/pact-collab/sandbox/)在内存里跑一遍 |
| 手写域声明，或需要完整生成协议 | [《PACT 域作者契约》](docs/authoring-contract.md) | [手册 §3](docs/manual.md)看语法、§5 看六项分析 |
| 跑一个域，并让 agent 驱动它 | [手册 §2](docs/manual.md)（daemon、`pact.toml`、身份）与 §4（CLI / REST / MCP） | [`examples/online/run.sh`](examples/online/run.sh)，再用[接入向导](https://onlookor.github.io/pact-collab/connect/) |
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

动手走一遍 daemon 的完整流程见下一节末尾的"试一试"。连接参数（`--unix` / `--tcp` / `--bearer`）与 `--json` 是所有命令的全局 flag；完整命令集见 `pact --help`。

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

本域的可运行版本与另外四份域（含扇入前置与 `via` 溯源的医疗路径、ReAct 循环、多智能体协作，以及以见证 token 作为准入条件的外部物化确认）都在 [`examples/domains/`](examples/README.md)。

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

## 项目结构

| Crate | 职责 |
|---|---|
| `pact-core` | 领域模型、`GateExpr`、SOS 规则、良构校验（纯函数，零 I/O） |
| `pact-parser` | 声明文法的词法 + 递归下降解析器、`via` 脱糖 |
| `pact-analysis` | 六项静态分析 |
| `pact-compose` | 并行组合与一致性检查 |
| `pact-store` | 只追加事件日志、快照、崩溃恢复 |
| `pact-engine` | 会话运行时、通知、确定性时钟/id |
| `pact-http` | Axum REST daemon（unix socket / TCP + bearer、SSE）与类型化 client |
| `pact-mcp` | MCP stdio 服务器（15 个 `pact_*` 工具，经 REST 接入） |
| `pact-cli` | 单一 `pact` 二进制：daemon 控制、在线操作、离线 validate/analyze/compose/replay |

## 测试

```bash
cargo test --workspace
```

workspace 内 100+ 项测试，覆盖：

- **黄金轨迹**——论文四个示例域：软件生命周期（上文）、医疗路径（扇入前置、`via` 溯源）、ReAct 循环（环形拓扑）、区分性死锁场景。
- **属性测试**——声明往返同一性与执行不变量（各 512 用例）。
- **各 crate 单元测试**——含良构拒绝用例（LHS 重复、空 LHS、零/前导零 count 字面量）与源生产合法性三层强制。
- **三接口端到端**——HTTP daemon 全生命周期（错误映射、SSE、compose/unload）、CLI 对真实子进程 daemon、MCP stdio 全部 15 个工具。
- **无人值守多智能体场景**——真实 `pact-mcp` 子进程零介入自驱至 `Terminated`：ReAct 循环与 5 个并发 worker 接力生命周期域。

CI 在每次 push / PR 时运行 `cargo fmt --check`、`cargo clippy -D warnings` 与全量测试。

## 许可证

- 软件与代码：[MIT](LICENSE)
- 论文手稿 [`paper/pactv1_rev3.md`](paper/pactv1_rev3.md)：[CC BY 4.0](paper/LICENSE)
