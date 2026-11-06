//! Correction 8: `Stepper::legal_next_representatives` (formerly
//! `legal_next`) lists one representative observation per class of accepted
//! observations, beside the labelled words, not every concrete word the
//! stepper accepts. Other words of an accepted class are accepted the same
//! way at run time; words of a class with no representative are refused.
//!
//! The data is a dial, with no recognizer: `dial` is evidence, a 4-bit
//! reading labelled only at 0; `lamp` is state.

use std::collections::BTreeSet;

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, NoObserver, Observation, Program,
    RefusalRule, Stepper, VerifiedProgram, VerifyLimits, clause, contract, dimension, set, verify,
};

#[bitpack(storage = u8)]
pub struct Dial {
    #[bits(4)]
    #[labels(Off = 0)]
    pub value: u8,
}

#[bitpack(storage = u8)]
pub struct Lamp {
    #[bits(1)]
    #[labels(Dark = 0, Lit = 1)]
    pub state: u8,
}

const DIAL: DimensionId = DimensionId(0);

/// Off turns the lamp off.
const DARK: ContractSpec = contract("dark")
    .when(&[clause("dial", FieldMatcher::eq("value", 0))])
    .transition(&[set("lamp", "state", 0)]);

/// Any reading in 6..=11 lights it: one class, six concrete words, none labelled.
const BAND: ContractSpec = contract("band")
    .when(&[clause("dial", FieldMatcher::range("value", 6, 11))])
    .transition(&[set("lamp", "state", 1)]);

fn program() -> VerifiedProgram {
    let spec = Program::new()
        .add(dimension!(Dial, "dial", Evidence))
        .add(dimension!(Lamp, "lamp", State))
        .contain(ContainmentSpec {
            max_interpretations: 1,
            max_candidates: 2,
            max_actualized: 2,
            max_stack_depth: 1,
            on_refusal: RefusalRule::SkipObservation,
        })
        .contract(DARK)
        .contract(BAND)
        .build()
        .expect("dial program builds");
    verify(
        spec,
        VerifyLimits {
            max_states: 100,
            max_witness_tries: 64,
        },
    )
    .unwrap_or_else(|r| panic!("{:?}", r.findings))
}

/// The contracts one fresh stepper commits for `dial = word`, by name; empty when refused.
fn outcome(program: &VerifiedProgram, word: u128) -> Vec<&'static str> {
    let mut stepper = Stepper::new(program, NoObserver);
    let r = stepper.push(Observation::new(0).with(DIAL, word));
    r[0].committed
        .iter()
        .map(|id| program.spec().contract(*id).unwrap().name)
        .collect()
}

#[test]
fn representatives_are_one_word_per_accepted_class() {
    let program = program();
    let stepper = Stepper::new(&program, NoObserver);
    let words: Vec<u128> = stepper
        .legal_next_representatives()
        .iter()
        .map(|o| o.word(DIAL).expect("no accepted observation holds nothing"))
        .collect();
    // Off (labelled) and the least word of 6..=11.
    assert_eq!(words, [0, 6]);
}

#[test]
fn other_members_of_an_accepted_class_are_accepted_the_same_way() {
    let program = program();
    let listed: Vec<u128> = Stepper::new(&program, NoObserver)
        .legal_next_representatives()
        .iter()
        .filter_map(|o| o.word(DIAL))
        .collect();
    // 7..=11 are not listed, and each is accepted exactly as 6 is.
    for word in 7..=11 {
        assert!(!listed.contains(&word));
        assert_eq!(outcome(&program, word), ["band"], "dial {word}");
    }
    // Over every concrete dial word: the accepted outcomes are exactly the
    // outcomes of the representatives, and a word of a class with no
    // representative (1..=5, 12..=15) is refused.
    let accepted: BTreeSet<Vec<&str>> = (0..=15)
        .map(|w| outcome(&program, w))
        .filter(|o| !o.is_empty())
        .collect();
    let represented: BTreeSet<Vec<&str>> = listed.iter().map(|w| outcome(&program, *w)).collect();
    assert_eq!(accepted, represented);
    for word in (1..=5).chain(12..=15) {
        assert!(outcome(&program, word).is_empty(), "dial {word}");
    }
}
