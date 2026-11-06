//! Correction 2: any registered evidence dimension receives values at run
//! time and takes part in contracts, not only the recognizer's item and
//! frame dimensions.
//!
//! The data is a door controller, to show the framework is not about
//! parsing: `badge`, `clock` and `door` are three independent evidence
//! dimensions, `access` is state, `log` is the effect.

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, FieldValue, Finding,
    Observation, Program, ProgramBuilder, ProgramSpec, RecognizerSpec, Refusal, RefusalRule,
    StepResult, Stepper, VerifiedProgram, VerifyLimits, clause, contract, dimension, set, verify,
};

#[bitpack(storage = u8)]
pub struct Badge {
    #[bits(2)]
    #[labels(Unknown = 0, Staff = 1, Visitor = 2, End = 3)]
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
pub struct Clock {
    #[bits(1)]
    #[labels(Day = 0, Night = 1)]
    pub shift: u8,
}

#[bitpack(storage = u8)]
pub struct Door {
    #[bits(1)]
    #[labels(Closed = 0, Open = 1)]
    pub state: u8,
}

#[bitpack(storage = u8)]
pub struct Access {
    #[bits(2)]
    #[labels(Locked = 0, Granted = 1)]
    #[invalid(2..=3)]
    pub state: u8,
}

#[bitpack(storage = u8)]
pub struct Log {
    #[bits(2)]
    #[labels(None = 0, Entry = 1, Propped = 2)]
    #[invalid(3)]
    pub kind: u8,
}

const STAFF: u128 = Badge::LABEL_KIND_STAFF.value;
const VISITOR: u128 = Badge::LABEL_KIND_VISITOR.value;
const END: u128 = Badge::LABEL_KIND_END.value;
const DAY: u128 = Clock::LABEL_SHIFT_DAY.value;
const NIGHT: u128 = Clock::LABEL_SHIFT_NIGHT.value;
const CLOSED: u128 = Door::LABEL_STATE_CLOSED.value;
const OPEN: u128 = Door::LABEL_STATE_OPEN.value;
const GRANTED: u128 = Access::LABEL_STATE_GRANTED.value;
const LOCKED: u128 = Access::LABEL_STATE_LOCKED.value;
const ENTRY: u128 = Log::LABEL_KIND_ENTRY.value;
const PROPPED: u128 = Log::LABEL_KIND_PROPPED.value;

const BADGE: DimensionId = DimensionId(0);
const FRAME: DimensionId = DimensionId(1);
const CLOCK: DimensionId = DimensionId(2);
const DOOR: DimensionId = DimensionId(3);
const ACCESS: DimensionId = DimensionId(4);

const fn badge(kind: u128) -> Clause {
    clause("badge", FieldMatcher::eq("kind", kind))
}

/// Badge and clock, two evidence dimensions, meet in one WHEN; a third, the door, in REQUIRE.
const NIGHT_STAFF_ENTRY: ContractSpec = contract("night-staff-entry")
    .when(&[
        badge(STAFF),
        clause("clock", FieldMatcher::eq("shift", NIGHT)),
    ])
    .require(&[clause("door", FieldMatcher::eq("state", CLOSED))])
    .transition(&[set("access", "state", GRANTED)])
    .emit(&[set("log", "kind", ENTRY)]);

/// Keyed on the clock, not on the item dimension; reads no badge at all.
const DAY_ENTRY: ContractSpec = contract("day-entry")
    .when(&[
        clause("clock", FieldMatcher::eq("shift", DAY)),
        clause("door", FieldMatcher::eq("state", CLOSED)),
    ])
    .forbid(&[badge(END)])
    .transition(&[set("access", "state", GRANTED)])
    .emit(&[set("log", "kind", ENTRY)]);

const DOOR_PROPPED: ContractSpec = contract("door-propped")
    .when(&[
        clause("door", FieldMatcher::eq("state", OPEN)),
        clause("badge", FieldMatcher::not_eq("kind", END)),
    ])
    .transition(&[set("access", "state", LOCKED)])
    .emit(&[set("log", "kind", PROPPED)]);

const CLOSE: ContractSpec = contract("close").when(&[badge(END)]);

const CONTRACTS: &[ContractSpec] = &[NIGHT_STAFF_ENTRY, DAY_ENTRY, DOOR_PROPPED, CLOSE];

const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 2,
    max_candidates: 4,
    max_stack_depth: 4,
    on_refusal: RefusalRule::SkipObservation,
};

