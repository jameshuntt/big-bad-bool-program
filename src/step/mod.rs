//! Containment and the one write path.
//!
//! `limits` counts and refuses with observed and permitted numbers.
//! `authorize` gathers the candidates, lets every compatible one actualize
//! together with their writes merged, refuses contradictions, and is the only
//! place an [`AuthorizedTransition`] is built. `commit` is the only place state is
//! written. It takes an authorized transition by value.

pub(crate) mod authorize;
pub(crate) mod commit;
pub(crate) mod limits;

pub use authorize::AuthorizedTransition;
pub use commit::{Effect, ProjectionState};
