# Changelog

## 0.1.0 (unreleased, not published)

First build of the approved architecture report (sections 9 to 24), steps 1 to 11
of section 24. Step 12 (measurement) is not in this release.

- `model`: dimensions over `#[bitpack]` types (`dimension!`), clauses, writes,
  stack ops, contracts, sequences, recognizer, containment, `Program::new()`
  builder. `build` refuses unknown names, out-of-range values and writes a role
  does not allow, with liaise codes. Its matcher checks give the same findings as
  interact's `validate_rule_table`, one for one (`tests/step01_model.rs`).
- `contract`: `evaluate` over a read-only `FactView`. Label holding goes through
  flaga_solver's `Schema::holds` (`tests/step02_contract.rs`).
- `step`: `limits`, `authorize` (the only builder of `AuthorizedTransition`) and
  `commit` (the only writer of `ProjectionState`). Changed words are checked with
  flaga_solver's `check`; a refusal rolls the step back (unit tests in
  `src/step/`; compile-fail cases in `tests/step03_compile_fail.rs`).
- `frame`: the recognizer, with longest-match, positions, boundary and end
  padding (`tests/step05_frame.rs`).
- `stepper`: section 16 end to end, against golden step results and observer
  events (`tests/step06_stepper.rs`, `tests/golden/step06_*.txt`).
- `digest`: spec material and its stable_digest SHA-256; trace header
  (`tests/step07_digest.rs`, `tests/golden/step07_section16.digest`).
- `verify`: fixed-word legality, sequence overlap, unsatisfiable contracts, and a
  bounded reachable-state search for ambiguity, no-exit states, dead contracts,
  impossible effects, dead sequences and unreachable labels. Section 16 passes
  with two info findings over 70 reachable states (`tests/step08_verify.rs`).
- `observe` and the `bowl` feature: observers get shared references and return
  nothing; the ob-serve-a-bowl adapter publishes `WakeKey64` packets
  (`tests/step09_observers.rs`).
- Proving: a contract-only shadow of core-1o 0.2.0's parser agrees with it on all
  37 published fixtures (`tests/step10_core1o.rs`); klepto 0.3.5 agrees on
  declarations and call references (`tests/step11_klepto.rs`).
