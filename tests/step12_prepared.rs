//! Step 12: a step in two phases. `prepare` runs everything up to and
//! excluding the write and changes nothing; `commit` applies exactly that
//! transition once, and refuses a stale preparation or one from another
//! stepper. `push` and `finish` are `prepare` and then `commit`, with the
//! same results, the same state and the same observer events as before.

mod common;

use big_bad_bool::bitpack;
use big_bad_bool_program::{
    CommitRefusal, ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, Liaise, NoObserver,
    Observation, Program, ProjectionState, Refusal, RefusalRule, StepResult, Stepper,
    VerifiedProgram, VerifyLimits, clause, contract, dimension, set, verify,
};
use common::Recorder;

fn section16(containment: ContainmentSpec) -> VerifiedProgram {
    verify(
        common::program_with(containment).build().unwrap(),
        common::LIMITS,
    )
    .expect("section 16 verifies")
}

const SKIP: ContainmentSpec = ContainmentSpec {
    on_refusal: RefusalRule::SkipObservation,
    ..common::CONTAINMENT
};

/// A token kind the schema refuses (`#[invalid(10..=15)]`): an admission refusal.
const ILLEGAL: u128 = 12;

/// Every fixture input, plus the module-level refusal, a pushed end word and
/// an illegal word, so admission, containment and `Stop` are all stepped.
fn inputs() -> Vec<Vec<u128>> {
    let mut all: Vec<Vec<u128>> = vec![
        common::ASYNC_FN_FOO.to_vec(),
        common::ASYNC_FN_FOO_CALLING_BAR.to_vec(),
        common::BAR_FOO.to_vec(),
    ];
    let mut with_end = common::ASYNC_FN_FOO.to_vec();
    with_end.insert(3, common::END);
    all.push(with_end);
    let mut with_illegal = common::ASYNC_FN_FOO.to_vec();
    with_illegal.insert(2, ILLEGAL);
    all.push(with_illegal);
    all
}

/// Results, final state, version and observer events of a run.
type Run = (Vec<StepResult>, ProjectionState, u64, Vec<String>);

fn by_push(program: &VerifiedProgram, kinds: &[u128]) -> Run {
    let mut recorder = Recorder {
        spec: program.spec().clone(),
        lines: Vec::new(),
    };
    let mut stepper = Stepper::new(program, &mut recorder);
    let mut results = Vec::new();
    for o in common::tokens(0, kinds) {
        results.extend(stepper.push(o));
    }
    results.extend(stepper.finish());
    let (state, version) = (stepper.state().clone(), stepper.version());
    (results, state, version, recorder.lines)
}

fn by_prepare_and_commit(program: &VerifiedProgram, kinds: &[u128]) -> Run {
    let mut recorder = Recorder {
        spec: program.spec().clone(),
        lines: Vec::new(),
    };
    let mut stepper = Stepper::new(program, &mut recorder);
    let mut results = Vec::new();
    for o in common::tokens(0, kinds) {
        let prepared = stepper.prepare(o);
        results.extend(stepper.commit(prepared).expect("fresh preparation"));
    }
    let prepared = stepper.prepare_finish();
    results.extend(stepper.commit(prepared).expect("fresh preparation"));
    let (state, version) = (stepper.state().clone(), stepper.version());
    (results, state, version, recorder.lines)
}

#[test]
fn push_is_prepare_then_commit_over_the_fixtures() {
    for containment in [common::CONTAINMENT, SKIP] {
        let program = section16(containment);
        for kinds in inputs() {
            let pushed = by_push(&program, &kinds);
            let committed = by_prepare_and_commit(&program, &kinds);
            assert_eq!(pushed, committed, "{kinds:?} under {containment:?}");
            // One commit per push and one for finish.
            assert_eq!(pushed.2, kinds.len() as u64 + 1);
        }
    }
}

#[test]
fn prepare_mutates_nothing() {
    for containment in [common::CONTAINMENT, SKIP] {
        let program = section16(containment);
        for kinds in inputs() {
            let mut recorder = Recorder {
                spec: program.spec().clone(),
                lines: Vec::new(),
            };
            let mut stepper = Stepper::new(&program, &mut recorder);
            let mut results = Vec::new();
            for o in common::tokens(0, &kinds) {
                // Prepare every observation three times and drop it: nothing moves.
                let state = stepper.state().clone();
                let version = stepper.version();
                let halted = stepper.is_halted();
                let next = stepper.legal_next_representatives();
                for _ in 0..3 {
                    drop(stepper.prepare(o.clone()));
                    drop(stepper.prepare_finish());
                }
                assert_eq!(stepper.state(), &state);
                assert_eq!(stepper.version(), version);
                assert_eq!(stepper.is_halted(), halted);
                assert_eq!(stepper.legal_next_representatives(), next);
                results.extend(stepper.push(o));
            }
            results.extend(stepper.finish());
            let state = stepper.state().clone();
            drop(stepper);
            // Preparing changed nothing later either: the same run as pushing alone,
            // observer events included (prepare calls no observer).
            assert_eq!(
                (results, state, recorder.lines),
                {
                    let (r, s, _, e) = by_push(&program, &kinds);
                    (r, s, e)
                },
                "{kinds:?}"
            );
        }
    }
}

