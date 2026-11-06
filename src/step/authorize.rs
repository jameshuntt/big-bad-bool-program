//! Authority: exactly one distinct candidate actualizes, within its stack bounds.

use crate::codes::Refusal;
use crate::contract::Candidate;
use crate::model::{ContractId, ProgramSpec, ResolvedStackOp};
use crate::step::commit::ProjectionState;
use crate::step::limits::{Limit, within};

/// The one candidate containment let through. Only `step::authorize` builds one,
/// and only commit consumes it. It is not `Clone`: it is used once.
#[derive(Debug, PartialEq, Eq)]
pub struct AuthorizedTransition {
    contract: ContractId,
    position: u32,
}

impl AuthorizedTransition {
    /// The contract whose writes commit applies.
    #[must_use]
    pub const fn contract(&self) -> ContractId {
        self.contract
    }

    /// The input position.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position
    }
}

/// Picks the transition for one position.
///
/// The candidate count is checked against `max_candidates` first. The same contract from several frames counts once. Different contracts
/// with equal writes count once, and the lowest id is committed. Different
/// writes are ambiguous. Stack ops must stay within each dimension's depth.
/// `refused` is how many contracts were refused here; it goes into
/// `NoContract`.
pub(crate) fn authorize(
    spec: &ProgramSpec,
    state: &ProjectionState,
    candidates: &[Candidate],
    position: u32,
    refused: usize,
) -> Result<AuthorizedTransition, Refusal> {
    within(
        Limit::Candidates,
        candidates.len(),
        spec.containment.max_candidates,
        position,
    )?;
    let mut ids: Vec<ContractId> = candidates.iter().map(|c| c.contract).collect();
    ids.sort_unstable();
    ids.dedup();
    let Some(&winner) = ids.first() else {
        return Err(Refusal::NoContract { position, refused });
    };
    let chosen = &spec.resolved[usize::from(winner.0)];
    for other in &ids[1..] {
        let other = &spec.resolved[usize::from(other.0)];
        if !chosen.same_writes(other) {
            return Err(Refusal::Ambiguous {
                position,
                first: chosen.name,
                second: other.name,
            });
        }
    }

    let mut depths: Vec<usize> = (0..spec.dims.len()).map(|d| state.depth_at(d)).collect();
    for op in &chosen.stack {
        match op {
            ResolvedStackOp::Pop { dim } => {
                if depths[*dim] == 0 {
                    return Err(Refusal::StackUnderflow {
                        position,
                        dimension: spec.dims[*dim].name(),
                    });
                }
                depths[*dim] -= 1;
            }
            ResolvedStackOp::Push { dim, .. } => {
                let permitted = spec.dims[*dim].stack_depth().map_or(0, usize::from);
                if depths[*dim] + 1 > permitted {
                    return Err(Refusal::StackOverflow {
                        position,
                        dimension: spec.dims[*dim].name(),
                        depth: depths[*dim] + 1,
                        permitted,
                    });
                }
                depths[*dim] += 1;
            }
        }
    }
    for writes in &chosen.transition {
        if spec.dims[writes.dim].stack_depth().is_some() && depths[writes.dim] == 0 {
            return Err(Refusal::StackUnderflow {
                position,
                dimension: spec.dims[writes.dim].name(),
            });
        }
    }

    Ok(AuthorizedTransition {
        contract: winner,
        position,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContractSpec, contract};
    use crate::test_support::{SET_INLINE, SET_TIMER, WHEN_B, spec};

    const BOTH: ContractSpec = contract("both")
        .when(WHEN_B)
        .transition(&[SET_TIMER, SET_INLINE]);
    const INLINE: ContractSpec = contract("inline").when(WHEN_B).transition(&[SET_INLINE]);
    const TWIN: ContractSpec = contract("twin")
        .when(WHEN_B)
        .transition(&[SET_TIMER, SET_INLINE]);

    fn candidates(ids: &[u16]) -> Vec<Candidate> {
        ids.iter()
            .enumerate()
            .map(|(frame, id)| Candidate {
                contract: ContractId(*id),
                position: 5,
                frame: Some(crate::model::SequenceId(frame as u16)),
            })
            .collect()
    }

    #[test]
    fn different_writes_are_ambiguous() {
        let spec = spec(&[BOTH, INLINE]);
        let state = ProjectionState::start(&spec);
        assert_eq!(
            authorize(&spec, &state, &candidates(&[1, 0]), 5, 0),
            Err(Refusal::Ambiguous {
                position: 5,
                first: "both",
                second: "inline"
            })
        );
    }

    #[test]
    fn equal_writes_count_once() {
        let spec = spec(&[BOTH, INLINE, TWIN]);
        let state = ProjectionState::start(&spec);
        // One contract under two frames.
        let t = authorize(&spec, &state, &candidates(&[1, 1]), 5, 0).unwrap();
        assert_eq!((t.contract(), t.position()), (ContractId(1), 5));
        // Two contracts with equal writes: the lowest id commits.
        let t = authorize(&spec, &state, &candidates(&[2, 0]), 5, 0).unwrap();
        assert_eq!(t.contract(), ContractId(0));
    }

    #[test]
    fn candidates_are_counted_before_merging() {
        let spec = spec(&[BOTH]);
        let state = ProjectionState::start(&spec);
        assert!(authorize(&spec, &state, &candidates(&[0; 4]), 5, 0).is_ok());
        assert_eq!(
            authorize(&spec, &state, &candidates(&[0; 5]), 5, 0),
            Err(Refusal::LimitExceeded {
                position: 5,
                limit: "candidates",
                observed: 5,
                permitted: 4
            })
        );
    }

    #[test]
    fn no_candidate_is_default_deny() {
        let spec = spec(&[BOTH]);
        let state = ProjectionState::start(&spec);
        assert_eq!(
            authorize(&spec, &state, &[], 5, 2),
            Err(Refusal::NoContract {
                position: 5,
                refused: 2
            })
        );
    }
}
