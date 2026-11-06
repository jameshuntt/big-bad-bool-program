//! The stepper: a verified program, a stream of observations, one commit per position.
//!
//! The focal item at index `i` is stepped once item `i + R` has arrived (R is
//! the window radius). `finish` pads with the end item, steps every remaining
//! item, then steps the end item itself at the position after the last
//! observation. Per position: admit the observation's evidence, frame,
//! evaluate every contract under every interpretation, authorize, commit.
//! Observers see each stage.
//!
//! Admission happens on `push`. An observation holding a word for a
//! dimension that is not observed evidence, or a word its schema refuses, is
//! refused at its own step and holds no evidence for its neighbours' windows.

use std::collections::VecDeque;

use crate::codes::Refusal;
use crate::contract::{FactView, Outcome, evaluate};
use crate::frame::{FrameFacts, Recognizer};
use crate::model::{ContractId, DimensionRole, Observation, ProgramSpec, RefusalRule};
use crate::observe::{Observer, StepEvent};
use crate::step::authorize::authorize;
use crate::step::commit::{check, commit};
use crate::step::limits::{Limit, within};
use crate::step::{Effect, ProjectionState};
use crate::verify::VerifiedProgram;

/// What happened at one position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    /// The input position.
    pub position: u32,
    /// The contracts that actualized together, in id order. Empty when nothing committed.
    pub committed: Vec<ContractId>,
    /// The effects they emitted: one word per effect dimension, in dimension order.
    pub effects: Vec<Effect>,
    /// When nothing committed: the contract refusals (for `NoContract` only)
    /// and then the containment or commit refusal.
    pub refusals: Vec<Refusal>,
}

/// Steps one input stream against one verified program.
#[derive(Debug)]
pub struct Stepper<'p, O: Observer> {
    program: &'p VerifiedProgram,
    state: ProjectionState,
    buffer: VecDeque<(Observation, Option<Refusal>)>,
    base: usize,
    received: usize,
    next: usize,
    end_position: u32,
    halted: bool,
    finished: bool,
    observer: O,
}

impl<'p, O: Observer> Stepper<'p, O> {
    /// A stepper at the program's start state.
    pub fn new(program: &'p VerifiedProgram, observer: O) -> Self {
        Self {
            program,
            state: ProjectionState::start(program.spec()),
            buffer: VecDeque::new(),
            base: 0,
            received: 0,
            next: 0,
            end_position: 0,
            halted: false,
            finished: false,
            observer,
        }
    }

    /// Adds one observation and steps every position it completes the window for.
    /// After a `Stop` refusal or after `finish`, nothing more is stepped.
    pub fn push(&mut self, observation: Observation) -> Vec<StepResult> {
        if self.halted || self.finished {
            return Vec::new();
        }
        self.end_position = observation.position().saturating_add(1);
        let refusal = admit(self.program.spec(), &observation).err();
        self.buffer.push_back((observation, refusal));
        self.received += 1;
        let radius = self.program.spec().window_radius();
        let mut results = Vec::new();
        while !self.halted && self.next + radius < self.received {
            results.push(self.step(false));
        }
        results
    }

    /// Steps every remaining position, then the end item. A program
    /// without a recognizer has no end item and nothing left to step.
    pub fn finish(&mut self) -> Vec<StepResult> {
        if self.finished {
            return Vec::new();
        }
        let mut results = Vec::new();
        let last = self.received + usize::from(self.program.spec().end_word().is_some());
        while !self.halted && self.next < last {
            results.push(self.step(true));
        }
        self.finished = true;
        results
    }

    /// The committed state.
    #[must_use]
    pub const fn state(&self) -> &ProjectionState {
        &self.state
    }

    /// The observer.
    #[must_use]
    pub const fn observer(&self) -> &O {
        &self.observer
    }

    /// The observer, ending the stepper.
    pub fn into_observer(self) -> O {
        self.observer
    }

    /// True after a `Stop` refusal.
    #[must_use]
    pub const fn is_halted(&self) -> bool {
        self.halted
    }

    /// The observations some contract would accept and containment would
    /// authorize at the next focal position, from the current state, over
    /// every frame interpretation the recognizer can produce for the item.
    /// Each is one of [`VerifiedProgram::inputs`], at the next position.
    /// Read only: prediction may rank this set, never add to it.
    #[must_use]
    pub fn legal_next(&self) -> Vec<Observation> {
        let program = self.program;
        let spec = program.spec();
        let recognizer = program.recognizer();
        let frames = recognizer.map(Recognizer::universe).unwrap_or_default();
        let next = u32::try_from(self.next).unwrap_or(u32::MAX);
        program
            .inputs()
            .iter()
            .filter(|input| {
                let item = spec.item_dimension().and_then(|d| input.word(d));
                let mut views = std::iter::once(None).chain(
                    frames
                        .iter()
                        .filter(|f| item.zip(recognizer).is_some_and(|(w, r)| r.accepts(f, w)))
                        .map(Some),
                );
                views.any(|frame: Option<&FrameFacts>| {
                    let words = self
                        .state
                        .fact_words(spec, input, frame.map_or(0, |f| f.word));
                    let view = FactView::new(spec, &words, next, frame.map(|f| f.sequence));
                    let candidates: Vec<_> = program
                        .index()
                        .lookup(input)
                        .into_iter()
                        .filter_map(|id| match evaluate(id, &view) {
                            Outcome::Candidate(c) => Some(c),
                            _ => None,
                        })
                        .collect();
                    authorize(spec, &self.state, &candidates, next, 0).is_ok()
                })
            })
            .map(|input| at(next, input))
            .collect()
    }

