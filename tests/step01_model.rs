//! Step 1: dimensions resolve by name; bad clauses are refused with liaise
//! codes, one for one with interact's `validate_rule_table`.

mod common;

use big_bad_bool_contract::{
    ContractSpec, DimensionId, DimensionRole, FieldMatcher, Finding, Liaise, Program,
    ProgramBuilder, RecognizerSpec, clause, contract, dimension, pop, push, set,
};
use big_bad_bool_interact::{Action, Rule, RuleTable, RuleValidationFinding, validate_rule_table};
use common::{CONTAINMENT, FrameWord, StateWord, TokenWord};

#[test]
fn five_dimension_program_builds() {
    let spec = common::program().build().expect("section 16 builds");
    assert_eq!(spec.dimensions().len(), 5);
    let names: Vec<&str> = spec.dimensions().iter().map(|d| d.name()).collect();
    assert_eq!(names, ["token", "frame", "context", "state", "effect"]);
    assert_eq!(spec.dimension_id("context"), Some(DimensionId(2)));
    assert_eq!(spec.dimension_id("nothing"), None);
    let context = spec.dimension(DimensionId(2)).unwrap();
    assert_eq!(context.role(), DimensionRole::State);
    assert_eq!(context.stack_depth(), Some(32));
    assert_eq!(spec.item_dimension(), Some(DimensionId(0)));
    assert_eq!(spec.frame_dimension(), Some(DimensionId(1)));
    assert_eq!(spec.contracts().len(), 15);
    assert_eq!(spec.contract_id("callee-reference").map(|c| c.0), Some(11));
    assert_eq!(spec.sequence_id("CallWithArg").map(|s| s.0), Some(4));
    assert_eq!(spec.window_radius(), 3);
    assert_eq!(spec.end_word(), Some(common::END));
    let state = spec.dimension(DimensionId(3)).unwrap();
    assert_eq!(state.default_word(), 0b1000);
    assert_eq!(
        state.describe(state.default_word()),
        "{phase=Items, marks=can_declare}"
    );
}

const EMPTY: &[FieldMatcher] = &[];
const UNKNOWN: &[FieldMatcher] = &[FieldMatcher::eq("knd", 1)];
const EQ_OUT: &[FieldMatcher] = &[FieldMatcher::eq("kind", 16)];
const NOT_EQ_OUT: &[FieldMatcher] = &[FieldMatcher::not_eq("kind", 99)];
const RANGE_EMPTY: &[FieldMatcher] = &[FieldMatcher::range("kind", 5, 2)];
const RANGE_OUT: &[FieldMatcher] = &[FieldMatcher::range("kind", 2, 20)];
const MASK_ALL_OUT: &[FieldMatcher] = &[FieldMatcher::mask_all("kind", 0x20)];
const MASK_ANY_OUT: &[FieldMatcher] = &[FieldMatcher::mask_any("kind", 0x10)];
const MIXED: &[FieldMatcher] = &[
    FieldMatcher::eq("kind", 2),
    FieldMatcher::any("kind"),
    FieldMatcher::range("kind", 0, 15),
    FieldMatcher::eq("other", 0),
    FieldMatcher::mask_any("kind", 0xf),
];

const CASES: &[(&str, &[FieldMatcher])] = &[
    ("empty", EMPTY),
    ("unknown", UNKNOWN),
    ("eq-out", EQ_OUT),
    ("not-eq-out", NOT_EQ_OUT),
    ("range-empty", RANGE_EMPTY),
    ("range-out", RANGE_OUT),
    ("mask-all-out", MASK_ALL_OUT),
    ("mask-any-out", MASK_ANY_OUT),
    ("mixed", MIXED),
];

const ACTION: Action = Action::new(0, "none", "");
static RULES: [Rule; 9] = [
    Rule::new(CASES[0].0, CASES[0].1, ACTION),
    Rule::new(CASES[1].0, CASES[1].1, ACTION),
    Rule::new(CASES[2].0, CASES[2].1, ACTION),
    Rule::new(CASES[3].0, CASES[3].1, ACTION),
    Rule::new(CASES[4].0, CASES[4].1, ACTION),
    Rule::new(CASES[5].0, CASES[5].1, ACTION),
    Rule::new(CASES[6].0, CASES[6].1, ACTION),
    Rule::new(CASES[7].0, CASES[7].1, ACTION),
    Rule::new(CASES[8].0, CASES[8].1, ACTION),
];