/// No sequences: the badge is the item dimension only so that `finish` has an end item.
const RECOGNIZER: RecognizerSpec = RecognizerSpec {
    item_dimension: "badge",
    frame_dimension: "frame",
    sequence_field: "sequence",
    offset_field: "offset",
    position_field: "position",
    boundary_flag: "boundary",
    end_item: FieldValue::new("kind", END),
    sequences: &[],
};

fn builder(contracts: &[ContractSpec]) -> ProgramBuilder {
    contracts.iter().fold(
        Program::new()
            .add(dimension!(Badge, "badge", Evidence))
            .add(dimension!(Frame, "frame", Evidence))
            .add(dimension!(Clock, "clock", Evidence))
            .add(dimension!(Door, "door", Evidence))
            .add(dimension!(Access, "access", State))
            .add(dimension!(Log, "log", Effect))
            .recognize(RECOGNIZER)
            .contain(CONTAIN),
        |b, c| b.contract(*c),
    )
}

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 1_000,
    max_witness_tries: 1_024,
};

fn program() -> VerifiedProgram {
    verify(builder(CONTRACTS).build().unwrap(), LIMITS).expect("door program verifies")
}

fn observe(position: u32, words: &[(DimensionId, u128)]) -> Observation {
    words
        .iter()
        .fold(Observation::new(position), |o, (d, w)| o.with(*d, *w))
}

/// `position: contract` or `position: refused [..]`, with effects.
fn line(spec: &ProgramSpec, r: &StepResult) -> String {
    let effects: Vec<String> = r
        .effects
        .iter()
        .map(|e| spec.dimension(e.dimension).unwrap().describe(e.word))
        .collect();
    match r.committed {
        Some(id) => format!(
            "{}: {} {}",
            r.position,
            spec.contract(id).unwrap().name,
            effects.join(" ")
        ),
        None => {
            let codes: Vec<String> = r.refusals.iter().map(ToString::to_string).collect();
            format!("{}: refused {}", r.position, codes.join("; "))
        }
    }
}

#[test]
fn three_evidence_dimensions_meet_in_one_contract() {
    let program = program();
    let spec = program.spec();
    let mut stepper = Stepper::new(&program, big_bad_bool_contract::NoObserver);
    let inputs = [
        observe(0, &[(BADGE, STAFF), (CLOCK, NIGHT), (DOOR, CLOSED)]),
        observe(1, &[(BADGE, VISITOR), (CLOCK, NIGHT), (DOOR, CLOSED)]),
        // No clock: the clock clause holds nothing.
        observe(2, &[(BADGE, STAFF), (DOOR, CLOSED)]),
        // No badge: day-entry reads only the clock and the door.
        observe(3, &[(CLOCK, DAY), (DOOR, CLOSED)]),
        observe(4, &[(BADGE, STAFF), (CLOCK, NIGHT), (DOOR, OPEN)]),
    ];
    let mut results = Vec::new();
    for o in inputs {
        let stepped = stepper.push(o);
        assert_eq!(stepped.len(), 1, "no sequences: each push steps at once");
        results.extend(stepped);
    }
    let access_after_4 = stepper.state().word(ACCESS);
    results.extend(stepper.finish());
    let lines: Vec<String> = results.iter().map(|r| line(spec, r)).collect();
    assert_eq!(
        lines,
        [
            "0: night-staff-entry {kind=Entry}",
            "1: refused [BBBC0101] no contract accepts position 1 (0 refused)",
            "2: refused [BBBC0101] no contract accepts position 2 (0 refused)",
            "3: day-entry {kind=Entry}",
            "4: door-propped {kind=Propped}",
            "5: close ",
        ]
    );
    assert_eq!(access_after_4, LOCKED);
}

#[test]
fn an_absent_dimension_reads_as_nothing() {
    let program = program();
    let mut stepper = Stepper::new(&program, big_bad_bool_contract::NoObserver);
    // Staff at night with no door word: WHEN holds, REQUIRE on the door does not.
    let r = stepper.push(observe(0, &[(BADGE, STAFF), (CLOCK, NIGHT)]));
    assert_eq!(
        r[0].refusals,
        [
            Refusal::ContractRefused {
                position: 0,
                contract: "night-staff-entry",
                clause: "require",
                missing: "door.state = Closed".to_owned(),
                observed: "nothing".to_owned(),
            },
            Refusal::NoContract {
                position: 0,
                refused: 1
            },
        ]
    );
}