    fn item_at(&self, index: usize, finishing: bool) -> Option<u128> {
        if index < self.base {
            None
        } else if index < self.received {
            let (observation, refusal) = self.buffer.get(index - self.base)?;
            if refusal.is_some() {
                None
            } else {
                observation.word(self.program.spec().item_dimension()?)
            }
        } else if finishing {
            self.program.spec().end_word()
        } else {
            None
        }
    }

    fn step(&mut self, finishing: bool) -> StepResult {
        let program = self.program;
        let spec = program.spec();
        let index = self.next;
        let (observation, admission) = if index < self.received {
            self.buffer[index - self.base].clone()
        } else {
            // Only reached with a recognizer: `finish` steps one past the input only then.
            let end = spec
                .item_dimension()
                .zip(spec.end_word())
                .map_or(Observation::new(self.end_position), |(d, w)| {
                    Observation::new(self.end_position).with(d, w)
                });
            (end, None)
        };
        let position = observation.position();
        self.observer.observe(&StepEvent::Observed(&observation));

        let frames = program
            .recognizer()
            .map(|r| r.recognize_with(index, |i| self.item_at(i, finishing)))
            .unwrap_or_default();
        self.observer.observe(&StepEvent::Framed(&frames));

        let result = if let Some(refusal) = admission {
            self.refuse(position, vec![refusal])
        } else {
            match within(
                Limit::Interpretations,
                frames.len(),
                spec.containment().max_interpretations,
                position,
            ) {
                Err(refusal) => self.refuse(position, vec![refusal]),
                Ok(()) => self.decide(&observation, &frames),
            }
        };

        self.next += 1;
        let radius = spec.window_radius();
        while self.base + radius < self.next && !self.buffer.is_empty() {
            self.buffer.pop_front();
            self.base += 1;
        }
        result
    }

    fn decide(&mut self, observation: &Observation, frames: &[FrameFacts]) -> StepResult {
        let program = self.program;
        let spec = program.spec();
        let position = observation.position();
        let views: Vec<Option<&FrameFacts>> = if frames.is_empty() {
            vec![None]
        } else {
            frames.iter().map(Some).collect()
        };
        let ids = program.index().lookup(observation);
        let mut candidates = Vec::new();
        let mut refused: Vec<(ContractId, Refusal)> = Vec::new();
        for frame in views {
            let words = self
                .state
                .fact_words(spec, observation, frame.map_or(0, |f| f.word));
            let view = FactView::new(spec, &words, position, frame.map(|f| f.sequence));
            for &id in &ids {
                match evaluate(id, &view) {
                    Outcome::NotApplicable => {}
                    Outcome::Candidate(c) => {
                        self.observer.observe(&StepEvent::Candidate(&c));
                        candidates.push(c);
                    }
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

        match authorize(spec, &self.state, &candidates, position, refused.len()) {
            Err(refusal) => {
                let mut all: Vec<Refusal> = if matches!(refusal, Refusal::NoContract { .. }) {
                    refused.into_iter().map(|(_, r)| r).collect()
                } else {
                    Vec::new()
                };
                all.push(refusal);
                self.refuse(position, all)
            }
            Ok(transition) => {
                let contracts = transition.contracts().to_vec();
                self.observer.observe(&StepEvent::Authorized(&contracts));
                let before = self.state.clone();
                match commit(spec, &mut self.state, transition) {
                    Err(refusal) => self.refuse(position, vec![refusal]),
                    Ok(effects) => {
                        self.observer.observe(&StepEvent::Committed {
                            before: &before,
                            after: &self.state,
                        });
                        for effect in &effects {
                            self.observer.observe(&StepEvent::Emitted(effect));
                        }
                        StepResult {
                            position,
                            committed: contracts,
                            effects,
                            refusals: Vec::new(),
                        }
                    }
                }
            }
        }
    }

    fn refuse(&mut self, position: u32, refusals: Vec<Refusal>) -> StepResult {
        for refusal in &refusals {
            self.observer.observe(&StepEvent::Refused(refusal));
        }
        if self.program.spec().containment().on_refusal == RefusalRule::Stop {
            self.halted = true;
        }
        StepResult {
            position,
            committed: Vec::new(),
            effects: Vec::new(),
            refusals,
        }
    }
}

/// `input`'s evidence at `position`.
fn at(position: u32, input: &Observation) -> Observation {
    input
        .evidence()
        .iter()
        .fold(Observation::new(position), |o, (d, w)| o.with(*d, *w))
}

/// Checks every word an observation holds: the dimension exists, is
/// evidence, is not written by the recognizer, and flaga_solver accepts the word.
fn admit(spec: &ProgramSpec, observation: &Observation) -> Result<(), Refusal> {
    let position = observation.position();
    for &(id, word) in observation.evidence() {
        let Some(dim) = spec.dimension(id) else {
            return Err(Refusal::Evidence {
                position,
                dimension: format!("#{}", id.0),
                rule: "no such dimension",
            });
        };
        let refuse = |rule| Refusal::Evidence {
            position,
            dimension: dim.name().to_owned(),
            rule,
        };
        if dim.role() != DimensionRole::Evidence {
            return Err(refuse("only evidence dimensions are observed"));
        }
        if !spec.is_observed(id) {
            return Err(refuse("the recognizer writes this dimension"));
        }
        check(position, dim, word)?;
    }
    Ok(())
}
