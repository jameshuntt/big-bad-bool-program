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
//!
//! The recognizer's end word is the end of input, not an input: only
//! `finish` produces it. An observation whose item is the end word is refused
//! on `push` (`Evidence`, "the end word is reserved for `finish`"), so it
//! commits nothing and the stream goes on, or stops, from the state before
//! it. This is what the verifier models: it evaluates the end item and does
//! not follow its successor.
//!
//! A step has two phases, so a host can admit a transition before it
//! happens. [`Stepper::prepare`] (and [`Stepper::prepare_finish`])
//! authorizes a step's semantic transition without mutating live state: it
//! runs everything a step does up to and excluding the write (admission,
//! framing, contract evaluation, authorization and the commit rules) on a
//! copy of the stepper's cursor (committed state, window, counters, halted
//! and finished flags), and returns a [`PreparedStep`]. It changes nothing in
//! the stepper and calls no observer. The host may inspect a preparation and
//! decline it by dropping it; nothing changes then. [`Stepper::commit`] is
//! the sole application boundary: it applies exactly the prepared transition
//! once, recomputing nothing. It hands the observer the events of every
//! prepared position, in the order `push` always gave them, and puts the
//! prepared cursor in place of the stepper's. `push` is `prepare` and then
//! `commit`, and `finish` is `prepare_finish` and then `commit`. What a host
//! admits by is the host's: the stepper names none.
//!
//! Each commit adds one to the stepper's state version
//! ([`Stepper::version`]). A prepared step carries the version and the
//! stepper it was prepared from; `commit` refuses one from an older version
//! ([`CommitRefusal::Stale`], `BBBC0201`) or from another stepper
//! ([`CommitRefusal::OtherStepper`], `BBBC0202`), and changes nothing then.
//! A prepared step is not `Clone` and `commit` takes it by value, so one
//! preparation is committed at most once. The version never wraps: at
//! `u64::MAX` it cannot advance, `commit` refuses every preparation
//! ([`CommitRefusal::Exhausted`], `BBBC0203`), and `push` and `finish` apply
//! nothing and return no results.
//!
//! If the observer panics during `commit`, the panic goes on to the caller
//! and the stepper keeps its cursor and version from before the commit; the
//! observer has seen the events up to the panic.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::codes::{CommitRefusal, Refusal};
use crate::contract::Candidate;
use crate::frame::FrameFacts;
use crate::model::{ContractId, DimensionId, DimensionRole, Observation, ProgramSpec, RefusalRule};
use crate::observe::{Observer, StepEvent};
use crate::step::authorize::{Gathered, authorize, gather};
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
    cursor: Cursor,
    instance: u64,
    version: u64,
    observer: O,
}

/// Numbers every stepper, so a prepared step names the stepper it came from.
/// It decides no result: it only tells one stepper from another. Which number
/// a stepper gets depends on the order threads make steppers in; that no two
/// get the same one does not.
static INSTANCES: AtomicU64 = AtomicU64::new(0);

/// The next stepper number. Each number is handed out once, by one
/// successful compare-exchange, whatever the thread. The counter never wraps:
/// once every number below `u64::MAX` is taken it panics, as `std::thread`
/// does when thread ids run out.
fn next_instance(counter: &AtomicU64) -> u64 {
    let mut n = counter.load(Ordering::Relaxed);
    loop {
        let next = n
            .checked_add(1)
            .expect("every stepper number is taken: u64::MAX steppers were made in this process");
        match counter.compare_exchange_weak(n, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return n,
            Err(now) => n = now,
        }
    }
}

/// Where a stepper is in its input: the committed state, the window of
/// observations still needed, and the counters. `prepare` steps a copy of it;
/// `commit` puts the copy in its place.
#[derive(Debug, Clone)]
struct Cursor {
    state: ProjectionState,
    buffer: VecDeque<(Observation, Option<Refusal>)>,
    base: usize,
    received: usize,
    next: usize,
    end_position: u32,
    halted: bool,
    finished: bool,
}

