//! Authority: every compatible candidate actualizes, together, within its
//! limits and stack bounds.
//!
//! Containment takes the candidates of one position and merges the writes of
//! every distinct contract among them into one plan:
//! - the same contract under several frames counts once;
//! - a field written by several contracts must get one value from all of
//!   them; equal values count once, different values are a `Conflict`;
//! - every contract that pushes, pops or writes a stacked dimension must
//!   carry the same stack ops on it; equal stack ops apply once;
//! - effect writes merge the same way, one effect word per effect dimension;
//! - contracts that read a frame field must agree on its declared value:
//!   two frame interpretations whose sequences declare different values for
//!   a field are both fine, until actualizing contracts read that field
//!   under both. Then it is a `Conflict`. A field a sequence does not declare
//!   constrains nothing, so a longer sequence may refine a shorter one.
//!
//! The merge reads contracts in name order and the plan is kept in
//! dimension and field order, so the result does not depend on the order
//! contracts were declared in. An exactly-one policy is the limit
//! `max_actualized = 1`.

use big_bad_bool_interact::FieldValue;

use crate::codes::Refusal;
use crate::contract::{Candidate, FactView, Outcome, evaluate};
use crate::frame::FrameFacts;
use crate::model::{
    ContractId, DimWrites, Observation, ProgramSpec, ResolvedContract, ResolvedStackOp, SequenceId,
    render_value,
};
use crate::step::commit::ProjectionState;
use crate::step::limits::{Limit, within};

/// What commit applies: the merged writes of every actualized contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Stack ops per stacked dimension, in dimension order. Applied first.
    pub(crate) stack: Vec<(usize, Vec<ResolvedStackOp>)>,
    /// One value per field per state dimension, in dimension and field
    /// order. A stacked dimension's values go to its top entry.
    pub(crate) transition: Vec<(usize, Vec<FieldValue>)>,
    /// One value per field per effect dimension: one effect word each.
    pub(crate) emit: Vec<(usize, Vec<FieldValue>)>,
}

/// The candidates containment let through. Only `step::authorize` builds one,
/// and only commit consumes it. It is not `Clone`: it is used once.
#[derive(Debug, PartialEq, Eq)]
pub struct AuthorizedTransition {
    contracts: Vec<ContractId>,
    position: u32,
    plan: Plan,
}

impl AuthorizedTransition {
    /// The contracts that actualize together, in id order.
    #[must_use]
    pub fn contracts(&self) -> &[ContractId] {
        &self.contracts
    }

    /// The input position.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position
    }

    pub(crate) const fn plan(&self) -> &Plan {
        &self.plan
    }
}

/// What evaluation found at one position.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gathered {
    /// Every contract that held, once per frame interpretation it held under.
    pub(crate) candidates: Vec<Candidate>,
    /// One refusal per contract whose WHEN held but that is no candidate
    /// under any interpretation, in contract name order.
    pub(crate) refused: Vec<Refusal>,
}

/// Collects the applicable candidates of one position: every contract in
/// `ids` under every frame interpretation in `frames`, or under no frame when
/// there is none. Shared references only.
pub(crate) fn gather(
    spec: &ProgramSpec,
    state: &ProjectionState,
    ids: &[ContractId],
    observation: &Observation,
    frames: &[FrameFacts],
) -> Gathered {
    let position = observation.position();
    let views: Vec<Option<&FrameFacts>> = if frames.is_empty() {
        vec![None]
    } else {
        frames.iter().map(Some).collect()
    };
    let mut candidates = Vec::new();
    let mut refused: Vec<(ContractId, Refusal)> = Vec::new();
    for frame in views {
        let words = state.fact_words(spec, observation, frame.map_or(0, |f| f.word));
        let view = FactView::new(spec, &words, position, frame.copied());
        for &id in ids {
            match evaluate(id, &view) {
                Outcome::NotApplicable => {}
                Outcome::Candidate(c) => candidates.push(c),
                Outcome::Refused(r) => {
                    if !refused.iter().any(|(seen, _)| *seen == id) {
                        refused.push((id, r));
                    }
                }
            }
        }
    }
    refused.retain(|(id, _)| !candidates.iter().any(|c| c.contract == *id));
    // Name order, so the refusals do not depend on the order contracts were declared in.
    refused.sort_by_key(|(id, _)| spec.resolved[usize::from(id.0)].name);
    Gathered {
        candidates,
        refused: refused.into_iter().map(|(_, r)| r).collect(),
    }
}

