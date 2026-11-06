//! Contracts across Big Bad Bool packed words.
//!
//! A program is a list of dimensions (each one `#[bitpack]` type with a role),
//! a frame recognizer over declared sequences, contracts between dimensions,
//! and containment limits. `build` resolves names. `verify` checks coherence
//! once and returns a verified program. The stepper takes a verified program
//! and a stream of observations, and only its commit step writes state.
//!
//! The crate never names a language. Every name below a dimension comes from
//! the user's `#[bitpack]` labels and sequence specs.
//!
//! Modules are future crate boundaries:
//! `codes` <- `model` <- (`frame`, `contract`) <- `step` <- `verify` (+ `digest`)
//! <- `observe` <- `stepper`, and the `bowl` adapter on top.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod codes;
mod contract;
mod frame;
mod model;
mod observe;
mod step;
mod stepper;
#[cfg(test)]
mod test_support;
mod verify;

pub use codes::{Finding, Refusal};
pub use contract::{Candidate, FactView, Outcome, evaluate};
pub use frame::{FrameFacts, Recognizer};
pub use model::{
    Clause, ContainmentSpec, ContractId, ContractSpec, DimensionId, DimensionRole, DimensionSpec,
    ItemMatch, MAX_SEQUENCE_LEN, Observation, OffsetFacts, Position, Program, ProgramBuilder,
    ProgramSpec, RecognizerSpec, RefusalRule, SequenceId, SequenceSpec, StackOp, Write, clause,
    contract, dimension, pop, push, sequence, set,
};
pub use observe::{NoObserver, Observer, StepEvent};
pub use step::{AuthorizedTransition, Effect, ProjectionState};
pub use stepper::{StepResult, Stepper};
pub use verify::{VerificationReport, VerifiedProgram, VerifyLimits, verify};

pub use big_bad_bool_interact::{FieldMatcher, FieldValue, MatchOp};
pub use flaga_solver::Schema;
pub use liaise::Liaise;

/// Paths the macros expand to. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use flaga_solver::schema_of;
}

/// A dimension over a `#[bitpack]` type: `dimension!(TokenWord, "token", Evidence)`.
///
/// Expands to `dimension::<TokenWord>("token", schema_of!(TokenWord), DimensionRole::Evidence)`.
/// The schema and the field writes both come from the one type.
#[macro_export]
macro_rules! dimension {
    ($t:ty, $name:expr, $role:ident) => {
        $crate::dimension::<$t>(
            $name,
            $crate::__private::schema_of!($t),
            $crate::DimensionRole::$role,
        )
    };
}
