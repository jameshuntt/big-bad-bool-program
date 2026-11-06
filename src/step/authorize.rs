//! Authority: exactly one distinct candidate actualizes, within its stack bounds.

use crate::codes::Refusal;
use crate::contract::Candidate;
use crate::model::{ContractId, ProgramSpec, ResolvedStackOp};
use crate::step::commit::ProjectionState;

/// The one candidate containment let through. Only [`authorize`] builds one,
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
/// The same contract from several frames counts once. Different contracts
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
