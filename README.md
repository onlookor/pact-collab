# PACT Reference Implementation

[![CI](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml/badge.svg)](https://github.com/onlookor/pact-collab/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

A Rust implementation of the **PACT** coordination paradigm — domain declaration, execution semantics, static analysis, and a multi-agent runtime, exposed through three interfaces: REST, MCP, and CLI.

## What is PACT?

PACT (Precondition-governed Autonomous Counter-based Token) is a coordination paradigm for multi-agent systems: work items are typed tokens on a per-domain *Counter*, every state change is an explicit rule application (produce / read / consume / seed), and who may do what is governed by typed preconditions plus per-participant gate conditions. Coordination becomes fully explicit, auditable, and amenable to static analysis — no hidden transitions, no implicit side effects.

## Features

- **Domain declarations** — a small four-section `.pact` grammar (`types` / `preconditions` / `gates` / `seed_authority`), with all six well-formedness constraints from the paper enforced: RHS & LHS uniqueness, non-empty LHS, type closure, positive-nat literals, and source-production legality.
- **Execution semantics** — full SOS rules (Rule-P / R / E / S / W / T) with the unified `GateHolds` admission predicate (`via` / `exists` / `count` conditions). Event-sourced engine: append-only log, snapshots, crash recovery, replay equivalence.
- **Source-production legality** — three-layer enforcement (runtime premise of Rule-P, declaration-time validation, independent static re-check).
- **Static analysis** — six analyses from the paper's appendix: reachability, gate coverage, consumption reachability, structural deadlock (SCC-based), precondition well-formedness (A.5), and stranding risk.
- **Fragment composition** — parallel composition of domains with the four consistency conditions (C1–C4).
- **Three interfaces** — REST daemon (unix socket or TCP, bearer auth, SSE notifications), MCP stdio server (15 `pact_*` tools), and a single `pact` CLI for humans and scripts.

## Quick start

Requires a recent stable Rust toolchain.

```bash
cargo build --release
```

The [`examples/`](examples/README.md) directory ships four ready-to-run domains, two composition fragments, and two reproducible demos with recorded traces:

```bash
examples/offline/run.sh    # six analyses × 4 domains, well-formedness rejections,
                           # compose accept + C2 rejection, event replay — no daemon needed
examples/online/run.sh     # daemon full lifecycle: load → seed → produce/read/consume
                           # → gate rejection → termination → replay
```

Or run the analyses directly (offline, no daemon):

```bash
pact analyze examples/domains/se_lifecycle.pact          # human-readable report
pact analyze --json examples/domains/react_loop.pact     # machine-readable report
```

For a hand-on daemon walkthrough, see "Try it live" at the end of the next section. Connection options (`--unix` / `--tcp` / `--bearer`) and `--json` are global flags on every command; `pact --help` lists the full command set.

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

A runnable version of this domain and four more (healthcare pathway with fan-in preconditions and `via` provenance, ReAct loop, cross-agent harness) live in [`examples/domains/`](examples/README.md).

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

## Project layout

| Crate | Role |
|---|---|
| `pact-core` | Domain model, `GateExpr`, SOS rules, well-formedness validation (pure, no I/O) |
| `pact-parser` | Lexer + recursive-descent parser for the declaration grammar, `via` desugaring |
| `pact-analysis` | The six static analyses |
| `pact-compose` | Parallel composition and consistency checks |
| `pact-store` | Append-only event log, snapshots, crash recovery |
| `pact-engine` | Session runtime, notifications, deterministic clock/id |
| `pact-http` | Axum REST daemon (unix socket / TCP + bearer, SSE) and typed client |
| `pact-mcp` | MCP stdio server (15 `pact_*` tools, backed by REST) |
| `pact-cli` | Single `pact` binary: daemon control, online operations, offline validate/analyze/compose/replay |

## Testing

```bash
cargo test --workspace
```

100+ tests across the workspace, covering:

- **Golden traces** for the paper's four example domains — the software lifecycle above, a healthcare pathway (fan-in preconditions, `via` provenance), a ReAct loop (cyclic topology), and a discriminating deadlock scenario.
- **Property-based tests** — declaration round-trip identity and execution invariants (512 cases each).
- **Unit tests** per crate, including well-formedness rejections (duplicate LHS, empty LHS, zero/leading-zero count literals) and all three enforcement layers of source-production legality.
- **End-to-end over all three interfaces** — HTTP daemon full lifecycle (error mapping, SSE, compose/unload), CLI against a real subprocess daemon, MCP stdio with all 15 tools.
- **Unattended multi-agent scenarios** — real `pact-mcp` subprocesses autonomously driving domains to `Terminated` with zero human intervention: a ReAct loop and five concurrent workers on the lifecycle domain.

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, and the full test suite on every push and pull request.

## License

[MIT](LICENSE)
