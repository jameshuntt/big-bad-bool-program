# big-bad-bool-contract

Experimental. Not published (`publish = false`). The crate name is the
architecture report's placeholder.

Contracts across Big Bad Bool packed words. A program is:

- a list of **dimensions**, each one `#[bitpack]` type with a name and a role
  (evidence, state, effect);
- **contracts** between dimensions: WHEN, REQUIRE, REQUIRE-ANY, FORBID, and the
  writes they propose (TRANSITION, STACK, EMIT);
- **containment** limits;
- optionally, a **frame recognizer** over declared sequences of item classes.
  Without one, a program is dimensions, contracts and containment only.

`build` resolves every name once. `verify` checks coherence once and returns a
`VerifiedProgram`. A `Stepper` takes the verified program and a stream of
observations. An observation holds a word for any evidence dimension of the
program. Per position the stepper admits the observation, frames the item when
there is a recognizer, evaluates every contract under every frame
interpretation, lets every compatible candidate actualize together, and commits
their merged writes. Only commit writes state. Observers see each stage and
return nothing.

The library names no language. Every name below a dimension comes from the
user's `#[bitpack]` labels and sequence specs.

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

Modules are future crate boundaries. Each depends only on the ones before it.

| Module | Does |
|---|---|
| `codes` | `Finding` (build and verify) and `Refusal` (stepping), as liaise codes `BBBC0001`.. and `BBBC0101`.. |
| `model` | Spec types, constructors, `Observation`, `Program` / `ProgramBuilder`, name resolution, interact's matcher checks one for one, role checks |
| `frame` | `Recognizer`: declared sequences in a window of R items each side, every match kept, positions, boundary, end padding; the exact sets of matches an item can have |
| `contract` | `FactView`, `evaluate`: not applicable, refused with the failed clause, or a candidate. Shared references only |
| `step::limits` | Counts against containment limits, refused with observed and permitted numbers |
| `step::authorize` | `gather` (candidates of one position) and `authorize`: merges the writes of every candidate, refuses contradictions, checks stack bounds. The only place an `AuthorizedTransition` is built |
| `step::commit` | `ProjectionState` (fields private to this file) and `commit`, the only write path. Applies the merged plan, checks every changed word with flaga_solver and rolls back on refusal |
| `digest` | Stable spec material, its digest, the trace header |
| `verify` | `verify`: fixed-word legality, contained sequences and overlapping facts, unsatisfiable contracts, and a bounded search of reachable states for conflicts, states with no exit, dead contracts, impossible effects, dead sequences and unreachable labels. The search runs every input through the stepper's own `gather`, `authorize` and `commit`. The only place a `VerifiedProgram` is built |
| `observe` | `StepEvent`, `Observer`, `NoObserver` |
| `stepper` | `Stepper`, `StepResult`, `legal_next` |
| `bowl` (feature) | `BowlObserver`: publishes each event to an `ObservableWakeRuntime`, drops the report |

## Assembly

All of this is user data. The section 16 example of the report, in full, is
`tests/common/mod.rs`.

```rust
use big_bad_bool::bitpack;
use big_bad_bool_contract::*;

#[bitpack(storage = u8)]
pub struct StateWord {
    #[bits(3)]
    #[labels(Items = 0, FnHead = 1, SignatureName = 2, Params = 3, SignatureEnd = 4, Body = 5)]
    #[invalid(6..=7)]
    pub phase: u8,
    #[bits(1)]
    #[labels(can_declare = 1)]
    #[flags(can_declare = 0)]
    #[default(1)]
    pub marks: u8,
}
// TokenWord, FrameWord, ContextWord, EffectWord the same way.

const K3_DECLARE_FUNCTION_NAME: ContractSpec = contract("declare-function-name")
    .when(&[
        clause("frame", FieldMatcher::eq("construct", FUNCTION_DECLARATION)),
        clause("frame", FieldMatcher::eq("role", DECLARED_IDENTITY)),
        clause("token", FieldMatcher::eq("kind", IDENT)),
    ])
    .require(&[clause("state", FieldMatcher::eq("marks", CAN_DECLARE)), clause("state", FieldMatcher::eq("phase", FN_HEAD))])
    .forbid(&[clause("context", FieldMatcher::eq("marks", CONST_EXPRESSION))])
    .transition(&[set("state", "phase", SIGNATURE_NAME)])
    .emit(&[set("effect", "kind", DECLARES_IDENTITY), set("effect", "class", FUNCTION)]);

let spec = Program::new()
    .add(dimension!(TokenWord, "token", Evidence))
    .add(dimension!(FrameWord, "frame", Evidence))
    .add(dimension!(ContextWord, "context", State).stacked(32))
    .add(dimension!(StateWord, "state", State))
    .add(dimension!(EffectWord, "effect", Effect))
    .recognize(RECOGNIZER) // sequences S1..S5
    .contract(K1_FN_HEAD_BEGIN)
    // .. K2 to K15
    .contract(K3_DECLARE_FUNCTION_NAME)
    .contain(ContainmentSpec {
        max_interpretations: 4,
        max_candidates: 4,
        max_actualized: 4,
        max_stack_depth: 32,
        on_refusal: RefusalRule::Stop,
    })
    .build()?;

let program = verify(spec, VerifyLimits { max_states: 100_000, max_witness_tries: 4_096 })?;
let token = program.spec().dimension_id("token").unwrap();
let mut stepper = Stepper::new(&program, NoObserver);
for (kind, position) in tokens.iter().zip(0..) {
    let observation = Observation::new(position).with_word(token, TokenWord::new(*kind));
    for result in stepper.push(observation) { /* StepResult */ }
}
let rest = stepper.finish();         // pads with the end item and steps it
```

