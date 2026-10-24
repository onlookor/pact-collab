# PACT examples — generic domains, composition fragments, and reproducible traces

**English** | [简体中文](README.zh-CN.md)

This examples directory is tied to no specific skill or methodology — it demonstrates the PACT paradigm itself: what kinds of collaboration it can describe, how to write a scenario as a `.pact` domain declaration, what each of the six static analyses tells you, how the daemon's Gate and precondition graph intercept actions, and how the event log rebuilds state offline.

Artifacts adapted specifically to the two skills (requirements-engineering / software-engineering) live in another repository; see [`docs/skill-adaptation.md`](../docs/skill-adaptation.md). This directory is the **generic** set.

Conventions: every `pact ...` below means `./target/debug/pact` (or the same-named binary installed on your system); `§x.y` refers to the paradigm specification [`paper/pactv1_rev3.md`](../paper/pactv1_rev3.md) (the normative paper manuscript, checked into this repository).

---

## 1. Coverage

### 1.1 Five generic domains (`domains/`)

Counts were measured with `pact domain validate` (an online command — needs the daemon, see §2).

| File | Scenario | types | participants | gates | seed_authority | analyze exit code |
|------|----------|-------|--------------|-------|----------------|-------------------|
| [`domains/se_lifecycle.pact`](domains/se_lifecycle.pact) | Generic software-engineering pipeline (medium granularity: development mainline + read-only review + a `via`-guarded deployment gate) | 5 | 8 | 17 | 1 | 0 |
| [`domains/re_pipeline.pact`](domains/re_pipeline.pact) | Generic requirements-engineering pipeline (conjunctive preconditions + a change-control loop) | 7 | 7 | 19 | 2 | 0 |
| [`domains/react_loop.pact`](domains/react_loop.pact) | Single-agent ReAct loop (cyclic precondition graph that still terminates naturally) | 4 | 2 | 12 | 1 | 0 |
| [`domains/cross_harness.pact`](domains/cross_harness.pact) | Multi-agent collaboration with no central orchestrator | 5 | 4 | 13 | 1 | 0 |
| [`domains/external_materialization.pact`](domains/external_materialization.pact) | External materialization confirmation via a witness token (`readiness` token gates the consume) | 3 | 4 | 4 | 2 | 0 |

Measured results of the six analyses for the five domains (verbatim from `examples/offline/trace.txt`):

| Domain | Well-formed (violations/warnings) | seed_types | reachable/unreachable | gate_coverage | consumption_reachability | structural_deadlock | stranding_risk |
|--------|-----------------------------------|------------|------------------------|---------------|--------------------------|---------------------|----------------|
| se_lifecycle | 0 / 0 | 1 | 5 / 0 | 0 | 0 | 0 | **1** |
| re_pipeline | 0 / 0 | 2 | 7 / 0 | 0 | **2** | 0 | 0 |
| react_loop | 0 / **1** | 1 | 4 / 0 | 0 | 0 | 0 | 0 |
| cross_harness | 0 / 0 | 1 | 5 / 0 | 0 | 0 | 0 | 0 |
| external_materialization | 0 / 0 | 2 | 3 / 0 | 0 | 0 | 0 | 0 |

The three non-zero items are not defects — they are all **advisory**; the rationale is in §4.

### 1.2 Two composition fragments (`fragments/`)

A fragment is "half a domain": it declares only the types, preconditions, and gates it cares about, and is merged into a host domain by `pact compose` under the C1–C4 conditions of §1.8.

| File | Purpose | types | participants | gates | seed_authority |
|------|---------|-------|--------------|-------|----------------|
| [`fragments/security_review.pact`](fragments/security_review.pact) | **Composable**: attach a security-scanning branch to any domain containing `implementation` | 2 | 2 | 4 | 0 |
| [`fragments/security_blocking.pact`](fragments/security_blocking.pact) | **Rejected**: disagrees with the host about `deployment`'s precondition, triggering a C2 conflict | 4 | 1 | 2 | 0 |

