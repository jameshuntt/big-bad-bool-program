//! Correction 3: one observation may actualize several independent
//! contracts. Compatible writes and effects merge, identical writes count
//! once, contradictory writes are refused, the fan-out is limited, and the
//! result does not depend on the order contracts were declared in.
//!
//! The data is a loading bay, with no recognizer: `parcel` and `gate` are
//! evidence; `bay`, `tally` and the stacked `crate` are state; `label` and
//! `notify` are two effect dimensions.

use big_bad_bool::bitpack;
use big_bad_bool_program::{
    Clause, ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, FieldValue, Finding,
    NoObserver, Observation, Program, ProgramBuilder, ProjectionState, Refusal, RefusalRule,
    StepResult, Stepper, VerificationReport, VerifiedProgram, VerifyLimits, clause, contract,
    dimension, pop, push, set, verify,
};

#[bitpack(storage = u8)]
pub struct Parcel {
    #[bits(2)]
    #[labels(Letter = 0, Box = 1, Fragile = 2)]
    #[invalid(3)]
    pub kind: u8,
}

#[bitpack(storage = u8)]
pub struct Gate {
    #[bits(1)]
    #[labels(Dock = 0, Shelf = 1)]
    pub at: u8,
}

#[bitpack(storage = u8)]
pub struct Bay {
    #[bits(2)]
    #[labels(Empty = 0, Loaded = 1, Held = 2)]
    #[invalid(3)]
    pub status: u8,
    #[bits(1)]
    #[labels(Normal = 0, Careful = 1)]
    pub care: u8,
}

#[bitpack(storage = u8)]
pub struct Tally {
    #[bits(1)]
    #[labels(Even = 0, Odd = 1)]
    pub parity: u8,
}

#[bitpack(storage = u8)]
pub struct Crate {
    #[bits(1)]
    #[labels(Any = 0, Glass = 1)]
    pub content: u8,
}

#[bitpack(storage = u8)]
pub struct Label {
    #[bits(2)]
    #[labels(None = 0, Stamp = 1, Warn = 2)]
    #[invalid(3)]
    pub kind: u8,
}

#[bitpack(storage = u8)]
pub struct Notify {
    #[bits(2)]
    #[labels(None = 0, Driver = 1, Clerk = 2)]
    #[invalid(3)]
    pub kind: u8,
}

const LETTER: u128 = Parcel::LABEL_KIND_LETTER.value;
const BOX: u128 = Parcel::LABEL_KIND_BOX.value;
const FRAGILE: u128 = Parcel::LABEL_KIND_FRAGILE.value;
const DOCK: u128 = Gate::LABEL_AT_DOCK.value;
const SHELF: u128 = Gate::LABEL_AT_SHELF.value;

const PARCEL: DimensionId = DimensionId(0);
const GATE: DimensionId = DimensionId(1);

const fn parcel(kind: u128) -> Clause {
    clause("parcel", FieldMatcher::eq("kind", kind))
}
const fn gate(at: u128) -> Clause {
    clause("gate", FieldMatcher::eq("at", at))
}
const fn tally(parity: u128) -> Clause {
    clause("tally", FieldMatcher::eq("parity", parity))
}
const GLASS: &[FieldValue] = &[FieldValue::new("content", 1)];

