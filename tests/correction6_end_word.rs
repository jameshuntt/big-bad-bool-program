//! Correction 6: the recognizer's end word is the end of input. Only
//! `finish` produces it; `push` refuses an observation whose item is the end
//! word, so the stream cannot step it in the middle and go on from what it
//! committed. The verifier evaluates the end item and does not follow its
//! successor; the stepper now does the same.
//!
//! The data is a run with a closing signal: `signal` is the item (`Tick`,
//! `Pause`, and `Close` as the end word), `phase` is state, and `close`
//! moves it to `Closed` only on the end item.

use big_bad_bool::bitpack;
use big_bad_bool_program::{
    ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, FieldValue, Finding, NoObserver,
    Observation, Program, ProgramSpec, RecognizerSpec, Refusal, RefusalRule, StepResult, Stepper,
    VerifiedProgram, VerifyLimits, clause, contract, dimension, set, verify,
};

#[bitpack(storage = u8)]
pub struct Signal {
    #[bits(2)]
    #[labels(Tick = 1, Pause = 2, Close = 3)]
    pub kind: u8,
}

#[bitpack(storage = u8)]
pub struct Frame {
    #[bits(1)]
    pub sequence: u8,
    #[bits(1)]
    pub offset: u8,
    #[bits(3)]
    #[invalid(5..=7)]
    pub position: u8,
    #[bits(1)]
    #[labels(boundary = 1)]
    #[flags(boundary = 0)]
    pub marks: u8,
}

#[bitpack(storage = u8)]
pub struct Phase {
    #[bits(2)]
    #[labels(Running = 0, Paused = 1, Closed = 2)]
    #[invalid(3)]
    pub state: u8,
}

#[bitpack(storage = u8)]
pub struct Note {
    #[bits(1)]
    #[labels(Quiet = 0, Ticked = 1)]
    pub kind: u8,
}

const TICK: u128 = Signal::LABEL_KIND_TICK.value;
const PAUSE: u128 = Signal::LABEL_KIND_PAUSE.value;
const CLOSE: u128 = Signal::LABEL_KIND_CLOSE.value;
const RUNNING: u128 = Phase::LABEL_STATE_RUNNING.value;
const PAUSED: u128 = Phase::LABEL_STATE_PAUSED.value;
const CLOSED: u128 = Phase::LABEL_STATE_CLOSED.value;

const SIGNAL: DimensionId = DimensionId(0);
const PHASE: DimensionId = DimensionId(2);

const fn signal(kind: u128) -> big_bad_bool_program::Clause {
    clause("signal", FieldMatcher::eq("kind", kind))
}

const fn phase(state: u128) -> big_bad_bool_program::Clause {
    clause("phase", FieldMatcher::eq("state", state))
}

/// A tick runs, from running or paused.
const TICK_RUNS: ContractSpec = contract("tick")
    .when(&[signal(TICK)])
    .forbid(&[phase(CLOSED)])
    .transition(&[set("phase", "state", RUNNING)])
    .emit(&[set("note", "kind", 1)]);

const PAUSES: ContractSpec = contract("pause")
    .when(&[signal(PAUSE)])
    .forbid(&[phase(CLOSED)])
    .transition(&[set("phase", "state", PAUSED)]);

/// Only the end item closes the run.
const CLOSES: ContractSpec = contract("close")
    .when(&[signal(CLOSE)])
    .transition(&[set("phase", "state", CLOSED)]);

const CONTRACTS: &[ContractSpec] = &[TICK_RUNS, PAUSES, CLOSES];

/// No sequences: the signal is the item dimension so that `finish` has an end item.
const RECOGNIZER: RecognizerSpec = RecognizerSpec {
    item_dimension: "signal",
    frame_dimension: "frame",
    sequence_field: "sequence",
    offset_field: "offset",
    position_field: "position",
    boundary_flag: "boundary",
    end_item: FieldValue::new("kind", CLOSE),
    sequences: &[],
};

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 1_000,
    max_witness_tries: 1_024,
};

fn containment(on_refusal: RefusalRule) -> ContainmentSpec {
    ContainmentSpec {
        max_interpretations: 2,
        max_candidates: 4,
        max_actualized: 4,
        max_stack_depth: 4,
        on_refusal,
    }
}

fn spec(on_refusal: RefusalRule) -> ProgramSpec {
    CONTRACTS
        .iter()
        .fold(
            Program::new()
                .add(dimension!(Signal, "signal", Evidence))
                .add(dimension!(Frame, "frame", Evidence))
                .add(dimension!(Phase, "phase", State))
                .add(dimension!(Note, "note", Effect))
                .recognize(RECOGNIZER)
                .contain(containment(on_refusal)),
            |b, c| b.contract(*c),
        )
        .build()
        .expect("run program builds")
}

fn program(on_refusal: RefusalRule) -> VerifiedProgram {
    verify(spec(on_refusal), LIMITS).unwrap_or_else(|r| panic!("{:?}", r.findings))
}

fn item(position: u32, kind: u128) -> Observation {
    Observation::new(position).with(SIGNAL, kind)
}

fn reserved(position: u32) -> Refusal {
    Refusal::Evidence {
        position,
        dimension: "signal".to_owned(),
        rule: "the end word is reserved for `finish`",
    }
}

fn refused(position: u32, refusal: Refusal) -> StepResult {
    StepResult {
        position,
        committed: Vec::new(),
        effects: Vec::new(),
        refusals: vec![refusal],
    }
}

