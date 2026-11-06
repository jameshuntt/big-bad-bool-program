//! Correction 4: overlapping recognitions are kept. A shorter match inside a
//! longer one is not dropped for its length; compatible facts coexist, the
//! longer match may refine the shorter, and contradictory facts are listed by
//! the verifier and refused when actualizing contracts rely on both.
//!
//! The runtime refusal itself is unit-tested in `src/step/authorize.rs`
//! (`frames::contracts_that_read_contradictory_facts_conflict`): a verified
//! program cannot reach it, because the state search finds every reachable
//! case first and verification fails.

mod common;

use big_bad_bool_program::{
    ContractSpec, FieldMatcher, FieldValue, Finding, ItemMatch, NoObserver, OffsetFacts,
    Recognizer, RecognizerSpec, SequenceSpec, StepResult, Stepper, VerificationReport,
    VerifiedProgram, clause, contract, sequence, verify,
};
use common::{CONTRACTS, FrameWord, IDENT, KW_ASYNC, KW_FN, LPAREN, RPAREN, Recorder};

fn program_with(
    contracts: &[ContractSpec],
    sequences: &'static [SequenceSpec],
) -> VerificationReport {
    let builder = contracts.iter().fold(
        common::dimensions(big_bad_bool_program::Program::new()).recognize(RecognizerSpec {
            sequences,
            ..common::RECOGNIZER
        }),
        |b, c| b.contract(*c),
    );
    match verify(
        builder.contain(common::CONTAINMENT).build().unwrap(),
        common::LIMITS,
    ) {
        Ok(p) => p.report().clone(),
        Err(r) => r,
    }
}

fn verified(contracts: &[ContractSpec]) -> VerifiedProgram {
    let builder = contracts.iter().fold(
        common::dimensions(big_bad_bool_program::Program::new()).recognize(common::RECOGNIZER),
        |b, c| b.contract(*c),
    );
    verify(
        builder.contain(common::CONTAINMENT).build().unwrap(),
        common::LIMITS,
    )
    .expect("verifies")
}

fn run(program: &VerifiedProgram, kinds: &[u128]) -> (Vec<StepResult>, Vec<String>) {
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
    (results, recorder.lines)
}

/// The focal item's (sequence, offset, modifier) per frame.
fn frames_at(kinds: &[u128], focal: usize) -> Vec<(&'static str, u8, u32)> {
    let spec = common::program().build().unwrap();
    let items: Vec<u128> = common::tokens(0, kinds)
        .iter()
        .map(|o| o.word(common::TOKEN).unwrap())
        .collect();
    Recognizer::new(&spec)
        .unwrap()
        .recognize(&items, focal)
        .iter()
        .map(|f| {
            let w = FrameWord::from_raw(f.word as u32);
            assert_eq!(
                u128::from(w.sequence()),
                f.sequence.key(),
                "quantized identity"
            );
            assert_eq!(w.offset(), u32::from(f.offset));
            (
                spec.sequence(f.sequence).unwrap().name,
                f.offset,
                w.modifier(),
            )
        })
        .collect()
}

/// Reads `modifier`, which only AsyncFnHead declares. Writes nothing.
const ASYNC_NAME: ContractSpec = contract("async-name").when(&[
    clause("token", FieldMatcher::eq("kind", IDENT)),
    clause("frame", FieldMatcher::eq("modifier", 1)),
]);

#[test]
fn compatible_contained_recognitions_coexist() {
    // `foo` in `async fn foo (`: AsyncFnHead@2 refines FnHead@1 with modifier Async.
    assert_eq!(
        frames_at(&[KW_ASYNC, KW_FN, IDENT, LPAREN], 2),
        [("AsyncFnHead", 2, 1), ("FnHead", 1, 0)]
    );
    // Without `async` to the left only FnHead holds: the left context decides.
    assert_eq!(frames_at(&[KW_FN, IDENT, LPAREN], 1), [("FnHead", 1, 0)]);

    // declare-function-name reads construct and role under both; they agree.
    // async-name reads modifier, which only the longer match declares.
    let mut contracts = CONTRACTS.to_vec();
    contracts.push(ASYNC_NAME);
    let program = verified(&contracts);
    assert!(
        !program
            .report()
            .findings
            .iter()
            .any(|f| matches!(f, Finding::Conflict { .. }))
    );
    let (results, events) = run(&program, common::ASYNC_FN_FOO);
    assert_eq!(
        common::render_result(program.spec(), &results[2]),
        "2: commit declare-function-name+async-name effects [effect {kind=DeclaresIdentity, class=Function} at 2]"
    );
    assert!(events.contains(&"framed [AsyncFnHead@2, FnHead@1]".to_owned()));
    // Plain `fn foo` has no Async fact, so async-name does not apply there.
    let (plain, _) = run(&program, &common::ASYNC_FN_FOO[1..]);
    assert_eq!(
        common::render_result(program.spec(), &plain[1]),
        "1: commit declare-function-name effects [effect {kind=DeclaresIdentity, class=Function} at 1]"
    );
}