/// A step's semantic transition, authorized and not applied: the result of
/// every position it steps, with their effects and refusals, and the state
/// and window the stepper would have after them.
///
/// It is read-only. A host inspects it ([`results`](Self::results),
/// [`effects`](Self::effects), [`is_refused`](Self::is_refused),
/// [`halts`](Self::halts), [`state_after`](Self::state_after),
/// [`version`](Self::version)) and either hands it to [`Stepper::commit`], the
/// one place it is applied, or declines it by dropping it; dropping changes
/// nothing. No accessor gives a way to change it or the stepper.
///
/// Only [`Stepper::prepare`] and [`Stepper::prepare_finish`] make one (its
/// fields are private, and it has no `Default` or `From`), it is not `Clone`,
/// and `commit` takes it by value: it is applied at most once, to the stepper
/// and state version it was prepared from.
///
/// One prepared step holds every position its call steps, in order, each
/// prepared from the state the one before it leaves. A `push` steps zero or
/// one position, a `finish` the rest of the window and the end item.
#[derive(Debug)]
#[must_use = "a prepared step changes nothing until `Stepper::commit` applies it"]
pub struct PreparedStep {
    instance: u64,
    version: u64,
    results: Vec<StepResult>,
    seen: Vec<Seen>,
    after: Cursor,
}

/// What one prepared position showed, kept so that `commit` gives the
/// observer the same events, in the same order, as stepping in place.
#[derive(Debug)]
struct Seen {
    observation: Observation,
    frames: Vec<FrameFacts>,
    candidates: Vec<Candidate>,
    decision: Decision,
}

#[derive(Debug)]
enum Decision {
    /// Refused before authorization: admission, the interpretations limit,
    /// or containment.
    Refused,
    /// Authorized these contracts; commit refused the write.
    Unwritten(Vec<ContractId>),
    /// Authorized and written: the state after. The contracts and effects are
    /// the position's result.
    Written(ProjectionState),
}

impl PreparedStep {
    /// The stepper's state version this was prepared from.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// What each stepped position would give: committed contracts and effects,
    /// or refusals. The same values `commit` returns.
    #[must_use]
    pub fn results(&self) -> &[StepResult] {
        &self.results
    }

    /// Every effect the step would emit, position by position, in dimension order.
    pub fn effects(&self) -> impl Iterator<Item = &Effect> {
        self.results.iter().flat_map(|r| r.effects.iter())
    }

    /// True when some position would refuse.
    #[must_use]
    pub fn is_refused(&self) -> bool {
        self.results.iter().any(|r| !r.refusals.is_empty())
    }

    /// True when the stepper would be halted after it (a `Stop` refusal).
    #[must_use]
    pub const fn halts(&self) -> bool {
        self.after.halted
    }

    /// The committed state the stepper would have after it.
    #[must_use]
    pub const fn state_after(&self) -> &ProjectionState {
        &self.after.state
    }
}

impl<'p, O: Observer> Stepper<'p, O> {
    /// A stepper at the program's start state, state version 0.
    ///
    /// # Panics
    ///
    /// When `u64::MAX` steppers were already made in this process, so no
    /// number is left to tell this one from every earlier one.
    pub fn new(program: &'p VerifiedProgram, observer: O) -> Self {
        Self {
            program,
            cursor: Cursor {
                state: ProjectionState::start(program.spec()),
                buffer: VecDeque::new(),
                base: 0,
                received: 0,
                next: 0,
                end_position: 0,
                halted: false,
                finished: false,
            },
            instance: next_instance(&INSTANCES),
            version: 0,
            observer,
        }
    }

    /// Adds one observation and steps every position it completes the window for.
    /// After a `Stop` refusal or after `finish`, nothing more is stepped.
    /// An observation whose item is the recognizer's end word is refused at
    /// its own step with `Evidence`: the end word is reserved for `finish`.
    ///
    /// The same as [`prepare`](Self::prepare) and then [`commit`](Self::commit).
    /// At the last state version, where `commit` refuses
    /// ([`CommitRefusal::Exhausted`]), it applies nothing and returns no results.
    pub fn push(&mut self, observation: Observation) -> Vec<StepResult> {
        let prepared = self.prepare(observation);
        self.commit(prepared).unwrap_or_default()
    }

    /// Steps every remaining position, then the end item. The end item is
    /// made here, never pushed. A program without a recognizer has no end
    /// item and nothing left to step.
    ///
    /// The same as [`prepare_finish`](Self::prepare_finish) and then
    /// [`commit`](Self::commit). At the last state version it applies nothing
    /// and returns no results, as `push` does.
    pub fn finish(&mut self) -> Vec<StepResult> {
        let prepared = self.prepare_finish();
        self.commit(prepared).unwrap_or_default()
    }

