# Changelog

## Corrective pass (unreleased, not published)

James's four corrections of 2026-11-06, then the evidence-domain pass, then
the final core-correctness pass (6 to 8).
Module boundaries and authority rules are unchanged: only `verify` builds a
`VerifiedProgram`, only `step::authorize` builds an `AuthorizedTransition`,
only `step::commit` writes state, observers get shared references and return
nothing.

### 1. Framing is optional

- `ProgramBuilder::build` no longer requires a recognizer. Without one a
  program is dimensions, contracts and containment; the window radius is
  zero, each push steps its own position, and `finish` has no end item.
- `ProgramSpec::recognizer`, `item_dimension`, `frame_dimension` and
  `end_word` return `Option`. `VerifiedProgram::recognizer` returns `Option`.
  `Recognizer::new` on a program without one is `Missing { part: "recognizer" }`.
- The digest material has no `recognizer.` lines without a recognizer; with
  one it is unchanged.
- Behaviour change: `build` no longer reports `Missing { part: "recognizer" }`
  (`tests/step01_model.rs` updated).
- Proof: `tests/correction1_contract_only.rs`. Framed goldens and the digest
  were unchanged by this commit.

### 2. Any evidence dimension takes runtime values

- `Observation` is now a position and one word per evidence dimension it
  holds: `Observation::new(position).with(dimension, word)` or
  `.with_word(dimension, packed_value)`; `position()`, `word()`, `evidence()`.
  Its fields are private. The recognizer's item word is the item dimension's
  word in it.
- A dimension an observation does not hold reads as nothing. The stepper
  admits each observation on `push`: a word for a non-evidence dimension, the
  frame dimension or no dimension is the new refusal `Evidence` (`BBBC0108`);
  a word its schema refuses is `IllegalWord`. A refused observation holds no
  evidence for its neighbours' windows.
- `ProgramSpec::is_observed` names the dimensions an observation may hold.
  The contract index keys on any observed evidence dimension.
- The verifier tries every combination of observed evidence dimensions (each
  absent or at a labelled word), bounded by `max_witness_tries`.
  `VerifiedProgram::item_words` is replaced by `inputs`; `legal_next` returns
  `Vec<Observation>`.
- Behaviour change: the input universe is now bounded by `max_witness_tries`,
  not `max_states`; a conflict witness names its evidence as
  `evidence=token={..}` instead of `item={..}`.
- Proof: `tests/correction2_evidence.rs`. Section 16 goldens and digest were
  unchanged by this commit.

### 3. Compatible contracts actualize together

- `authorize` lets every distinct candidate contract actualize, and merges
  their writes into one plan: equal field values count once, different values
  are the new refusal `Conflict` (`BBBC0109`); contracts touching one stacked
  dimension must carry equal stack ops on it (applied once); emits merge into
  one effect word per effect dimension. Contracts are merged in name order and
  the plan is kept in dimension and field order, so results do not depend on
  declaration order. Contract refusals with `NoContract` come in name order.
- New limit `ContainmentSpec::max_actualized` (limit name `actualized`). An
  exactly-one policy is `max_actualized = 1`.
- `AuthorizedTransition::contract` is replaced by `contracts`;
  `StepResult::committed` is `Vec<ContractId>`; `StepEvent::Authorized`
  carries `&[ContractId]`. The bowl adapter puts the count in `PayloadRef.len`.
- The verifier's ambiguity check is now the same `authorize` call: finding
  `Conflict` (`BBBC0030`). Retired codes: `Refusal::Ambiguous` (`BBBC0103`) and
  `Finding::Ambiguous` (`BBBC0021`).
- Commit checks changed words once, after the writes of all actualized
  contracts; effects come in effect-dimension order.
- Golden change: `tests/golden/step07_section16.digest`, because the material
  carries `containment.max_actualized` and `SPEC_FORMAT` is now
  `big-bad-bool-contract.spec.v2`.
- Proof: `tests/correction3_actualization.rs`, unit tests in
  `src/step/authorize.rs` and `src/step/commit.rs`
  (`merged_writes_are_checked_as_one_word`), and two new compile-fail cases.

### 4. Overlapping recognitions are kept