Analyzing a fragment on its own yields large unreachable sets (2 for security_review, 4 for security_blocking) — this is **expected**: fragments have no seed_authority, so A.1's BFS has no starting point. Fragments only make sense after composition, so the tables in §1.1 do not list them.

### 1.3 Two scripts and two recorded traces

| File | Description |
|------|-------------|
| [`online/run.sh`](online/run.sh) → [`online/trace.txt`](online/trace.txt) | Full daemon lifecycle in 14 steps (238 lines): start → load → all four action types → Gate interception → Rule-T termination → event replay → unload |
| [`offline/run.sh`](offline/run.sh) → [`offline/trace.txt`](offline/trace.txt) | Offline trio in 5 steps (200 lines): analyze × 5 domains, three kinds of well-formedness violations, compose success and C2 rejection, replay |
| [`offline/fixtures/se_lifecycle.events.jsonl`](offline/fixtures/se_lifecycle.events.jsonl) | A 12-line event log so the offline demo is self-sufficient on a fresh clone (no need to run the online script first) |

`trace.txt` files are **real recorded output**, not hand-written samples: token ids (ULIDs), timestamps, daemon pids, and absolute paths differ per run and have been sanitized to `<REPO>`; step structure, error codes, analysis numbers, and exit codes should match your runs exactly.

---

## 2. Prerequisites and running

```bash
cargo build -p pact-cli                     # produces ./target/debug/pact

examples/offline/run.sh                     # the offline trio, no daemon needed
examples/online/run.sh                      # full online lifecycle, self-starting and self-stopping daemon
```

Both scripts run from any working directory (they locate the repo root via `BASH_SOURCE`); use `PACT=/path/to/pact` to point at a specific binary. To record your own trace:

```bash
examples/offline/run.sh > my-trace.txt 2>&1
```

**Online vs offline commands** — the easiest thing to get wrong:

| Category | Commands | Notes |
|----------|----------|-------|
| Offline (no daemon) | `analyze`, `compose`, `replay` | Pure-function layer only; no processes started, no storage touched |
| Online (must reach the daemon) | `daemon start`, `domain validate\|load\|list\|info\|unload`, `seed`, `produce`, `read`, `consume`, `counter ls`, `archive ls`, `gates show`, `enabled`, `status` | Without a daemon they all fail with `error [Transport]: client error (Connect)` |

Note that `domain validate` is **online**: it parses, checks well-formedness, and dry-runs loading, over the REST channel. For a purely offline well-formedness check use `analyze` (its `precondition_wf` entry is the same judgment).

The online script's side effects land only in `examples/online/run-data/` (emptied at startup); the offline ones land in `examples/offline/out/`. Both directories are gitignored.

---

## 3. The online trace: full daemon lifecycle in 14 steps

Full output: [`online/trace.txt`](online/trace.txt). The table below is a step-by-step guide; the "Rule" column refers to the SOS rules of the paradigm specification (Rule-P produce / Rule-R read / Rule-E consume / Rule-S seed / Rule-W witness / Rule-T termination).

