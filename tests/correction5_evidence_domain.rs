//! Correction 5: the verifier's evidence domain follows the matchers, not
//! the labels. A legal evidence word with no label is still a word the
//! stepper accepts, so when a range, a mask or an intersection of clauses
//! holds only on unlabelled values, verification must try such a value.
//!
//! The data is a tank monitor, with no recognizer: `level` (a 4-bit reading
//! labelled only at 0, 6, 7, 8 and 15) and `alarm` (4 alarm bits labelled
//! only at 0 and 1) are evidence; `pump` is state.

use std::collections::BTreeSet;

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractId, ContractSpec, DimensionId, FactView, FieldMatcher,
    Finding, NoObserver, Observation, Outcome, Program, ProgramSpec, Refusal, RefusalRule, Stepper,
    VerificationReport, VerifiedProgram, VerifyLimits, clause, contract, dimension, evaluate, set,
    verify,
};

#[bitpack(storage = u8)]
pub struct Level {
    #[bits(4)]
    #[labels(Off = 0, Six = 6, Seven = 7, Eight = 8, Full = 15)]
    pub value: u8,
}

#[bitpack(storage = u8)]
pub struct Alarm {
    #[bits(4)]
    #[labels(Clear = 0, Ready = 1)]
    pub bits: u8,
}

#[bitpack(storage = u8)]
pub struct Pump {
    #[bits(2)]
    #[labels(Idle = 0, Run = 1, Hold = 2)]
    #[invalid(3)]
    pub state: u8,
}

const LEVEL: DimensionId = DimensionId(0);
const ALARM: DimensionId = DimensionId(1);

const OFF: u128 = Level::LABEL_VALUE_OFF.value;
const SIX: u128 = Level::LABEL_VALUE_SIX.value;
const SEVEN: u128 = Level::LABEL_VALUE_SEVEN.value;
const RUN: u128 = Pump::LABEL_STATE_RUN.value;
const HOLD: u128 = Pump::LABEL_STATE_HOLD.value;

const fn level(m: FieldMatcher) -> Clause {
    clause("level", m)
}

const fn alarm(m: FieldMatcher) -> Clause {
    clause("alarm", m)
}

/// Writes nothing: every state has an exit through `level = Off`.
const IDLE: ContractSpec = contract("idle").when(&[level(FieldMatcher::eq("value", OFF))]);

// ---- 1: two ranges overlapping only on unlabelled values ----

/// Holds on Six and Seven (labelled) and on 8..=11.
const LOW_BAND: ContractSpec = contract("low-band")
    .when(&[level(FieldMatcher::range("value", 6, 11))])
    .transition(&[set("pump", "state", RUN)]);

/// Holds on Full (labelled) and on 10..=14. Overlaps low-band on 10 and 11 only.
const HIGH_BAND: ContractSpec = contract("high-band")
    .when(&[level(FieldMatcher::range("value", 10, 15))])
    .transition(&[set("pump", "state", HOLD)]);

// ---- 2: masks holding only on unlabelled values ----

/// Both bits 1 and 2: 6, 7, 14, 15. No alarm label has them.
const ARMED: ContractSpec = contract("armed")
    .when(&[alarm(FieldMatcher::mask_all("bits", 0b0110))])
    .transition(&[set("pump", "state", RUN)]);

/// Bit 0 or bit 3: holds on Ready (labelled), and with armed on 7, 14, 15.
const FAULT: ContractSpec = contract("fault")
    .when(&[alarm(FieldMatcher::mask_any("bits", 0b1001))])
    .transition(&[set("pump", "state", HOLD)]);

// ---- 3: three contracts whose intersections no label witnesses ----

/// 4..=11 except the labelled Six and Seven: Eight (labelled), 4, 5, 9, 10, 11.
const WIDE: ContractSpec = contract("wide")
    .when(&[level(FieldMatcher::range("value", 4, 11))])
    .forbid(&[
        level(FieldMatcher::eq("value", SIX)),
        level(FieldMatcher::eq("value", SEVEN)),
    ])
    .transition(&[set("pump", "state", RUN)]);