/// Every contract here is compatible with every other one.
const COMPATIBLE: &[ContractSpec] = &[
    contract("load")
        .when(&[parcel(BOX), gate(DOCK)])
        .transition(&[set("bay", "status", 1)])
        .emit(&[set("label", "kind", 1)]),
    // Another dimension than `load`.
    contract("count-up")
        .when(&[gate(DOCK)])
        .require(&[tally(0)])
        .transition(&[set("tally", "parity", 1)]),
    contract("count-down")
        .when(&[gate(DOCK)])
        .require(&[tally(1)])
        .transition(&[set("tally", "parity", 0)]),
    // An independent effect dimension.
    contract("notify-driver")
        .when(&[parcel(BOX), gate(DOCK)])
        .emit(&[set("notify", "kind", 1)]),
    // The same effect write as `load`, and the same state write.
    contract("stamp-again")
        .when(&[parcel(BOX)])
        .emit(&[set("label", "kind", 1)]),
    contract("mark-loaded")
        .when(&[parcel(BOX), gate(DOCK)])
        .transition(&[set("bay", "status", 1)]),
    // The same push twice, and a field of another state dimension.
    contract("open-crate")
        .when(&[parcel(FRAGILE), gate(SHELF)])
        .stack(&[push("crate", GLASS)]),
    contract("glass-crate")
        .when(&[parcel(FRAGILE), gate(SHELF)])
        .stack(&[push("crate", GLASS)]),
    contract("careful")
        .when(&[parcel(FRAGILE), gate(SHELF)])
        .transition(&[set("bay", "care", 1)]),
    contract("close-crate")
        .when(&[parcel(LETTER), gate(SHELF)])
        .stack(&[pop("crate")]),
    contract("unload")
        .when(&[parcel(LETTER), gate(DOCK)])
        .transition(&[set("bay", "status", 0), set("bay", "care", 0)]),
];

const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 1,
    max_candidates: 8,
    max_actualized: 8,
    max_stack_depth: 3,
    on_refusal: RefusalRule::SkipObservation,
};

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 10_000,
    max_witness_tries: 1_024,
};

fn builder(contracts: &[ContractSpec], containment: ContainmentSpec) -> ProgramBuilder {
    contracts.iter().fold(
        Program::new()
            .add(dimension!(Parcel, "parcel", Evidence))
            .add(dimension!(Gate, "gate", Evidence))
            .add(dimension!(Bay, "bay", State))
            .add(dimension!(Tally, "tally", State))
            .add(dimension!(Crate, "crate", State).stacked(3))
            .add(dimension!(Label, "label", Effect))
            .add(dimension!(Notify, "notify", Effect))
            .contain(containment),
        |b, c| b.contract(*c),
    )
}

fn program(contracts: &[ContractSpec]) -> VerifiedProgram {
    verify(builder(contracts, CONTAIN).build().unwrap(), LIMITS).expect("bay program verifies")
}

fn report(contracts: &[ContractSpec]) -> VerificationReport {
    match verify(builder(contracts, CONTAIN).build().unwrap(), LIMITS) {
        Ok(p) => p.report().clone(),
        Err(r) => r,
    }
}

/// Parcel and gate per position; `None` leaves that dimension out.
const INPUT: &[(Option<u128>, Option<u128>)] = &[
    (Some(BOX), Some(DOCK)),
    (Some(FRAGILE), Some(SHELF)),
    (Some(LETTER), Some(SHELF)),
    (Some(LETTER), Some(DOCK)),
    (Some(BOX), None),
];

fn observations() -> Vec<Observation> {
    INPUT
        .iter()
        .zip(0..)
        .map(|((p, g), at)| {
            let o = Observation::new(at);
            let o = p.map_or(o.clone(), |p| o.with(PARCEL, p));
            g.map_or(o.clone(), |g| o.with(GATE, g))
        })
        .collect()
}

fn run(program: &VerifiedProgram) -> (Vec<StepResult>, ProjectionState) {
    let mut stepper = Stepper::new(program, NoObserver);
    let mut results = Vec::new();
    for o in observations() {
        results.extend(stepper.push(o));
    }
    results.extend(stepper.finish());
    (results, stepper.state().clone())
}

/// One line per position: contract names in name order, then effects. No ids,
/// so two programs that differ only in declaration order compare equal.
fn lines(program: &VerifiedProgram, results: &[StepResult]) -> Vec<String> {
    let spec = program.spec();
    results
        .iter()
        .map(|r| {
            let mut names: Vec<&str> = r
                .committed
                .iter()
                .map(|id| spec.contract(*id).unwrap().name)
                .collect();
            names.sort_unstable();
            let effects: Vec<String> = r
                .effects
                .iter()
                .map(|e| {
                    let d = spec.dimension(e.dimension).unwrap();
                    format!("{} {}", d.name(), d.describe(e.word))
                })
                .collect();
            let refusals: Vec<String> = r.refusals.iter().map(ToString::to_string).collect();
            format!(
                "{}: [{}] [{}] [{}]",
                r.position,
                names.join(" "),
                effects.join(", "),
                refusals.join("; ")
            )
        })
        .collect()
}

