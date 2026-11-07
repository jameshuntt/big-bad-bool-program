# Changelog

Dates are when crates.io got the version, in UTC.

## Unreleased

On branch `prepared-step`, not published. Code commit
[0e919c2](https://github.com/jameshuntt/big-bad-bool-program/commit/0e919c2f12a99a264ece91e2e847f5c90eae8b73),
from 0.1.0 at f582666.

### Added: a step in two phases

Why: `push` authorized and committed in one call, so the application saw a
step's effects only after the state had changed. A host that has its own
bounds (mailbox space, timers, effects per step) could not make them part of
the step: a step whose effects the host refused had still committed. Now the
host can see the effects first and refuse by dropping the preparation.

- Before: [src/stepper.rs L83-97 @ f582666](https://github.com/jameshuntt/big-bad-bool-program/blob/f582666faf913e132621cb0ef04b4a95000551c8/src/stepper.rs#L83-L97),
  `push` steps and writes in one call; [src/stepper.rs L244-291 @ f582666](https://github.com/jameshuntt/big-bad-bool-program/blob/f582666faf913e132621cb0ef04b4a95000551c8/src/stepper.rs#L244-L291),
  `decide` runs `gather`, `authorize` and `commit` on the stepper's own state
  and calls the observer as it goes.
- After: [src/stepper.rs L222-263 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/stepper.rs#L222-L263):
  `Stepper::prepare(&self, observation) -> PreparedStep` and
  `Stepper::prepare_finish(&self) -> PreparedStep` step a copy of the
  stepper's state and window (the private `Cursor`) through the same
  `admit`, frame, `gather`, `authorize` and `commit`, and call no observer.
- [src/stepper.rs L97-118 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/stepper.rs#L97-L118):
  `PreparedStep`. Private fields, not `Clone`. It reads out `results()`,
  `effects()`, `is_refused()`, `halts()`, `state_after()` and `version()`.
  One `PreparedStep` holds every position its call steps, in order, each from
  the state the one before left: `push` steps zero or one position, `finish`
  the rest of the window and the end item. Under `Stop` the preparation ends
  at the refusal, as stepping did.
- [src/stepper.rs L266-281 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/stepper.rs#L266-L281):
  `Stepper::commit(&mut self, PreparedStep) -> Result<Vec<StepResult>, CommitRefusal>`.
  [src/stepper.rs L363-404 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/stepper.rs#L363-L404): it
  gives the observer the prepared events in the order stepping always gave
  them, takes the prepared state and window, and adds one to the state
  version (`Stepper::version`). Commit still is the only write of
  `ProjectionState`; `commit` of the stepper only swaps in what `step::commit`
  wrote on the copy.
- [src/codes.rs L486-506 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/codes.rs#L486-L506):
  `CommitRefusal`. `BBBC0201` `Stale` (prepared at an older state version)
  and `BBBC0202` `OtherStepper` (prepared by another stepper; every stepper
  takes a number from one counter when it is made). Nothing changes on either.
- [src/stepper.rs L204-224 @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/src/stepper.rs#L204-L224):
  `push` is `prepare` then commit, `finish` is `prepare_finish` then commit.
- The verifier is unchanged. Its search calls `gather`, `authorize` and
  `commit` directly ([src/verify.rs L670-698 @ f582666](https://github.com/jameshuntt/big-bad-bool-program/blob/f582666faf913e132621cb0ef04b4a95000551c8/src/verify.rs#L670-L698)),
  and `prepare` reaches the same three functions.

Proof:

- The 107 tests of 0.1.0 (109 with `bowl`) pass unchanged, golden observer
  events included.
- [tests/step12_prepared.rs @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/tests/step12_prepared.rs),
  eleven tests: `push_is_prepare_then_commit_over_the_fixtures` (results,
  state, version and every observer event equal, three fixture inputs plus a
  pushed end word and an illegal word, under `Stop` and `SkipObservation`),
  `prepare_mutates_nothing`, `effects_are_visible_before_commit`,
  `commit_applies_exactly_once_and_a_second_preparation_is_stale`,
  `dropping_a_prepared_step_leaves_the_stepper_unchanged`,
  `a_preparation_made_stale_by_an_intervening_transition_is_refused`,
  `a_preparation_from_another_stepper_is_refused`,
  `a_refused_preparation_mutates_nothing`,
  `push_is_prepare_then_commit_through_a_commit_refusal` (authorized, then
  commit refuses the word: the same events either way),
  `several_positions_are_one_preparation_and_stop_holds`,
  `verification_and_preparation_agree`.
- [tests/step13_prepared_compile_fail.rs @ 0e919c2](https://github.com/jameshuntt/big-bad-bool-program/blob/0e919c2f12a99a264ece91e2e847f5c90eae8b73/tests/step13_prepared_compile_fail.rs):
  outside code cannot build a `PreparedStep`, clone one, commit one twice, or
  commit through a shared reference to the stepper.
- `cargo test`: 119 passed. `cargo test --all-features`: 121 passed.
  `cargo +1.85 test`: 119 passed. `cargo +1.88 test --all-features`: 121
  passed. fmt, clippy `-D warnings` both ways, docs `-D warnings`, both
  examples: clean.

## 0.1.0 (2026-11-06)

First release. A program over Big Bad Bool packed words: typed dimensions,
declarative contracts across them, optional framing, bounded verification,
containment, and deterministic stepping with one write path.

### Program model

- Dimensions over `#[bitpack]` types (`dimension!`), each with a role:
  evidence, state (optionally a bounded stack) or effect.
- Contracts with WHEN, REQUIRE, REQUIRE-ANY and FORBID clauses and the writes
  they propose: TRANSITION, STACK (push, pop) and EMIT. Clauses are
  big-bad-bool-interact matchers; label holding goes through flaga_solver's
  `Schema::holds`.
- `Program::new()` builder. `build` resolves every name once and refuses
  unknown names, out-of-range values and writes a role does not allow, with
  liaise codes (`Finding`, `BBBC0001`..). Its matcher checks give the same
  findings as interact's `validate_rule_table`, one for one.
- `Observation`: a position and one word for each evidence dimension it
  holds. A dimension it does not hold reads as nothing.

### Framing (optional)

- A `RecognizerSpec` declares sequences of item classes over one evidence
  dimension, with facts per offset. The `Recognizer` matches them in a
  bounded window on each side of each item and writes frame words: sequence,
  offset, position, boundary, and the declared facts.
- Every match is kept, including a match inside a longer one. Contracts are
  evaluated under each interpretation.
- The end word is the end of input: only `Stepper::finish` makes and steps
  the end item, and `push` refuses an observation that holds it.
- A program without a recognizer builds, verifies and steps the same way;
  each push steps its own position.

### Verification

- `verify` is the only way to get a `VerifiedProgram`. It reports fixed-word
  legality, contained sequences and overlapping facts (info), unsatisfiable
  contracts, and, from a bounded search of reachable states, conflicts,
  states with no exit, dead contracts, impossible effects, dead sequences and
  unreachable labels. The search runs every input through the stepper's own
  `gather`, `authorize` and `commit`.
- The inputs it searches are built from the program's matchers: each
  evidence field's values are split into classes by every predicate that
  reads them, and the domain holds one legal word for every class
  combination beside every labelled word. State satisfiability is decided
  over the same kind of classes.
- Bounded by `VerifyLimits` (`max_states`, `max_witness_tries`). Past a
  bound the finding is `TooManyStates`, never a verdict from part of the
  search.
- `Stepper::legal_next_representatives` lists one representative
  observation per class the program accepts next, not every legal word.

### Stepping and authority

- `Stepper::push` and `finish` return a `StepResult` per position: the
  contracts that actualized, their effects, or the refusals (`Refusal`,
  `BBBC0101`..).
- Every compatible candidate contract actualizes together, up to
  `ContainmentSpec::max_actualized`; their writes merge into one plan.
  Different values for one field, unequal stack ops on one stacked
  dimension, and contracts relying on contradictory frame facts are
  `Conflict`. Results do not depend on declaration order.
- No accepting contract is `NoContract` (default deny), with the contract
  refusals at that position.
- Containment limits on interpretations, candidates, actualized contracts
  and stack depth, refused with observed and permitted numbers.
- Only `step::authorize` builds an `AuthorizedTransition` and only
  `step::commit` writes `ProjectionState`. Commit checks every changed word
  with flaga_solver and leaves the state as it was on any refusal.
  Compile-fail cases show outside code can do neither.

### Digest and observation

- `spec_material` and `spec_digest`: the spec's stable text and its
  stable_digest SHA-256 (`SPEC_FORMAT` `big-bad-bool-program.spec.v2`), and a
  `TraceHeader` naming it.
- Observers see each stage by shared reference and return nothing
  (`Observer`, `StepEvent`, `NoObserver`).
- Feature `bowl`: `BowlObserver` publishes step events to ob-serve-a-bowl
  0.2.2 as `WakeKey64` packets. It needs Rust 1.88; the crate without it
  needs 1.85.

### Proof

- Two examples: [`examples/thermostat.rs`](examples/thermostat.rs) (no
  framing) and [`examples/declarations.rs`](examples/declarations.rs)
  (declarations and references from framing).
- A reference program in `tests/common/mod.rs` runs end to end against
  golden step results and observer events (`tests/step06_stepper.rs`); its
  digest is fixed in `tests/golden/step07_section16.digest`.
- A program-only shadow of core-1o 0.2.0's parser agrees with it on all 37
  published fixtures (`tests/step10_core1o.rs`); klepto 0.3.5 agrees on
  function declarations and call references in a small Rust corpus
  (`tests/step11_klepto.rs`).
- The evidence domain and the state witness are compared with brute force
  over every legal word of two small schemas, for 1,900 random clause sets
  and 4,000 random positive and FORBID sets (`src/verify/domain.rs`).
