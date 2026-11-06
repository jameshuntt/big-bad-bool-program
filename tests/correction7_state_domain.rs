//! Correction 7: a contract's state clauses are checked for a witness over
//! the same matcher classes the evidence domain uses, not over a list of
//! guessed values. A legal state word that meets a contract's positive
//! clauses and none of its FORBID clauses is found whatever its value, so a
//! satisfiable contract is never reported `Unsatisfiable`.
//!
//! The data is a gauge, with no recognizer: `cmd` is evidence, `gauge` is
//! state with a reading (`level`, labelled only at 0), four unlabelled
//! `bits`, a `mode` whose `High` requires the `guard` flag `armed`, and that
//! flag. Writers set unlabelled values; each reader holds only on them.
//!
//! The previous check (`field_candidates` / `state_witnesses`) tried zero,
//! the maximum, the default, the labels, all labels together and each clause
//! constant with its neighbours, in the named fields only, every other field
//! at its default. On the readers below it found no witness and reported
//! `Unsatisfiable { dimension: "gauge", field: "(joint)" }` for `read-range`,
//! `read-mask`, `read-high` and `read-joint`; each test names what it missed.

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, Finding, NoObserver,
    Observation, Program, ProgramSpec, RefusalRule, Stepper, VerificationReport, VerifiedProgram,
    VerifyLimits, clause, contract, dimension, set, verify,
};

#[bitpack(storage = u8)]
pub struct Cmd {
    #[bits(4)]
    #[labels(
        Reset = 0,
        SetSix = 1,
        SetBits = 2,
        Arm = 3,
        ReadRange = 4,
        ReadMask = 5,
        ReadHigh = 6,
        ReadJoint = 7
    )]
    pub op: u8,
}

#[bitpack(storage = u16)]
pub struct Gauge {
    #[bits(4)]
    #[labels(Zero = 0)]
    pub level: u16,
    #[bits(4)]
    pub bits: u16,
    #[bits(2)]
    #[labels(Off = 0, High = 2)]
    #[requires(High => guard.armed)]
    pub mode: u16,
    #[bits(1)]
    #[labels(armed = 1)]
    #[flags(armed = 0)]
    pub guard: u16,
}

const CMD: DimensionId = DimensionId(0);

const HIGH: u128 = Gauge::LABEL_MODE_HIGH.value;
const ARMED: u128 = Gauge::LABEL_GUARD_ARMED.value;

const fn cmd(op: u128) -> Clause {
    clause("cmd", FieldMatcher::eq("op", op))
}

const fn gauge(m: FieldMatcher) -> Clause {
    clause("gauge", m)
}

/// Every state has this exit.
const RESET: ContractSpec = contract("reset")
    .when(&[cmd(Cmd::LABEL_OP_RESET.value)])
    .transition(&[
        set("gauge", "level", 0),
        set("gauge", "bits", 0),
        set("gauge", "mode", 0),
        set("gauge", "guard", 0),
    ]);

const SET_SIX: ContractSpec = contract("set-six")
    .when(&[cmd(Cmd::LABEL_OP_SETSIX.value)])
    .transition(&[set("gauge", "level", 6)]);

const SET_BITS: ContractSpec = contract("set-bits")
    .when(&[cmd(Cmd::LABEL_OP_SETBITS.value)])
    .transition(&[set("gauge", "bits", 0b0110)]);

const ARM: ContractSpec = contract("arm")
    .when(&[cmd(Cmd::LABEL_OP_ARM.value)])
    .transition(&[set("gauge", "mode", HIGH), set("gauge", "guard", ARMED)]);

/// Level 4..=11, even, not 4 or 8: only 6 and 10, neither labelled. The old
/// candidates were 0, 15, 1, 3, 4, 5, 7, 8, 9, 11 and 12.
const READ_RANGE: ContractSpec = contract("read-range")
    .when(&[cmd(Cmd::LABEL_OP_READRANGE.value)])
    .require(&[gauge(FieldMatcher::range("level", 4, 11))])
    .forbid(&[
        gauge(FieldMatcher::mask_any("level", 0b0001)),
        gauge(FieldMatcher::eq("level", 4)),
        gauge(FieldMatcher::eq("level", 8)),
    ]);

