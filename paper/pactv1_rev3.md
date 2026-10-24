# PACT: A Precondition-Governed Autonomous Counter-Based Token Paradigm for Discrete Collaborative Production

## Abstract

This paper presents PACT (Precondition-governed Autonomous Counter-based Token paradigm), a formal paradigm for modeling discrete collaborative production processes. PACT abstracts collaboration into six primitives: Token, Participant, Precondition, Gate, Counter, and Exhaustion. Unlike workflow engines or orchestration frameworks, PACT specifies neither temporal constraints nor probabilistic behaviors, focusing exclusively on the structural topology of dependencies and action-level admission conditions. We provide a rigorous structural operational semantics in which reading is non-destructive and consumption is a purely explicit act: a gated participant removes a token from the Counter without any automatic cascade, reference counting, or reverse propagation. Dependency edges (`depends_on`) are recorded at production time for audit provenance but exert no constraint on token lifecycle. Gate constraints are enforced on reading and consumption via a unified predicate framework; production is governed by the precondition graph and static gate permissions. The precondition graph governs production; seed injection is exempt from precondition requirements, enabling cyclic topologies. **Source types (types with no precondition rule) enter the system exclusively via seed injection; Rule-P applies only to types with non-empty predecessor sets.Precondition rules require pairwise-distinct predecessor types; quantity constraints over homogeneous tokens are delegated to the gate condition language. This asymmetry guarantees well-defined termination.** We prove that PACT natively supports compositional assembly of fragments under explicit consistency conditions, define a constructive encoding into Colored Petri Nets using test arcs for both gate condition evaluation and non-destructive predecessor access, and establish a strict expressiveness separation of PACT over Petri nets based on provenance-sensitive witness checking. Static analysis algorithms for reachability, gate coverage, consumption reachability, structural deadlock, source-production legality, and stranding risk are provided with complexity bounds. Applications in software engineering, healthcare, and AI agent harness coordination demonstrate domain independence.

---

## Part I: The Paradigm

### Chapter 1: Intension

#### 1.1 Definition

PACT is a collaborative production paradigm. It specifies: a set of Participants, governed by a Precondition Topology and a Gate Matrix, autonomously issue, read, and exhaust Tokens on a shared Counter. There is no centralized decision logic; however, there exists a shared state medium (the Counter) that serves as the sole coordination substrate. The topology itself constitutes the order.

PACT is organized along three orthogonal axes:

| Axis | Question Answered | Instantiation |
|------|-------------------|---------------|
| **Structure** | What must precede what? | Precondition Graph |
| **Gate** | Who may act, under what conditions? | Gate Matrix |
| **State** | What currently exists, and where did it come from? | Counter + Archive |

#### 1.2 Primitives

The vocabulary of PACT consists of exactly six primitives:

| Primitive | Symbol | Definition |
|-----------|--------|------------|
| Token | `Token` | The seat of an artifact in the collaboration flow. The sole entity managed by the paradigm. |
| Participant | `Participant` | An actor capable of issuing, reading, or exhausting tokens. May be human, machine, organization, or natural process. A participant need not produce; read-only and consume-only roles are valid. |
| Precondition | `Precondition` | A directed relation between type names expressing "prerequisite." Governs Rule-P only. |
| Gate | `Gate` | An action-level admission condition between participants and type names expressing "who may do what, under what conditions." Subsumes static permissions, contextual predicates, and correlation clauses. The static permission matrix is its zero-order fragment. |
| Counter | `Counter` | The shared visible multiset of all unexhausted tokens. |
| Exhaustion | `Exhaust` | The terminal lifecycle event of a token. Occurs when a gated participant explicitly consumes it. |

Derived concepts (not independent primitives):

| Concept | Definition |
|---------|------------|
| TypeName | A symbolic label identifying a class of tokens. Used within Precondition and Gate rules. Not itself a primitive. |
| ThingRef | A token's reference to an external artifact. Artifacts are not managed by the paradigm. |
| Archive | Immutable record of exhausted tokens. Tokens enter the Archive upon leaving the Counter. |
| GateCondition | A predicate within a gate clause evaluated at action time. The `via` clause is a built-in gate condition; domains may define additional conditions (§1.7). |
| Seed | An externally injected initial token with no predecessors. Exempt from precondition requirements. The sole entry mechanism for source types. |

#### 1.3 What PACT Does Not Specify

PACT does not specify:

- Content, format, or storage location of artifacts (delegated via `ThingRef`)
- Internal decision processes of participants
- Domain semantics of precondition relations
- Acquisition mechanisms for gate conditions
- Physical implementation of the Counter
- **Counter instantiation and partitioning.** PACT specifies the semantics of a single Counter. Whether one domain declaration is executed against one Counter or multiple concurrent Counters is an implementation concern. When multiple Counter instances share a declaration, all rules and properties stated in this paper apply per instance; cross-instance coordination is outside the scope of the paradigm.
- Temporal constraints, probabilities, or continuous quantities
- Token lifecycle ordering. PACT does not enforce that upstream tokens must outlive downstream tokens. If a domain requires such ordering, it must be encoded in the Gate matrix.
- **Token identity vs. referent identity.** `TokenId` identifies a token instance. `ThingRef` is an opaque reference to an external referent. PACT defines neither equality, versioning, mutability, nor cardinality over `ThingRef`. Two distinct tokens may carry the same `ThingRef`, and one external referent may be referenced by multiple tokens. Any correspondence between token instances and external referents—including idempotent seeding, duplicate prevention, referent-version correspondence, and referent-level uniqueness—is a domain or implementation concern.
- **Timestamp semantics.** `Timestamp` records when a token was created or exhausted, for audit purposes only. PACT assigns it no ordering, causal, or freshness semantics; no rule or gate condition inspects it, and monotonicity is not guaranteed.

PACT specifies structure, not content; admission, not compulsion.

#### 1.4 Operational Semantics

Defined in Structural Operational Semantics (SOS). Each rule reads: if all premises above the line hold, then the conclusion below occurs.

**Design Principle (Zero Implicit Actions).** Every state transition in PACT is initiated by an explicit participant action. There are no automatic cascades, no system-triggered decrements, no implicit cleanups. The Counter changes if and only if a participant invokes Rule-P, Rule-E, or Rule-S.

##### §1.4.0 GateHolds (Unified Admission Predicate)

For brevity, the syntactic forms `reads` and `consumes` are normalized to `read` and `consume` when used as action parameters in the following definitions.

**Definition (GateHolds).** Let p be a participant, action ∈ {read, consume}, and t be the target token.

`GateHolds(p, action, t)` holds iff:

1. **Static gate:** `(p, action, type(t)) ∈ GateMatrix`; AND
2. **Gate conditions:** For every gate condition clause `(action', T_b, cond)` in Gate(p) where `action' = action` AND `type(t) = T_b`: `cond(t, Counter)` evaluates to true. If no clause matches the pair (action, type(t)), or if the matched clause carries no condition (`GateCondOpt = ε`), the universal quantification is vacuously satisfied and the gate holds.

The built-in condition `via T_a` is defined as:

> `via T_a (t, Counter)` holds iff ∃ w ∈ Counter such that `type(w) = T_a` AND `w.id ∈ t.depends_on`.

**Witness requirement.** The witness token w must satisfy both conditions simultaneously: (1) w is currently on the Counter, and (2) w.id appears in t.depends_on. Condition (1) ensures the witness is accessible at the time of the action; condition (2) ensures provenance-specific dependency: the witness must be the *exact instance* that participated in producing t, not merely any token of the same type. If the witness has been consumed and archived, condition (1) fails and the action is blocked.

**Provenance specificity.** The `via` condition is *provenance-sensitive*: it does not merely check that "some token of type T_a exists on the Counter," but that the *specific instance* recorded in t.depends_on is still present. This distinction is critical when multiple tokens of type T_a coexist on the Counter: only the one whose id appears in t.depends_on qualifies as witness.

**Defensive note on seed tokens.** A seed token has `depends_on = ∅` (§1.4.3). If a gate condition with a `via` constraint were applied to a seed token, the witness condition `w.id ∈ t.depends_on` could never be satisfied, and the action would be permanently blocked. Domain designers should ensure that no `via` clause targets a type that may appear solely as a seed. In all examples presented in this paper, seed tokens carry no `via` constraints.

**Scope.** This predicate applies only to Rule-R and Rule-E. Rule-P does not invoke gate condition checks; production legitimacy is determined solely by the precondition graph and static gate permissions (§1.4.1).

##### Rule-P (Production)

```
p ∈ Participant
Gate(p) ⊇ {produce: T}
Precond(T) ≠ ∅                        (* source types are seed-only; see §1.4.3 *)
Precond(T) = {T₁, T₂, …, Tₙ}          (* unique; see §1.4.4 *)
{t₁, t₂, …, tₙ} ⊆ₘ Counter            (* multiset inclusion *)
type(tᵢ) = Tᵢ for each i ∈ {1..n}
tᵢ ≠ tⱼ for all i ≠ j                 (* distinct instances *)
─────────────────────────────────────────────────────────
t ← new Token(type=T, participant=p, depends_on={t₁…tₙ}, thing_ref=θ)
Counter ← Counter ⊎ {t}
```

**Note (Predecessor preservation).** `depends_on` records the causal provenance of the new token for audit purposes. It does not modify any state of the predecessor tokens. **Predecessor tokens remain on the Counter unchanged.** Production does not invoke `GateHolds`; the precondition graph is the sole structural gate for production.

**Note (LHS Uniqueness Implication).** By §1.4.4, all predecessor types in a PrecondRule are distinct. Therefore, the premise `tᵢ ≠ tⱼ for all i ≠ j` (distinct token instances) automatically implies `type(tᵢ) ≠ type(tⱼ)` (distinct types). No runtime type-level deduplication is required during predecessor matching. The distinctness check operates solely on TokenId. Predecessor matching inspects only `TypeName` and `TokenId`; `ThingRef` and other token fields are not inspected. Which eligible instances a participant selects is an internal decision: PACT neither constrains nor records the selection policy, and `depends_on` records the instances actually used as provenance rather than as an admission condition.

**Note (Source-type exclusion).** The premise `Precond(T) ≠ ∅` excludes source types from internal production. This is a core design invariant with three consequences:

1. **Termination.** A participant holding only `produces T` for source type T would be perpetually enabled (the empty predecessor requirement is vacuously satisfied), making Rule-T's "no enabled participant" condition unreachable. Excluding source types from Rule-P restores termination.
2. **Unique entry point.** All source-type tokens carry `depends_on = ∅` and are traceable to a SeedAuthority declaration. There is exactly one mechanism (Rule-S) by which source types enter the system, eliminating ambiguity between "externally injected" and "internally produced" source tokens.
3. **Audit clarity.** Every source-type token in the Archive or Counter corresponds to a declared seeding event. New work entering the domain is always an explicit injection, never an implicit production.

**Well-formedness corollary.** A domain declaration containing a `produces T` gate clause where T is a source type is *ill-formed* and rejected at declaration time (§3.1). The operational premise `Precond(T) ≠ ∅` and the syntactic well-formedness check are equivalent formulations of the same invariant.

##### §1.4.1 Multiset Semantics

**Definition.** The Counter is a finite multiset over tokens. Two tokens are considered identical copies only if they share the same `TokenId`. `TokenId` is unique within its Counter; implementations may strengthen this to cross-instance uniqueness to support provenance resolution across concurrent instances. Within a single Counter, the multiset is effectively a set in implementation, but treated as a multiset in matching semantics to correctly handle resource quantity constraints.

##### Rule-R (Read — Non-Destructive)

```
t ∈ Counter
p ∈ Participant
Gate(p) ⊇ {read: type(t)}
GateHolds(p, read, t)
─────────────────────────────────────────────────────────
(* p observes t; no state change to Counter *)
(* Read is idempotent and non-destructive *)
```

##### Rule-E (Exhaustion — Explicit Removal)

```
t ∈ Counter
p ∈ Participant
Gate(p) ⊇ {consume: type(t)}
GateHolds(p, consume, t)
─────────────────────────────────────────────────────────
Counter ← Counter \ {t}
Archive ← Archive ∪ {archive_entry(t, p, now())}
```

**Note.** Consumption is unconditional with respect to downstream tokens. No cascade, no reverse propagation.

##### Rule-W (Wait)

```
p ∈ Participant
¬∃ combination satisfying Rule-P for p
─────────────────────────────────────────────────────────
p suspends production attempts until Counter changes
```

**Clarification.** Rule-W governs only production attempts. A participant in wait state may still execute Rule-R or Rule-E at any time. Waiting does not block reading or consumption. Since Rule-P excludes source types, a participant whose only produce gates target source types is never Rule-P-enabled and remains permanently in wait (such declarations are ill-formed per the well-formedness corollary and rejected at declaration time).

##### §1.4.2 Fairness and Liveness Assumptions

**Assumption (Weak Fairness).** If a participant p is continuously enabled for Rule-P, then p will eventually execute Rule-P.

**Remark (Deadlock Detection).** Weak fairness does not prevent deadlocks arising from cyclic wait dependencies. PACT's core semantics does not include deadlock detection. Static analysis algorithms for structural deadlock detection are provided in Appendix A.

##### Rule-T (Termination)

```
Counter = ∅
¬∃ p ∈ Participant: p is Rule-P-enabled
─────────────────────────────────────────────────────────
Execution terminates
```

**Definition (Rule-P-enabled).** A participant p is Rule-P-enabled iff ∃ T ∈ TypeSchema such that:
1. `Precond(T) ≠ ∅` (source types are excluded per Rule-P); AND
2. Gate(p) ⊇ {produce: T}; AND
3. All predecessor types of T have at least one token on the Counter satisfying the matching conditions of Rule-P.

