# Changelog

Dates are when crates.io got the version, in UTC.

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