| Step | What happens | What you observe | Rule / mechanism |
|------|--------------|------------------|------------------|
| 0 | Generate `pact.toml` | Two keys: `data_dir` + `listen_unix`; defaults `./data` and `./pact.sock` | Single-machine, single-process deployment |
| 1 | `daemon start` | Prints pid and socket path | The daemon is the **single authority** for the Counter; all three interfaces (CLI/REST/MCP) converge on it |
| 2 | `analyze` before loading | Six analyses; the only non-zero is `stranding_risk: 1` | Static analysis is a pure function — it runs before loading (step 12 turns it into a real rejection) |
| 3 | `domain validate/load/list/info/status` | `ok: types=5 participants=8 gates=17 seed_authority=1`; after loading `counter=0 archive=0 terminated=true` | **An empty domain is already terminated** (both Rule-T conditions hold); flips back to `false` after seeding |
| 4 | `gates show` + `enabled` | deployer has two clauses with `when via implementation`; empty Counter ⇒ `enabled productions for architect: 0` | GateMatrix static authorization + Rule-P dynamic availability |
| 5 | Force `produce`/`seed` on an empty Counter | `PredecessorNotOnCounter`, `SeedAuthorityDenied` | Rule-P requires predecessors **still on the Counter**; Rule-S is constrained by SeedAuthority |
| 6 | A legal `seed` | A token appears; `status` flips to `terminated=false`; `enabled` becomes `1` and reports `design witnesses=[<ULID>]` | Rule-S waives preconditions; the engine tells you which predecessor to use |
| 7 | Unauthorized `consume` / ghost-participant `produce` | Two `StaticGateDenied` | reviewer has `reads` but no `consumes`; `ghost` is not in the participant set |
| 8 | Mainline `read → produce → consume` × 3 | Every `produce` carries `--predecessors`; after `consume` the token shows up in `archive ls` | **Critical order**: produce before consume (see §5 gotcha 3); consumption is explicit, non-cascading, irreversible |
| 9 | Read-only participant + a `via` consumption | Reading the already-consumed design → `TokenNotOnCounter`; deployer reads test_report successfully (implementation is still on the Counter and a direct predecessor) | Rule-R applies only to tokens on the Counter; `via` is an **instance-level** witness |
| 10 | Empty the Counter | `counter: 0 tokens` | — |
| 11 | Rule-T termination | `counter=0 archive=5 terminated=true`; all five Archive entries carry `exhausted_by`/`at`; another `produce` gives `PredecessorNotOnCounter` | There is **no** separate "terminated" rule in the engine; termination is the corollary of Counter=∅ and no Rule-P-enabled participant |
| 12 | **The stranding trap**: load the same domain as `se_lifecycle_trap` with step 8's order swapped | tester consumes implementation early ⇒ deployer is denied even **reading** test_report: `GateConditionFailed: ... Via(TypeName("implementation"))`; the domain is stuck at `counter=1 archive=3 terminated=false` | The concrete instance of step 2's `stranding_risk`. Three possible fixes — all change the **domain declaration**, not the engine |
| 13 | `ls run-data` + `replay` | `replayed 10 events (domain se_lifecycle): counter=0 archive=5` — identical to step 11's `status` | JSONL is the source of truth; the SQLite index and snapshots are derivatives; crash recovery = snapshot + replay of subsequent events |
| 14 | `domain unload` × 2 | `domain list` prints `(no domains)` | Unloading does not delete the event log; post-hoc audit remains possible |

Step 12 deserves emphasis: it demonstrates a core PACT trade-off — `via` turns "ordering" into a mechanically checkable hard constraint, at the price of the witness outliving the actions that depend on it. The analyzer flagged the risk before loading (`potential=false` = a **deterministic** report for acyclic topologies), and at runtime the rejection came exactly as predicted. Fixes (pick one):

- a) have tester not `consume implementation` (the witness outlives test_report);
- b) switch deployer's `via` to a longer-lived witness type;
- c) drop the `via` and accept weaker provenance.

---

## 4. Reading the static analyses: the six outputs

`pact analyze` prints six items at once (Appendix A.1–A.6 of the paper). **A.5 well-formedness is the premise of the other five**: violations break their assumptions, so the aggregate report puts `precondition_wf` first — fix violations before interpreting the rest.

| Output line | The question it answers | A non-zero means | Blocks loading? |
|-------------|--------------------------|------------------|-----------------|
| `precondition_wf: ...` | §3.1's six well-formedness constraints + Seed/Produce overlap | violations N>0 ⇒ the declaration is unusable (non-zero exit code); warnings M>0 ⇒ re-examine the modeling intent | Violations block; warnings don't |
| `reachability: N source types, X reachable, Y unreachable` | BFS over the precondition graph from SeedAuthority types | Y>0 ⇒ some types can never be produced | Advisory |
| `gate_coverage: N non-source types without a producer` | Whether every non-source type has at least one producer | N>0 ⇒ some type nobody can produce | Advisory |
| `consumption_reachability: N types without a consumer` | Whether every type (including source types) has at least one consumer | N>0 ⇒ potential tech debt: tokens linger on the Counter | Advisory |
| `structural_deadlock: N strongly connected components` | SCCs of the precondition graph with no seed entry and no external feed | N>0 ⇒ types inside the cycle can never start | Advisory |
| `stranding_risk: N stranding risks` | Can the witness of `via T_a` be consumed before the action that depends on it | N>0 ⇒ there is a real ordering constraint | Advisory |

