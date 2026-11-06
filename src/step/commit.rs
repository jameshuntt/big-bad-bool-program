//! Commit: the one place that writes state.
//!
//! [`ProjectionState`]'s fields are private to this file. Nothing else in the
//! crate can change them. Commit applies stack ops, then transition writes
//! (to the top entry of a stacked dimension), each through interact's
//! `patch_word`; checks every changed word with flaga_solver's `check`; builds
//! the effect words; and replaces the state only when everything passed.

use big_bad_bool_interact::ReconstructionError;

use crate::codes::Refusal;
use crate::model::{DimensionId, DimensionRole, DimensionSpec, ProgramSpec, ResolvedStackOp};
use crate::step::authorize::AuthorizedTransition;

/// The committed words: one per dimension, and one bounded stack per stacked
/// dimension. Read only outside commit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectionState {
    words: Vec<u128>,
    stacks: Vec<Vec<u128>>,
}

impl ProjectionState {
    /// The start state: every state dimension at its field defaults, and every
    /// stacked dimension holding one entry at its field defaults.
    pub(crate) fn start(spec: &ProgramSpec) -> Self {
        let words = spec
            .dims
            .iter()
            .map(|d| match (d.role(), d.stack_depth()) {
                (DimensionRole::State, None) => d.default_word(),
                _ => 0,
            })
            .collect();
        let stacks = spec
            .dims
            .iter()
            .map(|d| match d.stack_depth() {
                Some(_) => vec![d.default_word()],
                None => Vec::new(),
            })
            .collect();
        Self { words, stacks }
    }

    /// A dimension's word: the top entry of a stacked dimension (zero when
    /// empty), the committed word of a state dimension, zero otherwise.
    #[must_use]
    pub fn word(&self, dimension: DimensionId) -> u128 {
        let i = usize::from(dimension.0);
        self.stacks[i].last().copied().unwrap_or(self.words[i])
    }

    /// True when `dimension` is a dimension of this state.
    #[must_use]
    pub fn has(&self, dimension: DimensionId) -> bool {
        usize::from(dimension.0) < self.words.len()
    }

    /// The top entry of a stacked dimension.
    #[must_use]
    pub fn top(&self, dimension: DimensionId) -> Option<u128> {
        self.stacks.get(usize::from(dimension.0))?.last().copied()
    }

    /// How many entries a stacked dimension holds.
    #[must_use]
    pub fn depth(&self, dimension: DimensionId) -> usize {
        self.depth_at(usize::from(dimension.0))
    }

    pub(crate) fn depth_at(&self, index: usize) -> usize {
        self.stacks.get(index).map_or(0, Vec::len)
    }

    /// Every entry of a stacked dimension, bottom first.
    #[must_use]
    pub fn stack(&self, dimension: DimensionId) -> &[u128] {
        self.stacks
            .get(usize::from(dimension.0))
            .map_or(&[], Vec::as_slice)
    }

    /// One word per dimension for a fact view: `item` for the item dimension,
    /// `frame` for the frame dimension, each state word (the top entry when
    /// stacked, `None` when the stack is empty), and `None` for every other
    /// dimension.
    pub(crate) fn fact_words(
        &self,
        spec: &ProgramSpec,
        item: u128,
        frame: u128,
    ) -> Vec<Option<u128>> {
        spec.dims
            .iter()
            .enumerate()
            .map(|(i, d)| {
                if i == spec.item_dim {
                    Some(item)
                } else if i == spec.frame_dim {
                    Some(frame)
                } else if d.role() != DimensionRole::State {
                    None
                } else if d.stack_depth().is_some() {
                    self.stacks[i].last().copied()
                } else {
                    Some(self.words[i])
                }
            })
            .collect()
    }
}

/// One effect word, emitted at one input position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Effect {
    /// The effect dimension.
    pub dimension: DimensionId,
    /// The effect word: the dimension's default word with the emit writes applied.
    pub word: u128,
    /// The input position. The application maps it to its own text.
    pub position: u32,
}

fn illegal(position: u32, dim: &DimensionSpec) -> impl Fn(ReconstructionError) -> Refusal + '_ {
    move |e| Refusal::IllegalWord {
        position,
        dimension: dim.name(),
        verdict: e.to_string(),
    }
}

fn check(position: u32, dim: &DimensionSpec, word: u128) -> Result<(), Refusal> {
    let verdict = flaga_solver::check(dim.schema(), word);
    if verdict.is_ok_ignoring_deprecated() {
        Ok(())
    } else {
        Err(Refusal::IllegalWord {
            position,
            dimension: dim.name(),
            verdict: verdict.to_string(),
        })
    }
}