- The recognizer keeps every match; nothing is dropped because a longer match
  contains it. Positions, boundary, offsets and sequence keys are as before.
- `FrameFacts` is the placement (sequence, offset, word); `Candidate::frame`
  and `FactView::new`/`frame` carry it instead of a `SequenceId`.
- Contradictions are decided in `step::authorize`, deterministically:
  actualizing contracts that read one frame field under interpretations whose
  sequences declare different values for it are `Conflict`. A field a
  sequence does not declare constrains nothing, so a longer sequence refines a
  shorter one.
- The verifier lists every pair of sequence offsets that can hold one focal
  item with different declared values: finding `OverlappingFacts`
  (`BBBC0031`, info). Its state search now feeds each input with each exact
  set of matches its item can have (`Recognizer` computes them), through the
  stepper's own `gather`, `authorize` and `commit`; `legal_next` uses the same
  sets. Retired code: `OverlappingSequences` (`BBBC0027`).
  `ContainedSequence` stays info and now says both matches are kept.
- Behaviour change, section 16: at `fn`, `foo` and `(` of `async fn foo(`
  FnHead now frames beside AsyncFnHead. At `fn` fn-head-begin (via FnHead) and
  fn-keyword-after-modifier (via AsyncFnHead) actualize together; their writes
  agree, so the state and every effect are as before. Golden changes:
  `tests/golden/step06_async_fn_foo.txt` and
  `tests/golden/step06_bar_foo_in_body.txt`. The section 16 report gains five
  `OverlappingFacts` info findings and still visits 70 states.
- Proof: `tests/correction4_overlap.rs`, unit tests
  `frames::contracts_that_read_contradictory_facts_conflict` and
  `frames::facts_nobody_reads_or_nobody_declares_coexist` in
  `src/step/authorize.rs`; core-1o (37 fixtures) and klepto still agree.

### 5. The verifier's evidence domain follows the matchers

Why: the stepper accepts any evidence word its schema accepts, labelled or
not, but the verifier tried only combinations of labelled values (the default
where a field had none). A range, a mask, or the intersection of several
contracts' clauses can hold only on values with no label, so a program could
verify and still refuse with `Conflict` at run time. Two contracts on
`level in 6..=11` and `level in 10..=15`, with labels only at 0, 6, 7, 8 and
15, verified and then conflicted on level 10.

- Each observed evidence dimension now has an evidence domain built from the
  predicates that read it (new private module `src/verify/domain.rs`). For
  each field, every contract clause on it (WHEN, REQUIRE, REQUIRE-ANY and
  FORBID; `Eq`, `NotEq`, ranges, mask-all and mask-any, with flag-field labels
  through `Schema::holds` as at run time), every recognizer item matcher on
  the item dimension, the end item, and the field's invalid and reserved
  ranges and the labels its requires and conflicts relations test, split the
  field's values into classes. The classes are found by splitting regions of
  values, without listing them, so the cost follows the clauses, not the
  number of values a field can hold.
- A field's domain values are its labelled values (its default when it has
  none) plus one value the search finds in each class none of them is in. The
  dimension's domain is every combination, kept when flaga_solver accepts it.
  Matcher answers and legality are both fixed by the class of each field, so
  this is one legal word for every class combination a legal word can have:
  the domain gives exactly the matcher answers the legal words give.
- `verify`, `satisfiable`, the sequence checks, the recognizer's frame sets
  and the state search all read the new domain in place of the labelled
  words. The domain is bounded by `max_witness_tries` as before; past it
  verification stops with `TooManyStates`, never with a smaller domain.
- Behaviour change: `VerifiedProgram::inputs` and `legal_next` can hold
  unlabelled words; a contract that holds only on unlabelled values is no
  longer `Unsatisfiable` or `DeadContract`; conflicts that only unlabelled
  values show are now `Conflict` findings. Where every class already holds a
  labelled value the domain is the labelled words, in the same order: every
  golden, the digest, section 16 (70 states), core-1o and klepto are
  unchanged.
- Proof: `tests/correction5_evidence_domain.rs` (overlapping ranges, masks,
  intersections of three contracts, two evidence dimensions together,
  labelled words unchanged, the stepper over every legal observation shows no
  conflict the verifier did not report, and the domain gives every contract
  outcome a legal observation gives) and the unit tests in
  `src/verify/domain.rs`, which compare the domain with brute force over
  every legal word of two tiny schemas for 1,900 random clause sets.