Exit code: **0 iff well-formed**, regardless of advisory counts. `--json` gives the same data structurally (keys fixed in Appendix A order) — machines read JSON, humans read the plain output.

### Why the three non-zero advisory results are fine

**se_lifecycle's `stranding_risk: 1`** — a real constraint, not a defect.

```json
{ "ty_b": "test_report", "ty_a": "implementation",
  "via_holder": "deployer", "consumer": "tester", "potential": false }
```

deployer guards test_report with `via implementation`, while tester is allowed to consume implementation. The analyzer honestly reports "the witness may vanish first", and step 12 proves that it does. `potential: false` = a deterministic report for acyclic topologies (STRANDING_RISK); `true` = a conservative report for cyclic topologies (POTENTIAL_STRANDING).

**re_pipeline's `consumption_reachability: 2` (`baseline`, `revised_baseline`)** — intentional design; the modeler decides.

A baseline is a **resident configuration asset**: it is read repeatedly (by change_analyst for change-impact assessment and by configuration_manager for revision baselines) but never "used up" by anyone. PACT's A.3 only checks the GateMatrix for `consumes` clauses — it cannot distinguish "forgot to write a consumer" from "there should be no consumer", so it reports honestly and leaves the verdict to the modeler.

Two ways to zero this number, with different trade-offs:

- add a consumer for `baseline` (e.g. configuration_manager consumes the old baseline when releasing `revised_baseline`) — stronger semantics: the old baseline is retired, but can no longer be read;
- accept the non-zero — semantics closer to "a baseline is a long-lived asset", at the cost that the Counter never drains and Rule-T stays unreachable while the baseline is present.

This domain chooses the latter, because requirements-engineering baselines genuinely need long-term visibility. **A non-zero A.3 value carries no judgment of good or bad by itself** — consistent with PACT's stance: analyses advise; the engine does not decide for you.

**react_loop's `SEED_PRODUCE_OVERLAP` warning** — dual-entry ambiguity, calling for an explicit ruling.

```json
{ "SeedProduceOverlap": { "participant": "user", "ty": "observation" } }
```

`observation` is not a source type (it has the precondition `tool_result`), yet user holds seed authority and agent can also produce it. This is safe per the spec (the seeded instance can be consumed as a predecessor; the produce path mints new instances), but two entry points for one type usually indicate modeling confusion or a deployment decision waiting to be made — hence a warning rather than silent acceptance. This domain is **deliberately** built this way: the loop needs an initial push; user injects the first observation, and afterwards agent produces observations from tool_result.

Granularity note: the implementation checks at **type level** (the produces-gate set is the union over all participants — the same person need not hold both), which is wider than the paper §1.4.3 wording of "the same participant" — a conservative superset in the false-positive direction (no false negatives). See [`docs/design.md`](../docs/design.md) D21.

---

## 5. Five gotchas in terminology and output

All five were hit in practice; listed here so you don't repeat them.

**Gotcha 1: the "source types" in `reachability: N source types` is not §1.4's source type.**

Within one `analyze` output, the two "source type" phrases mean different things:

- the N in the `reachability` line counts **`seed_types`**, i.e. `{T' : ∃p, (p,T') ∈ SeedAuthority}` — types holding seed authority;
- the "non-source types" excluded by `gate_coverage` use the **§1.4 definition**: `Precond(T) = ∅`.

