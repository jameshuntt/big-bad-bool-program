# big-bad-bool-program

Programs over Big Bad Bool packed words. A program is a set of typed packed
dimensions, declarative contracts across them, and containment limits. It is
verified once, then stepped over a stream of observations, one position at a
time, with one write path.

The parts:

- **Dimensions.** Each dimension is one `#[bitpack]` type with a name and a
  role: evidence (observed facts), state (what the program remembers) or
  effect (what it emits). A state dimension can be a bounded stack.
- **Contracts.** Declarative rules across dimensions: WHEN, REQUIRE,
  REQUIRE-ANY and FORBID clauses, and the writes they propose (TRANSITION,
  STACK, EMIT). A position with no accepting contract is refused (default
  deny).
- **Framing, optional.** A recognizer over declared sequences of items gives
  each item its contextual facts (which sequence, which offset, the facts
  declared there), inside a bounded window on each side. Contracts read those
  facts like any other dimension. Without a recognizer a program is
  dimensions, contracts and containment only.
- **Verification.** `verify` checks the program before any input and is the
  only way to get a `VerifiedProgram`. The evidence it searches is built
  from the program's own matchers, so the state search sees every case a
  legal observation can produce, within stated bounds.
- **Containment.** Limits on interpretations, candidates, contracts that act
  together, and stack depth, each refused with the observed and permitted
  numbers.
- **Deterministic stepping.** Every compatible contract at a position acts
  together and their writes merge; contradictions are refused. Results do
  not depend on declaration order.
- **Explicit authority.** Only `step::authorize` builds an
  `AuthorizedTransition`, only `step::commit` writes state, and outside code
  can do neither (compile-fail tests prove it).
