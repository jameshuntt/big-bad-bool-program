//! Correction 1: framing is optional. A program of dimensions, contracts
//! and containment builds, verifies and steps with no recognizer; adding a
//! recognizer changes nothing for contracts that do not read its frame.
//!
//! The data is a thermostat: `reading` and `mode` are evidence, `heater` is
//! state, `command` is the effect.

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, FieldValue, Finding, NoObserver,
    Observation, Program, ProgramBuilder, Recognizer, RecognizerSpec, RefusalRule, StepResult,
    Stepper, VerifiedProgram, VerifyLimits, clause, contract, dimension, set, spec_material,
    verify,
};

#[bitpack(storage = u8)]
pub struct Reading {
    #[bits(2)]
    #[labels(Cold = 0, Mild = 1, Hot = 2, End = 3)]
    pub level: u8,
}

#[bitpack(storage = u8)]
pub struct Mode {
    #[bits(1)]
    #[labels(Auto = 0, Manual = 1)]
    pub kind: u8,
}

#[bitpack(storage = u8)]
pub struct Heater {
    #[bits(1)]
    #[labels(Off = 0, On = 1)]
    pub state: u8,
}

#[bitpack(storage = u8)]
pub struct Command {
    #[bits(2)]
    #[labels(None = 0, StartHeat = 1, StopHeat = 2)]
    #[invalid(3)]
    pub kind: u8,
}

/// Only for the framed copy of the same program.
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

const COLD: u128 = Reading::LABEL_LEVEL_COLD.value;
const MILD: u128 = Reading::LABEL_LEVEL_MILD.value;
const HOT: u128 = Reading::LABEL_LEVEL_HOT.value;
const END: u128 = Reading::LABEL_LEVEL_END.value;
const AUTO: u128 = Mode::LABEL_KIND_AUTO.value;
const MANUAL: u128 = Mode::LABEL_KIND_MANUAL.value;
const OFF: u128 = Heater::LABEL_STATE_OFF.value;
const ON: u128 = Heater::LABEL_STATE_ON.value;

const fn reading(level: u128) -> big_bad_bool_contract::Clause {
    clause("reading", FieldMatcher::eq("level", level))
}
const fn mode(kind: u128) -> big_bad_bool_contract::Clause {
    clause("mode", FieldMatcher::eq("kind", kind))
}
const fn heater(state: u128) -> big_bad_bool_contract::Clause {
    clause("heater", FieldMatcher::eq("state", state))
}

const CONTRACTS: &[ContractSpec] = &[
    contract("heat-when-cold")
        .when(&[reading(COLD), mode(AUTO)])
        .require(&[heater(OFF)])
        .transition(&[set("heater", "state", ON)])
        .emit(&[set("command", "kind", 1)]),
    contract("stop-when-hot")
        .when(&[reading(HOT), mode(AUTO)])
        .require(&[heater(ON)])
        .transition(&[set("heater", "state", OFF)])
        .emit(&[set("command", "kind", 2)]),
    contract("hold").when(&[reading(MILD)]),
    contract("manual").when(&[mode(MANUAL)]),
    contract("end").when(&[reading(END)]),
];

const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 2,
    max_candidates: 4,
    max_actualized: 4,
    max_stack_depth: 4,
    on_refusal: RefusalRule::SkipObservation,
};

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 1_000,
    max_witness_tries: 1_024,
};

const READING: DimensionId = DimensionId(0);
const MODE: DimensionId = DimensionId(1);
const HEATER: DimensionId = DimensionId(2);

/// Dimensions, contracts and containment. Nothing else.
fn contract_only() -> ProgramBuilder {
    CONTRACTS.iter().fold(
        Program::new()
            .add(dimension!(Reading, "reading", Evidence))
            .add(dimension!(Mode, "mode", Evidence))
            .add(dimension!(Heater, "heater", State))
            .add(dimension!(Command, "command", Effect))
            .contain(CONTAIN),
        |b, c| b.contract(*c),
    )
}

/// The same program with a recognizer over the reading, declaring no sequences.
fn framed() -> ProgramBuilder {
    contract_only()
        .add(dimension!(Frame, "frame", Evidence))
        .recognize(RecognizerSpec {
            item_dimension: "reading",
            frame_dimension: "frame",
            sequence_field: "sequence",
            offset_field: "offset",
            position_field: "position",
            boundary_flag: "boundary",
            end_item: FieldValue::new("level", END),
            sequences: &[],
        })
}