/// Token and frame only: the smallest program the recognizer accepts.
fn base() -> ProgramBuilder {
    Program::new()
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(FrameWord, "frame", Evidence))
        .add(dimension!(StateWord, "state", State))
        .recognize(RecognizerSpec {
            sequences: &[],
            ..common::RECOGNIZER
        })
        .contain(CONTAINMENT)
}

fn leak_clauses(matchers: &[FieldMatcher]) -> &'static [big_bad_bool_contract::Clause] {
    let clauses: Vec<_> = matchers.iter().map(|m| clause("token", *m)).collect();
    Box::leak(clauses.into_boxed_slice())
}

#[test]
fn bad_clauses_match_interact_one_for_one() {
    let builder = CASES.iter().fold(base(), |b, (name, matchers)| {
        b.contract(contract(name).when(leak_clauses(matchers)))
    });
    let ours = builder.build().expect_err("bad clauses are refused");

    let theirs = validate_rule_table::<TokenWord>(RuleTable::new("token", &RULES));
    assert!(!theirs.accepted);
    let mapped: Vec<Finding> = theirs
        .findings
        .iter()
        .map(|f| match *f {
            RuleValidationFinding::EmptyRule { rule } => Finding::EmptyWhen { owner: rule },
            RuleValidationFinding::UnknownField { rule, field } => Finding::UnknownField {
                owner: rule,
                dimension: "token",
                field,
            },
            RuleValidationFinding::ValueOutOfRange {
                rule,
                field,
                value,
                max,
            } => Finding::ValueOutOfRange {
                owner: rule,
                dimension: "token",
                field,
                value,
                max,
            },
            RuleValidationFinding::InvalidRange {
                rule,
                field,
                min,
                max,
            } => Finding::InvalidRange {
                owner: rule,
                dimension: "token",
                field,
                min,
                max,
            },
            RuleValidationFinding::RangeOutOfRange {
                rule,
                field,
                min,
                max,
                field_max,
            } => Finding::RangeOutOfRange {
                owner: rule,
                dimension: "token",
                field,
                min,
                max,
                field_max,
            },
            RuleValidationFinding::MaskOutOfRange {
                rule,
                field,
                mask,
                field_max,
            } => Finding::MaskOutOfRange {
                owner: rule,
                dimension: "token",
                field,
                mask,
                field_max,
            },
        })
        .collect();
    assert_eq!(ours, mapped);
    assert_eq!(ours.len(), 9);

    let codes: Vec<String> = ours.iter().map(Liaise::code).collect();
    assert_eq!(
        codes,
        [
            "BBBC0001", "BBBC0003", "BBBC0004", "BBBC0004", "BBBC0005", "BBBC0006", "BBBC0007",
            "BBBC0007", "BBBC0003"
        ]
    );
    assert_eq!(
        ours[2].to_string(),
        "[BBBC0004] `eq-out`: `token.kind` value 16 exceeds max 15"
    );
    let report = ours[2].liaise_report();
    assert_eq!(report.evidence_value("owner"), Some("eq-out"));
    assert_eq!(report.evidence_value("max"), Some("15"));
}

#[test]
fn unknown_dimension_is_refused() {
    const BAD: ContractSpec = contract("bad").when(&[clause("tokn", FieldMatcher::eq("kind", 1))]);
    let found = base().contract(BAD).build().unwrap_err();
    assert_eq!(
        found,
        [Finding::UnknownDimension {
            owner: "bad",
            dimension: "tokn"
        }]
    );
    assert_eq!(found[0].code(), "BBBC0002");
}