**Termination argument.** When Counter = ∅, condition (3) fails for every type with Precond(T) ≠ ∅ (no predecessor tokens exist). Therefore no participant is Rule-P-enabled, and Rule-T fires. Participants whose only produce gates target source types are not Rule-P-enabled by definition; such declarations are ill-formed anyway and rejected at declaration time. Participants may still be Rule-R-enabled or Rule-E-enabled when Counter = ∅ only vacuously (no tokens to act upon); these do not block termination.

**Scope note.** This termination condition is stated for a single Counter instance. In a multi-instance deployment, each instance terminates independently when its own Counter becomes empty and no participant is Rule-P-enabled against that instance.

**Remark.** Rule-S is an *injection* mechanism, not a production mechanism. Whether seeding occurs after termination is a domain-level decision outside the scope of Rule-T. A domain that models repeated work batches simply re-seeds and re-enters the execution cycle.

##### Rule-S (Seed Injection)

```
θ is an external thing
T ∈ TypeSchema
(p, T) ∈ SeedAuthority
─────────────────────────────────────────────────────────
t ← new Token(type=T, participant=p, depends_on=∅, thing_ref=θ)
Counter ← Counter ⊎ {t}
```

**Note.** Rule-S applies to any type T for which (p, T) ∈ SeedAuthority, including source types and cyclic types. For source types, Rule-S is the *only* entry mechanism (Rule-P excludes them). For cyclic types, Rule-S provides bootstrapping (§1.4.3); subsequent tokens of the same type are produced via Rule-P.

##### §1.4.3 Seed Semantics, Source Types, and Cyclic Topologies

**Definition (Source Type).** A type T is a *source type* iff T has no PrecondRule, i.e., Precond(T) = ∅. Source types represent externally-originating work items.

**Definition (Seed Exemption).** Rule-S does not require `Precond(T) = ∅`. A seed token may be of any type, including types with non-empty precondition sets. The seed's `depends_on` is always `∅`, reflecting its external origin.

**Theorem (Source-Type Entry Uniqueness).** For any source type T, every token of type T that ever appears on the Counter was introduced by Rule-S.

*Proof.* Rule-P requires Precond(T) ≠ ∅. Since Precond(T) = ∅ for source type T, Rule-P never applies to T. The only remaining rule that adds tokens to the Counter is Rule-S. □

**Rationale for seed exemption.** The precondition graph constrains Rule-P (internal production). Seeds are not produced; they are injected from outside the system. The SeedAuthority relation is the sole gatekeeper for seeding. Requiring `Precond(T) = ∅` for seeds would:

1. Prevent initialization of cyclic topologies (e.g., ReAct loops), where every type has a predecessor;
2. Conflate two orthogonal concerns: production prerequisites (structural) and injection authority (administrative);
3. Force artificial "start" types that add no semantic content.

**Safety.** SeedAuthority is a finite, explicitly declared relation. Only declared `(participant, type)` pairs may seed. A domain that wishes to restrict seeding to source types simply declares SeedAuthority only for types with `Precond(T) = ∅`.

**Interaction summary.** The two mechanisms are complementary and non-overlapping in intent:

| Type category | Entry via Rule-S | Entry via Rule-P |
|---|---|---|
| Source type (Precond = ∅) | ✓ (sole mechanism) | ✗ (excluded) |
| Non-source DAG type | ✓ (optional re-injection) | ✓ (sole internal mechanism) |
| Cyclic type | ✓ (bootstrapping) | ✓ (iteration) |

**Advisory (Seed/Produce Overlap).** If a participant p holds both SeedAuthority for type T and a `produces T` gate clause (where T is non-source), p can introduce T via two distinct mechanisms. This is not a well-formedness violation, but it creates audit ambiguity: tokens of type T originating from p cannot be distinguished as "externally injected" vs. "internally produced" without inspecting `depends_on` (empty for seeds, non-empty for productions). Domain designers are advised to avoid such overlap unless the dual-entry semantics is intentional and documented. Static analysis may flag this as a warning (§A.5).

#### §1.4.4 Precondition Well-Formedness Constraints

**Definition (RHS Uniqueness).** For every type name T in the TypeSchema, there exists at most one PrecondRule whose right-hand side is T. All predecessor types of T must appear within that single rule. Formally:

∀ T ∈ TypeSchema: |{ R ∈ PreconditionGraph : rhs(R) = T }| ≤ 1

**Definition (LHS Uniqueness).** Within any single PrecondRule, all predecessor type names on the left-hand side must be distinct. Formally, for each rule R with lhs(R) = {T₁, …, Tₙ}:

∀ i, j ∈ {1..n}, i ≠ j: Tᵢ ≠ Tⱼ

**Rationale.** PACT’s Counter is effectively a set (§1.4.1): tokens carry IDs unique within their Counter (§1.4.1; implementations may strengthen this to cross-instance uniqueness), and two tokens of the same type are distinguishable only by identity, not by multiplicity. Permitting duplicate type names in a precondition rule (e.g., `{ A + A } → B`) would introduce semantic ambiguity: it could be interpreted as requiring two distinct A-instances (multiset semantics) or silently collapsed to a single A (set semantics). Neither interpretation is desirable. Multiset semantics conflicts with the set-based Counter model and invites silent deadlocks from accidental duplication; set semantics makes the syntax misleading. Quantity constraints over homogeneous tokens are better expressed via the GateExpr language (`count T >= n`, §1.7.3), which preserves the orthogonal separation between structural dependencies (precondition graph) and quantitative/contextual admission conditions (gates). GateExpr governs read and consume only (§1.5, §1.7.3); it cannot constrain production. Consequently, a PrecondRule cannot require multiple distinct instances of the same type as predecessors, and production arity is fixed by the precondition graph.

**Definition (Non-Empty LHS).** Every PrecondRule must have at least one predecessor type. Empty-LHS rules (`{ } → T`) are prohibited. Source types are defined exclusively by the *absence* of a PrecondRule (§1.4), not by an empty rule. An empty-LHS rule would create an ambiguous state where T has a rule (making it non-source) but no predecessors (making Rule-P inapplicable via the `Precond(T) ≠ ∅` premise), potentially rendering T uninstantiable if SeedAuthority also omits it.

**Self-loop note.** A rule of the form `A → A` (self-loop) is permitted. This models iterative refinement or state update within a type: each new token instance records the prior instance in `depends_on`. PACT establishes only a token-instance provenance chain; whether successive instances denote the same logical object, or successive versions of one referent, is a domain interpretation of `ThingRef` and `depends_on` (§1.3). The first instance of A must enter via Rule-S (seed exemption, §1.4.3); subsequent instances are produced via Rule-P using the prior A-instance as predecessor. Self-loops do not violate LHS Uniqueness (single element) or RHS Uniqueness (single rule for A). Termination is preserved because when Counter is empty, no predecessor of A exists in PreconditionGraph, Rule-P does not fire, and the self-loop cannot sustain infinite production; this aligns with the cycle-handling argument in §4.6.

OR-branching—two alternative predecessor sets for one successor—is not natively expressible: RHS Uniqueness permits at most one PrecondRule per type. An alternative must therefore be encoded by (i) giving each branch its own successor type and duplicating the downstream chain; (ii) collapsing the alternatives into one predecessor type whose instances are distinguished by `ThingRef`, with the branch label interpreted by the domain; or (iii) moving the choice into a participant, so that only the selected outcome enters the Counter. GateExpr's `or` applies to read/consume admission only and does not create a production-level OR. None of these encodings yields a single successor type shared by both branches while retaining branch identity.

**Note.** These constraints govern Rule-P only. Rule-S is exempt (§1.4.3).

#### 1.5 Gate Conditions (Formalized)

**Scope.** Gate conditions apply exclusively to reading and consumption. They verify that contextual admission criteria are satisfied before a participant may observe or exhaust a target token. Production is not subject to gate condition checks; its structural requirements are fully captured by the precondition graph.

##### §1.5.1 Formal Definition

`GateHolds(p, action, t)` is defined in §1.4.0. It is invoked in:

- Rule-R: `GateHolds(p, read, t)` — before a participant may read token t.
- Rule-E: `GateHolds(p, consume, t)` — before a participant may consume token t.

In both cases, the witness search space is the current Counter. If the required witness has been consumed and archived, the check fails and the action is blocked.

##### §1.5.2 Design Rationale

Why not check gate conditions at production time? Production legitimacy is a structural question answered by the precondition graph: "what must already exist for T to be produced?" Gate conditions are contextual availability questions: "is the dependency witness still accessible when I want to read/consume this token?" These are orthogonal concerns. Merging them would either (a) block legitimate productions whose predecessor sets don't contain gate condition witnesses, or (b) require extending the grammar with `produces T_b via T_a` clauses. Neither is needed for the domains modeled in this paper.

#### 1.6 What PACT Does Not Guarantee

| Property | Status | Responsibility |
|----------|--------|----------------|
| Upstream outlives downstream | Not guaranteed | Domain gate design |
| No token accumulation | Not guaranteed | Domain consumption policy |
| Complete archival of all tokens | Not guaranteed | Unconsumed tokens remain on Counter; Counter state is itself auditable |
| No dangling references | Not guaranteed | `depends_on` in Archive may reference tokens still on Counter or already archived |
| Gate condition witnesses outlive dependent tokens (instance of row 1) | Not guaranteed | Domain consumption ordering |
| Efficient ancestral provenance queries | Not guaranteed | Archive stores flat entries; transitive seeding ancestry requires recursive depends_on traversal (O(depth) lookups). Domains requiring O(1) ancestry resolution should maintain a materialized provenance index as an external overlay |

**Design Rationale.** These are deliberate non-guarantees. PACT answers "who may consume what," not "when the system must clean up."

**Stranding consequence.** Because PACT does not enforce upstream-outlives-downstream, a gate condition witness may be consumed before the tokens that depend on it. When this occurs, those dependent tokens become *stranded*: they remain on the Counter, and participants whose gates include `via` clauses targeting those token types retain the permission but cannot execute the action, because `GateHolds` fails when the witness is no longer present. Participants without `via` clauses on the same type are unaffected and may still read or consume normally. Stranded tokens are not a paradigm defect; they are a domain design signal. Static stranding-risk analysis is provided in Appendix A (§A.5).

#### 1.7 Gate as an Extensible Constraint Language

The `via` clause is a built-in gate condition. PACT's gate framework is designed to be extensible: domains may define additional gate conditions as predicates over `(token, Counter)`.

##### §1.7.1 Gate Condition Interface

A gate condition is a decidable predicate:

```
GateCond : Token × Counter → Bool
```

The paradigm mandates:
1. **Search space:** The predicate's search space is the current Counter (and optionally the target token's `depends_on` set). It must not reference the Archive or external state.
2. **Decidability:** Every gate condition must be decidable in finite time for any finite Counter.
3. **Purity:** Gate conditions are pure functions; they do not modify the Counter or any token.

##### §1.7.2 Built-in Condition: `via`

```
via T_a (t, Counter) ⇔ ∃ w ∈ Counter: type(w) = T_a ∧ w.id ∈ t.depends_on
```

##### §1.7.3 Composable Condition Language

Gate conditions can be composed using Boolean connectives. The condition language applies to `reads` and `consumes` actions only (§1.5.2).

```
GateExpr ::= GateAtom
           | GateExpr "and" GateExpr
           | GateExpr "or" GateExpr
           | "not" GateExpr

GateAtom ::= "exists" TypeName
           | "count" TypeName "≥" PosNat
           | "count" TypeName "=" PosNat
           | "via" TypeName
           
PosNat ::= [1-9][0-9]*    (* positive integers only; see Normalization rule *)
```

**Semantics:**
- `exists T`: ∃ w ∈ Counter such that type(w) = T.
- `count T ≥ n` (n ∈ PosNat): |{w ∈ Counter : type(w) = T}| ≥ n.
- `count T = n` (n ∈ PosNat): |{w ∈ Counter : type(w) = T}| = n. The case n = 0 is excluded by syntax (Normalization rule above); absence is expressed via `not exists T`.
- `via T_a`: ∃ w ∈ Counter such that type(w) = T_a ∧ w.id ∈ t.depends_on (provenance-sensitive; see §1.4.0).
- `not φ`: negation of condition φ.

> **Normalization rule.** `count T = 0` and `count T >= 0` are syntactically rejected at parse time. The `= 0` form is semantically equivalent to `not exists T` but admits no faithful CPN encoding under the arc-multiplicity construction of §B.3 (zero test arcs degenerate to an unconditional `true` guard, contradicting absence semantics); the `>= 0` form is vacuously true and carries no constraint. Domains requiring absence testing must use `not exists T` exclusively. This restriction keeps the GateExpr surface language unambiguous with respect to its CPN encoding (Appendix B).

**Decidability.** All atoms are decidable in O(|Counter|) time. Boolean composition preserves decidability. The full GateExpr language is decidable.

**Note on expressiveness boundary.** The `not` operator enables negative conditions (e.g., "no unresolved rework token exists"). Domains using `not` should verify absence of circular gate dependencies via static analysis (Appendix A, §A.5).

**Note on temporal limitations.** The GateExpr language evaluates the *current* Counter state. It cannot express temporal ordering constraints such as "T_b must be consumed before T_a may be consumed." Such ordering requirements must be enforced through domain conventions or external coordination mechanisms, not through gate conditions.

##### §1.7.4 Syntactic Sugar: `via` in GateCondOpt

The `via` clause appearing directly in `GateCondOpt` position (e.g., `reads T_b via T_a`) is syntactic sugar for the `when` form:

```
reads T_b via T_a
≡
reads T_b when via T_a
```