/// Bits 1 and 2 set, bits 0 and 3 clear: only 6. The old candidates were
/// 0, 15, 4, 2 and 9.
const READ_MASK: ContractSpec = contract("read-mask")
    .when(&[cmd(Cmd::LABEL_OP_READMASK.value)])
    .require(&[
        gauge(FieldMatcher::mask_all("bits", 0b0100)),
        gauge(FieldMatcher::mask_all("bits", 0b0010)),
    ])
    .forbid(&[gauge(FieldMatcher::mask_any("bits", 0b1001))]);

/// `mode = High` is legal only with `guard.armed`, a field the clause does
/// not name. The old check left `guard` at its default, where `High` is
/// illegal.
const READ_HIGH: ContractSpec = contract("read-high")
    .when(&[cmd(Cmd::LABEL_OP_READHIGH.value)])
    .require(&[gauge(FieldMatcher::eq("mode", HIGH))]);

/// Three fields at once, and a fourth through `requires`: level 5..=7, bits
/// 1 and 2 without bit 3, mode High (so guard armed).
const READ_JOINT: ContractSpec = contract("read-joint")
    .when(&[cmd(Cmd::LABEL_OP_READJOINT.value)])
    .require(&[
        gauge(FieldMatcher::range("level", 5, 7)),
        gauge(FieldMatcher::mask_all("bits", 0b0110)),
        gauge(FieldMatcher::eq("mode", HIGH)),
    ])
    .forbid(&[gauge(FieldMatcher::mask_any("bits", 0b1000))]);

const WRITERS: [ContractSpec; 4] = [RESET, SET_SIX, SET_BITS, ARM];

const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 1,
    max_candidates: 4,
    max_actualized: 4,
    max_stack_depth: 1,
    on_refusal: RefusalRule::SkipObservation,
};

const LIMITS: VerifyLimits = VerifyLimits {
    max_states: 1_000,
    max_witness_tries: 1_024,
};

fn spec(readers: &[ContractSpec]) -> ProgramSpec {
    WRITERS
        .iter()
        .chain(readers)
        .fold(
            Program::new()
                .add(dimension!(Cmd, "cmd", Evidence))
                .add(dimension!(Gauge, "gauge", State))
                .contain(CONTAIN),
            |b, c| b.contract(*c),
        )
        .build()
        .expect("gauge program builds")
}

fn verified(readers: &[ContractSpec]) -> VerifiedProgram {
    verify(spec(readers), LIMITS).unwrap_or_else(|r| panic!("{:?}", r.findings))
}

fn unsatisfiable(report: &VerificationReport) -> Vec<(&'static str, &'static str, &'static str)> {
    report
        .findings
        .iter()
        .filter_map(|f| match f {
            Finding::Unsatisfiable {
                owner,
                dimension,
                field,
            } => Some((*owner, *dimension, *field)),
            _ => None,
        })
        .collect()
}

/// Steps `ops` from the start and returns the contract names that committed at the last one.
fn last_committed(program: &VerifiedProgram, ops: &[u128]) -> Vec<&'static str> {
    let mut stepper = Stepper::new(program, NoObserver);
    let mut last = Vec::new();
    for (position, op) in (0..).zip(ops) {
        last = stepper.push(Observation::new(position).with(CMD, *op));
    }
    last[0]
        .committed
        .iter()
        .map(|id| program.spec().contract(*id).unwrap().name)
        .collect()
}

/// The reader verifies, and the stepper commits it on the unlabelled state the writers make.
fn verifies_and_commits(reader: ContractSpec, ops: &[u128]) {
    let program = verified(&[reader]);
    assert!(program.report().passed(), "{:?}", program.report().findings);
    assert!(unsatisfiable(program.report()).is_empty());
    assert_eq!(last_committed(&program, ops), [reader.name]);
}

#[test]
fn a_state_range_whose_witness_is_unlabelled_is_satisfiable() {
    // Witness: level 6 (and 10). Old: Unsatisfiable (joint).
    verifies_and_commits(
        READ_RANGE,
        &[Cmd::LABEL_OP_SETSIX.value, Cmd::LABEL_OP_READRANGE.value],
    );
}

#[test]
fn a_state_mask_witness_is_found() {
    // Witness: bits 6. Old: Unsatisfiable (joint).
    verifies_and_commits(
        READ_MASK,
        &[Cmd::LABEL_OP_SETBITS.value, Cmd::LABEL_OP_READMASK.value],
    );
}

