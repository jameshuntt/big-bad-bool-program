//! Step 8: every coherence finding is found, each from one deliberately
//! broken spec with its expected code and witness. Section 16 passes.

mod common;

use big_bad_bool_contract::{
    ContractSpec, FieldMatcher, FieldValue, Finding, ItemMatch, Liaise, OffsetFacts,
    ProgramBuilder, RecognizerSpec, SequenceSpec, VerificationReport, VerifyLimits, clause,
    contract, sequence, set, verify,
};

fn report(builder: ProgramBuilder) -> VerificationReport {
    match verify(builder.build().expect("builds"), common::LIMITS) {
        Ok(program) => program.report().clone(),
        Err(report) => report,
    }
}

fn with(contracts: &[ContractSpec], sequences: &'static [SequenceSpec]) -> ProgramBuilder {
    contracts
        .iter()
        .fold(
            common::dimensions(big_bad_bool_contract::Program::new()).recognize(RecognizerSpec {
                sequences,
                ..common::RECOGNIZER
            }),
            |b, c| b.contract(*c),
        )
        .contain(common::CONTAINMENT)
}

fn codes(report: &VerificationReport) -> Vec<String> {
    report.findings.iter().map(Liaise::code).collect()
}

#[test]
fn section_16_passes_with_info_only() {
    let program =
        verify(common::program().build().unwrap(), common::LIMITS).expect("section 16 verifies");
    let report = program.report();
    assert!(report.passed());
    assert_eq!(
        report.findings,
        [
            Finding::ContainedSequence {
                inner: "FnHead",
                outer: "AsyncFnHead",
                at: 1
            },
            Finding::Unreachable {
                dimension: "context",
                field: "marks",
                label: "const_expression"
            },
        ]
    );
    println!("states {}", report.states);
}

const fn token(kind: u128) -> big_bad_bool_contract::Clause {
    clause("token", FieldMatcher::eq("kind", kind))
}

#[test]
fn unsatisfiable() {
    const CONTRADICT: ContractSpec = contract("contradict")
        .when(&[token(common::IDENT)])
        .require(&[token(common::LPAREN)]);
    const JOINT: ContractSpec = contract("joint")
        .when(&[token(common::IDENT)])
        .require(&[clause("context", FieldMatcher::eq("expecting", 0))])
        .forbid(&[clause("context", FieldMatcher::eq("expecting", 0))]);
    let mut contracts = common::CONTRACTS.to_vec();
    contracts.extend([CONTRADICT, JOINT]);
    let report = report(with(&contracts, common::SEQUENCES));
    assert!(!report.passed());
    assert!(report.findings.contains(&Finding::Unsatisfiable {
        owner: "contradict",
        dimension: "token",
        field: "kind"
    }));
    assert!(report.findings.contains(&Finding::Unsatisfiable {
        owner: "joint",
        dimension: "context",
        field: "(joint)"
    }));
    assert!(codes(&report).contains(&"BBBC0020".to_owned()));
}

#[test]
fn ambiguous_k1_k2_pair() {
    // K2 without `frame.position = Interior`: plain `fn` matches K1 and K2.
    const K2_LOOSE: ContractSpec = contract("fn-keyword-after-modifier")
        .when(&[token(common::KW_FN)])
        .require(&[clause("state", FieldMatcher::eq("phase", 1))]);
    let mut contracts = common::CONTRACTS.to_vec();
    contracts[1] = K2_LOOSE;
    let report = report(with(&contracts, common::SEQUENCES));
    let ambiguous: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|f| matches!(f, Finding::Ambiguous { .. }))
        .collect();
    assert_eq!(
        ambiguous,
        [&Finding::Ambiguous {
            first: "fn-head-begin",
            second: "fn-keyword-after-modifier",
            witness: "context=[{scope=Module, expecting=Item, marks=none}] state={phase=FnHead, marks=can_declare} item={kind=KwFn} frame={sequence=FnHead, offset=0, position=Begin, marks=none, construct=FunctionDeclaration, role=None, modifier=None}".to_owned(),
        }]
    );
    assert_eq!(ambiguous[0].code(), "BBBC0021");
}

#[test]
fn no_exit() {
    // Without param-colon nothing leaves `x` in `fn f(x`.
    let contracts: Vec<ContractSpec> = common::CONTRACTS
        .iter()
        .copied()
        .filter(|c| c.name != "param-colon")
        .collect();
    let report = report(with(&contracts, common::SEQUENCES));
    assert_eq!(
        report.findings.iter().find(|f| matches!(f, Finding::NoExit { .. })),
        Some(&Finding::NoExit {
            witness: "context=[{scope=Module, expecting=Item, marks=none}, {scope=FnParams, expecting=AfterPattern, marks=none}] state={phase=Params, marks=can_declare}".to_owned()
        })
    );
    assert!(report.findings.contains(&Finding::ImpossibleEffect {
        owner: "type-reference"
    }));
}