#[test]
fn effects_are_visible_before_commit() {
    let program = section16(common::CONTAINMENT);
    let mut recorder = Recorder {
        spec: program.spec().clone(),
        lines: Vec::new(),
    };
    let mut stepper = Stepper::new(&program, &mut recorder);
    let mut seen_effects = 0;
    for o in common::tokens(0, common::ASYNC_FN_FOO) {
        let before = stepper.state().clone();
        let prepared = stepper.prepare(o);
        let results = prepared.results().to_vec();
        let effects: Vec<_> = prepared.effects().copied().collect();
        let after = prepared.state_after().clone();
        seen_effects += effects.len();
        // Nothing is written yet.
        assert_eq!(stepper.state(), &before);
        let committed = stepper.commit(prepared).unwrap();
        assert_eq!(committed, results);
        let committed_effects: Vec<_> = committed.iter().flat_map(|r| r.effects.clone()).collect();
        assert_eq!(committed_effects, effects);
        assert_eq!(stepper.state(), &after);
    }
    assert!(seen_effects > 0, "the fixture emits effects");
}

#[test]
fn commit_applies_exactly_once_and_a_second_preparation_is_stale() {
    let program = job(RefusalRule::SkipObservation).expect("job program verifies");
    let mut stepper = Stepper::new(&program, NoObserver);
    let first = stepper.prepare(item(0, A));
    let again = stepper.prepare(item(0, A));
    assert_eq!(first.version(), 0);
    assert_eq!(again.version(), 0);
    let results = stepper.commit(first).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].effects.len(), 1);
    assert_eq!(stepper.version(), 1);
    let state = stepper.state().clone();
    // The same transition prepared twice is applied once: the other copy is stale.
    let refusal = stepper.commit(again).unwrap_err();
    assert_eq!(
        refusal,
        CommitRefusal::Stale {
            prepared: 0,
            current: 1
        }
    );
    assert_eq!(refusal.code(), "BBBC0201");
    assert_eq!(
        refusal.to_string(),
        "[BBBC0201] prepared at state version 0, the stepper is at 1"
    );
    assert_eq!(stepper.state(), &state);
    assert_eq!(stepper.version(), 1);
    // The next commit is the next position, at the next version.
    let next = stepper.push(item(1, B));
    assert_eq!(next[0].position, 1);
    assert_eq!(stepper.version(), 2);
}

#[test]
fn dropping_a_prepared_step_leaves_the_stepper_unchanged() {
    let program = section16(common::CONTAINMENT);
    let tokens = common::tokens(0, common::ASYNC_FN_FOO);
    let mut dropped = Stepper::new(&program, NoObserver);
    let mut plain = Stepper::new(&program, NoObserver);
    for o in &tokens {
        drop(dropped.prepare(o.clone()));
        assert_eq!(dropped.state(), plain.state());
        assert_eq!(dropped.version(), plain.version());
        assert_eq!(dropped.push(o.clone()), plain.push(o.clone()));
    }
    assert_eq!(dropped.finish(), plain.finish());
    assert_eq!(dropped.state(), plain.state());
}

#[test]
fn a_preparation_made_stale_by_an_intervening_transition_is_refused() {
    let program = section16(common::CONTAINMENT);
    let mut stepper = Stepper::new(&program, NoObserver);
    let tokens = common::tokens(0, common::ASYNC_FN_FOO);
    let stale = stepper.prepare(tokens[0].clone());
    let _ = stepper.push(tokens[0].clone());
    let _ = stepper.push(tokens[1].clone());
    let state = stepper.state().clone();
    assert_eq!(
        stepper.commit(stale),
        Err(CommitRefusal::Stale {
            prepared: 0,
            current: 2
        })
    );
    assert_eq!(stepper.state(), &state);
    assert_eq!(stepper.version(), 2);
    // A preparation of `finish` is stale after a push too.
    let finish = stepper.prepare_finish();
    let _ = stepper.push(tokens[2].clone());
    assert!(matches!(
        stepper.commit(finish),
        Err(CommitRefusal::Stale { .. })
    ));
}