#[test]
fn positive_clauses_with_forbid_on_the_same_field_keep_their_witness() {
    // Without its FORBID clauses read-range has the labelled-adjacent 4;
    // with them the only witnesses are the unlabelled 6 and 10. Both verify.
    const LOOSE: ContractSpec = contract("read-range")
        .when(&[cmd(Cmd::LABEL_OP_READRANGE.value)])
        .require(&[gauge(FieldMatcher::range("level", 4, 11))]);
    let program = verified(&[LOOSE]);
    assert!(unsatisfiable(program.report()).is_empty());
    let program = verified(&[READ_RANGE]);
    assert!(unsatisfiable(program.report()).is_empty());
    // At run time the FORBID clauses decide: level 0 refuses, level 6 commits.
    assert!(last_committed(&program, &[Cmd::LABEL_OP_READRANGE.value]).is_empty());
}

#[test]
fn a_legality_rule_on_an_unnamed_field_does_not_hide_the_witness() {
    // Witness: mode High with guard armed. Old: Unsatisfiable (joint),
    // because guard stayed at its default and High alone is illegal.
    verifies_and_commits(
        READ_HIGH,
        &[Cmd::LABEL_OP_ARM.value, Cmd::LABEL_OP_READHIGH.value],
    );
}

#[test]
fn intersections_across_several_state_fields_are_found() {
    // Witness: level 6, bits 6, mode High, guard armed. Old: Unsatisfiable (joint).
    verifies_and_commits(
        READ_JOINT,
        &[
            Cmd::LABEL_OP_SETSIX.value,
            Cmd::LABEL_OP_SETBITS.value,
            Cmd::LABEL_OP_ARM.value,
            Cmd::LABEL_OP_READJOINT.value,
        ],
    );
    // All four readers together.
    let program = verified(&[READ_RANGE, READ_MASK, READ_HIGH, READ_JOINT]);
    assert!(program.report().passed(), "{:?}", program.report().findings);
}

#[test]
fn contradictory_state_clauses_are_still_unsatisfiable() {
    // No level in 4..=5 outside 3..=6.
    const NEVER_RANGE: ContractSpec = contract("never-range")
        .when(&[cmd(Cmd::LABEL_OP_READRANGE.value)])
        .require(&[gauge(FieldMatcher::range("level", 4, 5))])
        .forbid(&[gauge(FieldMatcher::range("level", 3, 6))]);
    // Two values for one field: the field is named.
    const NEVER_FIELD: ContractSpec = contract("never-field")
        .when(&[cmd(Cmd::LABEL_OP_READMASK.value)])
        .require(&[
            gauge(FieldMatcher::eq("level", 3)),
            gauge(FieldMatcher::eq("level", 5)),
        ]);
    // High needs armed, and armed is forbidden: only legality rules it out.
    const NEVER_LEGAL: ContractSpec = contract("never-legal")
        .when(&[cmd(Cmd::LABEL_OP_READHIGH.value)])
        .require(&[gauge(FieldMatcher::eq("mode", HIGH))])
        .forbid(&[gauge(FieldMatcher::eq("guard", ARMED))]);
    let report = verify(spec(&[NEVER_RANGE, NEVER_FIELD, NEVER_LEGAL]), LIMITS).unwrap_err();
    assert_eq!(
        unsatisfiable(&report),
        [
            ("never-range", "gauge", "(joint)"),
            ("never-field", "gauge", "level"),
            ("never-legal", "gauge", "(joint)"),
        ]
    );
}

#[test]
fn a_state_domain_past_the_bound_is_too_many_states_not_unsatisfiable() {
    // cmd's domain is its 8 labels and one unlabelled class (9 words, 10
    // inputs); read-joint's gauge domain is 3 level classes x 4 bits classes
    // x 2 mode classes x 2 guard classes = 48 words.
    let limits = VerifyLimits {
        max_states: 1_000,
        max_witness_tries: 16,
    };
    let report = verify(spec(&[READ_JOINT]), limits).unwrap_err();
    assert!(unsatisfiable(&report).is_empty(), "{:?}", report.findings);
    assert!(report.findings.contains(&Finding::TooManyStates {
        explored: 17,
        permitted: 16,
    }));
    // Within the bound the same program verifies.
    let limits = VerifyLimits {
        max_states: 1_000,
        max_witness_tries: 48,
    };
    assert!(verify(spec(&[READ_JOINT]), limits).is_ok());
}