    /// Authorizes what [`push`](Self::push) would do with `observation`,
    /// without mutating live state: every position it completes, stepped
    /// through admission, framing, contract evaluation, authorization and the
    /// commit rules on a copy of the stepper's state and window. The stepper
    /// and its observer are not touched. The result is applied only by
    /// [`commit`](Self::commit); dropping it declines it. After a `Stop`
    /// refusal or after `finish` it steps nothing.
    pub fn prepare(&self, observation: Observation) -> PreparedStep {
        let mut after = self.cursor.clone();
        let mut results = Vec::new();
        let mut seen = Vec::new();
        if !(after.halted || after.finished) {
            after.end_position = observation.position().saturating_add(1);
            let refusal = admit(self.program.spec(), &observation).err();
            after.buffer.push_back((observation, refusal));
            after.received += 1;
            let radius = self.program.spec().window_radius();
            while !after.halted && after.next + radius < after.received {
                let (result, position) = after.step(self.program, false);
                results.push(result);
                seen.push(position);
            }
        }
        self.prepared(results, seen, after)
    }

    /// Authorizes what [`finish`](Self::finish) would do, without mutating
    /// live state: every remaining position and the end item, each from the
    /// state the one before it leaves, stopping at a `Stop` refusal. Applied
    /// only by [`commit`](Self::commit), like [`prepare`](Self::prepare).
    pub fn prepare_finish(&self) -> PreparedStep {
        let mut after = self.cursor.clone();
        let mut results = Vec::new();
        let mut seen = Vec::new();
        if !after.finished {
            let last = after.received + usize::from(self.program.spec().end_word().is_some());
            while !after.halted && after.next < last {
                let (result, position) = after.step(self.program, true);
                results.push(result);
                seen.push(position);
            }
            after.finished = true;
        }
        self.prepared(results, seen, after)
    }

    /// The sole application boundary: applies exactly the transition
    /// `prepared` holds and returns its results. The observer gets every event
    /// of every prepared position, the stepper takes the prepared state and
    /// window, and the state version goes up by one; nothing is recomputed.
    ///
    /// Refused, with nothing changed and no event observed, when `prepared`
    /// comes from another stepper ([`CommitRefusal::OtherStepper`]), from an
    /// older state version ([`CommitRefusal::Stale`]), or when the stepper is
    /// at the last state version, `u64::MAX` ([`CommitRefusal::Exhausted`]).
    pub fn commit(&mut self, prepared: PreparedStep) -> Result<Vec<StepResult>, CommitRefusal> {
        if prepared.instance != self.instance {
            return Err(CommitRefusal::OtherStepper);
        }
        if prepared.version != self.version {
            return Err(CommitRefusal::Stale {
                prepared: prepared.version,
                current: self.version,
            });
        }
        let Some(next) = self.version.checked_add(1) else {
            return Err(CommitRefusal::Exhausted {
                version: self.version,
            });
        };
        Ok(self.apply(prepared, next))
    }

    /// The state version: how many commits this stepper has applied. A
    /// `push` or `finish` is one commit. It never wraps; see
    /// [`CommitRefusal::Exhausted`].
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// The committed state.
    #[must_use]
    pub const fn state(&self) -> &ProjectionState {
        &self.cursor.state
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
        self.cursor.halted
    }

    /// One representative observation for each class of observations some
    /// contract would accept and containment would authorize at the next
    /// focal position, from the current state, under some full set of frame
    /// interpretations the recognizer can give the item. Each is one of
    /// [`VerifiedProgram::inputs`], at the next position.
    ///
    /// These are representatives, not every legal observation. Two words of
    /// an evidence dimension are in one class when every matcher of the
    /// program (contract clauses, item matchers, the end item) and every
    /// legality rule of its schema gives them the same answer; labelled
    /// words are listed as themselves. Any other observation whose words are
    /// in the same classes as a listed one is accepted in the same way at run
    /// time: the contracts constrain classes, not these raw words.
    ///
    /// The end item is never among them: `push` refuses it, only `finish`
    /// steps it. Read only.
    #[must_use]
    pub fn legal_next_representatives(&self) -> Vec<Observation> {
        let program = self.program;
        let spec = program.spec();
        let next = u32::try_from(self.cursor.next).unwrap_or(u32::MAX);
        let permitted = usize::from(spec.containment().max_interpretations);
        program
            .inputs()
            .iter()
            .filter(|input| !is_end(spec, input))
            .map(|input| at(next, input))
            .filter(|input| {
                let ids = program.index().lookup(input);
                program
                    .frame_sets(input)
                    .iter()
                    .filter(|frames| frames.len() <= permitted)
                    .any(|frames| {
                        let found = gather(spec, &self.cursor.state, &ids, input, frames);
                        authorize(spec, &self.cursor.state, &found.candidates, next, 0).is_ok()
                    })
            })
            .collect()
    }