Both forms invoke the same `via T_a` atom defined in §1.7.3. The sugar form is provided for readability in the common case of a single provenance-sensitive condition. The `when` form is required when composing multiple conditions via Boolean connectives.

#### 1.8 Composability of PACT Fragments

**Goal.** PACT supports the composition of multiple small PACT fragments into larger collaborative processes.

**Definition (Fragment).** A PACT fragment is a tuple D = (Types, Pre, Gates, Seeds) satisfying the well-formedness conditions of §1.4.4, §3.1, and the source-production constraint (§1.4: no `produces T` clause where T is a source type in Types). In particular, all type names referenced in Pre and Gates must belong to Types.

**Definition (Parallel Composition).** Let D₁ = (Types₁, Pre₁, Gates₁, Seeds₁) and D₂ = (Types₂, Pre₂, Gates₂, Seeds₂) be two well-formed PACT fragments. Their parallel composition D₁ ∥ D₂ is defined as:

```
Types    = Types₁ ∪ Types₂
Pre      = Pre₁ ∪ Pre₂
Gates    = Gates₁ ∪ Gates₂
Seeds    = Seeds₁ ∪ Seeds₂
```

**Theorem (Compositionality under Consistency).** D₁ ∥ D₂ is a well-formed PACT domain declaration provided:

