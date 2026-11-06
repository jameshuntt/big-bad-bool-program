//! Step 2: WHEN, REQUIRE, REQUIRE-ANY and FORBID behave as stated, and label
//! holding goes through `Schema::holds`.

mod common;

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Candidate, ContractId, ContractSpec, FactView, FieldMatcher, MatchOp, Outcome, ProgramSpec,
    Refusal, SequenceId, clause, contract, dimension, evaluate,
};
use common::{ContextWord, FrameWord, StateWord, TokenWord};

fn spec() -> ProgramSpec {
    common::program().build().unwrap()
}

fn token(kind: u128) -> Option<u128> {
    Some(u128::from(TokenWord::new(kind as u8).raw()))
}

fn context(scope: u8, expecting: u8, marks: u8) -> Option<u128> {
    Some(u128::from(ContextWord::new(scope, expecting, marks).raw()))
}

fn state(phase: u8, marks: u8) -> Option<u128> {
    Some(u128::from(StateWord::new(phase, marks).raw()))
}

/// S1 offset 2: FunctionDeclaration, DeclaredIdentity, Async, params_follow.
fn s1_offset2(role: u32) -> Option<u128> {
    Some(u128::from(FrameWord::new(1, 2, 2, 0b10, 1, role, 1).raw()))
}

fn outcome(spec: &ProgramSpec, name: &str, words: &[Option<u128>]) -> Outcome {
    let id = spec.contract_id(name).unwrap();
    evaluate(id, &FactView::new(spec, words, 2, Some(SequenceId(0))))
}

fn refused(
    position: u32,
    contract: &'static str,
    clause: &'static str,
    missing: &str,
    observed: &str,
) -> Outcome {
    Outcome::Refused(Refusal::ContractRefused {
        position,
        contract,
        clause,
        missing: missing.to_owned(),
        observed: observed.to_owned(),
    })
}

#[test]
fn declare_function_name_is_a_candidate() {
    let spec = spec();
    let words = [
        token(common::IDENT),
        s1_offset2(1),
        context(0, 0, 0),
        state(1, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "declare-function-name", &words),
        Outcome::Candidate(Candidate {
            contract: ContractId(2),
            position: 2,
            frame: Some(SequenceId(0)),
        })
    );
}

#[test]
fn when_decides_applicability() {
    let spec = spec();
    let referenced = [
        token(common::IDENT),
        s1_offset2(2),
        context(0, 0, 0),
        state(1, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "declare-function-name", &referenced),
        Outcome::NotApplicable
    );
    let not_ident = [
        token(common::KW_FN),
        s1_offset2(1),
        context(0, 0, 0),
        state(1, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "declare-function-name", &not_ident),
        Outcome::NotApplicable
    );
}

#[test]
fn require_names_the_failed_clause() {
    let spec = spec();
    let cannot_declare = [
        token(common::IDENT),
        s1_offset2(1),
        context(0, 0, 0),
        state(1, 0),
        None,
    ];
    assert_eq!(
        outcome(&spec, "declare-function-name", &cannot_declare),
        refused(
            2,
            "declare-function-name",
            "require",
            "state.marks = can_declare",
            "none"
        )
    );
    let wrong_phase = [
        token(common::IDENT),
        s1_offset2(1),
        context(0, 0, 0),
        state(0, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "declare-function-name", &wrong_phase),
        refused(
            2,
            "declare-function-name",
            "require",
            "state.phase = FnHead",
            "Items"
        )
    );
}

#[test]
fn forbid_refuses_when_it_holds() {
    let spec = spec();
    let in_const = [
        token(common::IDENT),
        s1_offset2(1),
        context(0, 0, 1),
        state(1, 1),
        None,
    ];
    let Outcome::Refused(refusal) = outcome(&spec, "declare-function-name", &in_const) else {
        panic!("forbid refuses");
    };
    assert_eq!(
        refusal.to_string(),
        "[BBBC0102] `declare-function-name` refused at position 2: forbid needs not context.marks = const_expression, observed const_expression"
    );
}