    fn prepared(&self, results: Vec<StepResult>, seen: Vec<Seen>, after: Cursor) -> PreparedStep {
        PreparedStep {
            instance: self.instance,
            version: self.version,
            results,
            seen,
            after,
        }
    }

    /// Applies a step prepared from this stepper at this version, and moves
    /// the version to `next`.
    fn apply(&mut self, prepared: PreparedStep, next: u64) -> Vec<StepResult> {
        let PreparedStep {
            results,
            seen,
            after,
            ..
        } = prepared;
        let mut before: &ProjectionState = &self.cursor.state;
        for (position, result) in seen.iter().zip(&results) {
            let observer = &mut self.observer;
            observer.observe(&StepEvent::Observed(&position.observation));
            observer.observe(&StepEvent::Framed(&position.frames));
            for c in &position.candidates {
                observer.observe(&StepEvent::Candidate(c));
            }
            match &position.decision {
                Decision::Refused => {}
                Decision::Unwritten(contracts) => {
                    observer.observe(&StepEvent::Authorized(contracts));
                }
                Decision::Written(state) => {
                    observer.observe(&StepEvent::Authorized(&result.committed));
                    observer.observe(&StepEvent::Committed {
                        before,
                        after: state,
                    });
                    for effect in &result.effects {
                        observer.observe(&StepEvent::Emitted(effect));
                    }
                    before = state;
                }
            }
            for refusal in &result.refusals {
                observer.observe(&StepEvent::Refused(refusal));
            }
        }
        self.cursor = after;
        self.version = next;
        results
    }
}

impl Cursor {
    fn item_at(&self, program: &VerifiedProgram, index: usize, finishing: bool) -> Option<u128> {
        if index < self.base {
            None
        } else if index < self.received {
            let (observation, refusal) = self.buffer.get(index - self.base)?;
            if refusal.is_some() {
                None
            } else {
                observation.word(program.spec().item_dimension()?)
            }
        } else if finishing {
            program.spec().end_word()
        } else {
            None
        }
    }

    fn step(&mut self, program: &VerifiedProgram, finishing: bool) -> (StepResult, Seen) {
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

        let frames = program
            .recognizer()
            .map(|r| r.recognize_with(index, |i| self.item_at(program, i, finishing)))
            .unwrap_or_default();

        let (result, candidates, decision) = if let Some(refusal) = admission {
            (
                self.refuse(spec, position, vec![refusal]),
                Vec::new(),
                Decision::Refused,
            )
        } else {
            match within(
                Limit::Interpretations,
                frames.len(),
                spec.containment().max_interpretations,
                position,
            ) {
                Err(refusal) => (
                    self.refuse(spec, position, vec![refusal]),
                    Vec::new(),
                    Decision::Refused,
                ),
                Ok(()) => self.decide(program, &observation, &frames),
            }
        };

        self.next += 1;
        let radius = spec.window_radius();
        while self.base + radius < self.next && !self.buffer.is_empty() {
            self.buffer.pop_front();
            self.base += 1;
        }
        let seen = Seen {
            observation,
            frames,
            candidates,
            decision,
        };
        (result, seen)
    }

    fn decide(
        &mut self,
        program: &VerifiedProgram,
        observation: &Observation,
        frames: &[FrameFacts],
    ) -> (StepResult, Vec<Candidate>, Decision) {
        let spec = program.spec();
        let position = observation.position();
        let ids = program.index().lookup(observation);
        let Gathered {
            candidates,
            refused,
        } = gather(spec, &self.state, &ids, observation, frames);

        match authorize(spec, &self.state, &candidates, position, refused.len()) {
            Err(refusal) => {
                let mut all: Vec<Refusal> = if matches!(refusal, Refusal::NoContract { .. }) {
                    refused
                } else {
                    Vec::new()
                };
                all.push(refusal);
                (
                    self.refuse(spec, position, all),
                    candidates,
                    Decision::Refused,
                )
            }
            Ok(transition) => {
                let contracts = transition.contracts().to_vec();
                match commit(spec, &mut self.state, transition) {
                    Err(refusal) => (
                        self.refuse(spec, position, vec![refusal]),
                        candidates,
                        Decision::Unwritten(contracts),
                    ),
                    Ok(effects) => (
                        StepResult {
                            position,
                            committed: contracts,
                            effects,
                            refusals: Vec::new(),
                        },
                        candidates,
                        Decision::Written(self.state.clone()),
                    ),
                }
            }
        }
    }