#[test]
fn right_context_still_decides() {
    // `bar(foo)`: foo is an argument; `fn bar(foo: i32)`: foo is a parameter.
    assert_eq!(
        frames_at(&[IDENT, LPAREN, IDENT, RPAREN], 2),
        [("CallWithArg", 2, 0)]
    );
    assert_eq!(
        frames_at(
            &[KW_FN, IDENT, LPAREN, IDENT, common::COLON, IDENT, RPAREN],
            3
        ),
        [("ParamBinding", 1, 0)]
    );
}

const fn kind(value: u128) -> ItemMatch {
    ItemMatch::One(FieldMatcher::eq("kind", value))
}

/// An identifier before `(` is a callee, says this sequence: it contradicts
/// FnHead and AsyncFnHead on `construct` at every function name.
const S8_IDENT_OPEN: SequenceSpec =
    sequence("IdentOpen", &[kind(IDENT), kind(LPAREN)]).facts(&[OffsetFacts {
        offset: 0,
        values: &[FieldValue::new("construct", 4)],
    }]);
const WITH_S8: &[SequenceSpec] = &[
    common::S1_ASYNC_FN_HEAD,
    common::S2_FN_HEAD,
    common::S3_PARAM_BINDING,
    common::S4_TYPE_AFTER_COLON,
    common::S5_CALL_WITH_ARG,
    S8_IDENT_OPEN,
];

/// Reads only the sequence key of IdentOpen (the recognizer's own field, never
/// a declared fact), so it relies on no contradiction.
const IN_IDENT_OPEN: ContractSpec =
    contract("in-ident-open").when(&[clause("frame", FieldMatcher::eq("sequence", 6))]);

/// Reads `construct` under IdentOpen, beside declare-function-name reading it under FnHead.
const CALLEE_SEEN: ContractSpec = contract("callee-seen").when(&[
    clause("token", FieldMatcher::eq("kind", IDENT)),
    clause("frame", FieldMatcher::eq("construct", 4)),
]);

#[test]
fn contradictory_overlapping_facts_are_detected_and_refused() {
    // Listed, not dropped: section 16's own lexical overlaps are info findings.
    let section_16 = verify(common::program().build().unwrap(), common::LIMITS).unwrap();
    assert!(
        section_16
            .report()
            .findings
            .contains(&Finding::OverlappingFacts {
                first: "AsyncFnHead",
                first_offset: 2,
                second: "CallWithArg",
                second_offset: 0,
                field: "construct",
            })
    );

    // Nothing reads construct under IdentOpen: info only, and it verifies.
    let mut contracts = CONTRACTS.to_vec();
    contracts.push(IN_IDENT_OPEN);
    let listed = program_with(&contracts, WITH_S8);
    assert!(listed.passed(), "{:?}", listed.findings);
    assert!(listed.findings.contains(&Finding::OverlappingFacts {
        first: "FnHead",
        first_offset: 1,
        second: "IdentOpen",
        second_offset: 0,
        field: "construct",
    }));

    // callee-seen relies on CallCallee where declare-function-name relies on
    // FunctionDeclaration, for one item: a reachable conflict, so verification fails.
    contracts.push(CALLEE_SEEN);
    let refused = program_with(&contracts, WITH_S8);
    assert!(!refused.passed());
    let conflict = refused
        .findings
        .iter()
        .find_map(|f| match f {
            Finding::Conflict {
                first,
                second,
                target,
                ..
            } => Some((*first, *second, target.clone())),
            _ => None,
        })
        .expect("a conflict finding");
    assert_eq!(
        (conflict.0, conflict.1),
        ("callee-seen", "declare-function-name")
    );
    assert!(
        conflict
            .2
            .starts_with("frame.construct: IdentOpen@0 says CallCallee, "),
        "{}",
        conflict.2
    );
    assert!(
        conflict.2.ends_with(" says FunctionDeclaration"),
        "{}",
        conflict.2
    );
}

#[test]
fn the_section_16_examples_still_work() {
    let program = verified(CONTRACTS);
    let effects = |kinds: &[u128]| -> Vec<String> {
        let (results, _) = run(&program, kinds);
        assert!(results.iter().all(|r| !r.committed.is_empty()));
        results
            .iter()
            .flat_map(|r| &r.effects)
            .map(|e| {
                let d = program.spec().dimension(e.dimension).unwrap();
                format!("{} {}", e.position, d.describe(e.word))
            })
            .collect()
    };
    // `async fn foo(x: i32) { bar(foo) }`: foo declared at 2, referenced at 11.
    assert_eq!(
        effects(common::ASYNC_FN_FOO_CALLING_BAR),
        [
            "2 {kind=DeclaresIdentity, class=Function}",
            "4 {kind=DeclaresIdentity, class=Binding}",
            "6 {kind=ReferencesIdentity, class=Type}",
            "8 {kind=OpensScope, class=None}",
            "9 {kind=ReferencesIdentity, class=Value}",
            "11 {kind=ReferencesIdentity, class=Value}",
            "13 {kind=ClosesScope, class=None}",
        ]
    );
    let mut stepper = Stepper::new(&program, NoObserver);
    let mut module_level = Vec::new();
    for o in common::tokens(0, common::BAR_FOO) {
        module_level.extend(stepper.push(o));
    }
    module_level.extend(stepper.finish());
    assert!(
        module_level[0].committed.is_empty(),
        "bar(foo) at module level is refused"
    );
}
