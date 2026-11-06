# big-bad-bool-contract

Experimental. Not published (`publish = false`). The crate name is the
architecture report's placeholder.

Contracts across Big Bad Bool packed words. A program is:

- a list of **dimensions**, each one `#[bitpack]` type with a name and a role
  (evidence, state, effect);
- a **frame recognizer** over declared sequences of item classes;
- **contracts** between dimensions: WHEN, REQUIRE, REQUIRE-ANY, FORBID, and the
  writes they propose (TRANSITION, STACK, EMIT);
- **containment** limits.

`build` resolves every name once. `verify` checks coherence once and returns a
`VerifiedProgram`. A `Stepper` takes the verified program and a stream of
observations. Per position it frames the item, evaluates every contract,
authorizes exactly one, and commits it. Only commit writes state. Observers see
each stage and return nothing.

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
| `model` | Spec types, constructors, `Program` / `ProgramBuilder`, name resolution, interact's matcher checks one for one, role checks |
| `frame` | `Recognizer`: declared sequences in a window of R items each side, longest-match, positions, boundary, end padding |
| `contract` | `FactView`, `evaluate`: not applicable, refused with the failed clause, or a candidate. Shared references only |
| `step::limits` | Counts against containment limits, refused with observed and permitted numbers |
| `step::authorize` | Exactly one distinct candidate, within stack bounds. The only place an `AuthorizedTransition` is built |
| `step::commit` | `ProjectionState` (fields private to this file) and `commit`, the only write path. Checks every changed word with flaga_solver and rolls back on refusal |
| `digest` | Stable spec material, its digest, the trace header |
| `verify` | `verify`: fixed-word legality, sequence overlap, unsatisfiable contracts, and a bounded search of reachable states for ambiguity, states with no exit, dead contracts, impossible effects, dead sequences and unreachable labels. The only place a `VerifiedProgram` is built |
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
    .contain(ContainmentSpec { max_interpretations: 4, max_candidates: 4, max_stack_depth: 32, on_refusal: RefusalRule::Stop })
    .build()?;

let program = verify(spec, VerifyLimits { max_states: 100_000, max_witness_tries: 4_096 })?;
let mut stepper = Stepper::new(&program, NoObserver);
for observation in tokens {          // Observation { position, item }
    for result in stepper.push(observation) { /* StepResult */ }
}
let rest = stepper.finish();         // pads with the end item and steps it
```

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
- Commit applies stack ops first, then transition writes (to the top entry of
  a stacked dimension), then builds effect words. Any refusal leaves the state
  as it was. Deprecated values are allowed; every other flaga_solver refusal is
  `IllegalWord`.
- The same contract under several frames counts once. Different contracts with
  equal writes count once, and the lowest id commits. Different writes are
  `Ambiguous`.
- No candidate is `NoContract` (default deny). The contract refusals at that
  position come with it.
- `Position` values are fixed: Begin 1, Interior 2, End 3, Complete 4. The frame
  dimension's position field must use them. The sequence key is the sequence
  index plus one.

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
| `tests/step01_model.rs` | Dimensions resolve by name; bad clauses give interact's `validate_rule_table` findings one for one; role rules |
| `tests/step02_contract.rs` | WHEN, REQUIRE, REQUIRE-ANY, FORBID; flag-field labels through `Schema::holds` |
| `src/step/commit.rs` (unit) | Only commit writes; an illegal word is refused and rolled back; stack bounds |
| `tests/step03_compile_fail.rs` | Outside code cannot build an `AuthorizedTransition` or a `VerifiedProgram`, write `ProjectionState`, reach `commit` or `authorize`, or get `&mut` state from the stepper or an event. Case crates build under `target/tmp` |
| `src/step/limits.rs`, `src/step/authorize.rs` (unit) | Observed 9, permitted 8; ambiguity; equal writes count once |
| `tests/step05_frame.rs` | Offsets, positions, boundary, longest-match, end padding |
| `tests/step06_stepper.rs` | Section 16 end to end against golden step results and observer events |
| `tests/step07_digest.rs` | The digest is fixed in `tests/golden/step07_section16.digest` and changes with any clause |
| `tests/step08_verify.rs` | Each verifier finding from one broken spec; section 16 passes with info only |
| `tests/step09_observers.rs` | Same results with no observer, a counting observer and the bowl adapter |
| `tests/step10_core1o.rs` | A contract-only shadow of core-1o's parser agrees with it on all 37 published fixtures |
| `tests/step11_klepto.rs` | klepto agrees on function declarations, `is_async`, and call references in `tests/corpus/klepto` |

Step 10 reads core-1o's fixtures from the cargo registry, read only.