#[test]
fn a_preparation_from_another_stepper_is_refused() {
    let program = section16(common::CONTAINMENT);
    let tokens = common::tokens(0, common::ASYNC_FN_FOO);
    let a = Stepper::new(&program, NoObserver);
    let mut b = Stepper::new(&program, NoObserver);
    // Both at version 0, same program, same observation.
    let from_a = a.prepare(tokens[0].clone());
    let refusal = b.commit(from_a).unwrap_err();
    assert_eq!(refusal, CommitRefusal::OtherStepper);
    assert_eq!(refusal.code(), "BBBC0202");
    assert_eq!(b.version(), 0);
    assert_eq!(a.version(), 0);
    assert_eq!(b.state(), a.state());
}

// ---- a program with no recognizer: one push, one position ----

#[bitpack(storage = u8)]
pub struct Item {
    #[bits(2)]
    #[labels(a = 0, b = 1, c = 2)]
    pub kind: u8,
}

/// flaga_solver's `timer` requires `inline_queue`.
#[bitpack(storage = u16)]
pub struct Job {
    #[bits(3)]
    #[labels(signal = 0, timer = 1)]
    #[requires(timer => profile.inline_queue)]
    #[invalid(7)]
    pub reason: u16,
    #[bits(2)]
    #[labels(pooled = 0, inline_queue = 1)]
    pub profile: u16,
}

#[bitpack(storage = u8)]
pub struct Out {
    #[bits(2)]
    #[labels(none = 0, said = 1)]
    pub kind: u8,
}

const ITEM: DimensionId = DimensionId(0);
const A: u128 = 0;
const B: u128 = 1;
const C: u128 = 2;

/// `a` arms an inline timer and says so; `b` asks for a timer and writes no
/// profile, which commit refuses from the start state (`pooled`) and takes
/// once `a` has set `inline_queue`; `c` has no contract.
const ARM: ContractSpec = contract("arm")
    .when(&[clause("item", FieldMatcher::eq("kind", 0))])
    .transition(&[set("job", "reason", 1), set("job", "profile", 1)])
    .emit(&[set("out", "kind", 1)]);
const BARE: ContractSpec = contract("bare-timer")
    .when(&[clause("item", FieldMatcher::eq("kind", 1))])
    .transition(&[set("job", "reason", 1)]);

fn job(on_refusal: RefusalRule) -> Option<VerifiedProgram> {
    let spec = Program::new()
        .add(dimension!(Item, "item", Evidence))
        .add(dimension!(Job, "job", State))
        .add(dimension!(Out, "out", Effect))
        .contain(ContainmentSpec {
            max_interpretations: 1,
            max_candidates: 2,
            max_actualized: 2,
            max_stack_depth: 1,
            on_refusal,
        })
        .contract(ARM)
        .contract(BARE)
        .build()
        .expect("job program builds");
    verify(
        spec,
        VerifyLimits {
            max_states: 100,
            max_witness_tries: 64,
        },
    )
    .ok()
}

fn item(position: u32, kind: u128) -> Observation {
    Observation::new(position).with(ITEM, kind)
}

#[test]
fn a_refused_preparation_mutates_nothing() {
    let Some(program) = job(RefusalRule::SkipObservation) else {
        panic!("job program verifies");
    };
    let mut stepper = Stepper::new(&program, NoObserver);
    let start = stepper.state().clone();
    for (kind, code) in [(B, "BBBC0105"), (C, "BBBC0101"), (3, "BBBC0101")] {
        let prepared = stepper.prepare(item(0, kind));
        assert!(prepared.is_refused());
        assert_eq!(prepared.effects().count(), 0);
        assert_eq!(prepared.state_after(), &start, "a refusal writes nothing");
        assert_eq!(
            prepared.results()[0].refusals.last().map(Refusal::code),
            Some(code.to_owned())
        );
        drop(prepared);
        assert_eq!(stepper.state(), &start);
    }
    // Committed, a refusal consumes its position and still writes nothing.
    let prepared = stepper.prepare(item(0, B));
    let results = stepper.commit(prepared).unwrap();
    assert!(results[0].committed.is_empty());
    assert_eq!(stepper.state(), &start);
    let armed = stepper.push(item(1, A));
    assert_eq!(armed[0].position, 1);
    assert_ne!(stepper.state(), &start);
}