- **Observation, optional.** Observers see every stage by shared reference
  and return nothing. The `bowl` feature publishes step events to
  [ob-serve-a-bowl](https://crates.io/crates/ob-serve-a-bowl).

Parsing is one use. The library names no language: every name below a
dimension comes from the user's `#[bitpack]` labels and sequence specs. A
thermostat, an access rule or a protocol state machine is written the same
way as a token stream.

```toml
[dependencies]
big-bad-bool = "0.1"          # #[bitpack] for the dimension types
big-bad-bool-program = "0.1"
```

## Example: a program with no framing

[`examples/thermostat.rs`](examples/thermostat.rs), in part. Two evidence
dimensions (`reading`, `mode`), one state dimension (`heater`), one effect
dimension (`command`):

```rust
const CONTRACTS: &[ContractSpec] = &[
    contract("heat-when-cold")
        .when(&[reading(COLD), mode(AUTO)])
        .require(&[heater(OFF)])
        .transition(&[set("heater", "state", ON)])
        .emit(&[set("command", "kind", Command::LABEL_KIND_STARTHEAT.value)]),
    contract("stop-when-hot")
        .when(&[reading(HOT), mode(AUTO)])
        .require(&[heater(ON)])
        .transition(&[set("heater", "state", OFF)])
        .emit(&[set("command", "kind", Command::LABEL_KIND_STOPHEAT.value)]),
    contract("hold").when(&[reading(MILD)]),
    contract("manual").when(&[mode(MANUAL)]),
];

let spec = CONTRACTS
    .iter()
    .fold(
        Program::new()
            .add(dimension!(Reading, "reading", Evidence))
            .add(dimension!(Mode, "mode", Evidence))
            .add(dimension!(Heater, "heater", State))
            .add(dimension!(Command, "command", Effect))
            .contain(ContainmentSpec {
                max_interpretations: 1,
                max_candidates: 4,
                max_actualized: 4,
                max_stack_depth: 1,
                on_refusal: RefusalRule::SkipObservation,
            }),
        |program, c| program.contract(*c),
    )
    .build()?;
let program = verify(spec, VerifyLimits { max_states: 1_000, max_witness_tries: 1_024 })?;

let mut stepper = Stepper::new(&program, NoObserver);
for (position, (level, kind)) in (0..).zip(input) {
    let observation = Observation::new(position).with(reading, level).with(mode, kind);
    for result in stepper.push(observation) { /* StepResult */ }
}
```

`cargo run --example thermostat` prints:

```text
0: heat-when-cold {kind=StartHeat}
1: hold
2: stop-when-hot {kind=StopHeat}
3: refused [BBBC0101] no contract accepts position 3 (1 refused)
4: manual
heater at the end: {state=Off}
```

At position 3 the heater is already off, so `stop-when-hot` is refused and
no other contract accepts the observation. With `SkipObservation` the state
stays as it was and the stream goes on.

## Example: framing, declarations and references

[`examples/declarations.rs`](examples/declarations.rs). In
`let a = b ; let c = a ;` the token word for a name is the same everywhere.
Two declared sequences give the name its role from its left context, and the
contracts read that role (in part):

```rust
const SEQUENCES: &[SequenceSpec] = &[
    sequence(
        "binding",
        &[ItemMatch::One(kind(LET)), ItemMatch::One(kind(NAME))],
    )
    .facts(&[OffsetFacts {
        offset: 1,
        values: &[FieldValue::new("role", DECLARED)],
    }]),
    sequence(
        "value",
        &[ItemMatch::One(kind(ASSIGN)), ItemMatch::One(kind(NAME))],
    )
    .facts(&[OffsetFacts {
        offset: 1,
        values: &[FieldValue::new("role", REFERENCED)],
    }]),
];

contract("declare")
    .when(&[token(NAME), role(DECLARED)])
    .require(&[phase(START)])
    .transition(&[set("statement", "phase", NAMED)])
    .emit(&[set("effect", "kind", Effect::LABEL_KIND_DECLARES.value)]),
contract("reference")
    .when(&[token(NAME), role(REFERENCED)])
    .require(&[phase(ASSIGNING)])
    .transition(&[set("statement", "phase", VALUED)])
    .emit(&[set("effect", "kind", Effect::LABEL_KIND_REFERENCES.value)]),
```

The program adds `.recognize(RecognizerSpec { item_dimension: "token",
frame_dimension: "frame", .., sequences: SEQUENCES })`. Each `push` steps the
item whose window is complete, and `finish` pads with the end item and steps
the rest. `cargo run --example declarations` prints:

```text
verified: 4 states, 0 findings
a at 1: {kind=Declares}
b at 3: {kind=References}
c at 6: {kind=Declares}
a at 8: {kind=References}
```

An effect carries the input position; the application maps it back to its
own text.

## How a step works

`build` resolves every name once. `verify` checks coherence once and returns
a `VerifiedProgram`. A `Stepper` takes the verified program and a stream of
observations. An observation holds a word for any evidence dimension of the
program. Per position the stepper admits the observation, frames the item
when there is a recognizer, evaluates every contract under every frame
interpretation, lets every compatible candidate actualize together, and
commits their merged writes. Only commit writes state. Observers see each
stage and return nothing.

A step can be taken in two phases, so a host can admit each transition
before it happens. `Stepper::prepare(observation)` (and `prepare_finish()`)
authorizes the semantic transition without mutating live state: it runs all
of the above on a copy of the stepper's state and window and returns a
`PreparedStep`, each position's result, effects and refusals, the state after
and the version it was prepared from. The stepper and its observer are
untouched. The host inspects it and declines it by dropping it.
`Stepper::commit(prepared)` is the sole application boundary: it applies
exactly that transition once and returns the results; it refuses a
preparation from an older state version (`BBBC0201`) or from another stepper
(`BBBC0202`), and at the last state version, `u64::MAX`, it refuses every
preparation (`BBBC0203`) rather than wrap. `push` is `prepare` and then
commit; `finish` is `prepare_finish` and then commit. The admission rule is
the host's; the crate names none.

Rules worth knowing:

- A clause's `Eq` and `NotEq` hold through flaga_solver's `Schema::holds`:
  in a flag field a label holds when its bits are set. Every other op is
  big-bad-bool-interact's `MatchOp`.
- The start state is every field at its `#[default(n)]`. A stacked dimension
  starts with one such entry. A pushed entry is the default word with the
  push values set. An effect word is the default word with the emit values
  set.
- An observation holds at most one word per dimension. A dimension it does
  not hold reads as nothing: no clause on it holds. A word for a dimension
  that is not evidence, for the frame dimension, or for no dimension is
  `Evidence`; a word its schema refuses is `IllegalWord`. A refused
  observation holds no evidence for its neighbours' windows either.
- The recognizer's end word is the end of input, not an input. Only
  `finish` makes and steps the end item. An observation whose item is the
  end word is refused on `push` with `Evidence` ("the end word is reserved
  for `finish`"): it commits nothing, and the stream goes on, or stops, from
  the state before it. The verifier evaluates the end item and does not
  follow its successor, so the two agree.
- Every match is a frame interpretation. A match inside a longer one is kept
  beside it. Contracts are evaluated under each interpretation, or under
  none when there is none.
- Every distinct contract that is a candidate actualizes, together, up to
  `max_actualized`. Their writes merge: a field written by several
  contracts gets one value (equal values count once, different values are
  `Conflict`); contracts touching one stacked dimension must carry equal
  stack ops on it (applied once), or it is `Conflict`; emits merge the same
  way into one effect word per effect dimension. Contracts that read one
  frame field under interpretations declaring different values for it are
  `Conflict`; a field a sequence does not declare constrains nothing.
  Contracts are merged in name order and the plan is kept in dimension and
  field order, so results do not depend on declaration order.
  `max_actualized = 1` is an exactly-one policy.
- Commit applies the merged stack ops first, then the merged transition
  writes (to the top entry of a stacked dimension), then builds effect
  words. Changed words are checked once, after all writes. Any refusal
  leaves the state as it was. Deprecated values are allowed; every other
  flaga_solver refusal is `IllegalWord`.
- No candidate is `NoContract` (default deny). The contract refusals at
  that position come with it, in contract name order.
- `Position` values are fixed: Begin 1, Interior 2, End 3, Complete 4. The
  frame dimension's position field must use them. The sequence key is the
  sequence index plus one.

## What `verify` checks, and its bounds

`verify` reports fixed-word legality, contained sequences and overlapping
facts (info), unsatisfiable contracts, and, from a bounded search of
reachable states, conflicts, states with no exit, dead contracts,
impossible effects, dead sequences and unreachable labels. The search runs
every input through the stepper's own `gather`, `authorize` and `commit`.

The inputs it tries are classes, not every raw word. Each evidence field's
values are split into classes by every predicate that reads the field
(contract clauses, item matchers, the end item, and the field's invalid,
reserved, requires and conflicts rules). The evidence domain holds one legal
word for every combination of classes a legal word can have, beside every
labelled word. A legal word with no label gives the same matcher answers as
some domain word, so a verified program does not refuse with `Conflict` at
run time. `Unsatisfiable` on a state dimension is decided over the same
kind of classes, so a satisfiable contract is not reported `Unsatisfiable`.

The guarantees hold within `VerifyLimits`: at most `max_states` reachable
states and `max_witness_tries` words per domain. Past either bound `verify`
reports `TooManyStates`; it never gives a verdict from part of the search.
It checks the program's own rules; it does not prove anything about inputs
the program does not describe.

`Stepper::legal_next_representatives` lists one representative observation
for each class the program accepts next (labelled words as themselves),
drawn from `VerifiedProgram::inputs`. It is not every legal observation: any
other word in the same class as a listed one (same matcher answers, same
legality) is accepted the same way at run time. The contracts constrain
classes, not the listed raw words. The end item is never listed.

## What it reuses

| Crate | For |
|---|---|
| big-bad-bool-core 0.1.1 | `BitFieldSpec`, `PackedWord`: field layout of every dimension |
| big-bad-bool-interact 0.1.0 | `FieldMatcher`, `MatchOp` (clauses), `FieldValue` and `patch_word` (every field write) |
| flaga_solver 0.1.1 | `Schema` and `schema_of!` (a dimension's tables), `Schema::holds` (label holding), `check` (every committed and fixed word) |
| liaise 0.2.4 | `LiaiseCodes` for every `Finding` and `Refusal`: code, message, evidence, severity |
| stable_digest 0.1.1 | the spec digest |
| ob-serve-a-bowl 0.2.2 | the `bowl` feature only: step events as `WakeKey64` packets |

## Module map

The public API is re-exported at the crate root. Inside, each module
depends only on the ones before it.

| Module | Does |
|---|---|
| `codes` | `Finding` (build and verify), `Refusal` (stepping) and `CommitRefusal` (applying a prepared step), as liaise codes `BBBC0001`.., `BBBC0101`.. and `BBBC0201`.. |
| `model` | Spec types, constructors, `Observation`, `Program` / `ProgramBuilder`, name resolution, interact's matcher checks one for one, role checks |
| `frame` | `Recognizer`: declared sequences in a window of R items each side, every match kept, positions, boundary, end padding; the exact sets of matches an item can have |
| `contract` | `FactView`, `evaluate`: not applicable, refused with the failed clause, or a candidate. Shared references only |
| `step::limits` | Counts against containment limits, refused with observed and permitted numbers |
| `step::authorize` | `gather` (candidates of one position) and `authorize`: merges the writes of every candidate, refuses contradictions, checks stack bounds. The only place an `AuthorizedTransition` is built |
| `step::commit` | `ProjectionState` (fields private to this file) and `commit`, the only write path. Applies the merged plan, checks every changed word with flaga_solver and rolls back on refusal |
| `digest` | Stable spec material, its digest, the trace header |
| `verify` | `verify`, the evidence domain, the bounded state search. The only place a `VerifiedProgram` is built |
| `observe` | `StepEvent`, `Observer`, `NoObserver` |
| `stepper` | `Stepper`, `StepResult`, `PreparedStep` (`prepare`, `prepare_finish`, `commit`), `legal_next_representatives` |
| `bowl` (feature) | `BowlObserver`: publishes each event to an `ObservableWakeRuntime`, drops the report |

## Rust version

The crate is edition 2024 with `rust-version = "1.85"`. The `bowl` feature
needs Rust 1.88, the `rust-version` of ob-serve-a-bowl 0.2.2. CI tests the
default features on 1.85 and everything on stable.

## Tests

```sh
cargo test                      # no features
cargo test --all-features       # with the bowl adapter
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
BLESS_GOLDEN=1 cargo test       # rewrites tests/golden/*; review the diff before committing
```

The reference program in `tests/common/mod.rs` ("section 16": five
dimensions, five sequences, fifteen contracts) recognizes function
declarations and call references in a small Rust-like token stream. It is
test data, not part of the library.

| File | Proves |
|---|---|
| `tests/step01_model.rs` | Dimensions resolve by name; bad clauses give interact's `validate_rule_table` findings one for one; role rules |
| `tests/step02_contract.rs` | WHEN, REQUIRE, REQUIRE-ANY, FORBID; flag-field labels through `Schema::holds` |
| `tests/step03_compile_fail.rs` | Outside code cannot build an `AuthorizedTransition` or a `VerifiedProgram`, read a transition's plan, write `ProjectionState`, reach `commit` or `authorize`, get `&mut` state from the stepper or an event, or change the authorized set from an observer. Case crates build under `target/tmp` |
| `tests/step05_frame.rs` | Offsets, positions, boundary, contained matches kept, end padding |
| `tests/step06_stepper.rs` | The reference program end to end against golden step results and observer events |
| `tests/step07_digest.rs` | The digest is fixed in `tests/golden/step07_section16.digest` and changes with any clause |
| `tests/step08_verify.rs` | Each verifier finding from one broken spec; the reference program passes with info only |
| `tests/step09_observers.rs` | Same results with no observer, a counting observer and the bowl adapter |
| `tests/step10_core1o.rs` | A program-only shadow of core-1o's parser agrees with it on all 37 published fixtures (read from the cargo registry, read only) |
| `tests/step11_klepto.rs` | klepto agrees on function declarations, `is_async`, and call references in `tests/corpus/klepto` |
| `tests/step12_prepared.rs` | `push` equals `prepare` then `commit` over the fixtures, observer events included; `prepare` changes nothing; effects before commit; commit once; stale and other-stepper preparations refused; drop and refusal change nothing; several positions and `Stop`; an observer panic during commit applies nothing; the verifier's representatives prepare as authorized |
| `tests/step13_prepared_compile_fail.rs` | Outside code cannot build a `PreparedStep`, clone one, commit one twice, or commit through `&Stepper` |
| `tests/correction1_contract_only.rs` | A program with no recognizer builds, verifies and steps; adding a recognizer changes nothing for contracts that read no frame |
| `tests/correction2_evidence.rs` | Three independent evidence dimensions in one contract; absent evidence reads as nothing; the verifier reads every evidence dimension |
| `tests/correction3_actualization.rs` | Compatible contracts actualize together; identical writes count once; conflicting writes are refused; results do not depend on declaration order; the fan-out limit |
| `tests/correction4_overlap.rs` | Contained and overlapping matches coexist; contradictory facts relied on together are refused |
| `tests/correction5_evidence_domain.rs` | Conflicts that only unlabelled values show are found; over every legal observation the stepper shows no conflict the verifier did not report |
| `tests/correction6_end_word.rs` | A pushed end word is refused and opens no path; `finish` still steps the end item; verifier and stepper agree |
| `tests/correction7_state_domain.rs` | State contracts whose only witnesses are unlabelled verify and commit; contradictions are still `Unsatisfiable`; past the bound it is `TooManyStates` |
| `tests/correction8_representatives.rs` | `legal_next_representatives` lists one word per accepted class; the other words of a listed class are accepted the same way |
| `src/verify/domain.rs` (unit) | On two tiny schemas, for 1,900 random clause sets, the domain gives exactly the matcher answers brute force over every legal word gives; for 4,000 random positive and FORBID sets, the state witness exists exactly when brute force finds one |
| `src/step/authorize.rs`, `src/step/commit.rs`, `src/step/limits.rs` (unit) | Merging and conflicts; only commit writes, an illegal word is rolled back, stack bounds; observed 9, permitted 8 |

## License

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE).
