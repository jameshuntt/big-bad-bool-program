//! Framing: declared sequences over the item dimension give each item its
//! contextual facts, and contracts read those facts beside the item itself.
//! In `let a = b ; let c = a ;` the name after `let` is a declaration and the
//! name after `=` is a reference. The token word is the same in both places;
//! only the frame tells them apart.
//!
//! `cargo run --example declarations`

use big_bad_bool::bitpack;
use big_bad_bool_program::{
    Clause, ContainmentSpec, ContractSpec, FieldMatcher, FieldValue, ItemMatch, NoObserver,
    Observation, OffsetFacts, Program, RecognizerSpec, RefusalRule, SequenceSpec, Stepper,
    VerifyLimits, clause, contract, dimension, sequence, set, verify,
};

#[bitpack(storage = u8)]
pub struct Token {
    #[bits(3)]
    #[labels(Let = 0, Name = 1, Assign = 2, Semi = 3, End = 4)]
    #[invalid(5..=7)]
    pub kind: u8,
}

/// Written by the recognizer, never observed.
#[bitpack(storage = u16)]
pub struct Frame {
    #[bits(2)]
    pub sequence: u16,
    #[bits(1)]
    pub offset: u16,
    #[bits(3)]
    #[labels(None = 0, Begin = 1, Interior = 2, End = 3, Complete = 4)]
    #[invalid(5..=7)]
    pub position: u16,
    #[bits(1)]
    #[labels(boundary = 1)]
    #[flags(boundary = 0)]
    pub marks: u16,
    #[bits(2)]
    #[labels(None = 0, Declared = 1, Referenced = 2)]
    #[invalid(3)]
    pub role: u16,
}

#[bitpack(storage = u8)]
pub struct Statement {
    #[bits(2)]
    #[labels(Start = 0, Named = 1, Assigning = 2, Valued = 3)]
    pub phase: u8,
}

#[bitpack(storage = u8)]
pub struct Effect {
    #[bits(2)]
    #[labels(None = 0, Declares = 1, References = 2)]
    #[invalid(3)]
    pub kind: u8,
}

const LET: u128 = Token::LABEL_KIND_LET.value;
const NAME: u128 = Token::LABEL_KIND_NAME.value;
const ASSIGN: u128 = Token::LABEL_KIND_ASSIGN.value;
const SEMI: u128 = Token::LABEL_KIND_SEMI.value;
const END: u128 = Token::LABEL_KIND_END.value;
const DECLARED: u128 = Frame::LABEL_ROLE_DECLARED.value;
const REFERENCED: u128 = Frame::LABEL_ROLE_REFERENCED.value;
const START: u128 = Statement::LABEL_PHASE_START.value;
const NAMED: u128 = Statement::LABEL_PHASE_NAMED.value;
const ASSIGNING: u128 = Statement::LABEL_PHASE_ASSIGNING.value;
const VALUED: u128 = Statement::LABEL_PHASE_VALUED.value;

const fn kind(k: u128) -> FieldMatcher {
    FieldMatcher::eq("kind", k)
}
const fn token(k: u128) -> Clause {
    clause("token", kind(k))
}
const fn role(r: u128) -> Clause {
    clause("frame", FieldMatcher::eq("role", r))
}
const fn phase(p: u128) -> Clause {
    clause("statement", FieldMatcher::eq("phase", p))
}

/// `let <name>`: the name is declared. `= <name>`: the name is referenced.
const SEQUENCES: &[SequenceSpec] = &[
    sequence(
        "binding",
        &[ItemMatch::One(kind(LET)), ItemMatch::One(kind(NAME))],
    )
    .facts(&[OffsetFacts {
        offset: 1,
        values: &[FieldValue::new("role", DECLARED)],
    }]),
    sequence(
        "value",
        &[ItemMatch::One(kind(ASSIGN)), ItemMatch::One(kind(NAME))],
    )
    .facts(&[OffsetFacts {
        offset: 1,
        values: &[FieldValue::new("role", REFERENCED)],
    }]),
];

const CONTRACTS: &[ContractSpec] = &[
    contract("let").when(&[token(LET)]).require(&[phase(START)]),
    contract("declare")
        .when(&[token(NAME), role(DECLARED)])
        .require(&[phase(START)])
        .transition(&[set("statement", "phase", NAMED)])
        .emit(&[set("effect", "kind", Effect::LABEL_KIND_DECLARES.value)]),
    contract("assign")
        .when(&[token(ASSIGN)])
        .require(&[phase(NAMED)])
        .transition(&[set("statement", "phase", ASSIGNING)]),
    contract("reference")
        .when(&[token(NAME), role(REFERENCED)])
        .require(&[phase(ASSIGNING)])
        .transition(&[set("statement", "phase", VALUED)])
        .emit(&[set("effect", "kind", Effect::LABEL_KIND_REFERENCES.value)]),
    contract("end-statement")
        .when(&[token(SEMI)])
        .require(&[phase(VALUED)])
        .transition(&[set("statement", "phase", START)]),
    contract("end-of-input")
        .when(&[token(END)])
        .require(&[phase(START)]),
];

fn main() {
    let spec = CONTRACTS
        .iter()
        .fold(
            Program::new()
                .add(dimension!(Token, "token", Evidence))
                .add(dimension!(Frame, "frame", Evidence))
                .add(dimension!(Statement, "statement", State))
                .add(dimension!(Effect, "effect", Effect))
                .recognize(RecognizerSpec {
                    item_dimension: "token",
                    frame_dimension: "frame",
                    sequence_field: "sequence",
                    offset_field: "offset",
                    position_field: "position",
                    boundary_flag: "boundary",
                    end_item: FieldValue::new("kind", END),
                    sequences: SEQUENCES,
                })
                .contain(ContainmentSpec {
                    max_interpretations: 2,
                    max_candidates: 4,
                    max_actualized: 1,
                    max_stack_depth: 1,
                    on_refusal: RefusalRule::Stop,
                }),
            |program, c| program.contract(*c),
        )
        .build()
        .expect("every name resolves");
    let program = verify(
        spec,
        VerifyLimits {
            max_states: 1_000,
            max_witness_tries: 1_024,
        },
    )
    .expect("the program verifies");
    // The bounded state search visited every reachable state and found nothing.
    println!(
        "verified: {} states, {} findings",
        program.report().states,
        program.report().findings.len()
    );
    let spec = program.spec();
    let token = spec.dimension_id("token").unwrap();

    let source = ["let", "a", "=", "b", ";", "let", "c", "=", "a", ";"];
    let kind_of = |text: &str| match text {
        "let" => LET,
        "=" => ASSIGN,
        ";" => SEMI,
        _ => NAME,
    };
    let mut stepper = Stepper::new(&program, NoObserver);
    let mut results = Vec::new();
    for (position, text) in (0..).zip(source) {
        results.extend(stepper.push(Observation::new(position).with(token, kind_of(text))));
    }
    // Pads with the end item and steps what is left.
    results.extend(stepper.finish());

    for result in &results {
        for effect in &result.effects {
            let what = spec
                .dimension(effect.dimension)
                .unwrap()
                .describe(effect.word);
            // The application maps the position back to its own text.
            println!(
                "{} at {}: {what}",
                source[effect.position as usize], effect.position
            );
        }
        if let Some(refusal) = result.refusals.last() {
            println!("refused at {}: {refusal}", result.position);
        }
    }
}
