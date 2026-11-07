//! Programs over Big Bad Bool packed words: dimensions, contracts across them,
//! optional framing, bounded verification, and stepping with one write path.
//!
//! A program is a list of dimensions (each one `#[bitpack]` type with a role),
//! contracts between dimensions, containment limits, and, optionally, a frame
//! recognizer over declared sequences. `build` resolves names. `verify`
//! checks coherence once and returns a verified program. The stepper takes a
//! verified program and a stream of observations, each holding words for any
//! of the program's evidence dimensions. Every compatible contract at a
//! position actualizes together, and only the commit step writes state.
//!
//! The crate never names a language. Every name below a dimension comes from
//! the user's `#[bitpack]` labels and sequence specs.
//!
//! The public API is re-exported here. Inside, each module depends only on
//! the ones before it:
//! `codes` <- `model` <- `frame` <- `contract` <- `step` <- `verify` (+ `digest`)
//! <- `observe` <- `stepper`, and the `bowl` adapter on top.
//!
//! `examples/thermostat.rs` is a program with no framing;
//! `examples/declarations.rs` uses framing to tell declarations from
//! references.
//!
//! # A step in two phases
//!
//! A host that admits each transition before it happens steps in two phases.
//! [`Stepper::prepare`] authorizes the semantic transition of a `push`
//! without mutating live state and returns a [`PreparedStep`] to inspect.
//! [`Stepper::commit`] is the sole application boundary: it applies exactly
//! that transition, once. Declining is dropping the preparation. `push` is
//! the two phases with nothing in between. The admission rule is the host's;
//! the crate names none.
//!
//! ```
//! use big_bad_bool::bitpack;
//! use big_bad_bool_program::{
//!     CommitRefusal, ContainmentSpec, ContractSpec, FieldMatcher, NoObserver, Observation,
//!     Program, RefusalRule, Stepper, VerifyLimits, clause, contract, dimension, set, verify,
//! };
//!
//! #[bitpack(storage = u8)]
//! pub struct Reading {
//!     #[bits(1)]
//!     #[labels(Cold = 0, Hot = 1)]
//!     pub level: u8,
//! }
//!
//! #[bitpack(storage = u8)]
//! pub struct Heater {
//!     #[bits(1)]
//!     #[labels(Off = 0, On = 1)]
//!     pub state: u8,
//! }
//!
//! const HEAT: ContractSpec = contract("heat")
//!     .when(&[clause("reading", FieldMatcher::eq("level", 0))])
//!     .transition(&[set("heater", "state", 1)]);
//! const HOLD: ContractSpec = contract("hold").when(&[clause("reading", FieldMatcher::eq("level", 1))]);
//!
//! let spec = Program::new()
//!     .add(dimension!(Reading, "reading", Evidence))
//!     .add(dimension!(Heater, "heater", State))
//!     .contract(HEAT)
//!     .contract(HOLD)
//!     .contain(ContainmentSpec {
//!         max_interpretations: 1,
//!         max_candidates: 2,
//!         max_actualized: 1,
//!         max_stack_depth: 1,
//!         on_refusal: RefusalRule::SkipObservation,
//!     })
//!     .build()
//!     .unwrap();
//! let limits = VerifyLimits { max_states: 100, max_witness_tries: 64 };
//! let program = verify(spec, limits).unwrap();
//! let (reading, heater) = (
//!     program.spec().dimension_id("reading").unwrap(),
//!     program.spec().dimension_id("heater").unwrap(),
//! );
//! let mut stepper = Stepper::new(&program, NoObserver);
//! let cold = Observation::new(0).with(reading, 0);
//!
//! // Prepare: the transition is visible, the stepper is unchanged.
//! let prepared = stepper.prepare(cold.clone());
//! assert_eq!(prepared.state_after().word(heater), 1);
//! assert_eq!(stepper.state().word(heater), 0);
//!
//! // Decline it: nothing happened.
//! drop(prepared);
//! assert_eq!(stepper.version(), 0);
//!
//! // Admit it: commit applies exactly what was prepared.
//! let prepared = stepper.prepare(cold.clone());
//! let older = stepper.prepare(cold);
//! stepper.commit(prepared).unwrap();
//! assert_eq!(stepper.state().word(heater), 1);
//!
//! // A preparation from before that commit is stale.
//! assert_eq!(
//!     stepper.commit(older).unwrap_err(),
//!     CommitRefusal::Stale { prepared: 0, current: 1 },
//! );
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "bowl")]
mod bowl;
mod codes;
mod contract;
mod digest;
mod frame;
mod model;
mod observe;
mod step;
mod stepper;
#[cfg(test)]
mod test_support;
mod verify;

#[cfg(feature = "bowl")]
pub use bowl::{BowlObserver, EventKind};
pub use codes::{CommitRefusal, Finding, Refusal};
pub use contract::{Candidate, FactView, Outcome, evaluate};
pub use digest::{SPEC_FORMAT, TRACE_FORMAT, TraceHeader, spec_digest, spec_material};
pub use frame::{FrameFacts, Recognizer};
pub use model::{
    Clause, ContainmentSpec, ContractId, ContractSpec, DimensionId, DimensionRole, DimensionSpec,
    ItemMatch, MAX_SEQUENCE_LEN, Observation, OffsetFacts, Position, Program, ProgramBuilder,
    ProgramSpec, RecognizerSpec, RefusalRule, SequenceId, SequenceSpec, StackOp, Write, clause,
    contract, dimension, pop, push, sequence, set,
};
/// The ob-serve-a-bowl crate the adapter publishes to.
#[cfg(feature = "bowl")]
pub use ob_serve_a_bowl;
pub use observe::{NoObserver, Observer, StepEvent};
pub use step::{AuthorizedTransition, Effect, ProjectionState};
pub use stepper::{PreparedStep, StepResult, Stepper};
pub use verify::{VerificationReport, VerifiedProgram, VerifyLimits, verify};

pub use big_bad_bool_interact::{FieldMatcher, FieldValue, MatchOp};
pub use flaga_solver::Schema;
pub use liaise::Liaise;
pub use stable_digest::StableDigest;

/// Paths the macros expand to. Not part of the API.
#[doc(hidden)]
pub mod __private {
    pub use flaga_solver::schema_of;
}

/// A dimension over a `#[bitpack]` type: `dimension!(ItemWord, "item", Evidence)`.
///
/// Expands to `dimension::<ItemWord>("item", schema_of!(ItemWord), DimensionRole::Evidence)`.
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
