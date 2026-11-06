//! Containment and the one write path.
//!
//! `limits` counts and refuses with observed and permitted numbers.
//! `authorize` picks exactly one distinct candidate and is the only place an
//! [`AuthorizedTransition`] is built. `commit` is the only place state is
//! written. It takes an authorized transition by value.

pub(crate) mod authorize;
pub(crate) mod commit;

pub use authorize::AuthorizedTransition;
pub use commit::{Effect, ProjectionState};
