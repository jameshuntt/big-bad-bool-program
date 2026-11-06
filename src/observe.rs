//! Observers: read-only events after each stage of a step.
//!
//! An observer gets shared references and returns nothing to the stepper, so
//! it cannot change a result. Its own `&mut self` is for its own records.

use crate::codes::Refusal;
use crate::contract::Candidate;
use crate::frame::FrameFacts;
use crate::model::{ContractId, Observation};
use crate::step::{Effect, ProjectionState};

/// One stage of one step, in the order the stepper reaches it.
#[derive(Debug, Clone, Copy)]
pub enum StepEvent<'a> {
    /// The focal item. The end item has the position after the last observation.
    Observed(&'a Observation),
    /// The frame interpretations of the focal item, after longest-match.
    Framed(&'a [FrameFacts]),
    /// A contract held under one interpretation.
    Candidate(&'a Candidate),
    /// A contract refusal (only when nothing commits) or a containment refusal.
    Refused(&'a Refusal),
    /// Containment authorized this contract.
    Authorized(ContractId),
    /// Commit replaced the state.
    Committed {
        /// The state before.
        before: &'a ProjectionState,
        /// The state after.
        after: &'a ProjectionState,
    },
    /// Commit emitted this effect.
    Emitted(&'a Effect),
}

/// Receives step events. Returns nothing.
pub trait Observer {
    /// One event.
    fn observe(&mut self, event: &StepEvent<'_>);
}

/// Ignores every event.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoObserver;

impl Observer for NoObserver {
    fn observe(&mut self, _event: &StepEvent<'_>) {}
}

impl<O: Observer + ?Sized> Observer for &mut O {
    fn observe(&mut self, event: &StepEvent<'_>) {
        (**self).observe(event);
    }
}