1. **Precondition Uniqueness is preserved:** For every T ∈ Types, |{R ∈ Pre : rhs(R) = T}| ≤ 1.
2. **Precondition consistency for shared types:** If T ∈ Types₁ ∩ Types₂, then Pre₁(T) = Pre₂(T).
3. **Gate consistency:** If a triple (p, action, T) appears in both Gates₁ and Gates₂, then the associated GateCond must be syntactically identical. That is, for all (p, action, T) ∈ dom(Gates₁) ∩ dom(Gates₂): GateCond₁(p, action, T) = GateCond₂(p, action, T). If this condition is violated, the composition is undefined.
4. **Source-production closure:** No fragment contains `produces T` for a type T that is a source type in the merged Types. (By condition 2, a type's source/non-source status is consistent across fragments sharing it.)

**Proof.** The composition introduces no new primitives, no new rules, and no new semantic mechanisms. It is a set-union of declarations. Conditions (1)–(2) ensure the precondition well-formedness constraints of §1.4.4 are preserved under union: condition (1) directly enforces RHS Uniqueness on the merged Pre (at most one predecessor rule per type), and condition (2) ensures that a shared type carries the identical predecessor set in both fragments, so the union cannot introduce a conflicting LHS for any rule (LHS Uniqueness). Non-Empty LHS is preserved automatically because composition is a pure set-union of declarations: no rule loses or gains predecessors, so every rule that had a non-empty LHS in its fragment retains exactly that LHS in D₁ ∥ D₂. All three constraints of §1.4.4 are thus maintained. Condition (3) ensures that GateHolds evaluates identically regardless of which fragment contributed the clause. If two fragments assign different conditions to the same (p, action, T), the semantics of the merged gate would be ambiguous: AND-merge could render gates unsatisfiable, OR-merge could weaken intended constraints. Requiring syntactic identity is the minimal consistency condition that avoids both hazards. Condition (4) preserves the source-production invariant (§1.4): since precondition consistency (condition 2) ensures that a shared type has the same predecessor set in all fragments, its source status is stable under merging, and the union of gate matrices cannot introduce a `produces` clause for a source type unless some fragment already contained one, contradicting fragment well-formedness. SeedAuthority is merged by union; since it is a permission relation, overlapping entries produce no conflict. All type names referenced in gate conditions are declared in their respective fragment's TypeSchema by the well-formedness assumption, and thus remain valid in the merged Types = Types₁ ∪ Types₂. □

**Remark (AND-merge alternative).** A domain that intentionally wishes to strengthen a gate upon composition may define an explicit AND-merge operator: `GateCond_merged = GateCond₁ and GateCond₂`. This is a domain-level design decision, not a paradigm-level composition rule. The paradigm mandates syntactic identity as the default to preserve predictability.

**Sequential composition** is achieved through type sharing: if fragment D₁ produces type T and fragment D₂ consumes type T (i.e., T appears in D₂'s precondition graph), then D₁ ∥ D₂ with shared type T yields sequential dependency.

**Example.** Let D₁ encode a "design phase" producing `design`, and D₂ encode an "implementation phase" requiring `design`. Then:

```
D₁: types { requirement; design }
    preconditions { requirement → design }
    gates { architect: reads requirement, produces design }
    seeds { external_stakeholder: seeds requirement }

D₂: types { design; implementation }
    preconditions { design → implementation }
    gates { developer: reads design, produces implementation }

D₁ ∥ D₂:
    types { requirement; design; implementation }
    preconditions { requirement → design; design → implementation }
    gates { architect: reads requirement, produces design;
            developer: reads design, produces implementation }
    seeds { external_stakeholder: seeds requirement }
```

The shared type `design` acts as the composition interface. `requirement` is a source type entered only via seed injection. No additional mechanism is required.

**Corollary.** PACT's compositionality is *native*: it requires no composition operators, no hierarchy, and no additional syntax beyond the four consistency conditions. The flat declaration structure is itself compositional.

---

### Chapter 2: Extension

#### 2.1 Applicability Criteria

A process can be expressed in PACT iff it satisfies all of the following:

1. **Discrete Artifacts:** Outcomes participate in collaboration as discrete existence/non-existence states.
2. **Precedence Relations:** Some outputs must occur before others.
3. **Transferable Outputs:** There exist discrete outcomes usable by other participants.
4. **Participant Count ≥ 1:** One or more participants exist. Single-participant systems are valid degenerate cases.
5. **Identifiable Entry Points:** Every source type (work item with no internal predecessor) can be associated with a seeding authority.

#### 2.2 Domain Independence

PACT accepts arbitrary domain fillings through four injection points (§3.1): TypeSchema, PreconditionGraph, GateMatrix, SeedAuthorityDecl. Replacing these with another domain's specifications transforms PACT into that domain's collaboration model. The paradigm remains invariant.

**Optional Extension Point (Archival Policy).** Domains requiring automatic cleanup may define an Archival Policy as a fifth injection point. An Archival Policy is a partial function `AP: Counter → 2^Counter` specifying which tokens a designated system participant should consume when certain conditions hold. The policy is executed via normal Rule-E invocations; it introduces no new semantic primitives and no implicit system actions.

#### 2.3 Inapplicability Boundaries

| Cannot Express | Reason |
|----------------|--------|
| Continuous processes | Paradigm is discrete |
| Real-time constraints | Paradigm has no clock |
| Probabilistic branching | Paradigm is deterministic |
| OR-branching | Only AND conjunction supported (§1.4.4) |
| Internal artifact structure | Delegated via ThingRef |
| Automatic lifecycle ordering | By design (§1.6) |
| Production-time gate conditions beyond precondition graph | By design (§1.5.2); use precondition graph instead |
| Temporal ordering between consumptions | GateExpr evaluates current state only (§1.7.3) |
| Internal production of source types | By design (§1.4); source types enter only via Rule-S |

---

### Chapter 3: Formal Grammar

#### 3.1 Domain Declaration

```
Domain ::= TypeSchema PreconditionGraph GateMatrix SeedAuthorityDecl

(* ─── Injection Point 1: Type Schema ─── *)
TypeSchema ::= "types" "{" TypeName { ";" TypeName } "}"
TypeName ::= Identifier

(* ─── Injection Point 2: Precondition Graph ─── *)
(* Constraint: each TypeName appears on the RHS of at most one PrecondRule *)
PreconditionGraph ::= "preconditions" "{" PrecondRule { ";" PrecondRule } "}"
PrecondRule ::= TypeName "→" TypeName
              | "{" TypeName { "+" TypeName } "}" "→" TypeName

(* ─── Injection Point 3: Gate Matrix ─── *)
(* GateDefs are separated by ";". ActionClauses within a GateDef
   are separated by ",". *)
GateMatrix ::= "gates" "{" GateDef { ";" GateDef } [ ";" ] "}"
GateDef ::= ParticipantName ":" ActionClause { "," ActionClause }
ActionClause ::= "reads" TypeName GateCondOpt
               | "consumes" TypeName GateCondOpt
               | "produces" TypeName
GateCondOpt ::= ε | "via" TypeName | "when" GateExpr
GateExpr ::= GateAtom
           | GateExpr "and" GateExpr
           | GateExpr "or" GateExpr
           | "not" GateExpr
 GateAtom ::= "via" TypeName
            | "exists" TypeName
            | "count" TypeName ">=" PosNat
            | "count" TypeName "=" PosNat
ParticipantName ::= Identifier
Identifier ::= [a-zA-Z_][a-zA-Z0-9_]*
Nat ::= [0-9]+
PosNat ::= [1-9][0-9]*
```

**Note on ActionClause granularity.** Each clause names exactly one TypeName. Multiple types require multiple clauses. This is intentional: gate conditions bind to individual types, and a multi-type shorthand would create ambiguity about which type the condition modifies.

**Note on `produces`.** The `produces` action does not accept a `GateCondOpt`. Production-time structural requirements are expressed exclusively through the precondition graph. This is a core design invariant (§1.5.2): production legitimacy is determined by what predecessors exist, not by contextual conditions at the moment of production.

**Well-formedness constraints.** A Domain declaration is well-formed iff:
1. **(RHS Uniqueness)** Each TypeName appears on the RHS of at most one PrecondRule (§1.4.4).
2. **(LHS Uniqueness)** Within each PrecondRule, all predecessor TypeNames are distinct (§1.4.4).
3. **(Non-Empty LHS)** No PrecondRule has an empty predecessor set. Source types are indicated solely by absence of a rule (§1.4.4).
4. **(Type Closure)** Every TypeName referenced in PreconditionGraph, GateMatrix, or SeedAuthorityDecl is declared in TypeSchema.
5. **(Positive-Nat Literal)** Every numeric literal in a `count` GateAtom conforms to PosNat (`[1-9][0-9]*`). Literals matching Nat but not PosNat (i.e., `"0"`) are rejected at parse time per §1.7.3 Normalization rule.
6. **(Source-Production Constraint)** No ActionClause of the form `"produces" T` exists where T is a source type

Violations of any constraint render the domain declaration ill-formed and subject to rejection by static analysis (§A.5).

#### 3.2 Token Structure

```
Token ::= "{" TokenId "," TypeName "," ThingRef "," ParticipantName ","
          PrecondRefs "," Timestamp "}"
TokenId ::= Identifier
ThingRef ::= URI | Identifier | "∅"
PrecondRefs ::= "[" TokenId { "," TokenId } "]" | "∅"
Timestamp ::= ISO-8601
URI ::= <RFC 3986 compliant string>
ISO-8601 ::= <ISO 8601 datetime string>
```

#### 3.3 Counter State

```
Counter ::= "⟦" Token { "," Token } "⟧" | "∅"
```

#### 3.4 Archive

```
Archive ::= "⟦" ArchiveEntry { "," ArchiveEntry } "⟧" | "∅"
ArchiveEntry ::= Token
                 "exhausted_at" Timestamp
                 "exhausted_by" ParticipantName
```

#### 3.5 Seed Rule

```
SeedRule ::= "seed" TypeName "by" ParticipantName "referencing" ThingRef
```

---

### Chapter 4: Completeness and Positioning

#### 4.1 Constructive Encoding Theorem (Restricted)

**Theorem (Constructive Encoding for SOAD Workflows).** Any Single-Output AND-structured DAG (SOAD) workflow satisfying §2.1 applicability criteria can be constructively encoded as a PACT domain declaration without loss of structural information.

**Proof Sketch.** Given a SOAD workflow with activities {A₁…Aₙ}, precedence relations {(Aᵢ, Aⱼ)}, and capability mapping:

1. Map each Aᵢ to type Tᵢ ≜ output-of-Aᵢ.
2. Map each (Aᵢ, Aⱼ) to Tᵢ → Tⱼ. Each Tⱼ has at most one predecessor set, satisfying §1.4.4.
3. Map capabilities to produce/read/consume gates.
4. Map source activities (no incoming edges) to SeedAuthority entries. Their corresponding types are source types and receive no `produces` gate clauses (§1.4); the responsible participant holds SeedAuthority for the type instead.

Verification is mechanical. The source-production constraint is satisfied by construction: source activities contribute only to SeedAuthority, not to GateMatrix produce clauses. □

**Honest Boundary.** This theorem covers only SOAD workflows. General processes involving OR-branching, multi-output activities, dynamic participant sets, resource contention, and unbounded loops require extensions beyond current PACT primitives.

#### 4.2 Expressiveness Separation: PACT ⊄ Petri Nets

**Theorem (PACT ⊄ Petri Nets).** There exist PACT domain declarations whose observable behavior cannot be reproduced by any Place/Transition Petri net (with or without self-loops).

**Proof.** The separation rests on PACT's *provenance-sensitive* witness checking, which is strictly more discriminating than the type-level existence testing available in P/T nets.

Consider a PACT domain with types S, W, T and participants p₁, p₂, producer:

```
preconditions { S → W; { W + S } → T }
gates {
    p₁: reads T via W;
    p₂: reads T;
    producer: reads S, reads W, produces W, produces T
}
seed_authority { external: seeds S }
```

By §1.4.4, T's unique precondition rule is `{W + S} → T`. When T is produced via Rule-P, `T.depends_on` contains the specific W-instance and S-instance used. Note that S is a source type (no precondition rule) and enters only via Rule-S; W and T are non-source types produced via Rule-P. The domain is well-formed under the source-production constraint.

**Construction of a discriminating scenario.** Execute the following sequence:

1. Seed S₁. Counter = {S₁}.
2. Producer produces W₁ from S₁. Counter = {S₁, W₁}.
3. Producer produces T₁ from {W₁, S₁}. T₁.depends_on = {W₁.id, S₁.id}. Counter = {S₁, W₁, T₁}.
4. Seed S₂. Counter = {S₁, W₁, T₁, S₂}.
5. Producer produces W₂ from S₂. Counter = {S₁, W₁, T₁, S₂, W₂}.

Now consider two sub-scenarios:

- **State A:** Counter = {S₁, W₁, T₁, S₂, W₂}. Here, `GateHolds(p₁, read, T₁)` holds because W₁ ∈ Counter and W₁.id ∈ T₁.depends_on.
- **State B:** W₁ is consumed (by some participant with `consumes W` gate). Counter = {S₁, T₁, S₂, W₂}. Here, `GateHolds(p₁, read, T₁)` **fails** because although W₂ ∈ Counter and type(W₂) = W, W₂.id ∉ T₁.depends_on. The witness must be the *specific* instance that produced T₁.

In a P/T net, tokens within a place are **indistinguishable**. Place p_W holds tokens of type W, but the net has no mechanism to record *which* W-token participated in producing a specific T-token. A self-loop arc from p_W to a "read T" transition tests only M(p_W) ≥ 1, i.e., "some W exists." In State B, M(p_W) = 1 (W₂ is present), so the self-loop-enabled transition fires. But PACT's semantics requires the transition to be disabled, because the *specific* witness W₁ is gone.

**Formal contradiction.** Suppose a P/T net N with self-loops expresses this domain. Let t_{p₁,read} be the transition modeling p₁'s read of T. For t_{p₁,read} to be disabled in State B while M(p_T) ≥ 1, some input place must be unmarked. The only candidate is p_W (the witness place). But M(p_W) = 1 in State B (W₂ is present). A self-loop from p_W tests M(p_W) ≥ 1, which holds. An inhibitor arc from p_W would disable the transition when M(p_W) = 0, but M(p_W) = 1. No arc construction in P/T nets can distinguish "the correct W is present" from "a wrong W is present," because P/T tokens carry no identity.

Therefore, no P/T Petri net (with or without self-loops) can reproduce the observable behavior of this PACT domain. □

**Remark.** The separation exploits a fundamental asymmetry: PACT tokens carry persistent identity (TokenId) and provenance links (depends_on), while P/T net tokens are anonymous and indistinguishable within a place. Colored Petri Nets can address this by encoding token identity as color, but this requires explicit color construction and guard programming for each domain, whereas PACT provides it as a built-in semantic (§4.5).

#### 4.3 Expressiveness Separation: Petri Nets ⊄ PACT

**Theorem (Petri Nets ⊄ PACT).** There exist Petri net behaviors that cannot be expressed by any PACT domain declaration.

**Proof.** Consider a Petri net with transition t having input places {p₁, p₂} and output places {q₁, q₂}. Firing t atomically consumes one token from p₁ and one from p₂, and produces one token in q₁ and one in q₂. This is a multi-output transition.

In PACT, Rule-P produces exactly one token per invocation. To produce both q₁ and q₂, two separate Rule-P invocations are needed, requiring two distinct type names and two separate precondition rules. However, atomicity is lost: between the two productions, an intermediate state exists where q₁ is on the Counter but q₂ is not. No PACT execution can reproduce the atomic simultaneous production of Petri net firing. □

#### 4.4 Expressiveness Separation: PACT vs. Linda

**Theorem (PACT ⊄ Linda).** There exist PACT behaviors inexpressible in Linda.

**Proof.** Linda's `in` operation removes a tuple matching a pattern; `rd` reads non-destructively; `out` adds a tuple. Linda has no notion of:
1. **Typed precondition topology:** Linda tuples are unstructured; there is no mechanism to require that tuple A must exist before tuple B can be produced.
2. **Participant-gated access:** Any Linda process can perform any operation on any matching tuple. There is no gate matrix.
3. **Archival:** Consumed tuples vanish; there is no native archive.

A PACT domain requiring "only participant p may consume type T, and only while witness W exists" has no Linda encoding. □

**Theorem (Linda ⊄ PACT).** There exist Linda behaviors inexpressible in PACT.

**Proof.** Linda supports content-addressed pattern matching: `in("request", ?x, ?y)` retrieves any tuple matching the pattern, binding variables. PACT tokens are accessed by type name and instance identity, not by content pattern matching.

**Formal argument.** PACT's TypeSchema is a finite, statically declared set of type names. Token selection in PACT is by (type, instance-id) pair. Linda's pattern variables range over an unbounded data domain: `in(?priority, ?payload)` can match any tuple regardless of field values, binding variables to previously unknown values. This constitutes an infinite selection space.

A PACT domain cannot replicate this because:
1. Encoding all possible field values as distinct type names requires an infinite TypeSchema, violating the finite declaration requirement.
2. Encoding field values into ThingRef does not help, because ThingRef is opaque to the paradigm; there is no gate condition or rule that inspects ThingRef content.
3. The gate condition language (§1.7.3) operates on Counter membership and type identity, not on token field values.

Therefore, Linda's content-addressed selection over unbounded domains is inexpressible in PACT. □

**Summary.** PACT and Petri nets, and PACT and Linda, are *incomparable* in expressiveness: each can express behaviors the other cannot.

| | PACT expresses | Other cannot |
|---|---|---|
| PACT vs. P/T Petri nets | Provenance-sensitive witness checking, participant-dependent gate conditions, native archive, seed-only source entry | Atomic multi-output transitions, OR-branching via net structure |
| PACT vs. Linda | Precondition topology, gate matrix, typed tokens, archive, participant identity | Content-addressed pattern matching over unbounded domains, blocking operations |

#### 4.5 Encoding into Colored Petri Nets

Although PACT and P/T nets are incomparable in raw expressiveness (§4.2–4.3), a constructive encoding into *Colored* Petri Nets exists. CPNs support token identity via colors, enabling provenance-sensitive guards. The encoding captures participant identity by generating distinct transitions per (participant, action, type) triple; CPN size is therefore linear in the number of gate matrix entries.

**Scope.** The encoding N(D) models a single Counter instance. When a declaration is executed against multiple concurrent Counter instances, each instance is encoded independently; the resulting CPNs share no places or transitions. Cross-instance coordination is not represented in N(D).

**Definition (CPN Encoding).** Given a PACT domain D = (Types, Pre, Gates, Seeds), construct a CPN N = (P, T, A, Σ, C, G, E, I) as follows:

| PACT Concept | CPN Construct |
|---|---|
| TypeName Tᵢ | Place pᵢ with color set Σᵢ = {TokenId × ParticipantName × ThingRef × PrecondRefs} |
| Counter | Union of all place markings |
| Rule-P for non-source type T with Precond(T) = {T₁…Tₙ} | Transition t_{p,T}^{prod} with **test arcs** from p₁…pₙ (non-consuming), output arc to p_T |
| Gate(p) ⊇ {produce: T} | One transition per participant with produce gate for T (only non-source T, per source-production constraint) |
| Rule-R (read) | Transition t_{p,T}^{read} with self-loop on p_T |
| Rule-E (consume) | Transition t_{p,T}^{consume} with consuming input arc from p_T, output arc to p_archive |
| Gate condition `via T_a` | Test arc (self-loop input + output) from p_{T_a}, binding witness variable y; guard: `y.id ∈ x.depends_on` |
| Rule-S | Transition with no input arcs, output to p_T |
| Archive | Place p_archive accumulating consumed tokens |

**Test arc mechanism for production.** When a production transition for non-source type T requires predecessors T₁, …, Tₖ, the transition receives for each predecessor place pᵢ:
- An input arc from pᵢ, binding variable xᵢ (ensures predecessor token exists);
- An output arc back to pᵢ, returning the same variable xᵢ (predecessor is not consumed).

This pair forms a *test arc* (read arc) on the predecessor: the token is borrowed for the enabling check but not consumed. Additionally, the transition has a normal output arc to p_T producing the new token with `depends_on = [x₁.id, …, xₖ.id]`.

**No production transitions for source types.** By the source-production constraint (§3.1), no `produces T` gate clause exists for source type T. Therefore the encoding generates no production transitions for source types. Source-type tokens enter exclusively via seed transitions (Rule-S), which have no input arcs. This mirrors the PACT semantics exactly and avoids the CPN analog of non-termination: a seed transition with no input arcs is always enabled, but its firing is controlled by the seed authority declaration and is not generated per participant-type produce gate.

**Test arc mechanism for gate conditions.** When a gate condition `via T_a` is present on a read or consume transition for type T, the transition receives:
- An input arc from place p_{T_a}, binding variable y;
- An output arc back to p_{T_a}, returning the same variable y.

This pair forms a *test arc* (read arc): the token is borrowed for the guard evaluation but not consumed. The guard then checks `y.id ∈ x.depends_on` where x is the target token bound from p_T. Because CPN tokens carry identity (TokenId is part of the color), the guard can verify provenance-specific membership.

**Theorem (Encoding Correctness).** For any well-formed PACT domain D, the CPN encoding N(D) is bisimilar to D with respect to Counter states: for every PACT execution trace σ, there exists a CPN firing sequence producing the same sequence of Counter multisets, and vice versa.

**Proof sketch.** The encoding maps each PACT rule to a CPN transition with equivalent enabling conditions. Test arcs on predecessors in production transitions preserve predecessor tokens (non-consuming), matching Rule-P semantics. The self-loop for Rule-R preserves non-destructive read semantics. Gate conditions become CPN guards evaluated on test-arc-bound variables; because CPN colors carry TokenId, the guard `y.id ∈ x.depends_on` correctly implements provenance-sensitive witness checking. Source types have no production transitions, matching Rule-P's source exclusion. Bisimulation follows by induction on trace length. □

**Size.** The encoding produces O(|GateEntries|) transitions, where |GateEntries| is the total number of (participant, action, type) triples in the GateMatrix. This is linear in the domain declaration size. Arc count is O(|GateEntries| × avg_fanin + |GateConditions|), accounting for test arc pairs on predecessors and gate condition witnesses.

**Limitation regarding `not` conditions.** GateExpr clauses containing `not` (e.g., `not exists T_b`) require inhibitor arcs in the CPN encoding. Standard CPN Tools [7] does not natively support inhibitor arcs. Domains using `not` conditions require either an extended CPN variant or a complementary-place encoding (see Appendix B.3 for details). For the positive fragment of GateExpr (without `not`), the encoding is directly compatible with standard CPN tools.

#### 4.6 Cycle Handling

PACT supports cyclic precondition graphs natively via the seed exemption mechanism (§1.4.3):

1. A cyclic graph has no source type (every type has a predecessor).
2. Rule-S seeds the initial token of any type, bypassing precondition requirements.
3. Subsequent iterations proceed via Rule-P, satisfying the precondition graph normally. Since all types in a cycle have non-empty precondition sets, Rule-P's source-type exclusion does not impede iteration.
4. Termination occurs when participants cease producing and consume all remaining tokens. When Counter = ∅, no participant is Rule-P-enabled (all produce gates target non-source types whose predecessors are absent), and Rule-T fires.

No special termination type, unrolling, or structural modification is required.

---

### Chapter 5: Static Analysis

PACT's structural simplicity (directed graph + gate matrix + finite Counter) enables static verification of several important properties. Algorithms and complexity results are provided in Appendix A.

| Property | Question | Complexity |
|----------|----------|------------|
| Reachability | From the seed set, can type T be reached via the precondition graph? | O(\|Types\| + \|Pre\|) |
| Gate Coverage | Does every non-seed type have at least one participant with `produces T` gate? | O(\|Gates\|) |
| Consumption Reachability | Does every type have at least one participant with `consumes T` gate? | O(\|Gates\|) |
| Structural Deadlock | Does the precondition graph contain a strongly connected component with no seed entry? | O(\|Types\| + \|Pre\|) via Tarjan's SCC |
| Source-Production Legality | Does any `produces T` clause target a source type T? | O(\|Gates\|) |
| Stranding Risk | Can a gate condition witness be consumed before all dependent actions complete? | O(\|Gates\| × \|Types\|) for acyclic; conservative for cyclic |

These analyses are performed on the static domain declaration, without executing the system.

---

## Part II: Instantiation

### Section 1: Software Engineering Domain

#### Chapter 1: Basic Lifecycle

##### 1.1 Type Schema

```
types { requirement; design; implementation; test_report; deployment }
```

`requirement` is a source type: it has no PrecondRule. Per the source-production constraint (§3.1), no participant may hold `produces requirement`; requirement enters exclusively via seed injection.

##### 1.2 Precondition Graph

```
preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}
```

##### 1.3 Gate Matrix

```
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, consumes implementation, produces test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}
```

**Note.** The role formerly labeled "analyst" (eliciting the initial requirement from the stakeholder) is modeled as a seeding authority rather than a production gate, since `requirement` is a source type. The elicitation activity is real domain work, but in PACT its output enters the system as an injection, not as an internal production. This is the paradigm's treatment of all source types.

##### 1.4 Seed Authority & Seed

```
seed_authority { external_stakeholder: seeds requirement }
seed requirement by external_stakeholder referencing "stakeholder-need-001"
```

##### 1.5 Execution Trace

```
t₀  Rule-S: requirement injected by external_stakeholder (depends_on=∅)
    Counter: ⟦requirement⟧

t₁  architect reads requirement (Rule-R; no gate condition → trivially holds)
    architect produces design (Rule-P; depends_on={requirement})
    Counter: ⟦requirement, design⟧

t₂  developer reads design (Rule-R)
    developer produces implementation (Rule-P; depends_on={design})
    Counter: ⟦requirement, design, implementation⟧

t₃  tester reads implementation (Rule-R)
    tester produces test_report (Rule-P; depends_on={implementation})
    Counter: ⟦requirement, design, implementation, test_report⟧

t₄  operator reads test_report (Rule-R)
    operator produces deployment (Rule-P; depends_on={test_report})
    Counter: ⟦requirement, design, implementation, test_report, deployment⟧

t₅  stakeholder consumes deployment (Rule-E) → archived
    Counter: ⟦requirement, design, implementation, test_report⟧

t₆  operator consumes test_report (Rule-E) → archived
    Counter: ⟦requirement, design, implementation⟧

t₇  tester consumes implementation (Rule-E) → archived
    Counter: ⟦requirement, design⟧

t₈  developer consumes design (Rule-E) → archived
    Counter: ⟦requirement⟧

t₉  architect consumes requirement (Rule-E) → archived
    Counter: ∅

t₁₀ Rule-T: Counter = ∅ and no participant is Rule-P-enabled
    (all produce gates target non-source types whose predecessors are absent).
    Execution terminates.
```

**Termination verification.** At t₁₀, Counter = ∅. The produce gates in this domain target: design (needs requirement), implementation (needs design), test_report (needs implementation), deployment (needs test_report). Since Counter is empty, no predecessor tokens exist, and no participant is Rule-P-enabled. Rule-T's premises are satisfied. Under the prior formulation (with `analyst: produces requirement`), the analyst would have been perpetually enabled because the empty predecessor requirement is vacuously satisfied, and Rule-T would never fire. The source-production constraint eliminates this anomaly.

#### Chapter 2: Permission Refinement

```
gates {
    architect: reads requirement, consumes requirement, produces design;
    reviewer:  reads requirement, reads design;
    developer: reads design, consumes design, produces implementation;
    qa_lead:   reads implementation, reads test_report;
    tester:    reads implementation, consumes implementation, produces test_report
}
```

`reviewer` and `qa_lead` hold only `reads` gates. They are Participants who never produce or consume.

#### Chapter 3: Approval Correlation

```
gates {
    deployer: reads test_report via implementation,
              consumes test_report via implementation,
              produces deployment
}
```

**Semantics.** Per §1.4.0: when `deployer` reads or consumes a `test_report` token t, `GateHolds` requires ∃ w ∈ Counter with `type(w) = implementation` AND `w.id ∈ t.depends_on`. This prevents acting on a test report whose originating implementation is no longer available.

#### Chapter 4: Quality Control & Parallel Fan-Out

##### 4.1 Multi-Reader Observation

```
Counter: ⟦implementation⟧
  tester_A reads implementation (Rule-R)
  tester_B reads implementation (Rule-R)
  security_scanner reads implementation (Rule-R)
Counter: ⟦implementation⟧    (* unchanged *)
```

##### 4.2 Fan-Out with Independent Consumption

```
types { requirement; design; implementation; test_report; deployment;
        test_report_A; test_report_B; integration_report }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment;
    implementation → test_report_A;
    implementation → test_report_B;
    { test_report_A + test_report_B } → integration_report
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, produces test_report, consumes test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    tester_A: reads implementation, produces test_report_A, consumes test_report_A;
    tester_B: reads implementation, produces test_report_B, consumes test_report_B;
    integrator: reads test_report_A, consumes test_report_A,
                reads test_report_B, consumes test_report_B,
                produces integration_report;
    stakeholder: consumes deployment, consumes integration_report;
    tech_lead: consumes implementation
}

seed_authority { external_stakeholder: seeds requirement }
```

`requirement` is the sole source type; all other types have non-empty predecessor sets.

##### 4.3 Compositionality Demonstration

This fan-out example can be decomposed into three composable fragments:

```
Fragment F₁ (Parallel Testing):
    types { implementation; test_report_A; test_report_B }
    preconditions { implementation → test_report_A;
                    implementation → test_report_B }
    gates { tester_A: reads implementation, produces test_report_A,
                      consumes test_report_A;
            tester_B: reads implementation, produces test_report_B,
                      consumes test_report_B }

Fragment F₂ (Integration):
    types { test_report_A; test_report_B; integration_report }
    preconditions { { test_report_A + test_report_B } → integration_report }
    gates { integrator: reads test_report_A, consumes test_report_A,
                        reads test_report_B, consumes test_report_B,
                        produces integration_report }

Fragment F₃ (Sequential Lifecycle):
    types { requirement; design; implementation; test_report; deployment }
    preconditions { requirement → design; design → implementation;
                    implementation → test_report;
                    test_report → deployment }
    gates { architect: reads requirement, consumes requirement,
                       produces design;
            developer: reads design, consumes design,
                       produces implementation;
            tester: reads implementation, produces test_report,
                    consumes test_report;
            operator: reads test_report, consumes test_report,
                      produces deployment;
            stakeholder: consumes deployment;
            tech_lead: consumes implementation }
    seeds { external_stakeholder: seeds requirement }
```

The full domain is F₁ ∥ F₂ ∥ F₃ with shared types `implementation`, `test_report_A`, `test_report_B`. Consistency conditions of §1.8 are satisfied: no (p, action, T) triple appears in multiple fragments with differing conditions; precondition uniqueness holds for all shared types; all types referenced in preconditions and gates are declared in the respective fragment's TypeSchema; and no fragment contains `produces T` for a source type in the merged domain (requirement is the sole source type, declared in F₃'s seeds, with no produce clause anywhere). No composition operator beyond set union is needed.

#### Chapter 5: Audit Trail

Every Counter→Archive transition is recorded with full `depends_on` provenance. Tokens that are never consumed remain on the Counter; the Counter's current state is itself an auditable artifact. Source-type tokens (e.g., requirement) additionally carry `depends_on = ∅` and a seeding participant, providing a complete provenance chain from external origin to archival.

#### Chapter 6: Technical Debt as Unconsumed Tokens

Tokens that remain on the Counter indefinitely represent structural technical debt, detectable through runtime Counter inspection and static stranding-risk analysis (Appendix A, §A.5).

---

### Section 2: Healthcare Domain Validation

#### Chapter 1: Clinical Pathway Encoding

##### 1.1 Domain Specification

```
types { admission; triage; diagnosis; lab_result; imaging_result; 
        treatment_plan; treatment; discharge_summary }
```

`admission` is the sole source type: it has no PrecondRule and enters exclusively via seed injection by the registrar.

```
preconditions {
    admission → triage;
    triage → diagnosis;
    diagnosis → lab_result;
    diagnosis → imaging_result;
    { lab_result + imaging_result } → treatment_plan;
    treatment_plan → treatment;
    treatment → discharge_summary
}

gates {
    nurse: reads admission, consumes admission, produces triage;
    physician: reads triage, consumes triage, produces diagnosis,
               reads treatment_plan, consumes treatment_plan, produces treatment,
               reads treatment, consumes treatment, produces discharge_summary;
    lab_tech: reads diagnosis via triage, consumes diagnosis via triage,
              produces lab_result, consumes lab_result;
    radiologist: reads diagnosis via triage, consumes diagnosis via triage,
                 produces imaging_result, consumes imaging_result;
    care_team: reads diagnosis, reads lab_result, consumes lab_result,
               reads imaging_result, consumes imaging_result,
               produces treatment_plan;
    admin: consumes discharge_summary;
    quality_officer: reads diagnosis, reads treatment_plan, reads treatment
}

seed_authority { registrar: seeds admission }
```

**Note.** The registrar's role (admitting the patient) is modeled as a seeding authority rather than a production gate, since `admission` is a source type. The admission event is real clinical work, but its output enters the PACT domain as an injection.

##### 1.2 Gate Condition Enforcement

`lab_tech` and `radiologist` hold `reads diagnosis via triage` and `consumes diagnosis via triage`. Per §1.4.0:

When `lab_tech` reads or consumes a `diagnosis` token t, `GateHolds(lab_tech, read/consume, t)` requires ∃ w ∈ Counter with `type(w) = triage` AND `w.id ∈ t.depends_on`. If `triage` has already been consumed and archived, the check fails and `lab_tech` cannot act on `diagnosis`.

##### 1.3 Fan-Out: Parallel Diagnostics

```
t₁  registrar seeds admission (Rule-S; depends_on=∅)
    Counter: ⟦admission⟧

t₂  nurse reads admission; produces triage (depends_on={admission})
    Counter: ⟦admission, triage⟧

t₃  physician reads triage; produces diagnosis (depends_on={triage})
    Counter: ⟦admission, triage, diagnosis⟧

t₄  lab_tech reads diagnosis via triage
      (GateHolds: ∃ w∈Counter, type(w)=triage ∧ w.id∈diagnosis.depends_on ✓)
    lab_tech produces lab_result (Rule-P; depends_on={diagnosis})
    radiologist reads diagnosis via triage (GateHolds ✓)
    radiologist produces imaging_result (Rule-P; depends_on={diagnosis})
    Counter: ⟦admission, triage, diagnosis, lab_result, imaging_result⟧

t₅  care_team produces treatment_plan (depends_on={lab_result, imaging_result})
    Counter: ⟦admission, triage, diagnosis, lab_result, imaging_result,
              treatment_plan⟧

t₆  physician produces treatment (depends_on={treatment_plan})
    physician produces discharge_summary (depends_on={treatment})
    Counter: ⟦admission, triage, diagnosis, lab_result, imaging_result,
              treatment_plan, treatment, discharge_summary⟧

t₇  admin consumes discharge_summary (Rule-E) → archived
t₈  physician consumes treatment (Rule-E) → archived
t₉  physician consumes treatment_plan (Rule-E) → archived
t₁₀ care_team consumes lab_result (Rule-E) → archived
    care_team consumes imaging_result (Rule-E) → archived
t₁₁ lab_tech consumes diagnosis (Rule-E)
      (GateHolds: ∃ w∈Counter, type(w)=triage ∧ w.id∈diagnosis.depends_on ✓)
      → archived
    Counter: ⟦admission, triage⟧
t₁₂ physician consumes triage (Rule-E) → archived
    Counter: ⟦admission⟧
t₁₃ nurse consumes admission (Rule-E) → archived
    Counter: ∅ → Rule-T terminates.
```

**Termination verification.** At t₁₃, Counter = ∅. All produce gates target non-source types (triage, diagnosis, lab_result, imaging_result, treatment_plan, treatment, discharge_summary), each requiring predecessors that are no longer on the Counter. No participant is Rule-P-enabled. Rule-T fires.

##### 1.4 Extended Gate Condition Example

The composable gate condition language (§1.7.3) enables contextual constraints on reading and consumption that go beyond the `via` clause. The following illustrates a negative condition using types already declared in the healthcare domain:

```
gates {
    physician: consumes treatment
               when not exists imaging_result;
    quality_officer: reads treatment_plan
                     when count lab_result >= 1 and count imaging_result >= 1
}
```

**Semantics.** The first clause means: `physician` may consume a `treatment` token only when no `imaging_result` token remains on the Counter (e.g., all imaging results have been reviewed and consumed). The second clause means: `quality_officer` may read `treatment_plan` only when at least one `lab_result` and one `imaging_result` are still present, ensuring the plan's evidentiary basis is auditable.

**Design note.** If a domain requires structural preconditions for production (e.g., "treatment_plan can only be produced after both lab_result and imaging_result exist"), this is expressed through the precondition graph (`{ lab_result + imaging_result } → treatment_plan`), not through gate conditions on `produces`. This separation is a core design invariant (§1.5.2, §3.1): production is governed by structure; reading and consumption are governed by gates.

##### 1.5 Audit Trail

```
Archive (chronological):
  1. discharge_summary  (exhausted_by=admin)
  2. treatment          (exhausted_by=physician)
  3. treatment_plan     (exhausted_by=physician)
  4. lab_result         (exhausted_by=care_team)
  5. imaging_result     (exhausted_by=care_team)
  6. diagnosis          (exhausted_by=lab_tech)
  7. triage             (exhausted_by=physician)
  8. admission          (exhausted_by=nurse, seeded_by=registrar)
```

**Domain Independence Confirmed.** The paradigm accommodates gate conditions, parallel fan-out, read-only oversight, and complete audit trails without modification.

---

## Part III: AI Harness Application

### Chapter 1: Single-Agent ReAct Loop as PACT Instance

#### 1.1 Encoding

```
types { observation; thought; action; tool_result }

preconditions {
    observation → thought;
    thought → action;
    action → tool_result;
    tool_result → observation
}

gates {
    agent: reads observation, consumes observation, produces thought,
           reads thought, consumes thought, produces action,
           reads action, consumes action, produces tool_result,
           reads tool_result, consumes tool_result, produces observation
}

seed_authority { user: seeds observation }
```

**Legality verification.** The precondition graph is cyclic: every type has exactly one predecessor. §1.4.4 is satisfied. There are no source types; the source-production constraint is vacuously satisfied. Rule-S seeds `observation` via the seed exemption (§1.4.3), bootstrapping the cycle. Subsequent `observation` tokens are produced via Rule-P from `tool_result`. The domain is well-formed.

#### 1.2 Cycle Semantics and Natural Termination

```
Initialization:
  user seeds observation₀ (Rule-S; depends_on=∅)
  Counter: ⟦observation₀⟧

Iteration k:
  agent reads observation_k (Rule-R)
  agent produces thought_k (Rule-P; depends_on={observation_k})
  agent consumes observation_k (Rule-E) → archived
  agent reads thought_k (Rule-R)
  agent produces action_k (Rule-P; depends_on={thought_k})
  agent consumes thought_k (Rule-E) → archived
  agent reads action_k (Rule-R)
  agent produces tool_result_k (Rule-P; depends_on={action_k})
  agent consumes action_k (Rule-E) → archived
  If continuing:
    agent reads tool_result_k (Rule-R)
    agent produces observation_{k+1} (Rule-P; depends_on={tool_result_k})
    agent consumes tool_result_k (Rule-E) → archived
    → next iteration
  If stopping:
    agent consumes tool_result_k (Rule-E) → archived
    Counter: ∅ → Rule-T terminates.
```

**Termination verification.** When the agent decides to stop and consumes the final tool_result, Counter = ∅. All produce gates target non-source types (thought, action, tool_result, observation), each requiring a predecessor. With no predecessor tokens present, no Rule-P-enabled participant exists, and Rule-T fires. The cyclic structure does not impede termination because Rule-P-enabledness depends on Counter content, not on graph cyclicity.

#### 1.3 Counter Hygiene in Cyclic Execution

In steady state, Counter never exceeds O(1) tokens. At any point during iteration k, Counter contains at most 2 tokens.

### Chapter 2: Cross-Harness Coordination

#### 2.1 Shared Counter as Coordination Substrate

Multiple AI harnesses coordinate through a shared Counter without direct inter-harness communication. No orchestrator exists. Each harness operates autonomously, triggered only by Rule-P satisfiability.

```
Harness_A produces plan_token → Counter
Harness_B reads plan_token (Rule-R), produces subtask_token → Counter  
Harness_C consumes subtask_token, produces result_token → Counter
Harness_A reads result_token, produces synthesis_token → Counter
```

#### 2.2 MVP Implementation

```python
def gate_holds(participant, action, token, counter):
    """§1.4.0 implementation. Applies to read/consume only."""
    for (act, T_b, cond) in gate_matrix[participant].conditions:
        if act == action and token.type == T_b:
            if cond is None:  # GateCondOpt = ε → vacuously satisfied
                continue
            if not cond(token, counter):
                return False
    return True

def via_condition(witness_type):
    """Built-in gate condition: via T_a (provenance-sensitive)"""
    def check(token, counter):
        return any(w.type == witness_type and w.id in token.depends_on
                   for w in counter)
    return check

def counter_put(token, participant, predecessors, counter,
                precondition_graph, gate_matrix):
    """Rule-P implementation. Predecessors are NOT consumed.
    Source types are rejected."""
    assert ("produce", token.type) in gate_matrix[participant]
    expected_types = precondition_graph.get(token.type, set())
    assert expected_types != set(), \
        "Source types cannot be produced via Rule-P; use counter_seed"
    actual_types = {p.type for p in predecessors}
    assert actual_types == expected_types
    for pred in predecessors:
        assert pred in counter  # must exist, but NOT removed
    assert len(set(id(p) for p in predecessors)) == len(predecessors)
    token.depends_on = [p.id for p in predecessors]
    counter.add(token)  # only the NEW token is added

def counter_read(token, participant, counter, gate_matrix):
    """Rule-R implementation"""
    assert ("read", token.type) in gate_matrix[participant]
    assert gate_holds(participant, "read", token, counter)
    return token  # Non-destructive

def counter_exhaust(token, participant, counter, archive, gate_matrix):
    """Rule-E implementation"""
    assert ("consume", token.type) in gate_matrix[participant]
    assert gate_holds(participant, "consume", token, counter)
    counter.remove(token)
    archive.append(ArchiveEntry(token, participant, now()))

def counter_seed(type_name, participant, thing_ref, counter, seed_authority):
    """Rule-S implementation. No precondition check per §1.4.3.
    Sole entry mechanism for source types."""
    assert (participant, type_name) in seed_authority
    token = Token(type=type_name, participant=participant,
                  depends_on=[], thing_ref=thing_ref)
    counter.add(token)

def validate_domain(domain):
    """Well-formedness check per §3.1."""
    source_types = {t for t in domain.types
                    if t not in {rhs(r) for r in domain.preconditions}}
    for p, clauses in domain.gates.items():
        for clause in clauses:
            if clause.action == "produce" and clause.type in source_types:
                raise WellFormednessError(
                    f"Source type '{clause.type}' cannot have produce gate "
                    f"on participant '{p}'; declare seed authority instead")
```

---

## Related Work

PACT occupies a distinct position within the landscape of coordination models. The three-axis framework (Structure × Gate × State) provides a unified classification:

| Model | Structure | Gate | State |
|-------|-----------|------|-------|
| P/T Petri Nets [1,4] | AND-hypergraph (implicit) | Trivial (enabling = legal) | Place marking multiset |
| Colored Petri Nets [7] | AND-hypergraph + guards | Guard expressions (transition-level) | Colored place marking |
| Linda [3] | None (content matching) | None (any process may act) | Content-addressed tuple space |
| BPMN / YAWL [5,6] | AND/OR gateways + flow | Swimlanes / roles / rules | Process instance data |
| Blackboard Systems | None (opportunistic) | None | Shared workspace |
| Dataflow | DAG (implicit AND) | None (auto-fire) | Token channels |
| **PACT** | **AND topology (DAG or cyclic) + seed exemption** | **Gate matrix + composable conditions** | **Single Counter + Archive** |

**Petri Nets** (Petri, 1962 [1]; Reisig, 1985 [4]). PACT separates reading from consumption; consumption does not cascade. CPNs partially address typing but lack native gate matrices. The encoding in §4.5 bridges the gap for analysis purposes. The expressiveness separation (§4.2) shows PACT can express provenance-sensitive contextual admission that P/T nets cannot, due to token identity and `depends_on` tracking. PACT's source-production constraint (§1.4) has no direct analog in Petri nets: P/T net source places are simply initially marked or fed by source transitions, with no formal distinction between "injection" and "production." This asymmetry is what guarantees PACT's termination property.

**Linda Tuple Spaces** (Gelernter, 1985 [3]). PACT extends Linda with typed tokens, precondition topology, gate matrices, and native archival. The expressiveness separation (§4.4) shows neither subsumes the other: PACT provides structure and access control Linda lacks; Linda provides content-addressed pattern matching over unbounded domains that PACT's finite type system cannot replicate.

**Workflow Nets and YAWL** (van der Aalst, 1998 [5]; ter Hofstede et al., 2003 [6]). Workflow nets provide richer control-flow (OR-join, cancel). PACT provides gate governance and zero-implicit-action semantics. The two are complementary: workflow nets excel at control-flow expressiveness; PACT excels at admission governance and audit. Workflow nets typically model task initiation as part of the flow graph; PACT models work entry as an explicit injection outside the production rule system.

**Linear Logic** (Girard, 1987 [2]). PACT's `consume` is linear; `read` is duplicable. The gate framework corresponds to context-sensitive introduction/elimination rules. Formal connections remain future work.

**Summary of Contributions.** PACT's novel combination: (1) six-primitive minimality with Gate as unified admission abstraction; (2) native separation of read and consume with zero implicit actions; (3) gate-governed access with composable condition language; (4) production governed solely by precondition graph with non-consuming predecessor access; (5) built-in archival auditability; (6) domain independence through four injection points; (7) seed exemption enabling cyclic topologies; (8) **source-production constraint ensuring well-defined termination**; (9) native compositionality of fragments under explicit consistency conditions; (10) strict expressiveness separation from Petri nets based on provenance-sensitive witness checking; (11) constructive CPN encoding with test arcs for both production predecessors and gate conditions; (12) static analysis algorithms for six key properties including source-production legality.

---

## Appendix A: Static Analysis Algorithms

### A.1 Reachability

**Problem.** Given a PACT domain D and a target type T, determine whether T is reachable from any seed-authorized type.

**Algorithm.**

```
REACHABILITY(D, T):
  S ← { T' : ∃ (p, T') ∈ SeedAuthority }
  Worklist ← S
  Visited ← ∅
  while Worklist ≠ ∅:
    X ← pop(Worklist)
    if X = T: return TRUE
    Visited ← Visited ∪ {X}
    for each rule R ∈ Pre where X ∈ lhs(R):
      Y ← rhs(R)
      if Y ∉ Visited:
        Worklist ← Worklist ∪ {Y}
  return FALSE
```

**Complexity.** O(|Types| + |Pre|) — linear in the size of the precondition graph.

### A.2 Gate Coverage (Production)

**Problem.** For every non-source type T, verify that at least one participant holds `produces T` in its gate. Source types are excluded because they enter via SeedAuthority, not Rule-P.

**Algorithm.**

```
GATE_COVERAGE(D):
  SourceTypes ← { T ∈ Types : ¬∃ R ∈ Pre with rhs(R) = T }
  for each T ∈ Types \ SourceTypes:
    if ¬∃ p ∈ Participants: ("produce", T) ∈ Gate(p):
      report UNCOVERED(T)
  return all non-source types covered
```

**Complexity.** O(|Types| + |Pre| + |GateEntries|). The source-type identification requires one pass over Pre; the coverage check requires one pass over GateEntries.

**Note.** Under the prior formulation (without source-production constraint), this algorithm would have checked all types including source types, potentially flagging legitimate seed-only types as uncovered. The revised algorithm correctly skips source types.

### A.3 Consumption Reachability

**Problem.** For every type T (including source types), verify that at least one participant holds `consumes T` in its gate. Unconsumed tokens represent potential technical debt (§II.1.6).

**Algorithm.** Identical structure to A.2 but without excluding source types:

```
CONSUMPTION_REACHABILITY(D):
  for each T ∈ Types:
    if ¬∃ p ∈ Participants: ("consume", T) ∈ Gate(p):
      report UNCONSUMABLE(T)
  return all types consumable
```

**Complexity.** O(|GateEntries|).

### A.4 Structural Deadlock

**Problem.** Determine whether the precondition graph contains a strongly connected component (SCC) with no seed entry point.

**Definition.** An SCC C is *deadlocked* if no type in C appears in SeedAuthority and no type outside C has an edge into C that could be satisfied without first producing a type within C.

**Algorithm.**

```
STRUCTURAL_DEADLOCK(D):
  Compute SCCs of PreconditionGraph using Tarjan's algorithm
  for each SCC C:
    if C is a singleton with no self-loop: continue  (* trivial *)
    if ∃ T ∈ C: (p, T) ∈ SeedAuthority for some p: continue  (* seedable *)
    if ∃ edge (X → T) with X ∉ C, T ∈ C, and X is reachable from seeds:
      continue  (* externally feedable *)
    report DEADLOCK(C)
  return no deadlock
```

**Complexity.** O(|Types| + |Pre|) for Tarjan's SCC + O(|SeedAuthority|) for seed check.

### A.5 Precondition & Source-Production Well-Formedness

**Problem.** Verify that the precondition graph and gate matrix satisfy all well-formedness constraints of §3.1. This is a prerequisite for all other static analyses.

**Algorithm.**

```
PRECONDITION_WELL_FORMEDNESS(D):
  (* 1. RHS Uniqueness *)
  for each T ∈ Types:
    rules ← { R ∈ Pre : rhs(R) = T }
    if |rules| > 1:
      report RHS_DUPLICATE(T, rules)
  
  (* 2. LHS Uniqueness *)
  for each R ∈ Pre:
    lhs_types ← [ T : T ∈ lhs(R) ]
    if |lhs_types| ≠ |set(lhs_types)|:
      duplicates ← lhs_types \ set(lhs_types)
      report LHS_DUPLICATE(rhs(R), duplicates)
  
  (* 3. Non-Empty LHS *)
  for each R ∈ Pre:
    if lhs(R) = ∅:
      report EMPTY_LHS(rhs(R))
  
  (* 4. Type Closure *)
  declared ← Types
  for each R ∈ Pre:
    for each T ∈ lhs(R) ∪ {rhs(R)}:
      if T ∉ declared: report UNDECLARED_TYPE(T, "precondition")
  for each (p, clauses) ∈ Gates:
    for each clause in clauses:
      if clause.type ∉ declared: report UNDECLARED_TYPE(clause.type, "gate")
  for each (p, T) ∈ Seeds:
    if T ∉ declared: report UNDECLARED_TYPE(T, "seed")
  
  (* 5. Positive-Nat Literal *)
  for each clause in Gates with action = "count":
    if clause.value matches /^0+$/:
      report ZERO_COUNT_LITERAL(clause.type, clause.op, clause.value)
  
  (* 6. Source-Production Constraint *)
  SourceTypes ← { T ∈ Types : ¬∃ R ∈ Pre with rhs(R) = T }
  for each (p, clauses) ∈ Gates:
    for each clause in clauses:
      if clause.action = "produce" and clause.type ∈ SourceTypes:
        report ILLEGAL_SOURCE_PRODUCTION(p, clause.type)
  
  (* Advisory: Seed/Produce Overlap *)
  for each (p, T) ∈ Seeds:
    if T ∉ SourceTypes and ("produce", T) ∈ Gate(p):
      warn SEED_PRODUCE_OVERLAP(p, T)
  
  return well_formed
```

**Complexity.** O(|Types| + |Pre| × max_lhs_size + |GateEntries| + |Seeds|). Linear in domain declaration size.

**Remediation.**
- **RHS_DUPLICATE:** Merge rules or rename types to restore uniqueness.
- **LHS_DUPLICATE:** Remove duplicate type from LHS. If multiple instances are intended, use `count T >= n` in a gate condition (§1.7.3) or model as distinct types.
- **EMPTY_LHS:** Delete the empty rule. If T is a source type, declare it via SeedAuthority instead.
- **UNDECLARED_TYPE:** Add missing TypeName to TypeSchema or correct the reference.
- **ZERO_COUNT_LITERAL:** Replace `count T = 0` with `not exists T`, or replace `count T >= 0` with a tautology removal (the condition is vacuously true and can be deleted). See §1.7.3 Normalization rule.
- **ILLEGAL_SOURCE_PRODUCTION:** Move production to SeedAuthority or add a PrecondRule.
- **SEED_PRODUCE_OVERLAP (warning):** Document intent or separate seeding and production responsibilities across participants.

### A.6 Stranding Risk

**Problem.** Determine whether a gate condition of the form `reads/consumes T_b via T_a` can lead to stranded tokens of type T_b, i.e., whether T_a can be consumed while tokens of type T_b still exist on the Counter.

**Definition.** A type T_b is at *stranding risk* if:
1. ∃ participant p with gate condition `via T_a` on action for T_b; AND
2. ∃ participant q with `consumes T_a` gate; AND
3. There is no mechanism in the current GateExpr language to enforce that all T_b-dependent actions complete before T_a consumption.

**Critical semantic note.** The `via T_a` condition checks `w.id ∈ t.depends_on`, where `depends_on` contains only *direct* predecessor token ids (§1.4.0). Therefore, stranding risk exists only when T_a is a *direct* predecessor of T_b in the precondition graph. If T_a is an indirect ancestor but not a direct predecessor, the `via` condition can never be satisfied regardless of T_a's presence, and no stranding risk arises from this clause.

**Limitation of GateExpr for temporal protection.** The GateExpr language (§1.7.3) evaluates the *current* Counter state. It cannot express temporal ordering constraints such as "T_b must be consumed before T_a may be consumed." Therefore, even if a consumer of T_a has additional gate conditions, those conditions cannot encode the specific temporal protection needed to prevent stranding. The analysis below reports stranding risk whenever the structural preconditions for stranding exist, regardless of other gate conditions on the T_a consumer. Preventing stranding requires domain-level conventions or external coordination mechanisms beyond the GateExpr language.

**Algorithm (Acyclic Topologies).**

For acyclic precondition graphs, a precise structural decision procedure exists:

```
STRANDING_RISK_ACYCLIC(D):
  Compute topological ordering τ of PreconditionGraph
  for each gate clause (p, action, T_b, via T_a):
    Consumers ← { q : ("consume", T_a) ∈ Gate(q) }
    if Consumers = ∅: continue  (* T_a is never consumed; no risk *)
    
    (* Check if T_a is a DIRECT predecessor of T_b *)
    DirectPreds ← { T' : ∃ rule R ∈ Pre with rhs(R) = T_b 
                     and T' ∈ lhs(R) }
    if T_a ∉ DirectPreds: continue  (* via never satisfiable; no risk *)
    
    (* In acyclic graph, T_b is produced AFTER T_a. T_b tokens exist on   *)
    (* Counter until consumed. T_a can be consumed independently.         *)
    (* GateExpr cannot express "consume T_b before T_a" ordering.         *)
    (* Therefore, stranding risk is structural and always present         *)
    (* when the above conditions hold.                                    *)
    
    for each q ∈ Consumers:
      report STRANDING_RISK(T_b, T_a, p, q)
  
  return no risk
```

**Correctness argument (acyclic case).** In an acyclic precondition graph, if T_a is a direct predecessor of T_b, then T_b is produced after T_a, and T_b.depends_on contains the specific T_a instance. Once T_b exists on the Counter, T_a can be consumed by any participant holding `consumes T_a` without any ordering constraint (PACT does not enforce lifecycle ordering, §1.6). Because GateExpr cannot express temporal ordering between consumptions (§1.7.3), no gate condition on the T_a consumer can prevent this. Therefore, stranding is structurally possible whenever a consumer of T_a exists and T_a is a direct predecessor of a type with a `via T_a` gate condition. The algorithm correctly identifies all such cases.

**Complexity.** O(|GateConditions| × |Types|) for the direct predecessor check (using precomputed adjacency lists); O(|GateConditions| × |Consumers|) for the consumer scan. Total: O(|GateConditions| × |Types|).

**Algorithm (Cyclic Topologies — Conservative).**

For cyclic topologies, exact analysis is undecidable in general (the execution trace space is unbounded). We provide a conservative over-approximation:

```
STRANDING_RISK_CYCLIC(D):
  for each gate clause (p, action, T_b, via T_a):
    (* Same direct-predecessor check as acyclic case *)
    DirectPreds ← { T' : ∃ rule R ∈ Pre with rhs(R) = T_b 
                     and T' ∈ lhs(R) }
    if T_a ∉ DirectPreds: continue
    
    Consumers ← { q : ("consume", T_a) ∈ Gate(q) }
    if Consumers ≠ ∅:
      (* Conservative: stranding is structurally possible.               *)
      (* GateExpr cannot prevent it temporally.                          *)
      for each q ∈ Consumers:
        report POTENTIAL_STRANDING(T_b, T_a, p, q)
  return no risk
```

This may produce false positives for cyclic domains (where runtime scheduling might coincidentally avoid stranding) but never misses a true structural stranding risk.

**Domain mitigation strategies.** Since GateExpr cannot enforce consumption ordering, domains concerned about stranding should employ one of the following strategies:
1. **Convention:** Document and enforce consumption ordering through operational procedures rather than gate conditions.
2. **Archival Policy:** Define an Archival Policy (§2.2) that consumes dependent tokens before their witnesses, executed by a designated system participant.
3. **Design avoidance:** Restructure the precondition graph so that types with `via` conditions do not share witnesses with independently consumable types.

---

## Appendix B: Encoding into Colored Petri Nets (Detailed Construction)

### B.1 Color Sets

For each TypeName Tᵢ in the PACT domain, define a CPN color set:

```
Σᵢ = TokenId × ParticipantName × ThingRef × PrecondRefs × Timestamp
```

where `PrecondRefs` is a list of TokenId values representing the `depends_on` set.

The unified color set for the archive place is:

```
Σ_archive = Σ × ParticipantName × Timestamp
```

### B.2 Place Construction

For each TypeName Tᵢ, create a place pᵢ. Additionally, create:
- One archive place p_archive with color set Σ_archive.

Total places: |Types| + 1.

**Scope.** N(D) models a single Counter instance; concurrent instances of the same declaration are encoded independently and share no places or transitions (see §4.5).

### B.3 Transition Construction

**Production transitions (non-source types only).** For each **non-source** type T with Precond(T) = {T₁, …, Tₖ} and each participant p with `produces T` gate:

- Create transition t_{p,T}^{prod}.
- **Test arcs for predecessors:** For each predecessor place pᵢ (i ∈ {1..k}), create:
  - Input arc: pᵢ --[xᵢ]--> t_{p,T}^{prod} (binds variable xᵢ; ensures predecessor exists)
  - Output arc: t_{p,T}^{prod} --[xᵢ]--> pᵢ (returns same token; predecessor is NOT consumed)
  
  This pair constitutes a test arc (read arc). The predecessor token is borrowed for the enabling check but remains in the place after firing. This matches Rule-P semantics where predecessors are not consumed (§1.4).
  
- **Normal output arc:** to p_T with expression `(new_id(), p, θ, [x₁.id, …, xₖ.id], now())` where `new_id()` generates a fresh TokenId, θ is the ThingRef, and `[x₁.id, …, xₖ.id]` constructs the `depends_on` list from predecessor token identifiers.
- Guard: `true`.

**No production transitions for source types.** By the source-production constraint (§3.1), no `produces T` gate clause exists for source type T. Therefore no production transitions are generated for source types. This is correct: in PACT, source types enter exclusively via Rule-S, and the CPN encoding mirrors this by having only seed transitions for source types. This also avoids the CPN analog of the non-termination problem: a production transition with no input arcs (which would correspond to a source-type produce gate) would be perpetually enabled, preventing the CPN from reaching a dead marking.

**Example.** For `preconditions { design → implementation }` and `developer: produces implementation`:

```
Transition t_{developer,implementation}^{prod}:
  Test arc:   p_design --[x]--> t_{developer,implementation}^{prod}
  Test arc:   t_{developer,implementation}^{prod} --[x]--> p_design
  Output arc: t_{developer,implementation}^{prod} 
              --[(new_id(), developer, θ, [x.id], now())]--> p_implementation
  Guard: true
```

Note the two arcs involving p_design: one input (binding) and one output (returning). After firing, the design token remains in p_design, and a new implementation token appears in p_implementation with `depends_on = [x.id]`.

**Multi-predecessor example.** For `preconditions { { lab_result + imaging_result } → treatment_plan }` and `care_team: produces treatment_plan`:

```
Transition t_{care_team,treatment_plan}^{prod}:
  Test arc:   p_lab_result --[x₁]--> t_{care_team,treatment_plan}^{prod}
  Test arc:   t_{care_team,treatment_plan}^{prod} --[x₁]--> p_lab_result
  Test arc:   p_imaging_result --[x₂]--> t_{care_team,treatment_plan}^{prod}
  Test arc:   t_{care_team,treatment_plan}^{prod} --[x₂]--> p_imaging_result
  Output arc: t_{care_team,treatment_plan}^{prod} 
              --[(new_id(), care_team, θ, [x₁.id, x₂.id], now())]--> p_treatment_plan
  Guard: true
```

Both predecessor tokens remain in their respective places after firing.

**Read transitions (without gate condition).** For each type T and each participant p with `reads T` gate and no GateCondOpt:

- Create transition t_{p,T}^{read}.
- Input arc: from p_T, binding variable x.
- Output arc: to p_T, returning variable x (self-loop; token is not consumed).
- Guard: `true`.

**Read transitions (with `via T_a` gate condition).** For each type T, participant p with `reads T via T_a`:

- Create transition t_{p,T}^{read}.
- Input arc from p_T, binding variable x (target token).
- Output arc to p_T, returning variable x (self-loop on target; non-destructive read).
- **Test arc from p_{T_a}:** Input arc from p_{T_a}, binding variable y. Output arc to p_{T_a}, returning variable y. This pair forms a test arc (read arc): the witness token y is borrowed for guard evaluation but not consumed.
- Guard: `y.id ∈ x.depends_on`.

**Example.** For `deployer: reads test_report via implementation`:

```
Transition t_{deployer,test_report}^{read}:
  Input arc:  p_test_report --[x]--> t_{deployer,test_report}^{read}
  Output arc: t_{deployer,test_report}^{read} --[x]--> p_test_report
  Test arc:   p_implementation --[y]--> t_{deployer,test_report}^{read}
  Test arc:   t_{deployer,test_report}^{read} --[y]--> p_implementation
  Guard: y.id ∈ x.depends_on
```

The test arc ensures that the implementation token y is present in p_implementation at firing time but is not consumed. The guard checks provenance-specific structural link: y's id must appear in x's depends_on list. Because CPN colors carry TokenId, this check is well-defined and executable.

**Read transitions (with composite GateExpr).** For gate conditions built from the GateExpr language (§1.7.3):

- `exists T_b`: Add test arc from p_{T_b} binding variable y_b. Guard: `true` (existence is guaranteed by the arc's enabling condition).
- `count T_b >= n`: Add n test arcs from p_{T_b} binding distinct variables y_b₁, …, y_bₙ. Guard: `y_b₁.id ≠ y_b₂.id ≠ … ≠ y_bₙ.id` (ensures n distinct tokens).

*By the Normalization rule (§1.7.3), n ≥ 1 always holds, so the arc construction is non-degenerate. Absence conditions (`count T = 0` semantics) arrive exclusively as `not exists T_b` and follow the inhibitor-arc / complementary-place path described below.*

- `not exists T_b`: Requires inhibitor arc from p_{T_b} to the transition. **Note:** Standard CPN Tools [7] does not support inhibitor arcs. Domains using `not` require either an extended CPN variant or a complementary-place encoding.

**Complementary-place encoding for `not exists`.** For domains restricted to standard CPN tools, `not exists T_b` can be encoded without inhibitor arcs as follows:

1. Introduce a complementary place p̄_{T_b} with initial marking M₀(p̄_{T_b}) = N, where N is a known upper bound on the number of T_b tokens that can ever exist simultaneously (obtainable from boundedness analysis or domain knowledge).
2. For every transition that *produces* a T_b token (output arc to p_{T_b}), add a corresponding input arc from p̄_{T_b} consuming one token.
3. For every transition that *consumes* a T_b token (input arc from p_{T_b}), add a corresponding output arc to p̄_{T_b} returning one token.
4. To encode `not exists T_b` on transition t, add a normal input arc from p̄_{T_b} to t with multiplicity N and a corresponding output arc back to p̄_{T_b} with multiplicity N. This tests M(p̄_{T_b}) = N, which holds iff M(p_{T_b}) = 0.

**Limitation.** This encoding requires a known finite bound N. For unbounded domains, the complementary-place technique is not applicable, and inhibitor arcs or an extended CPN variant are required. We recommend treating `not` conditions in unbounded domains as future work for tool integration.

- Boolean composition: Combine guard expressions with ∧, ∨, ¬.

**Consumption transitions (without gate condition).** For each type T and participant p with `consumes T` gate and no GateCondOpt:

- Create transition t_{p,T}^{consume}.
- Input arc: from p_T, binding variable x (**consuming** arc; no corresponding output arc back to p_T).
- Output arc: to p_archive with expression `(x, p, now())`.
- Guard: `true`.

**Consumption transitions (with `via T_a` gate condition).** Identical to read transitions with `via`, except:

- The input arc from p_T is a *consuming* arc (no corresponding output arc back to p_T).
- The output arc goes to p_archive.
- The test arc from p_{T_a} remains a self-loop (non-consuming).

**Example.** For `lab_tech: consumes diagnosis via triage`:

```
Transition t_{lab_tech,diagnosis}^{consume}:
  Input arc:  p_diagnosis --[x]--> t_{lab_tech,diagnosis}^{consume}
  Output arc: t_{lab_tech,diagnosis}^{consume} 
              --[(x, lab_tech, now())]--> p_archive
  Test arc:   p_triage --[y]--> t_{lab_tech,diagnosis}^{consume}
  Test arc:   t_{lab_tech,diagnosis}^{consume} --[y]--> p_triage
  Guard: y.id ∈ x.depends_on
```

**Seed transitions.** For each (p, T) ∈ SeedAuthority:

- Create transition t_{p,T}^{seed}.
- No input arcs (transition is always enabled; represents external injection).
- Output arc: to p_T with expression `(new_id(), p, θ, [], now())` where `depends_on = []` (empty list, per §1.4.3).

**Note on seed transition enabling.** Seed transitions have no input arcs and are therefore always enabled in the CPN. This mirrors Rule-S semantics: seeding is an external injection not gated by Counter state. In practice, seed transitions fire according to the domain's injection schedule, not autonomously. For bounded state-space analysis, the number of seed firings can be capped or the seed transition can be disabled after a specified number of firings.

### B.4 Initial Marking

All places are initially empty: M₀(p) = ∅ for all p ∈ P. Seeds are modeled as transitions that may fire at any time, representing external injection. Alternatively, for bounded analysis, initial markings can be set to reflect a known starting configuration.

### B.5 Correctness Argument

**Theorem (Encoding Bisimulation).** For any well-formed PACT domain D, the CPN encoding N(D) is bisimilar to D with respect to Counter states: for every PACT execution trace σ, there exists a CPN firing sequence producing the same sequence of Counter multisets, and vice versa.

**Proof.** By induction on trace length.

*Base case.* Initial state: Counter = ∅, all places empty. (∅, M₀) ∈ R. ✓

*Inductive step (Rule-P).* Suppose (C, M) ∈ R and p produces token t of **non-source** type T with predecessors {t₁, …, tₖ}. In the CPN, transition t_{p,T}^{prod} is enabled iff all predecessor places p₁, …, pₖ are marked (corresponding to {t₁, …, tₖ} ⊆ₘ Counter). Firing uses **test arcs** on predecessor places: each predecessor token xᵢ is borrowed from pᵢ and returned to pᵢ. Therefore, predecessor tokens remain in their places after firing. A new token is added to p_T with `depends_on = [x₁.id, …, xₖ.id]`. The resulting marking M' satisfies (C ⊎ {t}, M') ∈ R, with all predecessor tokens preserved. ✓

*Inductive step (Rule-R without condition).* Suppose (C, M) ∈ R and p reads token t of type T. In the CPN, transition t_{p,T}^{read} fires via self-loop: token x is borrowed from p_T and returned. Marking unchanged. (C, M) ∈ R still holds. ✓

*Inductive step (Rule-R with `via T_a`).* Suppose (C, M) ∈ R and p reads token t of type T with `via T_a`. GateHolds requires ∃ w ∈ Counter: type(w) = T_a ∧ w.id ∈ t.depends_on. In the CPN, the test arc from p_{T_a} is enabled iff p_{T_a} is marked (∃ y in p_{T_a}). The guard `y.id ∈ x.depends_on` checks the provenance-specific structural link. Because CPN colors carry TokenId, this guard correctly distinguishes "the correct witness is present" from "a different token of the same type is present." If both hold, the self-loop fires and marking is unchanged. If either fails, the transition is not enabled, matching GateHolds failure. ✓

*Inductive step (Rule-E).* Analogous to Rule-R but with consuming arc to p_archive. Token x is removed from p_T and `(x, p, now())` is added to p_archive. Matches Counter ← Counter \ {t} and Archive ← Archive ∪ {entry}. ✓

*Inductive step (Rule-S).* Seed transition has no input arcs; always enabled. Adds fresh token with empty depends_on to p_T. Matches Rule-S. Source types enter exclusively via this mechanism, matching PACT's source-production constraint. ✓

*Converse.* Every CPN firing sequence maps back to a PACT trace by construction: production transitions correspond to Rule-P (with test arcs preserving predecessors; only non-source types have production transitions), self-loop transitions to Rule-R, consuming transitions to Rule-E, seed transitions to Rule-S. No other transitions exist. □

### B.6 Complexity of the Encoding

| PACT Domain Size | CPN Construct Count |
|---|---|
| \|Types\| = n | n + 1 places |
| Non-source produce gate entries = g | g production transitions |
| Read gate entries = r | r read transitions |
| Consume gate entries = c | c consumption transitions |
| Seed authority entries = s | s seed transitions |
| Gate conditions with `via` = v | v × 2 additional arcs (test arc pairs) |
| Production predecessor test arcs | Σ_{g} \|Pre(rhs(g))\| × 2 arcs (input + output per predecessor) |
| Total transitions | g + r + c + s |
| Total arcs | O(g × avg_fanin × 2 + r + c + 2v + s) |

The encoding is linear in the number of gate matrix entries and predecessor relationships. For domains with k participants and n types, the worst case is O(k × n × 3) transitions (three actions per participant-type pair), but in practice the gate matrix is sparse. The factor of 2 on predecessor arcs accounts for the test arc pairs (input + output) required for non-consuming predecessor access. Source types contribute only seed transitions (no production transitions), reducing the transition count relative to a naive encoding.

### B.7 Tool Reuse

The CPN encoding enables direct application of:
- **CPN Tools** [7]: reachability analysis, state space exploration, invariant checking. Compatible with the positive fragment of GateExpr (without `not`). For `not` conditions, see complementary-place encoding in §B.3.
- **TAPAAL**: timed extensions for boundedness analysis.
- **LoLA / MARCIE**: liveness and deadlock detection on the underlying P/T net obtained by projecting away colors.

This provides PACT with an immediate ecosystem of mature verification tools without implementing bespoke analyzers. The test-arc construction (for both production predecessors and gate condition witnesses) is natively supported in CPN Tools and most CPN simulators.

---

## Appendix C: Proof Details for Expressiveness Separations

### C.1 PACT ⊄ Petri Nets (Detailed)

**Claim.** The PACT domain D_gate from §4.2 cannot be expressed by any P/T Petri net, including nets with self-loops.

**Domain.**

```
preconditions { S → W; { W + S } → T }
gates {
    p₁: reads T via W;
    p₂: reads T;
    producer: reads S, reads W, produces W, produces T
}
seed_authority { external: seeds S }
```

S is a source type (no precondition rule); W and T are non-source types. The domain satisfies the source-production constraint: no `produces S` clause exists.

**Key property.** By §1.4.4, T's unique precondition rule is `{W + S} → T`. Therefore, when T is produced via Rule-P, `T.depends_on` contains both the W-instance and the S-instance. The `via W` condition on p₁'s read is satisfiable: the W-instance in T.depends_on can serve as witness.

**Discriminating scenario.** Execute:
1. Seed S₁. Counter = {S₁}.
2. Producer produces W₁ from S₁. Counter = {S₁, W₁}.
3. Producer produces T₁ from {W₁, S₁}. T₁.depends_on = {W₁.id, S₁.id}. Counter = {S₁, W₁, T₁}.
4. Seed S₂. Counter = {S₁, W₁, T₁, S₂}.
5. Producer produces W₂ from S₂. Counter = {S₁, W₁, T₁, S₂, W₂}.

**State A:** Counter = {S₁, W₁, T₁, S₂, W₂}. `GateHolds(p₁, read, T₁)` holds (witness: W₁).
**State B:** W₁ consumed. Counter = {S₁, T₁, S₂, W₂}. `GateHolds(p₁, read, T₁)` **fails** (W₂.id ∉ T₁.depends_on).

**Proof (detailed).** Assume for contradiction that a P/T net N expresses this domain. Let t_{p₁,read} model p₁'s read of T. Since read is non-destructive, t_{p₁,read} must preserve M(p_T). This is achieved via a self-loop on p_T.

For t_{p₁,read} to be disabled in State B while M(p_T) ≥ 1, some other input place must be unmarked. The only candidate is p_W. But M(p_W) = 1 in State B (W₂ is present).

A self-loop from p_W tests M(p_W) ≥ 1, which holds in State B. Thus t_{p₁,read} would fire in State B, contradicting PACT semantics where p₁ cannot read T₁.

An inhibitor arc from p_W would disable when M(p_W) = 0, but M(p_W) = 1. No P/T net construction can distinguish "the correct W-token is present" from "a wrong W-token is present," because P/T tokens within a place are anonymous and indistinguishable.

One might argue that encoding participant identity as separate transitions (t_{p₁,read} vs t_{p₂,read}) is standard practice. We accept this. The issue is not participant differentiation but *provenance sensitivity*: even with separate transitions, t_{p₁,read} cannot test whether the W-token in p_W is the *same instance* that produced T₁, because P/T tokens carry no identity.

Therefore, no P/T Petri net can express D_gate. □

**Remark on self-loops.** Classical P/T nets do permit self-loops, and these correctly implement type-level existence testing. The separation does not rely on the absence of self-loops. It relies on the absence of *token identity* in P/T nets, which prevents provenance-sensitive witness checking. This distinction is crucial and was insufficiently emphasized in earlier drafts.

### C.2 Petri Nets ⊄ PACT (Detailed)

**Claim.** Atomic multi-output transitions in Petri nets cannot be expressed in PACT.

**Proof (detailed).** Consider a P/T net with transition t: •t = {p₁, p₂}, t• = {q₁, q₂}. Firing t atomically transforms marking M to M' where M'(p₁) = M(p₁)−1, M'(p₂) = M(p₂)−1, M'(q₁) = M(q₁)+1, M'(q₂) = M(q₂)+1. No intermediate marking exists.

In PACT, producing tokens of types Q₁ and Q₂ requires two separate Rule-P invocations. The intermediate state (Q₁ present, Q₂ absent) is observable. No PACT execution can skip this intermediate state, because Rule-P produces exactly one token per invocation (§1.4). There is no "batch production" rule.

Encoding atomicity via a single compound type Q_pair changes the type schema and prevents independent access to Q₁ and Q₂, violating behavioral equivalence.

Therefore, atomic multi-output is inexpressible in PACT. □

### C.3 PACT ⊄ Linda (Detailed)

**Proof.** Linda has no typed precondition topology, no participant-gated access, and no archival. A PACT domain requiring participant-gated consumption with contextual witness checking and archival has no Linda encoding. □

### C.4 Linda ⊄ PACT (Detailed)

**Proof.** Linda supports content-addressed retrieval with pattern variables ranging over unbounded domains. PACT's TypeSchema is finite; ThingRef is opaque; GateExpr operates on type membership, not field values. Encoding unbounded value spaces as types requires infinite TypeSchema. Therefore, Linda's content-addressed selection is inexpressible in PACT. □

---

## Appendix D: Static Analysis Complexity Summary

| Property | Algorithm | Time Complexity | Space Complexity | Notes |
|----------|-----------|-----------------|------------------|-------|
| Reachability | BFS on precondition graph | O(\|V\| + \|E\|) | O(\|V\|) | V = Types, E = Pre edges |
| Gate Coverage | Linear scan of gate matrix (non-source only) | O(\|V\| + \|E\| + \|G\|) | O(\|V\|) | G = gate entries; skips source types |
| Consumption Reachability | Linear scan of gate matrix | O(\|G\|) | O(\|V\|) | Includes source types |
| Structural Deadlock | Tarjan's SCC + seed check | O(\|V\| + \|E\|) | O(\|V\|) | Conservative for cyclic domains |
| Source-Production Legality | Source-type identification + gate scan | O(\|V\| + \|E\| + \|G\|) | O(\|V\|) | Rejects ill-formed declarations |
| Stranding Risk (acyclic) | Direct predecessor check + consumer scan | O(\|GC\| × \|V\|) | O(\|V\|) | GC = gate conditions; exact structural analysis |
| Stranding Risk (cyclic) | Conservative over-approximation | O(\|GC\| × \|V\|) | O(\|V\|) | May produce false positives |

All algorithms operate on the static domain declaration and require no execution. They are suitable for integration into CI/CD pipelines as pre-deployment checks.

**Implementation note.** For the acyclic stranding risk analysis, the direct predecessor check uses precomputed adjacency lists from the precondition graph. Each gate condition clause triggers one lookup. The consumer scan is a linear pass over the gate matrix filtered by type. The algorithm reports all structurally possible stranding risks; mitigation requires domain-level conventions or Archival Policies (§A.6).

**Source-production legality as a prerequisite.** The Source-Production Legality check (§A.5) should be run before all other analyses. If it reports violations, the domain declaration is ill-formed and subsequent analysis results may be misleading. In particular, Gate Coverage (§A.2) assumes source types are correctly identified; an undeclared source type with a spurious `produces` clause would cause Gate Coverage to incorrectly consider it covered.

---

## References

1. Petri, C. A. (1962). *Kommunikation mit Automaten*. PhD thesis, University of Bonn.
2. Girard, J.-Y. (1987). Linear logic. *Theoretical Computer Science*, 50(1), 1–101.
3. Gelernter, D. (1985). Generative communication in Linda. *ACM Transactions on Programming Languages and Systems*, 7(1), 80–112.
4. Reisig, W. (1985). *Petri Nets: An Introduction*. Springer-Verlag.
5. van der Aalst, W. M. P. (1998). The application of Petri nets to workflow management. *Journal of Circuits, Systems and Computers*, 8(1), 21–66.
6. ter Hofstede, A. H. M., van der Aalst, W. M. P., Adams, M., & Russell, N. (2003). *Modern Business Process Automation: YAWL and its Support Environment*. Springer-Verlag.
7. Jensen, K., & Kristensen, L. M. (2009). *Coloured Petri Nets: Modelling and Validation of Concurrent Systems*. Springer-Verlag.

---