/// Applies an authorized transition. On any refusal the state is left as it was.
pub(crate) fn commit(
    spec: &ProgramSpec,
    state: &mut ProjectionState,
    transition: AuthorizedTransition,
) -> Result<Vec<Effect>, Refusal> {
    let position = transition.position();
    let c = &spec.resolved[usize::from(transition.contract().0)];
    let mut next = state.clone();
    let mut changed: Vec<usize> = Vec::new();

    for op in &c.stack {
        match op {
            ResolvedStackOp::Pop { dim } => {
                next.stacks[*dim].pop().ok_or(Refusal::StackUnderflow {
                    position,
                    dimension: spec.dims[*dim].name(),
                })?;
                changed.push(*dim);
            }
            ResolvedStackOp::Push { dim, values } => {
                let d = &spec.dims[*dim];
                let word = d
                    .patch(d.default_word(), values)
                    .map_err(illegal(position, d))?;
                next.stacks[*dim].push(word);
                changed.push(*dim);
            }
        }
    }

    for writes in &c.transition {
        let d = &spec.dims[writes.dim];
        let slot = if d.stack_depth().is_some() {
            next.stacks[writes.dim]
                .last_mut()
                .ok_or(Refusal::StackUnderflow {
                    position,
                    dimension: d.name(),
                })?
        } else {
            &mut next.words[writes.dim]
        };
        *slot = d
            .patch(*slot, &writes.values)
            .map_err(illegal(position, d))?;
        changed.push(writes.dim);
    }

    changed.sort_unstable();
    changed.dedup();
    for dim in changed {
        if let Some(&word) = next.stacks[dim]
            .last()
            .or_else(|| (spec.dims[dim].stack_depth().is_none()).then_some(&next.words[dim]))
        {
            check(position, &spec.dims[dim], word)?;
        }
    }

    let mut effects = Vec::with_capacity(c.emit.len());
    for writes in &c.emit {
        let d = &spec.dims[writes.dim];
        let word = d
            .patch(d.default_word(), &writes.values)
            .map_err(illegal(position, d))?;
        check(position, d, word)?;
        effects.push(Effect {
            dimension: DimensionId(writes.dim as u16),
            word,
            position,
        });
    }

    *state = next;
    Ok(effects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Candidate;
    use crate::model::{ContractId, ContractSpec, contract, pop, push, set};
    use crate::step::authorize::authorize;
    use crate::test_support::{SET_INLINE, SET_TIMER, WHEN_B, spec};
    use big_bad_bool_interact::FieldValue;

    const TIMER_INLINE: ContractSpec = contract("timer-inline")
        .when(WHEN_B)
        .transition(&[SET_TIMER, SET_INLINE])
        .emit(&[set("out", "kind", 1)]);
    const TIMER_ONLY: ContractSpec = contract("timer-only").when(WHEN_B).transition(&[SET_TIMER]);
    const PUSH_INNER: ContractSpec = contract("push-inner")
        .when(WHEN_B)
        .stack(&[push("scope", &[FieldValue::new("level", 1)])]);
    const POP: ContractSpec = contract("pop").when(WHEN_B).stack(&[pop("scope")]);
    const EMIT_INVALID: ContractSpec = contract("emit-invalid")
        .when(WHEN_B)
        .emit(&[set("out", "kind", 3)]);

    fn run(
        spec: &ProgramSpec,
        state: &mut ProjectionState,
        id: u16,
    ) -> Result<Vec<Effect>, Refusal> {
        let candidate = Candidate {
            contract: ContractId(id),
            position: 7,
            frame: None,
        };
        let t = authorize(spec, state, &[candidate], 7, 0)?;
        commit(spec, state, t)
    }

    #[test]
    fn commit_writes_state_and_effects() {
        let spec = spec(&[TIMER_INLINE]);
        let mut state = ProjectionState::start(&spec);
        assert_eq!(state.word(DimensionId(2)), 0);
        let effects = run(&spec, &mut state, 0).unwrap();
        assert_eq!(state.word(DimensionId(2)), 0b01_001);
        assert_eq!(
            effects,
            [Effect {
                dimension: DimensionId(4),
                word: 1,
                position: 7
            }]
        );
    }

    #[test]
    fn an_illegal_word_is_refused_and_rolled_back() {
        let spec = spec(&[TIMER_ONLY]);
        let mut state = ProjectionState::start(&spec);
        let before = state.clone();
        let refusal = run(&spec, &mut state, 0).unwrap_err();
        assert_eq!(
            refusal,
            Refusal::IllegalWord {
                position: 7,
                dimension: "job",
                verdict: "`reason`=timer requires `profile`=inline_queue".to_owned(),
            }
        );
        assert_eq!(state, before);
    }

    #[test]
    fn an_illegal_effect_rolls_back_the_whole_step() {
        let spec = spec(&[EMIT_INVALID]);
        let mut state = ProjectionState::start(&spec);
        let before = state.clone();
        let refusal = run(&spec, &mut state, 0).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "[BBBC0105] illegal `out` word at position 7: field `kind` = 3 is invalid"
        );
        assert_eq!(state, before);
    }

    #[test]
    fn stacks_push_and_pop_within_bounds() {
        let spec = spec(&[PUSH_INNER, POP]);
        let mut state = ProjectionState::start(&spec);
        let scope = DimensionId(3);
        assert_eq!(state.stack(scope), [0]);
        run(&spec, &mut state, 0).unwrap();
        assert_eq!(state.stack(scope), [0, 1]);
        assert_eq!(state.top(scope), Some(1));
        let refusal = run(&spec, &mut state, 0).unwrap_err();
        assert_eq!(
            refusal,
            Refusal::StackOverflow {
                position: 7,
                dimension: "scope",
                depth: 3,
                permitted: 2
            }
        );
        run(&spec, &mut state, 1).unwrap();
        run(&spec, &mut state, 1).unwrap();
        assert_eq!(state.depth(scope), 0);
        assert_eq!(state.top(scope), None);
        let refusal = run(&spec, &mut state, 1).unwrap_err();
        assert_eq!(
            refusal,
            Refusal::StackUnderflow {
                position: 7,
                dimension: "scope"
            }
        );
    }
}