#[test]
fn dead_contract_and_impossible_effect() {
    // phase FnHead only happens at module scope, never inside a body.
    const NEVER: ContractSpec = contract("never").when(&[token(common::RBRACE)]).require(&[
        clause("state", FieldMatcher::eq("phase", 1)),
        clause("context", FieldMatcher::eq("scope", 2)),
    ]);
    const NEVER_EMITS: ContractSpec = NEVER.emit(&[set("effect", "kind", 1)]);
    let mut contracts = common::CONTRACTS.to_vec();
    contracts.extend([
        NEVER,
        ContractSpec {
            name: "never-emits",
            ..NEVER_EMITS
        },
    ]);
    let report = report(with(&contracts, common::SEQUENCES));
    assert!(!report.passed());
    let dead: Vec<(String, &Finding)> = report
        .findings
        .iter()
        .filter(|f| {
            matches!(
                f,
                Finding::DeadContract { .. } | Finding::ImpossibleEffect { .. }
            )
        })
        .map(|f| (f.code(), f))
        .collect();
    assert_eq!(
        dead,
        [
            (
                "BBBC0024".to_owned(),
                &Finding::DeadContract { owner: "never" }
            ),
            (
                "BBBC0025".to_owned(),
                &Finding::ImpossibleEffect {
                    owner: "never-emits"
                }
            ),
        ]
    );
}

const fn kind(value: u128) -> ItemMatch {
    ItemMatch::One(FieldMatcher::eq("kind", value))
}

const S6_BRACE_IDENT: SequenceSpec =
    sequence("BraceIdent", &[kind(common::LBRACE), kind(common::IDENT)]).facts(&[OffsetFacts {
        offset: 1,
        values: &[FieldValue::new("construct", 7)],
    }]);
const WITH_S6: &[SequenceSpec] = &[
    common::S1_ASYNC_FN_HEAD,
    common::S2_FN_HEAD,
    common::S3_PARAM_BINDING,
    common::S4_TYPE_AFTER_COLON,
    common::S5_CALL_WITH_ARG,
    S6_BRACE_IDENT,
];

#[test]
fn dead_sequence() {
    let report = report(with(common::CONTRACTS, WITH_S6));
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| matches!(f, Finding::DeadSequence { .. }))
            .collect::<Vec<_>>(),
        [&Finding::DeadSequence {
            owner: "BraceIdent",
            offset: 1
        }]
    );
    assert!(!report.passed());
}

const S7_CALL_TWIN: SequenceSpec = sequence(
    "CallTwin",
    &[
        kind(common::IDENT),
        kind(common::LPAREN),
        kind(common::IDENT),
        kind(common::RPAREN),
    ],
)
.facts(&[OffsetFacts {
    offset: 0,
    values: &[FieldValue::new("construct", 1)],
}]);
const WITH_S7: &[SequenceSpec] = &[
    common::S1_ASYNC_FN_HEAD,
    common::S2_FN_HEAD,
    common::S3_PARAM_BINDING,
    common::S4_TYPE_AFTER_COLON,
    common::S5_CALL_WITH_ARG,
    S7_CALL_TWIN,
];

#[test]
fn overlapping_sequences() {
    let report = report(with(common::CONTRACTS, WITH_S7));
    let overlap = report
        .findings
        .iter()
        .find(|f| matches!(f, Finding::OverlappingSequences { .. }))
        .unwrap();
    assert_eq!(
        overlap,
        &Finding::OverlappingSequences {
            first: "CallWithArg",
            second: "CallTwin"
        }
    );
    assert_eq!(overlap.code(), "BBBC0027");
}

#[test]
fn contained_sequence_is_info() {
    let program = verify(common::program().build().unwrap(), common::LIMITS).unwrap();
    let contained = &program.report().findings[0];
    assert_eq!(contained.code(), "BBBC0028");
    assert_eq!(contained.severity(), liaise::LiaiseSeverity::Info);
}

#[test]
fn unreachable_is_info() {
    let program = verify(common::program().build().unwrap(), common::LIMITS).unwrap();
    let unreachable = &program.report().findings[1];
    assert_eq!(unreachable.code(), "BBBC0022");
    assert_eq!(
        unreachable.to_string(),
        "[BBBC0022] `context.marks` = const_expression is never reached"
    );
}

#[test]
fn too_many_states() {
    let limits = VerifyLimits {
        max_states: 10,
        ..common::LIMITS
    };
    let report = verify(common::program().build().unwrap(), limits).unwrap_err();
    assert_eq!(
        report.findings.last(),
        Some(&Finding::TooManyStates {
            explored: 11,
            permitted: 10
        })
    );
    assert_eq!(report.findings.last().unwrap().code(), "BBBC0029");
}

#[test]
fn illegal_fixed_word() {
    // effect.kind 5 fits the field but is #[invalid(5..=7)].
    const BAD_EMIT: ContractSpec = contract("bad-emit")
        .when(&[token(common::COMMA)])
        .emit(&[set("effect", "kind", 5)]);
    let mut contracts = common::CONTRACTS.to_vec();
    contracts.push(BAD_EMIT);
    let report = report(with(&contracts, common::SEQUENCES));
    assert_eq!(
        report.findings[0],
        Finding::IllegalWord {
            owner: "bad-emit",
            dimension: "effect",
            verdict: "field `kind` = 5 is invalid".to_owned()
        }
    );
}