/// Bit 1: Six, Seven and Full (labelled); with wide on 10 and 11.
const BIT: ContractSpec = contract("bit")
    .when(&[level(FieldMatcher::mask_all("value", 0b0010))])
    .transition(&[set("pump", "state", HOLD)]);

/// Odd and 9..=15: Full (labelled, agrees with bit); with wide on 9 and 11.
const ODD: ContractSpec = contract("odd")
    .when(&[level(FieldMatcher::range("value", 9, 15))])
    .require(&[level(FieldMatcher::mask_any("value", 0b0001))])
    .transition(&[set("pump", "state", HOLD)]);

// ---- 5: one contract pair over two evidence dimensions ----

/// Level 8..=11 and alarm bit 2 or 3.
const PAIR_LOW: ContractSpec = contract("pair-low")
    .when(&[
        level(FieldMatcher::range("value", 8, 11)),
        alarm(FieldMatcher::mask_any("bits", 0b1100)),
    ])
    .transition(&[set("pump", "state", RUN)]);

/// Level bits 1 and 3 (10, 11, 14, Full) and alarm 2..=14.
const PAIR_HIGH: ContractSpec = contract("pair-high")
    .when(&[
        level(FieldMatcher::mask_all("value", 0b1010)),
        alarm(FieldMatcher::range("bits", 2, 14)),
    ])
    .transition(&[set("pump", "state", HOLD)]);

const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 1,
    max_candidates: 16,
    max_actualized: 16,
    max_stack_depth: 1,
    on_refusal: RefusalRule::SkipObservation,
};

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 1_000,
    max_witness_tries: 1_024,
};

fn spec(contracts: &[ContractSpec]) -> ProgramSpec {
    contracts
        .iter()
        .fold(
            Program::new()
                .add(dimension!(Level, "level", Evidence))
                .add(dimension!(Alarm, "alarm", Evidence))
                .add(dimension!(Pump, "pump", State))
                .contain(CONTAIN),
            |b, c| b.contract(*c),
        )
        .build()
        .expect("tank program builds")
}

fn verified(contracts: &[ContractSpec]) -> VerifiedProgram {
    verify(spec(contracts), LIMITS).unwrap_or_else(|r| panic!("{:?}", r.findings))
}

fn refused(contracts: &[ContractSpec]) -> VerificationReport {
    verify(spec(contracts), LIMITS).expect_err("verification refuses")
}

/// Every conflict the verifier reports, as (first, second, target, witness).
fn conflicts(report: &VerificationReport) -> Vec<(&'static str, &'static str, String, String)> {
    report
        .findings
        .iter()
        .filter_map(|f| match f {
            Finding::Conflict {
                first,
                second,
                target,
                witness,
            } => Some((*first, *second, target.clone(), witness.clone())),
            _ => None,
        })
        .collect()
}

fn observe(words: &[(DimensionId, u128)]) -> Observation {
    words
        .iter()
        .fold(Observation::new(0), |o, (d, w)| o.with(*d, *w))
}

/// Every observation a schema-legal runtime input can be: each evidence
/// dimension absent or at any word flaga_solver accepts, labelled or not.
fn every_legal_observation(spec: &ProgramSpec) -> Vec<Observation> {
    let mut out = vec![Observation::new(0)];
    for d in [LEVEL, ALARM] {
        let schema = spec.dimension(d).unwrap().schema();
        let words: Vec<u128> = (0..=0b1111)
            .filter(|w| flaga_solver::check(schema, *w).is_ok_ignoring_deprecated())
            .collect();
        out = out
            .iter()
            .flat_map(|o| {
                std::iter::once(o.clone()).chain(words.iter().map(|w| o.clone().with(d, *w)))
            })
            .collect();
    }
    out
}