#[test]
fn an_observation_holds_only_observed_evidence() {
    let program = program();
    let mut stepper = Stepper::new(&program, big_bad_bool_contract::NoObserver);
    let start = stepper.state().clone();
    let refusal = |o: Observation| stepper_refusal(&program, o);
    assert_eq!(
        refusal(observe(0, &[(ACCESS, GRANTED)])),
        Refusal::Evidence {
            position: 0,
            dimension: "access".to_owned(),
            rule: "only evidence dimensions are observed",
        }
    );
    assert_eq!(
        refusal(observe(0, &[(FRAME, 0)])),
        Refusal::Evidence {
            position: 0,
            dimension: "frame".to_owned(),
            rule: "the recognizer writes this dimension",
        }
    );
    assert_eq!(
        refusal(observe(0, &[(DimensionId(9), 0)])),
        Refusal::Evidence {
            position: 0,
            dimension: "#9".to_owned(),
            rule: "no such dimension",
        }
    );
    // A bit outside the clock's one-bit layout.
    let illegal = refusal(observe(0, &[(BADGE, STAFF), (CLOCK, 0b10)]));
    assert!(
        matches!(
            &illegal,
            Refusal::IllegalWord {
                dimension: "clock",
                ..
            }
        ),
        "{illegal}"
    );
    assert_eq!(
        illegal.to_string().split(':').next(),
        Some("[BBBC0105] illegal `clock` word at position 0")
    );
    // None of it reached the state.
    let _ = stepper.push(observe(0, &[(ACCESS, GRANTED)]));
    assert_eq!(stepper.state(), &start);
}

fn stepper_refusal(program: &VerifiedProgram, o: Observation) -> Refusal {
    let mut stepper = Stepper::new(program, big_bad_bool_contract::NoObserver);
    let r = stepper.push(o);
    assert!(r[0].committed.is_none());
    r[0].refusals.last().unwrap().clone()
}

#[test]
fn typed_and_raw_words_make_one_observation() {
    let typed = Observation::new(3)
        .with_word(CLOCK, Clock::new(1))
        .with_word(BADGE, Badge::new(1));
    let raw = Observation::new(3).with(BADGE, STAFF).with(CLOCK, NIGHT);
    assert_eq!(typed, raw);
    assert_eq!(typed.evidence(), [(BADGE, STAFF), (CLOCK, NIGHT)]);
    assert_eq!(typed.word(DOOR), None);
    // A second word for one dimension replaces the first.
    assert_eq!(raw.clone().with(CLOCK, DAY).word(CLOCK), Some(DAY));
}

#[test]
fn the_verifier_reads_every_evidence_dimension() {
    let program = program();
    // Badge 4 labels, clock 2, door 2, each also absent.
    assert_eq!(program.inputs().len(), 5 * 3 * 3);
    assert!(program.report().passed(), "{:?}", program.report().findings);

    // legal_next draws from every evidence dimension, not only the item.
    let stepper = Stepper::new(&program, big_bad_bool_contract::NoObserver);
    let next = stepper.legal_next();
    assert!(next.contains(&observe(
        0,
        &[(BADGE, STAFF), (CLOCK, NIGHT), (DOOR, CLOSED)]
    )));
    assert!(next.contains(&observe(0, &[(CLOCK, DAY), (DOOR, CLOSED)])));
    assert!(!next.contains(&observe(
        0,
        &[(BADGE, VISITOR), (CLOCK, NIGHT), (DOOR, CLOSED)]
    )));

    // A contract whose clock clauses contradict is found on the clock.
    const NEVER: ContractSpec = contract("never")
        .when(&[clause("clock", FieldMatcher::eq("shift", NIGHT))])
        .forbid(&[clause("clock", FieldMatcher::eq("shift", NIGHT))]);
    let mut contracts = CONTRACTS.to_vec();
    contracts.push(NEVER);
    let report = verify(builder(&contracts).build().unwrap(), LIMITS).unwrap_err();
    assert!(report.findings.contains(&Finding::Unsatisfiable {
        owner: "never",
        dimension: "clock",
        field: "(joint)"
    }));
}