#[test]
fn require_any_needs_one_whole_group() {
    let spec = spec();
    // S5 offset 0: CallCallee, ReferencedIdentity.
    let callee = Some(u128::from(FrameWord::new(5, 0, 1, 0, 4, 2, 0).raw()));
    let at_module = [
        token(common::IDENT),
        callee,
        context(0, 0, 0),
        state(0, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "callee-reference", &at_module),
        refused(
            2,
            "callee-reference",
            "require_any",
            "context.expecting in {Statement, Expr}",
            "Item"
        )
    );
    let in_body = [
        token(common::IDENT),
        callee,
        context(2, 5, 0),
        state(5, 1),
        None,
    ];
    assert!(matches!(
        outcome(&spec, "callee-reference", &in_body),
        Outcome::Candidate(_)
    ));
    let in_args = [
        token(common::IDENT),
        callee,
        context(3, 6, 0),
        state(5, 1),
        None,
    ];
    assert!(matches!(
        outcome(&spec, "callee-reference", &in_args),
        Outcome::Candidate(_)
    ));

    // close-params: [expecting AfterType] or [expecting Pattern].
    let no_frame = Some(0);
    let after_type = [
        token(common::RPAREN),
        no_frame,
        context(1, 4, 0),
        state(3, 1),
        None,
    ];
    assert!(matches!(
        outcome(&spec, "close-params", &after_type),
        Outcome::Candidate(_)
    ));
    let after_pattern = [
        token(common::RPAREN),
        no_frame,
        context(1, 2, 0),
        state(3, 1),
        None,
    ];
    assert_eq!(
        outcome(&spec, "close-params", &after_pattern),
        refused(
            2,
            "close-params",
            "require_any",
            "context.expecting in {AfterType, Pattern}",
            "AfterPattern"
        )
    );
}

#[test]
fn an_empty_stack_holds_nothing() {
    let spec = spec();
    // S1 offset 0: FunctionDeclaration, Begin.
    let begin = Some(u128::from(FrameWord::new(1, 0, 1, 0, 1, 0, 1).raw()));
    let words = [token(common::KW_ASYNC), begin, None, state(0, 1), None];
    assert_eq!(
        outcome(&spec, "fn-head-begin", &words),
        refused(
            2,
            "fn-head-begin",
            "require",
            "context.expecting = Item",
            "nothing"
        )
    );
}

/// A flag field with labels, as in flaga_solver's `Contract32`.
#[bitpack(storage = u8)]
pub struct JobWord {
    #[bits(5)]
    #[labels(none = 0, hot = 1, cold = 2, optimistic = 4, replicated = 8)]
    #[flags(hot = 0, cold = 1, optimistic = 2, replicated = 3)]
    #[conflicts(optimistic, replicated)]
    pub flags: u8,
}

const HOT: ContractSpec = contract("hot").when(&[clause("job", FieldMatcher::eq("flags", 1))]);
const NONE: ContractSpec = contract("none").when(&[clause("job", FieldMatcher::eq("flags", 0))]);
const NOT_COLD: ContractSpec =
    contract("not-cold").when(&[clause("job", FieldMatcher::not_eq("flags", 2))]);
const ANY_HOT_OR_COLD: ContractSpec =
    contract("any-hot-or-cold").when(&[clause("job", FieldMatcher::mask_any("flags", 3))]);

#[test]
fn flag_field_labels_hold_through_schema_holds() {
    let spec = common::dimensions(big_bad_bool_contract::Program::new())
        .add(dimension!(JobWord, "job", State))
        .recognize(common::RECOGNIZER)
        .contain(common::CONTAINMENT)
        .contract(HOT)
        .contract(NONE)
        .contract(NOT_COLD)
        .contract(ANY_HOT_OR_COLD)
        .build()
        .unwrap();
    let applies = |name: &str, flags: u8| {
        let words = [
            token(common::IDENT),
            Some(0),
            context(0, 0, 0),
            state(0, 1),
            None,
            Some(u128::from(flags)),
        ];
        let id = spec.contract_id(name).unwrap();
        matches!(
            evaluate(id, &FactView::new(&spec, &words, 0, None)),
            Outcome::Candidate(_)
        )
    };
    // hot + cold set: the hot label holds, though the field is not equal to it.
    assert!(applies("hot", 0b0011));
    assert!(!MatchOp::Eq(1).matches(0b0011));
    assert!(!applies("hot", 0b0010));
    // A zero label holds only on zero.
    assert!(applies("none", 0));
    assert!(!applies("none", 0b0011));
    // NotEq is "the label does not hold".
    assert!(applies("not-cold", 0b0001));
    assert!(!applies("not-cold", 0b0011));
    // Other ops are interact's MatchOp as they are.
    assert!(applies("any-hot-or-cold", 0b0010));
    assert!(!applies("any-hot-or-cold", 0b0100));
}