A program with no recognizer is the same without `.recognize(..)` and without
the frame dimension. Its observations bring every evidence word, for example
`Observation::new(0).with(badge, STAFF).with(clock, NIGHT)`; each push steps
its own position at once, and `finish` has no end item. See
`tests/correction1_contract_only.rs` and `tests/correction2_evidence.rs`.

At `foo` in `async fn foo(x: i32) {}` the stepper returns
`2: commit declare-function-name effects [effect {kind=DeclaresIdentity, class=Function} at 2]`.
The application looks up the text at position 2 itself. The full step results
and observer events are in `tests/golden/step06_*.txt`.

Rules worth knowing:

- A clause's `Eq` and `NotEq` hold through `Schema::holds`: in a flag field a
  label holds when its bits are set. Every other op is interact's `MatchOp`.
- The start state is every field at its `#[default(n)]`. A stacked dimension
  starts with one such entry. A pushed entry is the default word with the push
  values set. An effect word is the default word with the emit values set.
- An observation holds at most one word per dimension. A dimension it does not
  hold reads as nothing: no clause on it holds. A word for a dimension that is
  not evidence, for the frame dimension, or for no dimension is `Evidence`; a
  word its schema refuses is `IllegalWord`. A refused observation holds no
  evidence for its neighbours' windows either.
- Every match is a frame interpretation. A match inside a longer one is kept
  beside it. Contracts are evaluated under each interpretation, or under none
  when there is none.
- Every distinct contract that is a candidate actualizes, together, up to
  `max_actualized`. Their writes merge: a field written by several contracts
  gets one value (equal values count once, different values are `Conflict`);
  contracts touching one stacked dimension must carry equal stack ops on it
  (applied once), or it is `Conflict`; emits merge the same way into one effect
  word per effect dimension. Contracts that read one frame field under
  interpretations declaring different values for it are `Conflict`; a field a
  sequence does not declare constrains nothing. Contracts are merged in name
  order and the plan is kept in dimension and field order, so results do not
  depend on declaration order. `max_actualized = 1` is an exactly-one policy.
- Commit applies the merged stack ops first, then the merged transition writes
  (to the top entry of a stacked dimension), then builds effect words. Changed
  words are checked once, after all writes. Any refusal leaves the state as it
  was. Deprecated values are allowed; every other flaga_solver refusal is
  `IllegalWord`.
- No candidate is `NoContract` (default deny). The contract refusals at that
  position come with it, in contract name order.
- `Position` values are fixed: Begin 1, Interior 2, End 3, Complete 4. The frame
  dimension's position field must use them. The sequence key is the sequence
  index plus one.
- The verifier's state search feeds each evidence combination (each observed
  dimension absent or at a labelled word) with each exact set of matches its
  item can have, through the stepper's own `gather`, `authorize` and `commit`.
  So a verified program never refuses with `Conflict` at run time.

## Tests

```sh
export CARGO_TARGET_DIR="$PWD/target"
cargo test                      # no features
cargo test --all-features       # with the bowl adapter
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
BLESS_GOLDEN=1 cargo test       # rewrites tests/golden/*; review the diff before committing
```

| File | Proves |
|---|---|
| `tests/correction1_contract_only.rs` | A program with no recognizer builds, verifies and steps; adding a recognizer changes nothing for contracts that read no frame |
| `tests/correction2_evidence.rs` | Three independent evidence dimensions in one contract; absent evidence reads as nothing; observations hold only observed evidence; the verifier reads every evidence dimension |
| `tests/correction3_actualization.rs` | Contracts writing different dimensions and emitting independent effects actualize together; identical writes count once; conflicting writes are refused; results do not depend on declaration order; the fan-out limit |
| `tests/correction4_overlap.rs` | Contained and overlapping matches coexist; left and right context still decide; contradictory facts are listed and, when relied on together, refused; the section 16 examples still work |
| `src/step/authorize.rs` (unit) | Merging, conflicts on fields, stacks, effects and frame facts, fan-out and candidate limits |
| `tests/step01_model.rs` | Dimensions resolve by name; bad clauses give interact's `validate_rule_table` findings one for one; role rules |
| `tests/step02_contract.rs` | WHEN, REQUIRE, REQUIRE-ANY, FORBID; flag-field labels through `Schema::holds` |
| `src/step/commit.rs` (unit) | Only commit writes; an illegal word is refused and rolled back; stack bounds |
| `tests/step03_compile_fail.rs` | Outside code cannot build an `AuthorizedTransition` or a `VerifiedProgram`, read a transition's plan, write `ProjectionState`, reach `commit` or `authorize`, get `&mut` state from the stepper or an event, or change the authorized set from an observer. Case crates build under `target/tmp` |
| `src/step/limits.rs` (unit) | Observed 9, permitted 8 |
| `tests/step05_frame.rs` | Offsets, positions, boundary, contained matches kept, end padding |
| `tests/step06_stepper.rs` | Section 16 end to end against golden step results and observer events |
| `tests/step07_digest.rs` | The digest is fixed in `tests/golden/step07_section16.digest` and changes with any clause |
| `tests/step08_verify.rs` | Each verifier finding from one broken spec; section 16 passes with info only (one contained sequence, five overlapping facts, one unreachable label) |
| `tests/step09_observers.rs` | Same results with no observer, a counting observer and the bowl adapter |
| `tests/step10_core1o.rs` | A contract-only shadow of core-1o's parser agrees with it on all 37 published fixtures |
| `tests/step11_klepto.rs` | klepto agrees on function declarations, `is_async`, and call references in `tests/corpus/klepto` |

Step 10 reads core-1o's fixtures from the cargo registry, read only.