#[test]
fn push_is_prepare_then_commit_through_a_commit_refusal() {
    // `b` is authorized and then refused by commit: the observer sees
    // `authorized` and then `refused` either way.
    for rule in [RefusalRule::SkipObservation, RefusalRule::Stop] {
        let program = job(rule).expect("job program verifies");
        let kinds = [B, A, B, C, A, 3, B];
        let run = |two_phase: bool| {
            let mut recorder = Recorder {
                spec: program.spec().clone(),
                lines: Vec::new(),
            };
            let mut stepper = Stepper::new(&program, &mut recorder);
            let mut results = Vec::new();
            for (position, kind) in (0..).zip(kinds) {
                if two_phase {
                    let prepared = stepper.prepare(item(position, kind));
                    results.extend(stepper.commit(prepared).unwrap());
                } else {
                    results.extend(stepper.push(item(position, kind)));
                }
            }
            let state = stepper.state().clone();
            drop(stepper);
            (results, state, recorder.lines)
        };
        let pushed = run(false);
        assert_eq!(pushed, run(true), "{rule:?}");
        // Position 0: `b` from the start state is authorized, then commit refuses it.
        let at0: Vec<&str> = pushed.2.iter().take(5).map(String::as_str).collect();
        assert_eq!(at0[3], "authorized bare-timer", "{at0:?}");
        assert!(at0[4].starts_with("refused [BBBC0105]"), "{at0:?}");
    }
}

#[test]
fn several_positions_are_one_preparation_and_stop_holds() {
    // `finish` on section 16 steps the rest of the window and the end item:
    // several positions, each prepared from the state the one before left.
    let program = section16(common::CONTAINMENT);
    let mut stepper = Stepper::new(&program, NoObserver);
    for o in common::tokens(0, common::ASYNC_FN_FOO) {
        let _ = stepper.push(o);
    }
    let prepared = stepper.prepare_finish();
    let positions: Vec<u32> = prepared.results().iter().map(|r| r.position).collect();
    assert!(positions.len() > 1, "{positions:?}");
    assert!(positions.windows(2).all(|w| w[0] + 1 == w[1]));
    let results = stepper.commit(prepared).unwrap();
    assert_eq!(
        results.iter().map(|r| r.position).collect::<Vec<_>>(),
        positions
    );
    // After finish nothing more is stepped, prepared or not.
    let after = stepper.prepare(common::tokens(20, &[common::IDENT])[0].clone());
    assert!(after.results().is_empty());

    // Under `Stop` a preparation ends at the refusal and halts the stepper.
    let program = job(RefusalRule::Stop).expect("job program verifies");
    let mut stepper = Stepper::new(&program, NoObserver);
    let prepared = stepper.prepare(item(0, C));
    assert!(prepared.halts());
    assert!(!stepper.is_halted());
    let _ = stepper.commit(prepared).unwrap();
    assert!(stepper.is_halted());
    assert!(stepper.prepare(item(1, A)).results().is_empty());
    assert!(stepper.push(item(1, A)).is_empty());
}

#[test]
fn verification_and_preparation_agree() {
    // `legal_next_representatives` (the verifier's domain, through the same
    // `gather` and `authorize` the stepper and the verifier's search use)
    // lists the classes containment would authorize next. A preparation of a
    // listed word is authorized: it commits, or commit refuses its write; a
    // preparation of any other word is refused before authorization. Over
    // every concrete word a preparation gives what a fresh push gives.
    let program = job(RefusalRule::SkipObservation).expect("job program verifies");
    assert!(program.report().passed());
    let stepper = Stepper::new(&program, NoObserver);
    let listed: Vec<u128> = stepper
        .legal_next_representatives()
        .iter()
        .filter_map(|o| o.word(ITEM))
        .collect();
    assert_eq!(listed, [A, B]);
    for kind in 0..=3 {
        let prepared = stepper.prepare(item(0, kind));
        let result = &prepared.results()[0];
        let authorized = !result.committed.is_empty()
            || matches!(result.refusals.last(), Some(Refusal::IllegalWord { .. }));
        assert_eq!(authorized, listed.contains(&kind), "kind {kind}");
        let mut fresh = Stepper::new(&program, NoObserver);
        assert_eq!(prepared.results(), fresh.push(item(0, kind)).as_slice());
    }
    // From the armed state `b` commits: the class the verifier listed is
    // accepted the way its search says.
    let mut stepper = Stepper::new(&program, NoObserver);
    let _ = stepper.push(item(0, A));
    let prepared = stepper.prepare(item(1, B));
    assert!(!prepared.is_refused());

    // On section 16, along every fixture run, each representative prepares
    // without changing the stepper, and the run is the pushed run.
    let program = section16(common::CONTAINMENT);
    for kinds in inputs() {
        let mut stepper = Stepper::new(&program, NoObserver);
        for o in common::tokens(0, &kinds) {
            let state = stepper.state().clone();
            for r in stepper.legal_next_representatives() {
                drop(stepper.prepare(r));
            }
            assert_eq!(stepper.state(), &state);
            let _ = stepper.push(o);
        }
        let _ = stepper.finish();
        assert_eq!(stepper.state(), &by_push(&program, &kinds).1);
    }
}