The two diverge on pure-cycle domains or fragments without seed_authority. Measured, analyzing `security_review.pact` alone (the CLI's human-readable output is Chinese; JSON keys are English):

```
reachability: 0 源类型，0 可达，2 不可达
gate_coverage: 0 个无 producer 的非源类型
```

Per §1.4, `implementation`'s `Precond` is empty, so it **is** a source type — that's exactly why `gate_coverage` reports 0 (source types never go through Rule-P and are excluded). But `reachability` reports "0 source types" because the fragment has no SeedAuthority. **For machine processing, trust the JSON key `seed_types`** — it is accurate; on pure-cycle domains (like `react_loop`) the human-readable line creates the illusion of "1 source type despite having no §1.4 source type".

**Gotcha 2: `via X` in the source file prints back as `when via X` after serialization.**

`compose` output normalizes `deployer: ... reads test_report via implementation ...` into `reads test_report when via implementation`. The two forms are equivalent; `when` is the explicit keyword the serializer adds. Don't assume compose changed the semantics.

**Gotcha 3: the order must be `read → produce(--predecessors) → consume`; the last two steps cannot be swapped.**

Rule-P requires predecessors **still on the Counter**. Consume-then-produce yields `PredecessorNotOnCounter`, and consumption is irreversible (the token is already in the Archive) — there is no undo command. Steps 8 and 12 are the two orders over the same domain, good and bad.

By the way: `produce` requires the witness ids explicitly via `--predecessors` (comma-separated for several); the engine will not guess for you. If omitted, the token id in the error text is empty — not a bug, just not provided.

**Gotcha 4: a freshly loaded domain reports `terminated=true`.**

Rule-T holds when the Counter is empty and no participant is Rule-P-enabled — an empty domain satisfies both. This is not "a broken domain"; it flips back to `false` as soon as you seed. Likewise, after termination a `produce` fails with `PredecessorNotOnCounter` rather than some "already terminated" error code — there is no separate termination rule in the engine.

**Gotcha 5: `compose` runs well-formedness validation per fragment (including Type Closure).**

A fragment must **self-sufficiently** declare every type it references; don't count on the host to fill gaps. That's why `security_review.pact` explicitly declares `types { implementation; security_report }` even though `implementation` already exists in the host. The merge checks (C1–C4) run only after per-fragment validation.

---

## 6. The offline trio: analyze / compose / replay

None of the three touch the daemon — pure-function entry points you can drop into any script or CI stage. Full output: [`offline/trace.txt`](offline/trace.txt).

### 6.1 analyze — a static check-up before loading

```bash
pact analyze domains/se_lifecycle.pact          # human-readable, exit 0 = well-formed
pact --json analyze domains/se_lifecycle.pact   # machine-readable, six structured items
```

What three kinds of well-formedness violations actually look like (the script generates broken domains on the fly via heredocs; nothing is committed):

| Violated constraint | Violation (the `--json` `detail`) |
|---------------------|-----------------------------------|
| §3.1 constraint 6, Source-Production Constraint | `{"SourceTypeProduceGate":{"participant":"p","ty":"a"}}` |
| RHS Uniqueness (two precondition rules for one type) | `{"DuplicatePrecondRule":{"ty":"c"}}` |
| Type Closure (reference to an undeclared type) | `{"UndeclaredType":{"referenced_by":"preconditions","ty":"b"}}` + one report per occurrence in gates |

Type Closure is reported **per occurrence**: the same undeclared type `b` referenced 3 times yields 3 reports (1 in preconditions + 2 in gates) — easier to locate than a bare count.

### 6.2 compose — parallel fragment composition (§1.8, C1–C4)

```bash
pact compose domains/se_lifecycle.pact fragments/security_review.pact > out/merged.pact
pact analyze out/merged.pact      # the merged result must itself be a well-formed domain
```

The four sections merge by **set union**: types / preconditions / gates / seed_authority. A participant's host clauses and fragment clauses fuse into one gate set (measured: deployer goes from 3 clauses to 5). After the merge, `analyze` reports 6 reachable (`security_report` joins) and stranding_risk stays 1 (inherited from the host).

Of the four consistency conditions, C2 (preconditions of shared types must agree) is the easiest to hit:

```bash
pact compose domains/se_lifecycle.pact fragments/security_blocking.pact
# error [ComposeConflict]: fragment composition conflict (1)   -> exit=1
```

```json
{"error":{"code":"ComposeConflict","detail":{"conflicts":[
  "PreconditionMismatch { ty: TypeName(\"deployment\"), pre_a: {TypeName(\"test_report\")}, pre_b: {TypeName(\"security_report\"), TypeName(\"test_report\")} }"]},
  "message":"fragment composition conflict (1)"}}
```

The host says `deployment`'s precondition is `{test_report}`; the fragment says `{test_report, security_report}`. **This is not a tool defect but the paradigm's intent**: the precondition graph is the single source of production legality; when two fragments disagree about "what counts as done", the conflict must be resolved explicitly, never silently merged. The right fix is to edit the host declaration itself (write `deployment`'s rule as the conjunction directly), not to pile contradictory fragments.

C1 precondition-uniqueness preservation / C2 precondition agreement on shared types / C3 shared `(p, action, T)` GateConds must be syntactically identical / C4 a final `validate_domain` backstop.

### 6.3 replay — rebuild state offline from the event log

```bash
pact replay offline/fixtures/se_lifecycle.events.jsonl
# replayed 10 events (domain se_lifecycle): counter=0 archive=5
```

The fixture has 12 lines: 1 `DomainLoaded` (resets state), 1 `Terminated` (informational), and 10 state-changing action events (1 seed + 4 produce + 5 consume). The rebuilt state `counter=0 archive=5` matches step 11's `status` in the online trace exactly.

What event sourcing means here: **the JSONL is the source of truth**; the SQLite (turso) index and snapshots are derivatives that can be deleted and rebuilt. Crash recovery walks the same path — read the latest snapshot, then replay subsequent events. To replay a fresh log: `pact replay examples/online/run-data/se_lifecycle.events.jsonl`.

---

## 7. From scenario to `.pact`: modeling steps and applicability bounds

### 7.1 Five modeling steps

1. **List the discrete artifacts** → `types { ... }`. Test: does the artifact participate in the collaboration through a discrete exists/doesn't-exist state? Continuous quantities (progress percentages, quality scores) are not PACT types.
2. **Write "Y can be made only when X is in place"** → `preconditions { X → Y; {X + Z} → W; }`. **Conjunction (AND) only** — no OR branches. The right-hand side is unique (one precondition rule per type).
3. **List who may read / consume / produce what** → `gates { p: reads T, consumes T, produces T'; }`. When you need ordering, use `via`: `reads T_b when via T_a` requires that **the very instance** of T_a which preceded T_b is still on the Counter when T_b is read. `produces` clauses **cannot carry conditions** (production legality is decided entirely by the precondition graph).
4. **Find the entry points** → `seed_authority { p: seeds T; }`. §1.4 source types (`Precond(T)=∅`) enter **only** via Rule-S; giving them a `produces` gate is a well-formedness violation (`SourceTypeProduceGate`). Cycle domains also need a seed for the initial push.
5. **Run the six `analyze` items and rule on each**, then `domain load` and drive one real chain. A non-zero advisory does not mean "change something" — as with baseline in §4, some non-zeros are design intent.

### 7.2 Applicability criteria (§2.1; all five must hold)

| # | Criterion | Meaning |
|---|-----------|---------|
| 1 | Discrete Artifacts | Artifacts participate through discrete existence/absence states |
| 2 | Precedence Relations | "Some outputs must precede others" relations exist |
| 3 | Transferable Outputs | Discrete outputs usable by other participants exist |
| 4 | Participant Count ≥ 1 | A single-participant system is a legitimate degenerate case |
| 5 | Identifiable Entry Points | Every source type can be tied to a seed authority |

Where the five domains land on these five:

- **se_lifecycle / re_pipeline** hit all of 1–5, the classic pipelines;
- **react_loop** approaches the degenerate case of criterion 4 — agent holds all 12 gates; user holds only seed authority and no gates at all: effectively "one worker + one injector". Its criterion 5 is **vacuously satisfied**: every type has exactly one predecessor, so there are no §1.4 source types. Yet the cycle still needs `seed_authority { user: seeds observation }` for the initial push — **SeedAuthority serves more than source types**; it is also how you ignite a pure cycle (seed waives preconditions, §1.4.3);
- **cross_harness** highlights criterion 3: three harnesses never talk to each other, passing outputs purely through Counter tokens; ordering emerges from Rule-P availability, with no central scheduling.
- **external_materialization** exercises criterion 5 and the witness-token pattern: the external system's state cannot be written into a GateExpr (conditions read only the Counter), so "the artifact is ready" is carried as a `readiness` token seeded by external_system; coordinator's `consumes work_item` is admitted only `when exists readiness`, so the confirmation and the consume fall in the same GateHolds step.

### 7.3 Non-applicability bounds (§2.3; nine items)

| Cannot express | Why | Usual workaround |
|----------------|-----|------------------|
| Continuous processes | The paradigm is discrete | Discretize into milestone artifacts |
| Real-time constraints | The paradigm has no clock | Deadlines live in external systems; PACT only tracks artifact availability |
| Probabilistic branching | The paradigm is deterministic | Keep stochastic decisions inside participants |
| OR branching | Conjunction only (§1.4.4) | Split into several domains, or lift the "or" into a participant's internal choice |
| Artifact-internal structure | Delegated via ThingRef | `--thing-ref` attaches an external identifier (file path, issue id, URL) |
| Automatic lifecycle ordering | By design (§1.6) | Ordering comes from participants' explicit actions; the engine won't do it for you |
| Production-time gate conditions beyond the precondition graph | By design (§1.5.2) | Express them through the precondition graph instead |
| Temporal ordering between consumptions | GateExpr evaluates current state only (§1.7.3) | Encode ordering as witness liveness via `via` |
| Internal production of source types | By design (§1.4) | Source types enter only via Rule-S |

Row 8 is the reason `via` exists — and the root of step 12's stranding trap in §3: GateExpr cannot say "A after B", only "is A present now", so temporal constraints are encoded as witness lifetimes.

This table is the short form. The numbered boundary catalogue (`B1`–`B25`), the encoding patterns (type split, witness token, participant-internal choice, external orchestration), and the list of boundaries that genuinely need a paradigm extension are in [`docs/expressiveness-boundaries.md`](../docs/expressiveness-boundaries.md). Cite `B<number>` rather than restating the workaround.

When one token serves several downstream consumers, *when to consume it* is a separate decision from *what can be expressed*. The decision procedure (D1–D6), consumption patterns (downstream-first drain, keep-as-witness, ack barrier, copy fan-out, lease, archival policy), and anti-patterns are in [`docs/token-consumption-timing.md`](../docs/token-consumption-timing.md). Cite `TC<number>`.

### 7.4 Domain independence (§2.2)

The paradigm itself never changes; switching domains means re-filling four injection points: TypeSchema, PreconditionGraph, GateMatrix, SeedAuthorityDecl. The five domains in this directory are five different fillings — software engineering, requirements engineering, an agent loop, multi-agent collaboration, external materialization confirmation — over the same primitives and the same SOS rules.

The paper also mentions an **optional** fifth injection point, an Archival Policy (`AP: Counter → 2^Counter`, specifying which tokens a system participant should consume when conditions hold). It executes through normal Rule-E, introducing neither new primitives nor implicit system actions. **This implementation does not build it in** (out of scope per [`docs/requirements.md`](../docs/requirements.md) §6); domains that want automatic cleanup can model the janitor as an ordinary participant, driven externally to call `consume`.

---

## 8. References

| Document | Contents |
|----------|----------|
| [`docs/manual.md`](../docs/manual.md) | The authoritative user manual: full command surface of the three interfaces (MCP/REST/CLI), error-code tables, deployment and operations |
| [`docs/requirements.md`](../docs/requirements.md) | Requirements (FR1–FR14, NFR1–NFR6, out of scope) |
| [`docs/design.md`](../docs/design.md) | Design decisions (incl. D21, type-level SeedProduceOverlap detection; D22, examples and manual as deliverable assets) |
| [`docs/architecture.md`](../docs/architecture.md) | Architecture: crate dependencies, data flow, deployment view, networked evolution |
| [`docs/skill-adaptation.md`](../docs/skill-adaptation.md) | Artifacts adapted to the two skills (another repository); complements this directory |
| [`paper/pactv1_rev3.md`](../paper/pactv1_rev3.md) | The paradigm specification master (normative paper manuscript, in this repository); all §x.y references here point to it |