#[test]
fn roles_decide_who_writes() {
    const WHEN: &[big_bad_bool_contract::Clause] = &[clause("token", FieldMatcher::eq("kind", 1))];
    const WRITES_EVIDENCE: ContractSpec = contract("writes-evidence")
        .when(WHEN)
        .transition(&[set("token", "kind", 2)]);
    const EMITS_STATE: ContractSpec = contract("emits-state")
        .when(WHEN)
        .emit(&[set("state", "phase", 1)]);
    const PUSHES_FLAT: ContractSpec = contract("pushes-flat")
        .when(WHEN)
        .stack(&[push("state", &[]), pop("state")]);
    const READS_EFFECT: ContractSpec =
        contract("reads-effect").when(&[clause("effect", FieldMatcher::eq("kind", 1))]);
    const WRITES_EFFECT: ContractSpec = contract("writes-effect")
        .when(WHEN)
        .transition(&[set("effect", "kind", 1)]);
    let found = base()
        .add(dimension!(common::EffectWord, "effect", Effect))
        .contract(WRITES_EVIDENCE)
        .contract(EMITS_STATE)
        .contract(PUSHES_FLAT)
        .contract(READS_EFFECT)
        .contract(WRITES_EFFECT)
        .build()
        .unwrap_err();
    assert_eq!(
        found,
        [
            Finding::ForbiddenWrite {
                owner: "writes-evidence",
                dimension: "token",
                rule: "transition writes only a state dimension"
            },
            Finding::ForbiddenWrite {
                owner: "emits-state",
                dimension: "state",
                rule: "emit writes only an effect dimension"
            },
            Finding::ForbiddenWrite {
                owner: "pushes-flat",
                dimension: "state",
                rule: "stack ops need a stacked state dimension"
            },
            Finding::ForbiddenWrite {
                owner: "pushes-flat",
                dimension: "state",
                rule: "stack ops need a stacked state dimension"
            },
            Finding::ForbiddenRead {
                owner: "reads-effect",
                dimension: "effect"
            },
            Finding::ForbiddenWrite {
                owner: "writes-effect",
                dimension: "effect",
                rule: "transition writes only a state dimension"
            },
        ]
    );
}

#[test]
fn recognizer_must_read_and_write_evidence() {
    let found = Program::new()
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(FrameWord, "frame", State))
        .recognize(common::RECOGNIZER)
        .contain(CONTAINMENT)
        .build()
        .unwrap_err();
    assert_eq!(
        found[0],
        Finding::ForbiddenWrite {
            owner: "recognizer",
            dimension: "frame",
            rule: "the recognizer reads and writes only evidence dimensions"
        }
    );
}

#[test]
fn write_values_must_fit() {
    const TOO_BIG: ContractSpec = contract("too-big")
        .when(&[clause("token", FieldMatcher::eq("kind", 1))])
        .transition(&[set("state", "phase", 8), set("state", "nope", 0)]);
    let found = base().contract(TOO_BIG).build().unwrap_err();
    assert_eq!(
        found,
        [
            Finding::ValueOutOfRange {
                owner: "too-big",
                dimension: "state",
                field: "phase",
                value: 8,
                max: 7
            },
            Finding::UnknownField {
                owner: "too-big",
                dimension: "state",
                field: "nope"
            },
        ]
    );
}

#[test]
fn missing_parts_duplicates_and_stack_bounds() {
    let found = Program::new()
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(StateWord, "state", Evidence).stacked(4))
        .build()
        .unwrap_err();
    assert_eq!(
        found,
        [
            Finding::DuplicateName {
                kind: "dimension",
                name: "token"
            },
            Finding::Missing {
                part: "containment"
            },
            Finding::ForbiddenWrite {
                owner: "state",
                dimension: "state",
                rule: "only a state dimension can be stacked"
            },
            // No `Missing { part: "recognizer" }`: a recognizer is optional (correction 1).
        ]
    );

    let found = base()
        .add(dimension!(common::ContextWord, "context", State).stacked(33))
        .build()
        .unwrap_err();
    assert_eq!(
        found,
        [Finding::StackBound {
            dimension: "context",
            depth: 33,
            permitted: 32
        }]
    );
}

#[test]
fn sequence_checks() {
    use big_bad_bool_contract::{ItemMatch, OffsetFacts, SequenceSpec, sequence};
    const EMPTY_SEQ: SequenceSpec = sequence("empty", &[]);
    const LONG: SequenceSpec = sequence("long", &[ItemMatch::Any; 5]);
    const BAD_FACT: SequenceSpec = sequence("bad-fact", &[ItemMatch::Any]).facts(&[OffsetFacts {
        offset: 1,
        values: &[],
    }]);
    let found = Program::new()
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(FrameWord, "frame", Evidence))
        .recognize(RecognizerSpec {
            sequences: &[EMPTY_SEQ, LONG, BAD_FACT, EMPTY_SEQ],
            ..common::RECOGNIZER
        })
        .contain(CONTAINMENT)
        .build()
        .unwrap_err();
    assert_eq!(
        found,
        [
            Finding::EmptySequence { owner: "empty" },
            Finding::ValueOutOfRange {
                owner: "long",
                dimension: "frame",
                field: "offset",
                value: 4,
                max: 3
            },
            Finding::ValueOutOfRange {
                owner: "bad-fact",
                dimension: "frame",
                field: "offset",
                value: 1,
                max: 0
            },
            Finding::DuplicateName {
                kind: "sequence",
                name: "empty"
            },
            Finding::EmptySequence { owner: "empty" },
        ]
    );
}