fn state_text(program: &VerifiedProgram, state: &ProjectionState) -> String {
    let spec = program.spec();
    ["bay", "tally", "crate"]
        .iter()
        .map(|name| {
            let id = spec.dimension_id(name).unwrap();
            let d = spec.dimension(id).unwrap();
            let entries: Vec<String> = if d.stack_depth().is_some() {
                state.stack(id).iter().map(|w| d.describe(*w)).collect()
            } else {
                vec![d.describe(state.word(id))]
            };
            format!("{name}={}", entries.join(" "))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const EXPECTED: &[&str] = &[
    // Different dimensions (bay, tally), independent effects (label, notify),
    // identical writes (bay.status twice, label.kind twice) once.
    "0: [count-up load mark-loaded notify-driver stamp-again] [label {kind=Stamp}, notify {kind=Driver}] []",
    // One push from two contracts, and a field of another dimension.
    "1: [careful glass-crate open-crate] [] []",
    "2: [close-crate] [] []",
    "3: [count-down unload] [] []",
    "4: [stamp-again] [label {kind=Stamp}] []",
];

#[test]
fn compatible_contracts_actualize_together() {
    let program = program(COMPATIBLE);
    assert!(program.report().passed(), "{:?}", program.report().findings);
    let (results, state) = run(&program);
    assert_eq!(lines(&program, &results), EXPECTED);
    assert_eq!(
        state_text(&program, &state),
        "bay={status=Empty, care=Normal} tally={parity=Even} crate={content=Any}"
    );
    // After position 1 the crate holds one Glass entry above the start entry, not two.
    let mut stepper = Stepper::new(&program, NoObserver);
    for o in observations().into_iter().take(2) {
        stepper.push(o);
    }
    let crate_id = program.spec().dimension_id("crate").unwrap();
    assert_eq!(stepper.state().stack(crate_id), [0, 1]);
}

#[test]
fn identical_writes_count_once_in_one_effect_word() {
    let program = program(COMPATIBLE);
    let (results, _) = run(&program);
    // Two contracts wrote label.kind = Stamp at 0: one label effect, then one notify effect.
    let dims: Vec<u16> = results[0].effects.iter().map(|e| e.dimension.0).collect();
    assert_eq!(dims, [5, 6]);
    assert_eq!(results[0].committed.len(), 5);
}

#[test]
fn results_do_not_depend_on_declaration_order() {
    let forward = program(COMPATIBLE);
    let mut reversed_contracts = COMPATIBLE.to_vec();
    reversed_contracts.reverse();
    let reversed = program(&reversed_contracts);
    let mut rotated_contracts = COMPATIBLE.to_vec();
    rotated_contracts.rotate_left(4);
    let rotated = program(&rotated_contracts);
    let (f, fs) = run(&forward);
    for other in [&reversed, &rotated] {
        let (o, os) = run(other);
        assert_eq!(lines(&forward, &f), lines(other, &o));
        assert_eq!(fs, os, "the same committed words");
        for (a, b) in f.iter().zip(&o) {
            assert_eq!(a.effects, b.effects);
        }
    }
    // A conflict is named the same way whatever the order.
    let conflicting = |mut cs: Vec<ContractSpec>| {
        cs.push(HOLD_BOX);
        conflict_of(&report(&cs))
    };
    let mut reversed_with = COMPATIBLE.to_vec();
    reversed_with.reverse();
    assert_eq!(conflicting(COMPATIBLE.to_vec()), conflicting(reversed_with));
}

const HOLD_BOX: ContractSpec = contract("hold-box")
    .when(&[parcel(BOX), gate(DOCK)])
    .transition(&[set("bay", "status", 2)]);
const WARN_BOX: ContractSpec = contract("warn-box")
    .when(&[parcel(BOX)])
    .emit(&[set("label", "kind", 2)]);
const SEAL_CRATE: ContractSpec = contract("seal-crate")
    .when(&[parcel(FRAGILE), gate(SHELF)])
    .transition(&[set("crate", "content", 0)]);

fn conflict_of(report: &VerificationReport) -> Option<(String, String, String)> {
    report.findings.iter().find_map(|f| match f {
        Finding::Conflict {
            first,
            second,
            target,
            ..
        } => Some(((*first).to_owned(), (*second).to_owned(), target.clone())),
        _ => None,
    })
}

#[test]
fn contradictory_writes_are_refused() {
    let with = |extra: ContractSpec| {
        let mut cs = COMPATIBLE.to_vec();
        cs.push(extra);
        report(&cs)
    };
    // A state field: hold-box says Held, load says Loaded. Named in name order.
    let held = with(HOLD_BOX);
    assert!(!held.passed());
    assert_eq!(
        conflict_of(&held),
        Some(("hold-box".into(), "load".into(), "bay.status".into()))
    );
    let witness = held
        .findings
        .iter()
        .find_map(|f| match f {
            Finding::Conflict { witness, .. } => Some(witness.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        witness,
        "bay={status=Empty, care=Normal} tally={parity=Even} crate=[{content=Any}] evidence=parcel={kind=Box} gate={at=Dock} frame=none"
    );
    // An effect field. The first reachable meeting is a box with no gate:
    // stamp-again says Stamp, warn-box says Warn.
    assert_eq!(
        conflict_of(&with(WARN_BOX)),
        Some(("stamp-again".into(), "warn-box".into(), "label.kind".into()))
    );
    // A field write on a stack another contract pushes.
    assert_eq!(
        conflict_of(&with(SEAL_CRATE)),
        Some((
            "glass-crate".into(),
            "seal-crate".into(),
            "crate stack".into()
        ))
    );
    assert_eq!(
        held.findings
            .iter()
            .find(|f| matches!(f, Finding::Conflict { .. }))
            .map(liaise::Liaise::code),
        Some("BBBC0030".to_owned())
    );
}

/// Three contracts that meet only at (Box, Dock); each also holds alone or in a pair.
const FAN: &[ContractSpec] = &[
    contract("stamp-again")
        .when(&[parcel(BOX)])
        .emit(&[set("label", "kind", 1)]),
    contract("count-up")
        .when(&[gate(DOCK)])
        .require(&[tally(0)])
        .transition(&[set("tally", "parity", 1)]),
    contract("count-down")
        .when(&[gate(DOCK)])
        .require(&[tally(1)])
        .transition(&[set("tally", "parity", 0)]),
    contract("not-a-letter")
        .when(&[clause("parcel", FieldMatcher::not_eq("kind", LETTER))])
        .emit(&[set("notify", "kind", 2)]),
];

#[test]
fn the_fan_out_limit_is_enforced() {
    let two = ContainmentSpec {
        max_actualized: 2,
        ..CONTAIN
    };
    let program = verify(builder(FAN, two).build().unwrap(), LIMITS).expect("verifies at two");
    let mut stepper = Stepper::new(&program, NoObserver);
    let at_dock = stepper.push(Observation::new(0).with(PARCEL, BOX).with(GATE, DOCK));
    assert_eq!(
        at_dock[0].refusals,
        [Refusal::LimitExceeded {
            position: 0,
            limit: "actualized",
            observed: 3,
            permitted: 2
        }]
    );
    let at_shelf = stepper.push(Observation::new(1).with(PARCEL, BOX).with(GATE, SHELF));
    assert_eq!(at_shelf[0].committed.len(), 2);

    // An exactly-one policy is the same limit at one. The verifier agrees
    // with the stepper: stamp-again never holds alone, so it can never commit.
    let one = ContainmentSpec {
        max_actualized: 1,
        ..CONTAIN
    };
    let report = verify(builder(FAN, one).build().unwrap(), LIMITS).unwrap_err();
    assert!(
        report.findings.contains(&Finding::ImpossibleEffect {
            owner: "stamp-again"
        }),
        "{:?}",
        report.findings
    );
}
