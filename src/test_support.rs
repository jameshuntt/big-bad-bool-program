//! Small dimensions for unit tests. Compiled only for tests.

#![allow(missing_docs)]

use big_bad_bool::bitpack;
use big_bad_bool_interact::{FieldMatcher, FieldValue};

use crate::model::{
    ContainmentSpec, ContractSpec, Program, ProgramBuilder, ProgramSpec, RecognizerSpec,
    RefusalRule, clause, set,
};

#[bitpack(storage = u8)]
pub struct Item {
    #[bits(2)]
    #[labels(a = 0, b = 1, c = 2, end = 3)]
    pub kind: u8,
}

#[bitpack(storage = u16)]
pub struct Frame {
    #[bits(2)]
    pub sequence: u16,
    #[bits(2)]
    pub offset: u16,
    #[bits(3)]
    pub position: u16,
    #[bits(1)]
    #[labels(boundary = 1)]
    #[flags(boundary = 0)]
    pub marks: u16,
    /// A fact field sequences may declare.
    #[bits(2)]
    pub role: u16,
}

/// flaga_solver's `Job16`: `timer` requires `inline_queue`.
#[bitpack(storage = u16)]
pub struct Job {
    #[bits(3)]
    #[labels(signal = 0, timer = 1)]
    #[requires(timer => profile.inline_queue)]
    #[invalid(7)]
    pub reason: u16,
    #[bits(2)]
    #[labels(pooled = 0, inline_queue = 1)]
    pub profile: u16,
}

#[bitpack(storage = u8)]
pub struct Scope {
    #[bits(2)]
    #[labels(outer = 0, inner = 1)]
    pub level: u8,
}

#[bitpack(storage = u8)]
pub struct Out {
    #[bits(2)]
    #[labels(none = 0, said = 1)]
    #[invalid(3)]
    pub kind: u8,
}

pub const RECOGNIZER: RecognizerSpec = RecognizerSpec {
    item_dimension: "item",
    frame_dimension: "frame",
    sequence_field: "sequence",
    offset_field: "offset",
    position_field: "position",
    boundary_flag: "boundary",
    end_item: FieldValue::new("kind", 3),
    sequences: &[],
};

pub const CONTAIN: ContainmentSpec = ContainmentSpec {
    max_interpretations: 4,
    max_candidates: 4,
    max_actualized: 3,
    max_stack_depth: 2,
    on_refusal: RefusalRule::Stop,
};

pub const WHEN_B: &[crate::model::Clause] = &[clause("item", FieldMatcher::eq("kind", 1))];
pub const SET_TIMER: crate::model::Write = set("job", "reason", 1);
pub const SET_INLINE: crate::model::Write = set("job", "profile", 1);

/// item, frame, job (state), scope (stacked state, depth 2), out (effect).
pub fn builder() -> ProgramBuilder {
    Program::new()
        .add(crate::dimension!(Item, "item", Evidence))
        .add(crate::dimension!(Frame, "frame", Evidence))
        .add(crate::dimension!(Job, "job", State))
        .add(crate::dimension!(Scope, "scope", State).stacked(2))
        .add(crate::dimension!(Out, "out", Effect))
        .recognize(RECOGNIZER)
        .contain(CONTAIN)
}

pub fn spec(contracts: &[ContractSpec]) -> ProgramSpec {
    contracts
        .iter()
        .fold(builder(), |b, c| b.contract(*c))
        .build()
        .expect("test program builds")
}