/// Picks the transition for one position.
///
/// The candidate count is checked against `max_candidates` first, then the
/// distinct contracts against `max_actualized`. The frame facts they read
/// must agree, and their writes are merged (see the module notes); a
/// contradiction is a `Conflict`. Stack ops must stay
/// within each dimension's depth. `refused` is how many contracts were
/// refused here; it goes into `NoContract`.
pub(crate) fn authorize(
    spec: &ProgramSpec,
    state: &ProjectionState,
    candidates: &[Candidate],
    position: u32,
    refused: usize,
) -> Result<AuthorizedTransition, Refusal> {
    let limits = &spec.containment;
    within(
        Limit::Candidates,
        candidates.len(),
        limits.max_candidates,
        position,
    )?;
    let mut ids: Vec<ContractId> = candidates.iter().map(|c| c.contract).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Err(Refusal::NoContract { position, refused });
    }
    within(
        Limit::Actualized,
        ids.len(),
        limits.max_actualized,
        position,
    )?;
    let mut by_name: Vec<&ResolvedContract> = ids
        .iter()
        .map(|id| &spec.resolved[usize::from(id.0)])
        .collect();
    by_name.sort_by_key(|c| c.name);
    facts(spec, candidates, position)?;
    let plan = merge(spec, &by_name, position)?;
    bounds(spec, state, &plan, position)?;
    Ok(AuthorizedTransition {
        contracts: ids,
        position,
        plan,
    })
}

fn conflict(position: u32, first: &'static str, second: &'static str, target: String) -> Refusal {
    Refusal::Conflict {
        position,
        first,
        second,
        target,
    }
}

/// Every frame field an actualizing candidate reads and its frame's sequence
/// declares must have one declared value across all such candidates.
fn facts(spec: &ProgramSpec, candidates: &[Candidate], position: u32) -> Result<(), Refusal> {
    let Some(frame_dim) = spec.frame_dim() else {
        return Ok(());
    };
    let sequences = spec.sequences();
    let mut seen: Vec<(&'static str, u128, &'static str, SequenceId, u8)> = Vec::new();
    for c in candidates {
        let Some(frame) = c.frame else { continue };
        let contract = &spec.resolved[usize::from(c.contract.0)];
        let reads = contract.reads(frame_dim);
        for v in sequences[usize::from(frame.sequence.0)].declared(usize::from(frame.offset)) {
            if reads.contains(&v.field) {
                seen.push((
                    v.field,
                    v.value,
                    contract.name,
                    frame.sequence,
                    frame.offset,
                ));
            }
        }
    }
    let dim = &spec.dims[frame_dim];
    let name = |s: SequenceId| sequences[usize::from(s.0)].name;
    for field in dim.schema().fields {
        let mut on: Vec<_> = seen.iter().filter(|e| e.0 == field.name).collect();
        on.sort_by_key(|e| (e.2, name(e.3), e.4));
        let Some(first) = on.first() else { continue };
        if let Some(other) = on.iter().find(|e| e.1 != first.1) {
            let says = |e: &(&str, u128, &str, SequenceId, u8)| {
                format!(
                    "{}@{} says {}",
                    name(e.3),
                    e.4,
                    render_value(dim.schema(), field, e.1)
                )
            };
            return Err(conflict(
                position,
                first.2,
                other.2,
                format!(
                    "{}.{}: {}, {}",
                    dim.name(),
                    field.name,
                    says(first),
                    says(other)
                ),
            ));
        }
    }
    Ok(())
}

/// Merges the writes of `contracts`, given in name order.
fn merge(
    spec: &ProgramSpec,
    contracts: &[&ResolvedContract],
    position: u32,
) -> Result<Plan, Refusal> {
    let mut plan = Plan::default();
    for (dim, d) in spec.dims.iter().enumerate() {
        if d.stack_depth().is_none() {
            continue;
        }
        let mut touching = contracts.iter().filter(|c| c.touches_stack(dim));
        let Some(first) = touching.next() else {
            continue;
        };
        let ops = first.stack_on(dim);
        if let Some(other) = touching.find(|c| c.stack_on(dim) != ops) {
            return Err(conflict(
                position,
                first.name,
                other.name,
                format!("{} stack", d.name()),
            ));
        }
        if !ops.is_empty() {
            plan.stack.push((dim, ops));
        }
    }
    plan.transition = fields(spec, contracts, |c| &c.transition, position)?;
    plan.emit = fields(spec, contracts, |c| &c.emit, position)?;
    Ok(plan)
}