### 6. The end word is the end of input

Why: the verifier treats the recognizer's end word as terminal. It
evaluates the end item but does not follow the state it commits to. The
stepper accepted the same word as ordinary pushed evidence and went on
stepping from what it committed, so a pushed end word could reach states
the verifier never visited. On the program in
`tests/correction6_end_word.rs` the verifier reports `Closed` unreachable,
yet pushing `Close` mid-stream committed `close` and moved the phase to
`Closed`.

- `Stepper::push` refuses an observation whose item dimension holds exactly
  the end word: `Evidence` (`BBBC0108`) with the rule "the end word is
  reserved for `finish`", at the observation's own step. Like any refused
  observation it commits nothing, holds no evidence for its neighbours'
  windows, and the stream goes on (`SkipObservation`) or stops (`Stop`) from
  the state before it.
- `finish` makes and steps the end item exactly as before.
- `legal_next` leaves out observations whose item is the end word, since
  `push` refuses them. `VerifiedProgram::inputs` still holds them: the state
  search evaluates the end item as `finish` steps it, and does not follow its
  successor.
- Behaviour change: at the start of section 16 `legal_next` is `async` and
  `fn`; it no longer lists the end item (`tests/step06_stepper.rs` updated).
  Every golden, the digest, core-1o and klepto are unchanged.
- Proof: `tests/correction6_end_word.rs`:
  `an_end_word_pushed_mid_stream_is_refused`,
  `an_injected_end_word_changes_no_state_and_opens_no_path`,
  `finish_still_steps_the_end_item`,
  `verifier_and_runtime_agree_on_the_end_word`. Against the previous commit
  the first, second and fourth fail: the pushed end word committed `close`.

### 7. State satisfiability uses the matcher classes

Why: `satisfiable` looked for a state witness among guessed values (zero,
the maximum, the default, the labels, all labels together, each clause
constant and its neighbours) in the fields the clauses name, every other
field at its default. A satisfiable contract could be reported
`Unsatisfiable`: level 4..=11, even, not 4 or 8 (only 6 and 10, unlabelled);
bits 1 and 2 set with bits 0 and 3 clear (only 6); and `mode = High` where
`High` requires a flag in another field. On the four readers in
`tests/correction7_state_domain.rs` the old check reported
`Unsatisfiable { dimension: "gauge", field: "(joint)" }` for each, and the
stepper committed every one of them.

- `satisfiable` asks `domain::witness` for a state dimension: the domain of
  `src/verify/domain.rs`, built from the contract's positive and FORBID
  clauses on that dimension and its invalid, reserved, requires and conflicts
  rules, every field included, one legal word per class combination and no
  labels added. Some legal word meets the positive clauses and no FORBID
  clause exactly when one of these does.
- The field an `Unsatisfiable` finding names is the first field whose
  positive clauses alone no legal word meets, over the same domain;
  otherwise `(joint)`.
- Bounded by `max_witness_tries` as before; past it the finding is
  `TooManyStates`, never a verdict from part of the domain.
- Removed: `field_candidates` and `state_witnesses`.
- Behaviour change: such contracts verify. Every golden, the digest,
  section 16 (70 states), core-1o and klepto are unchanged.
- Proof: `tests/correction7_state_domain.rs`:
  `a_state_range_whose_witness_is_unlabelled_is_satisfiable`,
  `a_state_mask_witness_is_found`,
  `positive_clauses_with_forbid_on_the_same_field_keep_their_witness`,
  `a_legality_rule_on_an_unnamed_field_does_not_hide_the_witness`,
  `intersections_across_several_state_fields_are_found`,
  `contradictory_state_clauses_are_still_unsatisfiable`,
  `a_state_domain_past_the_bound_is_too_many_states_not_unsatisfiable`
  (all but the contradictions fail against the previous commit), and the unit
  tests `tiny_state_witness_agrees_with_brute_force` and
  `wide_state_witness_agrees_with_brute_force` in `src/verify/domain.rs`,
  which compare the witness with brute force over every legal word for 4,000
  random positive and FORBID clause sets.

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