/// Each observation with only labelled words: what the verifier tried before.
fn labelled_observations(spec: &ProgramSpec) -> Vec<Observation> {
    every_legal_observation(spec)
        .into_iter()
        .filter(|o| {
            o.evidence().iter().all(|(d, w)| {
                let schema = spec.dimension(*d).unwrap().schema();
                schema
                    .fields
                    .iter()
                    .all(|f| schema.label_for(f.name, (w & f.mask) >> f.shift).is_some())
            })
        })
        .collect()
}

/// The same contracts with their writes removed: no write can conflict, so
/// the program verifies and exposes its domain, and every contract still
/// reads what it read.
fn readers_only(contracts: &[ContractSpec]) -> Vec<ContractSpec> {
    contracts.iter().map(|c| c.transition(&[])).collect()
}

#[test]
fn a_conflict_only_on_unlabelled_values_inside_overlapping_ranges_is_found() {
    let contracts = [IDLE, LOW_BAND, HIGH_BAND];
    let report = refused(&contracts);
    assert_eq!(
        conflicts(&report),
        [(
            "high-band",
            "low-band",
            "pump.state".to_owned(),
            "pump={state=Idle} evidence=level={value=10} frame=none".to_owned()
        )]
    );
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);

    // The pair is real: both are candidates on 10 and 11. No labelled level
    // is in both ranges, so the labelled words alone never showed it.
    let spec = spec(&contracts);
    let pair = BTreeSet::from([("high-band", "low-band", "pump.state".to_owned())]);
    assert_eq!(
        write_conflicts(&spec, &contracts, &[observe(&[(LEVEL, 11)])]),
        pair
    );
    assert!(write_conflicts(&spec, &contracts, &labelled_observations(&spec)).is_empty());
}

#[test]
fn an_unlabelled_mask_match_is_in_the_domain() {
    // armed holds only on alarm words with bits 1 and 2, none of them labelled.
    let program = verified(&[IDLE, ARMED]);
    assert!(program.report().passed());
    assert!(
        program.inputs().contains(&observe(&[(ALARM, 0b0110)])),
        "the least alarm word with both bits"
    );
    let stepper = Stepper::new(&program, NoObserver);
    assert!(stepper.legal_next().contains(&observe(&[(ALARM, 0b0110)])));
    // At run time 14 is armed too; the domain holds one word for that class.
    let mut stepper = Stepper::new(&program, NoObserver);
    let r = stepper.push(observe(&[(ALARM, 14)]));
    assert_eq!(r[0].committed, [ContractId(1)]);

    // With fault, armed and fault meet on 7, 14 and 15: the least is 7.
    let report = refused(&[IDLE, ARMED, FAULT]);
    assert_eq!(
        conflicts(&report),
        [(
            "armed",
            "fault",
            "pump.state".to_owned(),
            "pump={state=Idle} evidence=alarm={bits=7} frame=none".to_owned()
        )]
    );
}

#[test]
fn intersections_of_several_contracts_are_covered_without_a_label_there() {
    let contracts = [IDLE, WIDE, BIT, ODD];
    let report = refused(&contracts);
    // wide meets bit on 10 and 11, wide meets odd on 9 and 11; bit and odd agree.
    assert_eq!(
        conflicts(&report),
        [
            (
                "odd",
                "wide",
                "pump.state".to_owned(),
                "pump={state=Idle} evidence=level={value=9} frame=none".to_owned()
            ),
            (
                "bit",
                "wide",
                "pump.state".to_owned(),
                "pump={state=Idle} evidence=level={value=10} frame=none".to_owned()
            ),
        ]
    );
    assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
}