#[test]
fn an_end_word_pushed_mid_stream_is_refused() {
    let program = program(RefusalRule::SkipObservation);
    let mut stepper = Stepper::new(&program, NoObserver);
    assert_eq!(stepper.push(item(0, TICK))[0].committed.len(), 1);
    let r = stepper.push(item(1, CLOSE));
    assert_eq!(r, [refused(1, reserved(1))]);
    assert_eq!(
        r[0].refusals[0].to_string(),
        "[BBBC0108] observation at position 1 may not hold `signal`: the end word is reserved for `finish`"
    );
    // As the very first observation too.
    let mut fresh = Stepper::new(&program, NoObserver);
    assert_eq!(fresh.push(item(0, CLOSE)), [refused(0, reserved(0))]);
    // Every time, not only the first.
    assert_eq!(stepper.push(item(2, CLOSE)), [refused(2, reserved(2))]);
}

#[test]
fn an_injected_end_word_changes_no_state_and_opens_no_path() {
    // Skipping the refused observation: the stream goes on from the state
    // before it, as if it had not been sent.
    let program = program(RefusalRule::SkipObservation);
    let mut stepper = Stepper::new(&program, NoObserver);
    stepper.push(item(0, PAUSE));
    let before = stepper.state().clone();
    assert_eq!(before.word(PHASE), PAUSED);
    let _ = stepper.push(item(1, CLOSE));
    assert_eq!(stepper.state(), &before, "the end word wrote nothing");
    assert!(!stepper.is_halted());
    // `close` did not commit, so `tick` still applies: a closed run would refuse it.
    let r = stepper.push(item(2, TICK));
    assert_eq!(r[0].position, 2);
    assert_eq!(r[0].committed.len(), 1);
    assert_eq!(stepper.state().word(PHASE), RUNNING);

    // Stopping on refusal: nothing more is stepped, and the state is the one before.
    let program = program_stop();
    let mut stepper = Stepper::new(&program, NoObserver);
    stepper.push(item(0, PAUSE));
    let before = stepper.state().clone();
    assert_eq!(stepper.push(item(1, CLOSE)), [refused(1, reserved(1))]);
    assert!(stepper.is_halted());
    assert_eq!(stepper.state(), &before);
    assert!(stepper.push(item(2, TICK)).is_empty());
    assert!(stepper.finish().is_empty());
    assert_eq!(stepper.state(), &before);
}

fn program_stop() -> VerifiedProgram {
    program(RefusalRule::Stop)
}

#[test]
fn finish_still_steps_the_end_item() {
    for rule in [RefusalRule::SkipObservation, RefusalRule::Stop] {
        let program = program(rule);
        let mut stepper = Stepper::new(&program, NoObserver);
        stepper.push(item(0, TICK));
        stepper.push(item(1, PAUSE));
        let rest = stepper.finish();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].position, 2, "one past the last observation");
        assert_eq!(rest[0].committed.len(), 1);
        assert_eq!(
            program.spec().contract(rest[0].committed[0]).unwrap().name,
            "close"
        );
        assert!(rest[0].refusals.is_empty());
        assert_eq!(stepper.state().word(PHASE), CLOSED);
        // Nothing steps after finish.
        assert!(stepper.push(item(3, TICK)).is_empty());
        assert!(stepper.finish().is_empty());
    }
    // With an empty stream the end item is the only step.
    let program = program(RefusalRule::Stop);
    let mut stepper = Stepper::new(&program, NoObserver);
    let rest = stepper.finish();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].position, 0);
    assert_eq!(stepper.state().word(PHASE), CLOSED);
}

#[test]
fn verifier_and_runtime_agree_on_the_end_word() {
    let program = program(RefusalRule::SkipObservation);
    // The verifier evaluates the end item (so `close` is not dead) but does
    // not follow its successor: no state it visits is closed.
    assert!(program.report().passed(), "{:?}", program.report().findings);
    assert!(program.report().findings.contains(&Finding::Unreachable {
        dimension: "phase",
        field: "state",
        label: "Closed",
    }));
    assert!(
        program
            .inputs()
            .iter()
            .any(|o| o.word(SIGNAL) == Some(CLOSE)),
        "the search still tries the end item"
    );

    // At run time no pushed input reaches a closed phase either: every input
    // the verifier tried, from every state the stream reaches, in turn.
    let mut stepper = Stepper::new(&program, NoObserver);
    let mut position = 0;
    for _ in 0..3 {
        for input in program.inputs() {
            let o = input
                .evidence()
                .iter()
                .fold(Observation::new(position), |o, (d, w)| o.with(*d, *w));
            for r in stepper.push(o) {
                if input.word(SIGNAL) == Some(CLOSE) {
                    assert_eq!(r.refusals, [reserved(position)]);
                }
            }
            assert_ne!(
                stepper.state().word(PHASE),
                CLOSED,
                "at position {position}"
            );
            // The representatives of the next step never include the end word.
            assert!(
                stepper
                    .legal_next_representatives()
                    .iter()
                    .all(|o| o.word(SIGNAL) != Some(CLOSE))
            );
            position += 1;
        }
    }
    // Only finish closes the run, as its last step.
    let rest = stepper.finish();
    assert_eq!(rest.len(), 1);
    assert_eq!(stepper.state().word(PHASE), CLOSED);
}