    fn refuse(&mut self, spec: &ProgramSpec, position: u32, refusals: Vec<Refusal>) -> StepResult {
        if spec.containment().on_refusal == RefusalRule::Stop {
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

/// Whether `word` on dimension `id` is the recognizer's end word.
fn is_end_word(spec: &ProgramSpec, id: DimensionId, word: u128) -> bool {
    spec.item_dimension() == Some(id) && spec.end_word() == Some(word)
}

/// Whether `observation`'s item is the recognizer's end word.
fn is_end(spec: &ProgramSpec, observation: &Observation) -> bool {
    observation
        .evidence()
        .iter()
        .any(|&(id, word)| is_end_word(spec, id, word))
}

/// `input`'s evidence at `position`.
fn at(position: u32, input: &Observation) -> Observation {
    input
        .evidence()
        .iter()
        .fold(Observation::new(position), |o, (d, w)| o.with(*d, *w))
}

/// Checks every word an observation holds: the dimension exists, is
/// evidence, is not written by the recognizer, is not the end word on the
/// item dimension (only `finish` steps the end item), and flaga_solver
/// accepts the word.
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
        if is_end_word(spec, id, word) {
            return Err(refuse("the end word is reserved for `finish`"));
        }
        check(position, dim, word)?;
    }
    Ok(())
}

#[cfg(test)]
impl<O: Observer> Stepper<'_, O> {
    /// Puts the stepper at `version`, to test the last versions.
    pub(crate) const fn set_version(&mut self, version: u64) {
        self.version = version;
    }
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;
    use std::sync::atomic::AtomicU64;

    use liaise::Liaise;

    use super::{CommitRefusal, Stepper, next_instance};
    use crate::model::{DimensionId, Observation, contract};
    use crate::observe::NoObserver;
    use crate::test_support::{SET_INLINE, SET_TIMER, WHEN_B, spec};
    use crate::verify::{VerifiedProgram, VerifyLimits, verify};

    fn program() -> VerifiedProgram {
        let set_timer = contract("set-timer")
            .when(WHEN_B)
            .transition(&[SET_TIMER, SET_INLINE]);
        verify(
            spec(&[set_timer]),
            VerifyLimits {
                max_states: 1_000,
                max_witness_tries: 1_024,
            },
        )
        .expect("test program verifies")
    }

    /// Item `b` at `position`.
    fn b(position: u32) -> Observation {
        Observation::new(position).with(DimensionId(0), 1)
    }

    #[test]
    fn the_version_never_wraps() {
        let program = program();
        let mut stepper = Stepper::new(&program, NoObserver);
        stepper.set_version(u64::MAX - 1);
        let stale = stepper.prepare(b(0));
        // The last commit there is: u64::MAX - 1 to u64::MAX.
        let _ = stepper.push(b(0));
        assert_eq!(stepper.version(), u64::MAX);
        assert_eq!(
            stepper.commit(stale).unwrap_err(),
            CommitRefusal::Stale {
                prepared: u64::MAX - 1,
                current: u64::MAX,
            }
        );

        // At u64::MAX every commit is refused and nothing changes, though
        // the preparations would step positions.
        let state = stepper.state().clone();
        let last = stepper.prepare_finish();
        assert_eq!(last.version(), u64::MAX);
        assert!(!last.results().is_empty());
        let refusal = stepper.commit(last).unwrap_err();
        assert_eq!(refusal, CommitRefusal::Exhausted { version: u64::MAX });
        assert_eq!(refusal.code(), "BBBC0203");
        assert!(stepper.push(b(1)).is_empty());
        assert!(stepper.finish().is_empty());
        assert_eq!(stepper.version(), u64::MAX);
        assert_eq!(stepper.state(), &state);
        assert!(!stepper.prepare_finish().results().is_empty());
    }

    #[test]
    fn stepper_numbers_are_never_reused() {
        let counter = AtomicU64::new(u64::MAX - 2);
        assert_eq!(next_instance(&counter), u64::MAX - 2);
        assert_eq!(next_instance(&counter), u64::MAX - 1);
        // u64::MAX is never handed out, and the counter does not wrap to 0.
        assert!(catch_unwind(|| next_instance(&counter)).is_err());
        assert!(catch_unwind(|| next_instance(&counter)).is_err());
        assert_eq!(counter.into_inner(), u64::MAX);
    }

    #[test]
    fn two_steppers_never_share_a_number() {
        let program = program();
        let a = Stepper::new(&program, NoObserver);
        let b = Stepper::new(&program, NoObserver);
        assert_ne!(a.instance, b.instance);
    }
}
