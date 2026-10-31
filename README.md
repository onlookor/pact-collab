# PACT

**A declarative coordination layer for multi-agent and human–agent workflows.**

[![CI](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml/badge.svg)](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

PACT is a small coordination engine for multi-agent and human–agent workflows. It decides **which collaboration actions are legal right now**, enforces that decision, and records every one of them. It is not an agent framework and not a workflow executor: the participants still do the work.

This repository is the reference implementation: a Rust engine with CLI, REST and MCP interfaces, static analyses, CPN export, and an optional participant runtime.

You describe a workflow as a **domain declaration** (`.pact`): which artifacts can exist, what must already exist before another artifact can be created, who may create / read / consume each artifact, and who may inject the first one. A single-authority daemon holds the current artifact set (the *Counter*), applies those rules to every call from a human, an agent harness, or an application, and appends each accepted action to an immutable event log. Consumed artifacts move to an *Archive*.

The process is therefore **data, not code**: it can be reviewed, statically analyzed, versioned and handed off, replayed, and shared by agents running in different harnesses. Participants still do the work; PACT governs admission and keeps the record.

## What is PACT?

PACT (Precondition-governed Autonomous Counter-based Token) is the formal paradigm behind that layer. Four ideas carry most of it:

- **Token** — an immutable, typed instance of an artifact. Its type, producer, predecessors and external reference are part of the record.
- **Precondition** — a type-level rule: producing a `B` requires one live `A` (or a conjunction such as `{A + C}`). Predecessors are witnesses; production never consumes them.
- **Gate** — who may `read`, `consume`, or `produce` which type. Read and consume gates may carry a state condition such as `via T` (the exact predecessor instance must still be present) or `count T >= n`; production gates are unconditional and rely on the precondition graph.
- **Counter + Archive** — the current artifact set and the append-only record of consumed artifacts. Reading is non-destructive; consumption is explicit, non-cascading and irreversible.

Three properties follow from that design:

1. **Zero implicit actions.** The Counter changes only when a participant explicitly seeds, produces or consumes. There is no auto-advance, hidden cleanup or implicit retry.
2. **Provenance-sensitive admission.** A gate can require the *specific* predecessor instance, not merely "some artifact of type T"; that is what makes approval chains mechanically checkable.
3. **One authority, replayable state.** Every accepted action is an event. Snapshots and indexes are derivatives; the log rebuilds state and doubles as the audit trail.

## Where it fits

**Category:** a declarative, state-based **coordination layer** for discrete artifact workflows. If you need a familiar label, call it a coordination or workflow engine — but note that it does not execute the work itself.

More precisely, PACT is **declarative process orchestration**: the process is defined in the declaration, not drawn as a graph or written as imperative code. The graph is a derived view; execution of the actual tasks stays with the participants.

PACT sits between workflow orchestration, multi-agent coordination, and authorization/audit. It overlaps with all three and is a drop-in replacement for none of them. The closest familiar reference points are:

| You may know it as… | It mainly decides | How PACT differs | Can they be combined? |
|---|---|---|---|
| **Graph agent orchestration** — LangGraph | How an agent flow moves through nodes and edges; the graph and state are defined in code. | PACT declares rules over artifact types and current state; the next legal action is computed from the Counter. A graph is a *derived* view (CPN export), not the runtime object. The declaration can be analyzed before execution. | Yes — graph nodes or agents can act as participants and call PACT. |
| **Durable execution / workflow-as-code** — Temporal, Step Functions | How to run business code durably, with retries, timers, signals and side effects. | The core PACT engine executes no business code, has no clock, retries or timers, and performs no automatic side effects. Every transition is an explicit participant action; the optional runtime can run side effects only after admission, with explicit compensation. | Yes — Temporal can execute the work behind a participant. |
| **BPMN process engines** — Camunda / Zeebe | Full business-process control flow: gateways, events, timers, human tasks, compensation. | PACT has a much smaller core: AND-only preconditions, no OR gateways, no timers, explicit read/consume separation, action-level gates, and built-in static analysis. | Choose BPMN for rich control flow; choose PACT for a small, analyzable coordination kernel. They can meet at the participant boundary. |
| **DAG schedulers / automation** — Airflow, n8n | When to run which task, on a schedule or trigger, with retries and integrations. | PACT has no scheduler, clock or auto-firing. A task happens only when a participant acts and the rules admit it. Coordination is artifact-state-driven, not schedule-driven. | Yes — a scheduler can wake participants; PACT decides whether their actions are legal. |
| **Agent frameworks** — CrewAI, AutoGen, OpenAI Agents SDK | How agents reason, converse, call tools, and hand off work. | PACT has no model calls, messages, roles, task assignment or agent loop. It is a shared rulebook for artifacts across agents and harnesses; agents remain responsible for their own decisions. | Yes — any agent framework can run inside a participant. |
| **Harness team / task-board plugins** — e.g. DSH Agent Teams | Who creates teammates, assigns tasks, and shares a board and messages inside one harness. | PACT does not create teams, assign tasks, or carry messages; it shares artifact rules across harnesses. | Yes — team plugins handle in-team division; PACT handles cross-team / cross-harness admission. |
| **Policy engines** — OPA, Cedar | Whether a request is authorized by a policy over request context. | PACT gates are bound to workflow state and artifact provenance; PACT also governs production and consumption and keeps an archive. It is not a general-purpose policy language. | Yes — OPA/Cedar can supply attribute-level decisions to a participant or gateway. |
| **Petri nets / CPN** — CPN Tools | Formal modeling and state-space analysis of concurrent systems. | PACT is an executable, declaration-based model with built-in participant gates, token identity, provenance and archive. Its raw expressiveness is *incomparable* with P/T nets (paper §4.2–§4.3); it can be encoded into Colored Petri Nets (CPN) for analysis and visualization. | Yes — `pact cpn` emits a CPN model; CPN Tools adds formal exploration. |

The one-line difference: most tools answer **“what should run next?”** or **“who may call this API?”**. PACT answers **“which artifact-level action is admissible now, given what exists, what preceded it, and who is acting?”** — and makes that answer replayable.

### What PACT is not

- **Not an agent framework.** No model calls, prompts, conversations, tool execution or agent lifecycle.
- **Not a task scheduler or workflow executor.** The core engine has no clock, timers, auto-advance or retries; side effects belong to participants or the optional runtime's explicit executor.
- **Not a general workflow engine.** AND-only preconditions; no OR gateways, engine-level cancel/compensation semantics, or dynamic participant sets.
- **Not a general policy engine.** Gates are domain-specific admission rules tied to typed artifacts, not an attribute-based authorization language.
- **Not a distributed system.** One authority per Counter instance; no consensus, replication or cross-instance coordination.
- **Not a graph editor.** You define the process as rules; the graph is a derived representation, not the source of truth.

For the full list of what PACT deliberately does not model, see [`docs/expressiveness-boundaries.md`](docs/expressiveness-boundaries.md).

### When to reach for it

- Your process produces **discrete artifacts** (documents, reports, builds, tickets), and “B after A” is a real constraint rather than a preference.
- More than one participant — human, agent, or service — must agree on **who may create, read, or consume** each artifact.
- You need **cross-harness coordination** or an auditable record, but want to keep your existing agent framework or executor.
- You want to **check the process before running it**: deadlock, stranding, unreachable states, gate coverage.
- You need **provenance-sensitive approval** (this specific review must survive until deployment), not just role-based access.

If your process is continuous, timer-driven, mostly OR-branches, or needs automatic retries and side effects, PACT is probably the wrong layer.

## What you get

**Model and check**

- **Domain declarations** — a four-section `.pact` grammar (`types` / `preconditions` / `gates` / `seed_authority`) with all six well-formedness constraints from the paper enforced.
- **Six static analyses** — reachability, gate coverage, consumption reachability, structural deadlock (SCC-based), precondition/source-production well-formedness, and stranding risk.
- **Fragment composition** — parallel composition of domains under the four consistency conditions (C1–C4), with explicit conflict reporting.
- **CPN export** — offline encoding of a declaration into a CPN Tools model (`pact cpn`) for formal validation and visualization.
- **ThingRef static lint** — offline `pact ref lint` checks RefBindings, ContractCards, Tombstones and the `.pact` declaration for consistent types, contract references, schemes, retention policy and deletion evidence before references are used.
- **ThingRef visibility** — Counter/Archive expose `thing_ref` only under `read-gate` (unconditional reads/consumes) by default, or per RefBindings `domain`/`hidden`; token existence is unaffected.

**Run and govern**

- **Full operational semantics** — Rule-P / R / E / S / W / T with the unified `GateHolds` admission predicate (`via` / `exists` / `count`, combinable with `and` / `or` / `not`).
- **Event-sourced engine** — append-only JSONL log, snapshots, crash recovery, deterministic replay, optional log compaction, and a rule-level idempotency key for consumption.
- **Multiple Counter instances and declaration handoff** — several independent instances per domain declaration, plus an explicit freeze → migrate → activate path for changing the declaration without losing accumulated state.
- **Optional transport and identity** — TLS 1.2/1.3 and mTLS, static bearer tokens, Ed25519 request signatures, domain-level ACL, optional read audit, and an OIDC resource server for `/mcp`.

**Integrate and operate**

- **Three interfaces, one authority** — REST (unix socket or TCP, SSE notifications), MCP (stdio and Streamable HTTP `/mcp`, 21 tools), and a single `pact` CLI.
- **Participant runtime** — `pact-runtime` provides reusable participant drive loops, multi-agent orchestration, and a consume-event side-effect executor with saga-style compensation.
- **Operations surface** — health/readiness/metrics, graceful shutdown, PID file, systemd/container artifacts.
- **Local embedding (WASM)** — when you want PACT's admission rules inside a Node or desktop app without running a daemon, a WASM build applies the same declarations, events and snapshots in-process for a single user. A local project keeps the native disk layout, so it can move to the daemon later. It does not serve HTTP/MCP, does not carry ACL, read audit or multi-user boundaries, and is not the authoritative deployment; the native daemon remains that. See [manual §2.7](docs/manual.md).
- **Harness adapter** — [`dsh-plugin-pact`](https://github.com/onlookor/pact-dsh) connects DeepSeek Harness agents to the engine. The default `observe` mode only mirrors events; in `govern` mode the PACT domain decides whether a step or tool may run, while DSH still owns the agent loop, model calls and tool execution. The adapter is not the coordination model; the engine is harness-independent and can serve agents from different harnesses at once.

## Where to start

PACT has one onboarding ladder: authoring contract → [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) → browser workbench → sandbox → connection wizard. The three browser steps are also online at **<https://onlookor.github.io/pact-collab/>**; every path ends at `pact analyze` and the [user manual](docs/manual.md).

| If you want to… | Start here | Then |
|---|---|---|
| Understand what PACT is and where it fits | [What is PACT?](#what-is-pact) and [Where it fits](#where-it-fits) | [manual §1](docs/manual.md) for the category comparison, then [`paper/pactv1_rev3.md`](paper/pactv1_rev3.md) |
| Decide whether PACT fits, or read the paradigm | [What is PACT?](#what-is-pact) and [`paper/pactv1_rev3.md`](paper/pactv1_rev3.md) | [`expressiveness-boundaries.md`](docs/expressiveness-boundaries.md) — what PACT deliberately does not model |
| Write your first domain inside a coding agent | [`pact-domain-authoring` skill](skills/pact-domain-authoring/SKILL.md) | `pact analyze <file>` until it exits 0; [example domains](examples/README.md) |
| Write your first domain with no toolchain installed | [Authoring workbench](https://onlookor.github.io/pact-collab/authoring/) ([offline file](web/authoring/index.html)) | [Sandbox](https://onlookor.github.io/pact-collab/sandbox/) to run it in memory |
| Write a domain by hand, or follow the full protocol | [`docs/authoring-contract.md`](docs/authoring-contract.md) | [manual §3](docs/manual.md) for grammar, §5 for the six analyses |
| Run a domain and drive agents | [manual §2](docs/manual.md) (daemon, `pact.toml`, identity) and §4 (CLI / REST / MCP) | [`examples/online/run.sh`](examples/online/run.sh), then the [connect wizard](https://onlookor.github.io/pact-collab/connect/) |
| Use PACT inside DeepSeek Harness | [`dsh-plugin-pact`](https://github.com/onlookor/pact-dsh#readme) | [manual §13](docs/manual.md) for `observe` / `govern`, configuration and limits |
| Deploy or operate it | [manual §2.2–§2.4](docs/manual.md) (TLS, tokens, signatures), §7 (audit/replay), §10 (troubleshooting) | [`architecture.md`](docs/architecture.md) §3–§4, [`design.md`](docs/design.md) §9 |
| Decide when a shared token may be consumed | [`token-consumption-timing.md`](docs/token-consumption-timing.md) | [`authoring-contract.md`](docs/authoring-contract.md) §5 |
| Change the engine or contribute | [`architecture.md`](docs/architecture.md) and [`design.md`](docs/design.md) | [`requirements.md`](docs/requirements.md), [Testing](#testing) |

The same routes are summarized from the manual's side in [manual §12](docs/manual.md); [`examples/README.md`](examples/README.md) ships five runnable domains, two composition fragments and two reproducible demos with recorded traces, and maps each domain to the modelling problem it demonstrates.

## Quick start

Requires a recent stable Rust toolchain.

```bash
cargo build --release

examples/offline/run.sh    # six analyses × 5 domains, well-formedness rejections,
                           # compose accept + C2 rejection, event replay — no daemon needed
examples/online/run.sh     # daemon full lifecycle: load → seed → produce/read/consume
                           # → gate rejection → termination → replay
```

Or run the analyses directly (offline, no daemon):

```bash
pact analyze examples/domains/se_lifecycle.pact          # human-readable report
pact analyze --json examples/domains/react_loop.pact     # machine-readable report
```

For a hand-on daemon walkthrough, see "Try it live" at the end of the next section. Connection options (`--unix` / `--tcp` / `--https` / `--tls-ca` / `--bearer` / `--bearer-file` / `--sign-key` + `--sign-as`) and `--json` are global flags on every command; `pact --help` lists the full command set.

## Domain declaration syntax

The software-lifecycle domain from the paper — a linear pipeline where each artifact type requires its predecessor:

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

- **types** — the token types in the domain. A type with **no** precondition rule is a *source type*: it cannot be produced (Rule-P requires `Precond(T) ≠ ∅`) and enters the Counter only via seed authority.
- **preconditions** — `lhs → rhs` means producing a `rhs` token requires one token of each type in `lhs` to be present on the Counter (they are witnesses — Rule-P never consumes them). Multi-predecessor rules use the `{ lab_result + imaging_result } → treatment_plan` form; self-loops (`a → a`) are legal.
- **gates** — per-participant capabilities: `reads` (Rule-R, non-destructive), `consumes` (Rule-E, explicit, archived), `produces` (Rule-P, no condition clause). Read/consume gates may carry conditions: `via T` (provenance witness), `exists T`, `count T >= n` (positive integers only).
- **seed_authority** — who may inject the initial tokens (Rule-S).

A runnable version of this domain and four more (healthcare pathway with fan-in preconditions and `via` provenance, ReAct loop, cross-agent harness, external materialization gated by a witness token) live in [`examples/domains/`](examples/README.md).

**Try it live** — save the declaration above as `lifecycle.pact`, then:

```bash
# Terminal 1 — start the daemon (defaults apply if no pact.toml exists)
pact daemon start

# Terminal 2 — load the domain, seed the entry type, and act
pact domain load lifecycle.pact --name proj
pact seed --domain proj --participant external_stakeholder --type requirement
pact enabled --domain proj --participant architect
pact produce --domain proj --participant architect --type design --predecessors <requirement-token-id>
pact status --domain proj
```

`pact seed` prints the token id to pass as `--predecessors`. A full 14-step session with real output (including gate rejections and Rule-T termination) is recorded in [`examples/online/trace.txt`](examples/online/trace.txt).

## Mapping to the paper

| Paper section | Implementation |
|---|---|
| §1.4 SOS rules (Rule-P/R/E/S/W/T) | `crates/pact-core/src/rules.rs` |
| §1.4.0 / §1.7 Gate conditions (`GateHolds`, `via`/`exists`/`count`) | `crates/pact-core/src/gate.rs` |
| §1.4.3–§1.4.4 Source types & well-formedness constraints | `crates/pact-core/src/domain.rs` (validation), `crates/pact-parser/src/convert.rs` (LHS uniqueness at parse time) |
| §1.7.3 Positive-nat count literals | `crates/pact-parser/src/parser.rs` |
| §1.8 Parallel composition (C1–C4) | `crates/pact-compose` |
| §3.1 Declaration grammar | `crates/pact-parser` |
| Appendix A Static analyses (A.1–A.6) | `crates/pact-analysis` |
| A.5 / Appendix D report ordering | `crates/pact-engine/src/engine.rs` |
| §4.5 / Appendix B CPN encoding (single-instance scope) | `crates/pact-cpn` (offline `pact cpn`) |

## Project layout

| Crate | Role |
|---|---|
| `pact-core` | Domain model, `GateExpr`, SOS rules, well-formedness validation (pure, no I/O) |
| `pact-parser` | Lexer + recursive-descent parser for the declaration grammar, `via` desugaring |
| `pact-analysis` | The six static analyses |
| `pact-compose` | Parallel composition and consistency checks |
| `pact-store` | Append-only event log, snapshots, crash recovery |
| `pact-engine` | Session runtime, notifications, deterministic clock/id |
| `pact-http` | Axum REST daemon (unix socket / TCP + TLS/mTLS, bearer or Ed25519 signature, SSE), daemon-hosted `/mcp`, OIDC resource server, and typed client |
| `pact-mcp` | MCP stdio thin wrapper (21 `pact_*` tools, backed by REST) |
| `pact-runtime` | Multi-agent orchestrator: participant drive loops, gates, outbox/saga executors |
| `pact-cpn` | CPN Tools XML encoding of a declaration (positive fragment + complementary places; offline, no I/O) |
| `pact-cli` | Single `pact` binary: daemon control, online operations, offline validate/analyze/compose/replay/cpn/ref lint |

## Testing

```bash
cargo test --workspace
```

245 tests across the workspace, covering:

- **Golden traces** for the paper's four example domains — the software lifecycle above, a healthcare pathway (fan-in preconditions, `via` provenance), a ReAct loop (cyclic topology), and a discriminating deadlock scenario.
- **Property-based tests** — declaration round-trip identity and execution invariants (512 cases each).
- **Unit tests** per crate, including well-formedness rejections (duplicate LHS, empty LHS, zero/leading-zero count literals) and all three enforcement layers of source-production legality.
- **End-to-end over all access paths** — HTTP daemon full lifecycle (error mapping, SSE, compose/unload), CLI against a real subprocess daemon, MCP stdio, and Streamable HTTP `/mcp` with all 21 tools.
- **NFR6 v2 security** — TLS/mTLS handshakes with a custom CA, Ed25519 signature positive/negative cases (tampered body, nonce replay, participant mismatch), OIDC positive/negative (iss/aud/exp/nbf/alg/crit/claim mapping/unknown kid), RFC 9728 metadata, and IdP-unreachable 503.
- **Unattended multi-agent scenarios** — real `pact-mcp` subprocesses autonomously driving domains to `Terminated` with zero human intervention: a ReAct loop and five concurrent workers on the lifecycle domain.
- **CPN encoding** — structural assertions for test arcs, complementary places, DNF expansion, instance namespacing, and error paths; generated models validate against the CPN DTD.

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, and the full test suite on every push and pull request.

## License

- Software and code: [MIT](LICENSE)
- Paper manuscript [`paper/pactv1_rev3.md`](paper/pactv1_rev3.md): [CC BY 4.0](paper/LICENSE)