/// One value per (dimension, field) over every contract's writes. Within one
/// contract the last write to a field is its value.
fn fields(
    spec: &ProgramSpec,
    contracts: &[&ResolvedContract],
    pick: impl Fn(&ResolvedContract) -> &[DimWrites],
    position: u32,
) -> Result<Vec<(usize, Vec<FieldValue>)>, Refusal> {
    let mut merged: Vec<(usize, Vec<(FieldValue, &'static str)>)> = Vec::new();
    for c in contracts {
        for writes in pick(c) {
            let slot = match merged.iter().position(|(d, _)| *d == writes.dim) {
                Some(i) => i,
                None => {
                    merged.push((writes.dim, Vec::new()));
                    merged.len() - 1
                }
            };
            let mut own: Vec<FieldValue> = Vec::new();
            for v in &writes.values {
                own.retain(|o| o.field != v.field);
                own.push(*v);
            }
            for v in own {
                let entries = &mut merged[slot].1;
                match entries.iter().find(|(seen, _)| seen.field == v.field) {
                    Some((seen, owner)) if seen.value != v.value => {
                        return Err(conflict(
                            position,
                            owner,
                            c.name,
                            format!("{}.{}", spec.dims[writes.dim].name(), v.field),
                        ));
                    }
                    Some(_) => {}
                    None => entries.push((v, c.name)),
                }
            }
        }
    }
    merged.sort_by_key(|(d, _)| *d);
    Ok(merged
        .into_iter()
        .map(|(d, mut entries)| {
            entries.sort_by_key(|(v, _)| v.field);
            (d, entries.into_iter().map(|(v, _)| v).collect())
        })
        .collect())
}

/// Stack ops must stay within each dimension's depth, and a field write to a
/// stacked dimension needs an entry to write.
fn bounds(
    spec: &ProgramSpec,
    state: &ProjectionState,
    plan: &Plan,
    position: u32,
) -> Result<(), Refusal> {
    let mut depths: Vec<usize> = (0..spec.dims.len()).map(|d| state.depth_at(d)).collect();
    for (dim, ops) in &plan.stack {
        let name = spec.dims[*dim].name();
        for op in ops {
            match op {
                ResolvedStackOp::Pop { .. } => {
                    if depths[*dim] == 0 {
                        return Err(Refusal::StackUnderflow {
                            position,
                            dimension: name,
                        });
                    }
                    depths[*dim] -= 1;
                }
                ResolvedStackOp::Push { .. } => {
                    let permitted = spec.dims[*dim].stack_depth().map_or(0, usize::from);
                    if depths[*dim] + 1 > permitted {
                        return Err(Refusal::StackOverflow {
                            position,
                            dimension: name,
                            depth: depths[*dim] + 1,
                            permitted,
                        });
                    }
                    depths[*dim] += 1;
                }
            }
        }
    }
    for (dim, _) in &plan.transition {
        if spec.dims[*dim].stack_depth().is_some() && depths[*dim] == 0 {
            return Err(Refusal::StackUnderflow {
                position,
                dimension: spec.dims[*dim].name(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProgramSpec;
    use crate::model::{ContractSpec, contract, pop, push, set};
    use crate::test_support::{SET_INLINE, SET_TIMER, WHEN_B, spec};

    const BOTH: ContractSpec = contract("both")
        .when(WHEN_B)
        .transition(&[SET_TIMER, SET_INLINE]);
    const INLINE: ContractSpec = contract("inline").when(WHEN_B).transition(&[SET_INLINE]);
    const TWIN: ContractSpec = contract("twin")
        .when(WHEN_B)
        .transition(&[SET_TIMER, SET_INLINE]);
    const POOLED: ContractSpec = contract("pooled")
        .when(WHEN_B)
        .transition(&[set("job", "profile", 0)]);
    const SAID: ContractSpec = contract("said").when(WHEN_B).emit(&[set("out", "kind", 1)]);
    const QUIET: ContractSpec = contract("quiet")
        .when(WHEN_B)
        .emit(&[set("out", "kind", 0)]);
    const PUSH_INNER: ContractSpec = contract("push-inner")
        .when(WHEN_B)
        .stack(&[push("scope", &[FieldValue::new("level", 1)])]);
    const PUSH_INNER_TOO: ContractSpec = contract("push-inner-too")
        .when(WHEN_B)
        .stack(&[push("scope", &[FieldValue::new("level", 1)])]);
    const POP_SCOPE: ContractSpec = contract("pop-scope").when(WHEN_B).stack(&[pop("scope")]);
    const WRITE_SCOPE: ContractSpec = contract("write-scope")
        .when(WHEN_B)
        .transition(&[set("scope", "level", 0)]);

    fn candidates(ids: &[u16]) -> Vec<Candidate> {
        ids.iter()
            .map(|id| Candidate {
                contract: ContractId(*id),
                position: 5,
                frame: None,
            })
            .collect()
    }

    fn conflict(first: &'static str, second: &'static str, target: &str) -> Refusal {
        Refusal::Conflict {
            position: 5,
            first,
            second,
            target: target.to_owned(),
        }
    }

    #[test]
    fn compatible_writes_actualize_together() {
        // both {timer, inline} and inline {inline}: inline agrees, so both actualize.
        let spec = spec(&[BOTH, INLINE, SAID]);
        let state = ProjectionState::start(&spec);
        let t = authorize(&spec, &state, &candidates(&[1, 0, 2]), 5, 0).unwrap();
        assert_eq!(t.contracts(), [ContractId(0), ContractId(1), ContractId(2)]);
        assert_eq!(
            t.plan().transition,
            [(
                2,
                vec![FieldValue::new("profile", 1), FieldValue::new("reason", 1)]
            )]
        );
        assert_eq!(t.plan().emit, [(4, vec![FieldValue::new("kind", 1)])]);
    }

    #[test]
    fn contradictory_writes_conflict() {
        let spec = spec(&[BOTH, INLINE, POOLED, SAID, QUIET]);
        let state = ProjectionState::start(&spec);
        // Named in name order, whatever the ids: both < inline < pooled.
        assert_eq!(
            authorize(&spec, &state, &candidates(&[2, 1]), 5, 0),
            Err(conflict("inline", "pooled", "job.profile"))
        );
        assert_eq!(
            authorize(&spec, &state, &candidates(&[0, 1, 2]), 5, 0),
            Err(conflict("both", "pooled", "job.profile"))
        );
        assert_eq!(
            authorize(&spec, &state, &candidates(&[4, 3]), 5, 0),
            Err(conflict("quiet", "said", "out.kind"))
        );
    }

    #[test]
    fn equal_writes_count_once() {
        let spec = spec(&[BOTH, INLINE, TWIN]);
        let state = ProjectionState::start(&spec);
        // One contract under two frames.
        let t = authorize(&spec, &state, &candidates(&[1, 1]), 5, 0).unwrap();
        assert_eq!((t.contracts(), t.position()), (&[ContractId(1)][..], 5));
        // Two contracts with equal writes: both actualize, each value once.
        let t = authorize(&spec, &state, &candidates(&[2, 0]), 5, 0).unwrap();
        assert_eq!(t.contracts(), [ContractId(0), ContractId(2)]);
        assert_eq!(
            t.plan().transition,
            [(
                2,
                vec![FieldValue::new("profile", 1), FieldValue::new("reason", 1)]
            )]
        );
    }

    #[test]
    fn stack_ops_on_one_dimension_must_agree() {
        let spec = spec(&[PUSH_INNER, PUSH_INNER_TOO, POP_SCOPE, WRITE_SCOPE]);
        let state = ProjectionState::start(&spec);
        // Equal pushes apply once.
        let t = authorize(&spec, &state, &candidates(&[0, 1]), 5, 0).unwrap();
        assert_eq!(
            t.plan().stack,
            [(
                3,
                vec![ResolvedStackOp::Push {
                    dim: 3,
                    values: vec![FieldValue::new("level", 1)]
                }]
            )]
        );
        // A push and a pop on one stack contradict.
        assert_eq!(
            authorize(&spec, &state, &candidates(&[2, 0]), 5, 0),
            Err(conflict("pop-scope", "push-inner", "scope stack"))
        );
        // So do a push and a field write meant for the entry below it.
        assert_eq!(
            authorize(&spec, &state, &candidates(&[3, 1]), 5, 0),
            Err(conflict("push-inner-too", "write-scope", "scope stack"))
        );
    }

    mod frames {
        use super::*;
        use crate::model::{
            ItemMatch, OffsetFacts, RecognizerSpec, SequenceSpec, clause, sequence,
        };
        use crate::test_support::{RECOGNIZER, builder};
        use big_bad_bool_interact::FieldMatcher;

        const A: ItemMatch = ItemMatch::One(FieldMatcher::eq("kind", 0));
        const B: ItemMatch = ItemMatch::One(FieldMatcher::eq("kind", 1));
        const C: ItemMatch = ItemMatch::One(FieldMatcher::eq("kind", 2));
        // At a `b` with `a` before and `c` after, all three hold together.
        const BC: SequenceSpec = sequence("bc", &[B, C]).facts(&[OffsetFacts {
            offset: 0,
            values: &[FieldValue::new("role", 1)],
        }]);
        const AB: SequenceSpec = sequence("ab", &[A, B]).facts(&[OffsetFacts {
            offset: 1,
            values: &[FieldValue::new("role", 2)],
        }]);
        const B_ONLY: SequenceSpec = sequence("b-only", &[B]);

        const READS_ONE: ContractSpec = contract("reads-one")
            .when(&[
                clause("item", FieldMatcher::eq("kind", 1)),
                clause("frame", FieldMatcher::eq("role", 1)),
            ])
            .transition(&[SET_INLINE]);
        const READS_TWO: ContractSpec = contract("reads-two")
            .when(&[
                clause("item", FieldMatcher::eq("kind", 1)),
                clause("frame", FieldMatcher::eq("role", 2)),
            ])
            .emit(&[set("out", "kind", 1)]);
        const READS_ANY: ContractSpec = contract("reads-any").when(&[
            clause("item", FieldMatcher::eq("kind", 1)),
            clause("frame", FieldMatcher::any("role")),
        ]);
        const IGNORES: ContractSpec = contract("ignores").when(WHEN_B);

        fn framed() -> ProgramSpec {
            [READS_ONE, READS_TWO, READS_ANY, IGNORES]
                .iter()
                .fold(
                    builder().recognize(RecognizerSpec {
                        sequences: &[BC, AB, B_ONLY],
                        ..RECOGNIZER
                    }),
                    |b, c| b.contract(*c),
                )
                .build()
                .unwrap()
        }

        fn via(contract: u16, sequence: u16, offset: u8) -> Candidate {
            Candidate {
                contract: ContractId(contract),
                position: 5,
                frame: Some(FrameFacts {
                    sequence: SequenceId(sequence),
                    offset,
                    word: 0,
                }),
            }
        }

        #[test]
        fn contracts_that_read_contradictory_facts_conflict() {
            let spec = framed();
            let state = ProjectionState::start(&spec);
            // reads-one saw role 1 under bc, reads-two saw role 2 under ab.
            assert_eq!(
                authorize(&spec, &state, &[via(1, 1, 1), via(0, 0, 0)], 5, 0),
                Err(conflict(
                    "reads-one",
                    "reads-two",
                    "frame.role: bc@0 says 1, ab@1 says 2"
                ))
            );
            // One contract reading role under both is a conflict with itself.
            assert_eq!(
                authorize(&spec, &state, &[via(2, 0, 0), via(2, 1, 1)], 5, 0),
                Err(conflict(
                    "reads-any",
                    "reads-any",
                    "frame.role: ab@1 says 2, bc@0 says 1"
                ))
            );
        }

        #[test]
        fn facts_nobody_reads_or_nobody_declares_coexist() {
            let spec = framed();
            let state = ProjectionState::start(&spec);
            // `ignores` reads no frame field: the two placements may disagree.
            let t = authorize(
                &spec,
                &state,
                &[via(3, 0, 0), via(3, 1, 1), via(0, 0, 0)],
                5,
                0,
            )
            .unwrap();
            assert_eq!(t.contracts(), [ContractId(0), ContractId(3)]);
            // b-only declares no role: it refines nothing and contradicts nothing.
            assert!(authorize(&spec, &state, &[via(0, 0, 0), via(2, 2, 0)], 5, 0).is_ok());
        }
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
    fn contracts_actualized_together_are_limited() {
        // test_support permits three.
        let spec = spec(&[BOTH, INLINE, TWIN, SAID]);
        let state = ProjectionState::start(&spec);
        assert!(authorize(&spec, &state, &candidates(&[0, 1, 2]), 5, 0).is_ok());
        assert_eq!(
            authorize(&spec, &state, &candidates(&[0, 1, 2, 3]), 5, 0),
            Err(Refusal::LimitExceeded {
                position: 5,
                limit: "actualized",
                observed: 4,
                permitted: 3
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