#[test]
fn labelled_words_keep_their_behaviour() {
    // Every class of these clauses holds a labelled value, so the domain is
    // the labelled words exactly, in the order they were tried before.
    const SIX_ONLY: ContractSpec = contract("six")
        .when(&[level(FieldMatcher::eq("value", SIX))])
        .transition(&[set("pump", "state", RUN)]);
    const FULL_ONLY: ContractSpec = contract("full")
        .when(&[level(FieldMatcher::eq("value", 15))])
        .transition(&[set("pump", "state", HOLD)]);
    let program = verified(&[IDLE, SIX_ONLY, FULL_ONLY]);
    let mut before = Vec::new();
    for l in [None, Some(0), Some(6), Some(7), Some(8), Some(15)] {
        for a in [None, Some(0), Some(1)] {
            let mut o = Observation::new(0);
            if let Some(l) = l {
                o = o.with(LEVEL, l);
            }
            if let Some(a) = a {
                o = o.with(ALARM, a);
            }
            before.push(o);
        }
    }
    assert_eq!(program.inputs(), before.as_slice());
    assert_eq!(program.report().findings, []);
    assert_eq!(program.report().states, 3);

    // Labelled words are in every domain, with or without unlabelled classes.
    let wider = verified(&readers_only(&[IDLE, LOW_BAND, HIGH_BAND, ARMED, FAULT]));
    for o in labelled_observations(wider.spec()) {
        assert!(wider.inputs().contains(&o), "{o:?}");
    }
}

#[test]
fn several_evidence_dimensions_stay_sound() {
    // pair-low and pair-high meet only on level 10 or 11 with an alarm
    // word in 4..=14 that has bit 2 or 3: unlabelled in both dimensions.
    let report = refused(&[IDLE, PAIR_LOW, PAIR_HIGH]);
    assert_eq!(
        conflicts(&report),
        [(
            "pair-high",
            "pair-low",
            "pump.state".to_owned(),
            "pump={state=Idle} evidence=level={value=10} alarm={bits=4} frame=none".to_owned()
        )]
    );
    // Either dimension absent: neither contract holds.
    let plain = verified(&readers_only(&[IDLE, PAIR_LOW, PAIR_HIGH]));
    assert!(plain.inputs().contains(&observe(&[(LEVEL, 10)])));
    assert!(plain.inputs().contains(&observe(&[(ALARM, 4)])));
    assert!(
        plain
            .inputs()
            .contains(&observe(&[(LEVEL, 10), (ALARM, 4)]))
    );
}

#[test]
fn runtime_exposes_no_conflict_the_verifier_did_not_report() {
    // A program that does not verify never reaches the stepper; its
    // conflicts are read from `evaluate`, which the stepper runs, over every
    // legal observation. These contracts read no state, so the start state
    // shows all of them.
    let refused_cases: &[&[ContractSpec]] = &[
        &[IDLE, LOW_BAND, HIGH_BAND],
        &[IDLE, ARMED, FAULT],
        &[IDLE, WIDE, BIT, ODD],
        &[IDLE, PAIR_LOW, PAIR_HIGH],
    ];
    for contracts in refused_cases {
        let spec = spec(contracts);
        let reported: BTreeSet<(&'static str, &'static str, String)> =
            conflicts(&refused(contracts))
                .into_iter()
                .map(|(a, b, t, _)| (a, b, t))
                .collect();
        let at_runtime = write_conflicts(&spec, contracts, &every_legal_observation(&spec));
        assert_eq!(at_runtime, reported, "{contracts:?}");
        // The labelled words alone show none of them.
        assert!(write_conflicts(&spec, contracts, &labelled_observations(&spec)).is_empty());
    }

    // A program that verifies is stepped for real: from every state one
    // observation reaches, every legal observation, and no `Conflict`.
    let verified_cases: &[&[ContractSpec]] = &[
        &[IDLE, ARMED],
        &[IDLE, LOW_BAND, ARMED],
        &[IDLE, BIT, ODD],
        &[IDLE, PAIR_LOW],
    ];
    for contracts in verified_cases {
        let program = verified(contracts);
        let every = every_legal_observation(program.spec());
        let mut prefixes: Vec<Option<Observation>> = vec![None];
        let mut states = vec![Stepper::new(&program, NoObserver).state().clone()];
        for o in &every {
            let mut stepper = Stepper::new(&program, NoObserver);
            let _ = stepper.push(o.clone());
            if !states.contains(stepper.state()) {
                states.push(stepper.state().clone());
                prefixes.push(Some(o.clone()));
            }
        }
        for prefix in &prefixes {
            for o in &every {
                let mut stepper = Stepper::new(&program, NoObserver);
                if let Some(p) = prefix {
                    let _ = stepper.push(p.clone());
                }
                let next = Observation::new(1);
                let next = o.evidence().iter().fold(next, |n, (d, w)| n.with(*d, *w));
                for r in stepper.push(next) {
                    assert!(
                        !r.refusals
                            .iter()
                            .any(|f| matches!(f, Refusal::Conflict { .. })),
                        "{contracts:?} after {prefix:?}: {:?}",
                        r.refusals
                    );
                }
            }
        }
    }
}