const INPUT: &[(u128, u128)] = &[
    (COLD, AUTO),
    (MILD, AUTO),
    (HOT, AUTO),
    (HOT, AUTO),
    (COLD, MANUAL),
];

fn observations() -> Vec<Observation> {
    INPUT
        .iter()
        .zip(0..)
        .map(|((r, m), p)| Observation::new(p).with(READING, *r).with(MODE, *m))
        .collect()
}

fn run(program: &VerifiedProgram) -> (Vec<StepResult>, Vec<StepResult>) {
    let mut stepper = Stepper::new(program, NoObserver);
    let mut pushed = Vec::new();
    for o in observations() {
        pushed.extend(stepper.push(o));
    }
    (pushed, stepper.finish())
}

fn names(program: &VerifiedProgram, results: &[StepResult]) -> Vec<String> {
    results
        .iter()
        .map(|r| match r.committed.as_slice() {
            [] => format!("refused {}", r.refusals.last().unwrap()),
            ids => ids
                .iter()
                .map(|id| program.spec().contract(*id).unwrap().name)
                .collect::<Vec<_>>()
                .join("+"),
        })
        .collect()
}

#[test]
fn a_contract_only_program_builds_verifies_and_steps() {
    let spec = contract_only().build().expect("builds with no recognizer");
    assert!(spec.recognizer().is_none());
    assert_eq!(spec.item_dimension(), None);
    assert_eq!(spec.frame_dimension(), None);
    assert_eq!(spec.end_word(), None);
    assert_eq!(spec.window_radius(), 0);
    assert!(spec.is_observed(READING) && spec.is_observed(MODE));
    assert!(!spec.is_observed(HEATER));
    assert_eq!(
        Recognizer::new(&spec).unwrap_err(),
        [Finding::Missing { part: "recognizer" }]
    );
    assert!(!spec_material(&spec).contains("recognizer."));

    let program = verify(spec, LIMITS).expect("verifies with no recognizer");
    assert!(program.recognizer().is_none());
    assert!(program.report().passed(), "{:?}", program.report().findings);
    // Reading 4 labels, mode 2, each also absent.
    assert_eq!(program.inputs().len(), 5 * 3);

    let mut stepper = Stepper::new(&program, NoObserver);
    let mut results = Vec::new();
    for o in observations() {
        // Radius zero: every push steps its own position at once.
        let stepped = stepper.push(o);
        assert_eq!(stepped.len(), 1);
        results.extend(stepped);
    }
    assert_eq!(
        names(&program, &results),
        [
            "heat-when-cold",
            "hold",
            "stop-when-hot",
            "refused [BBBC0101] no contract accepts position 3 (1 refused)",
            "manual",
        ]
    );
    let effects: Vec<(u32, u128)> = results
        .iter()
        .flat_map(|r| &r.effects)
        .map(|e| (e.position, e.word))
        .collect();
    assert_eq!(effects, [(0, 1), (2, 2)]);
    assert_eq!(stepper.state().word(HEATER), OFF);
    // No end item: finish has nothing left to step.
    assert!(stepper.finish().is_empty());
    // From heater Off: Mild or End with any mode (6), Manual with no, Cold or Hot reading (3), Cold and Auto (1).
    assert_eq!(stepper.legal_next_representatives().len(), 10);
}

#[test]
fn a_recognizer_is_an_optional_layer_over_the_same_contracts() {
    let plain = verify(contract_only().build().unwrap(), LIMITS).unwrap();
    let framed = verify(framed().build().unwrap(), LIMITS).unwrap();
    assert!(framed.recognizer().is_some());
    let (plain_pushed, plain_rest) = run(&plain);
    let (framed_pushed, framed_rest) = run(&framed);
    // Contracts that read no frame give the same results with or without one.
    assert_eq!(plain_pushed, framed_pushed);
    assert!(plain_rest.is_empty());
    // The recognizer adds only its own layer: the end item at finish.
    assert_eq!(names(&framed, &framed_rest), ["end"]);
    assert_eq!(framed_rest[0].position, INPUT.len() as u32);
}