/// The pairs of contracts that are candidates on one of `observations` and
/// write different values to one field, as the stepper's `Conflict` names
/// them: (first by name, second, `dimension.field`).
fn write_conflicts(
    spec: &ProgramSpec,
    contracts: &[ContractSpec],
    observations: &[Observation],
) -> BTreeSet<(&'static str, &'static str, String)> {
    let mut found = BTreeSet::new();
    for o in observations {
        let on: Vec<&ContractSpec> = outcomes(spec, o)
            .iter()
            .zip(contracts)
            .filter(|(r, _)| r.as_str() == "+")
            .map(|(_, c)| c)
            .collect();
        for a in &on {
            for b in &on {
                for wa in a.transition {
                    for wb in b.transition {
                        if a.name < b.name
                            && wa.dimension == wb.dimension
                            && wa.value.field == wb.value.field
                            && wa.value.value != wb.value.value
                        {
                            found.insert((
                                a.name,
                                b.name,
                                format!("{}.{}", wa.dimension, wa.value.field),
                            ));
                        }
                    }
                }
            }
        }
    }
    found
}

/// What each contract does with one observation: not applicable, refused by
/// a named clause kind, or a candidate.
fn outcomes(spec: &ProgramSpec, o: &Observation) -> Vec<String> {
    let words: Vec<Option<u128>> = (0..3u16)
        .map(|i| match i {
            2 => Some(0),
            d => o.word(DimensionId(d)),
        })
        .collect();
    let view = FactView::new(spec, &words, 0, None);
    (0..spec_len(spec))
        .map(|c| match evaluate(ContractId(c), &view) {
            Outcome::NotApplicable => "-".to_owned(),
            Outcome::Candidate(_) => "+".to_owned(),
            Outcome::Refused(Refusal::ContractRefused { clause, .. }) => clause.to_owned(),
            Outcome::Refused(other) => other.to_string(),
        })
        .collect()
}

fn spec_len(spec: &ProgramSpec) -> u16 {
    (0..)
        .take_while(|c| spec.contract(ContractId(*c)).is_some())
        .count() as u16
}

#[test]
fn the_domain_gives_every_contract_outcome_a_legal_observation_gives() {
    // Every contract above, reading only: ranges, masks, equality, forbid
    // and require, on two evidence dimensions.
    let program = verified(&readers_only(&[
        IDLE, LOW_BAND, HIGH_BAND, ARMED, FAULT, WIDE, BIT, ODD, PAIR_LOW, PAIR_HIGH,
    ]));
    let spec = program.spec();
    let brute: BTreeSet<Vec<String>> = every_legal_observation(spec)
        .iter()
        .map(|o| outcomes(spec, o))
        .collect();
    let modelled: BTreeSet<Vec<String>> =
        program.inputs().iter().map(|o| outcomes(spec, o)).collect();
    assert_eq!(brute, modelled);
    // The labelled words alone miss some of them.
    let labelled: BTreeSet<Vec<String>> = labelled_observations(spec)
        .iter()
        .map(|o| outcomes(spec, o))
        .collect();
    assert!(labelled.len() < brute.len());
}
