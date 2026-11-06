//! Verification: every offline check, once, before any stepping.
//!
//! `verify` is the only way to get a [`VerifiedProgram`]. It compiles the
//! recognizer and the contract index, checks every word the spec fixes with
//! flaga_solver, and answers the coherence questions. The stepper never calls
//! this module; it only takes what it returns.
//!
//! The searches try each observed evidence dimension absent or at one word
//! of its evidence domain. The domain is built from the matchers, not from
//! the labels: each field's values are split into classes by every predicate
//! that reads the field (contract clauses, recognizer item matchers, the end
//! item, and the invalid, reserved, requires and conflicts rules
//! flaga_solver checks), and the domain holds one legal word for every
//! combination of classes a legal word can have, beside every labelled
//! word. So any legal runtime word gives the same matcher answers as some
//! domain word, labelled or not.
//!
//! Whether a contract's clauses on a state dimension can hold is decided over
//! the same classes, split by those clauses and the dimension's legality
//! rules: some legal state word meets the positive clauses and no FORBID
//! clause exactly when one word per class combination does. Past
//! `max_witness_tries` words the answer is `TooManyStates`.

use std::collections::{BTreeMap, HashSet, VecDeque};

use big_bad_bool_core::BitFieldSpec;
use big_bad_bool_interact::MatchOp;
use stable_digest::StableDigest;

use crate::codes::Finding;
use crate::digest::{TRACE_FORMAT, TraceHeader, spec_digest};
use crate::frame::{FrameFacts, Recognizer};
use crate::model::{
    ContractId, DimensionId, DimensionRole, DimensionSpec, Observation, ProgramSpec,
    ResolvedClause, ResolvedItem, ResolvedStackOp, field_value,
};
use crate::step::ProjectionState;
use crate::step::authorize::{authorize, gather};
use crate::step::commit::commit;

mod domain;

/// Bounds on the verifier's searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifyLimits {
    /// Reachable states the search may visit before it reports `TooManyStates`.
    pub max_states: u128,
    /// Candidate words per dimension the witness search may try, and
    /// evidence observations the state search may try.
    pub max_witness_tries: u32,
}

/// Every finding of one verification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationReport {
    /// Findings in the order found.
    pub findings: Vec<Finding>,
    /// Reachable states visited.
    pub states: usize,
}

impl VerificationReport {
    /// True when no finding is `Error` or worse.
    #[must_use]
    pub fn passed(&self) -> bool {
        use liaise::{Liaise, LiaiseSeverity};
        self.findings
            .iter()
            .all(|f| f.severity() < LiaiseSeverity::Error)
    }
}

/// Contracts bucketed by the first `Eq` WHEN clause on a plain (non-flag)
/// field of an observed evidence dimension. Contracts with no such clause are
/// checked for every observation. A contract keyed on a dimension the
/// observation does not hold cannot apply, so it is not looked up.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContractIndex {
    keyed: Vec<(usize, BitFieldSpec, BTreeMap<u128, Vec<ContractId>>)>,
    unkeyed: Vec<ContractId>,
}

impl ContractIndex {
    fn new(spec: &ProgramSpec) -> Self {
        let mut index = Self::default();
        for (i, c) in spec.resolved.iter().enumerate() {
            let id = ContractId(i as u16);
            let key = c.when.iter().find_map(|cl| match cl.op {
                MatchOp::Eq(v)
                    if spec.is_observed(DimensionId(cl.dim as u16))
                        && !spec.dims[cl.dim].schema().is_flag_field(cl.field.name) =>
                {
                    Some((cl.dim, cl.field, v))
                }
                _ => None,
            });
            match key {
                Some((dim, field, value)) => {
                    let slot = match index
                        .keyed
                        .iter()
                        .position(|(d, f, _)| *d == dim && f.name == field.name)
                    {
                        Some(slot) => slot,
                        None => {
                            index.keyed.push((dim, field, BTreeMap::new()));
                            index.keyed.len() - 1
                        }
                    };
                    index.keyed[slot].2.entry(value).or_default().push(id);
                }
                None => index.unkeyed.push(id),
            }
        }
        index
    }

    /// The contracts that can apply to `observation`, in id order.
    pub(crate) fn lookup(&self, observation: &Observation) -> Vec<ContractId> {
        let mut ids = self.unkeyed.clone();
        for (dim, field, buckets) in &self.keyed {
            let Some(word) = observation.word(DimensionId(*dim as u16)) else {
                continue;
            };
            if let Some(found) = buckets.get(&field_value(field, word)) {
                ids.extend_from_slice(found);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// A spec that passed verification, with its compiled recognizer and contract
/// index. Only [`verify`] builds one.
#[derive(Debug, Clone)]
pub struct VerifiedProgram {
    spec: ProgramSpec,
    recognizer: Option<Recognizer>,
    index: ContractIndex,
    inputs: Vec<Observation>,
    frame_sets: BTreeMap<u128, Vec<Vec<FrameFacts>>>,
    digest: StableDigest,
    report: VerificationReport,
}

impl VerifiedProgram {
    /// The spec.
    #[must_use]
    pub const fn spec(&self) -> &ProgramSpec {
        &self.spec
    }

    /// The compiled recognizer, when the program has one.
    #[must_use]
    pub const fn recognizer(&self) -> Option<&Recognizer> {
        self.recognizer.as_ref()
    }

    /// The spec's digest: what a trace names, and what a parked session is checked against.
    #[must_use]
    pub const fn digest(&self) -> &StableDigest {
        &self.digest
    }

    /// The header every trace of this program starts with.
    #[must_use]
    pub fn trace_header(&self) -> TraceHeader {
        TraceHeader {
            format: TRACE_FORMAT,
            spec: self.digest.clone(),
        }
    }

    /// The report, holding any `info` findings.
    #[must_use]
    pub const fn report(&self) -> &VerificationReport {
        &self.report
    }

    /// Every observation the state search tried and `legal_next` draws
    /// from, at position 0: each observed evidence dimension absent or at
    /// one of its domain words. A dimension's domain holds every labelled
    /// word and one legal word for each distinct set of matcher answers a
    /// legal word can give (see the module notes).
    #[must_use]
    pub fn inputs(&self) -> &[Observation] {
        &self.inputs
    }

    pub(crate) const fn index(&self) -> &ContractIndex {
        &self.index
    }

    /// Every full set of frame interpretations `observation`'s item can have;
    /// one empty set without a recognizer or an item word.
    pub(crate) fn frame_sets(&self, observation: &Observation) -> &[Vec<FrameFacts>] {
        const NONE: &[Vec<FrameFacts>] = &[Vec::new()];
        self.spec
            .item_dimension()
            .and_then(|d| observation.word(d))
            .and_then(|item| self.frame_sets.get(&item))
            .map_or(NONE, Vec::as_slice)
    }
}

/// Every observation the search tries: each observed evidence dimension
/// absent or at one of its domain words, at position 0. `None` past `cap`.
fn evidence_inputs(
    spec: &ProgramSpec,
    domain: &[Vec<u128>],
    cap: u128,
) -> Option<Vec<Observation>> {
    let mut out = vec![Observation::new(0)];
    for d in spec.observed() {
        let words = &domain[d];
        if (out.len() as u128).saturating_mul(words.len() as u128 + 1) > cap {
            return None;
        }
        out = out
            .iter()
            .flat_map(|o| {
                std::iter::once(o.clone()).chain(
                    words
                        .iter()
                        .map(move |w| o.clone().with(DimensionId(d as u16), *w)),
                )
            })
            .collect();
    }
    Some(out)
}

fn check_fixed(owner: &'static str, dim: &DimensionSpec, word: u128, found: &mut Vec<Finding>) {
    let verdict = flaga_solver::check(dim.schema(), word);
    if !verdict.is_ok_ignoring_deprecated() {
        found.push(Finding::IllegalWord {
            owner,
            dimension: dim.name(),
            verdict: verdict.to_string(),
        });
    }
}

/// Checks every word the spec fixes: start words, the end item, push words
/// and emit words.
fn fixed_words(spec: &ProgramSpec, found: &mut Vec<Finding>) {
    let start = ProjectionState::start(spec);
    for (i, d) in spec.dims.iter().enumerate() {
        if d.role() == DimensionRole::State {
            check_fixed("start", d, start.word(DimensionId(i as u16)), found);
        }
    }
    if let Some(framing) = &spec.framing {
        check_fixed(
            "recognizer",
            &spec.dims[framing.item_dim],
            framing.end_word,
            found,
        );
    }
    for c in &spec.resolved {
        for op in &c.stack {
            if let ResolvedStackOp::Push { dim, values } = op {
                let d = &spec.dims[*dim];
                match d.patch(d.default_word(), values) {
                    Ok(w) => check_fixed(c.name, d, w, found),
                    Err(e) => found.push(Finding::IllegalWord {
                        owner: c.name,
                        dimension: d.name(),
                        verdict: e.to_string(),
                    }),
                }
            }
        }
        for writes in &c.emit {
            let d = &spec.dims[writes.dim];
            match d.patch(d.default_word(), &writes.values) {
                Ok(w) => check_fixed(c.name, d, w, found),
                Err(e) => found.push(Finding::IllegalWord {
                    owner: c.name,
                    dimension: d.name(),
                    verdict: e.to_string(),
                }),
            }
        }
    }
}

/// Checks `spec` once and returns the verified program.
///
/// # Errors
///
/// The report, when any finding is `Error` or worse.
pub fn verify(
    spec: ProgramSpec,
    limits: VerifyLimits,
) -> Result<VerifiedProgram, VerificationReport> {
    let mut report = VerificationReport::default();
    fixed_words(&spec, &mut report.findings);
    let recognizer = match spec.framing.as_ref().map(|_| Recognizer::new(&spec)) {
        None => None,
        Some(Ok(r)) => Some(r),
        Some(Err(found)) => {
            report.findings.extend(found);
            return Err(report);
        }
    };
    let tries = u128::from(limits.max_witness_tries);
    let too_many = |mut report: VerificationReport| {
        report.findings.push(Finding::TooManyStates {
            explored: tries + 1,
            permitted: tries,
        });
        Err(report)
    };
    let mut evidence = vec![Vec::new(); spec.dims.len()];
    for d in spec.observed() {
        match domain::evidence_words(&spec, d, tries) {
            Some(words) => evidence[d] = words,
            None => return too_many(report),
        }
    }
    let Some(inputs) = evidence_inputs(&spec, &evidence, tries) else {
        return too_many(report);
    };
    let index = ContractIndex::new(&spec);
    let mut frame_sets = BTreeMap::new();
    if let (Some(item_dim), Some(recognizer)) = (spec.item_dim(), &recognizer) {
        let items = &evidence[item_dim];
        sequences(&spec, item_dim, items, &mut report.findings);
        let mut choices = items.clone();
        choices.extend(spec.end_word().filter(|w| !items.contains(w)));
        for &item in items {
            match recognizer.realizable(item, &choices, limits.max_witness_tries as usize) {
                Some(sets) => {
                    frame_sets.insert(item, sets);
                }
                None => return too_many(report),
            }
        }
    }
    let frames: Vec<u128> = std::iter::once(0)
        .chain(
            recognizer
                .iter()
                .flat_map(Recognizer::universe)
                .map(|f| f.word),
        )
        .collect();
    for c in 0..spec.resolved.len() {
        satisfiable(&spec, c, &evidence, &frames, limits, &mut report.findings);
    }
    report.states = search(
        &spec,
        &index,
        &inputs,
        &frame_sets,
        limits,
        &mut report.findings,
    );
    if !report.passed() {
        return Err(report);
    }
    let digest = spec_digest(&spec);
    Ok(VerifiedProgram {
        spec,
        recognizer,
        index,
        inputs,
        frame_sets,
        digest,
        report,
    })
}

/// Whether two sequence items accept a common item word.
fn meet(
    a: &ResolvedItem,
    b: &ResolvedItem,
    schema: &flaga_solver::Schema<'_>,
    items: &[u128],
) -> bool {
    items
        .iter()
        .any(|w| a.matches(schema, *w) && b.matches(schema, *w))
}

/// Sequences that can match inside a longer one (info: both are kept), and
/// pairs of sequence offsets that can hold one focal item together while
/// declaring different values for one frame field (info: containment refuses
/// only when actualizing contracts read that field under both).
fn sequences(spec: &ProgramSpec, item_dim: usize, items: &[u128], found: &mut Vec<Finding>) {
    let schema = spec.dims[item_dim].schema();
    let seqs = spec.sequences();
    for (i, a) in seqs.iter().enumerate() {
        for (j, b) in seqs.iter().enumerate() {
            if i == j || a.items.is_empty() || a.items.len() >= b.items.len() {
                continue;
            }
            for at in 0..=b.items.len() - a.items.len() {
                if a.items
                    .iter()
                    .enumerate()
                    .all(|(k, x)| meet(x, &b.items[at + k], schema, items))
                {
                    found.push(Finding::ContainedSequence {
                        inner: a.name,
                        outer: b.name,
                        at,
                    });
                }
            }
        }
    }

    let Some(frame_dim) = spec.frame_dim() else {
        return;
    };
    let frame_fields = spec.dims[frame_dim].schema().fields;
    let placements: Vec<(usize, usize)> = seqs
        .iter()
        .enumerate()
        .flat_map(|(s, seq)| (0..seq.items.len()).map(move |o| (s, o)))
        .collect();
    for (n, &(sa, oa)) in placements.iter().enumerate() {
        for &(sb, ob) in &placements[n + 1..] {
            let (a, b) = (&seqs[sa], &seqs[sb]);
            // Align both on the focal item: a's item k sits where b's item k - oa + ob does.
            let together = (0..a.items.len()).all(|k| {
                let kb = k as isize - oa as isize + ob as isize;
                kb < 0
                    || kb >= b.items.len() as isize
                    || meet(&a.items[k], &b.items[kb as usize], schema, items)
            });
            if !together {
                continue;
            }
            let (da, db) = (a.declared(oa), b.declared(ob));
            let differs = frame_fields.iter().find(|f| {
                let va = da.iter().find(|v| v.field == f.name);
                let vb = db.iter().find(|v| v.field == f.name);
                matches!((va, vb), (Some(x), Some(y)) if x.value != y.value)
            });
            if let Some(field) = differs {
                found.push(Finding::OverlappingFacts {
                    first: a.name,
                    first_offset: oa,
                    second: b.name,
                    second_offset: ob,
                    field: field.name,
                });
            }
        }
    }
}

/// Finds a contract none of whose REQUIRE-ANY choices has a witness word in
/// every dimension it reads. Observed evidence words come from each
/// dimension's evidence domain, frame words from what the recognizer can
/// produce. A state word is looked for over the matcher classes of the
/// contract's clauses on that dimension and its legality rules
/// (`domain::witness`); past `max_witness_tries` words that is
/// `TooManyStates`, never a verdict from part of the domain.
fn satisfiable(
    spec: &ProgramSpec,
    contract: usize,
    domain: &[Vec<u128>],
    frames: &[u128],
    limits: VerifyLimits,
    found: &mut Vec<Finding>,
) {
    let c = &spec.resolved[contract];
    let tries = u128::from(limits.max_witness_tries);
    let empty = Vec::new();
    let groups: Vec<&Vec<ResolvedClause>> = if c.require_any.is_empty() {
        vec![&empty]
    } else {
        c.require_any.iter().collect()
    };
    let mut first_failure: Option<(usize, &'static str)> = None;
    for group in groups {
        let positive: Vec<&ResolvedClause> = c
            .when
            .iter()
            .chain(&c.require)
            .chain(group.iter())
            .collect();
        let mut dims: Vec<usize> = positive
            .iter()
            .chain(c.forbid.iter().collect::<Vec<_>>().iter())
            .map(|cl| cl.dim)
            .collect();
        dims.sort_unstable();
        dims.dedup();
        let mut failure = None;
        for d in dims {
            let dim = &spec.dims[d];
            let pos: Vec<&ResolvedClause> =
                positive.iter().copied().filter(|cl| cl.dim == d).collect();
            let neg: Vec<&ResolvedClause> = c.forbid.iter().filter(|cl| cl.dim == d).collect();
            let fits = |w: u128| {
                pos.iter().all(|cl| cl.holds(dim.schema(), w))
                    && !neg.iter().any(|cl| cl.holds(dim.schema(), w))
            };
            let witnessed = if spec.is_observed(DimensionId(d as u16)) {
                domain[d].iter().any(|w| fits(*w))
            } else if Some(d) == spec.frame_dim() {
                frames.iter().any(|w| fits(*w))
            } else {
                match domain::witness(dim, &pos, &neg, tries) {
                    Some(found) => found.is_some(),
                    None => {
                        found.push(Finding::TooManyStates {
                            explored: u128::from(limits.max_witness_tries) + 1,
                            permitted: u128::from(limits.max_witness_tries),
                        });
                        true
                    }
                }
            };
            if !witnessed {
                // A field whose positive clauses alone no legal word meets.
                let field = pos
                    .iter()
                    .map(|cl| cl.field.name)
                    .find(|name| {
                        let only: Vec<&ResolvedClause> = pos
                            .iter()
                            .copied()
                            .filter(|cl| cl.field.name == *name)
                            .collect();
                        domain::witness(dim, &only, &[], tries) == Some(None)
                    })
                    .unwrap_or("(joint)");
                failure = Some((d, field));
                break;
            }
        }
        match failure {
            None => return,
            Some(f) => {
                first_failure.get_or_insert(f);
            }
        }
    }
    if let Some((d, field)) = first_failure {
        found.push(Finding::Unsatisfiable {
            owner: c.name,
            dimension: spec.dims[d].name(),
            field,
        });
    }
}

/// One input of the state search: an observation and one full set of frame
/// interpretations its item can have.
struct Input<'a> {
    observation: &'a Observation,
    frames: &'a [FrameFacts],
}

/// The evidence an observation holds, as `dimension={fields}` pairs.
fn render_evidence(spec: &ProgramSpec, observation: &Observation) -> String {
    let parts: Vec<String> = observation
        .evidence()
        .iter()
        .map(|(d, w)| {
            let dim = &spec.dims[usize::from(d.0)];
            format!("{}={}", dim.name(), dim.describe(*w))
        })
        .collect();
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join(" ")
    }
}

/// Frame words joined by ` & `, or `none`.
fn render_frames(frames: &[FrameFacts], dim: Option<&DimensionSpec>) -> String {
    match dim {
        Some(dim) if !frames.is_empty() => frames
            .iter()
            .map(|f| dim.describe(f.word))
            .collect::<Vec<_>>()
            .join(" & "),
        _ => "none".to_owned(),
    }
}

fn render_state(spec: &ProgramSpec, state: &ProjectionState) -> String {
    let mut parts = Vec::new();
    for (i, d) in spec.dims.iter().enumerate() {
        if d.role() != DimensionRole::State {
            continue;
        }
        let id = DimensionId(i as u16);
        if d.stack_depth().is_some() {
            let entries: Vec<String> = state.stack(id).iter().map(|w| d.describe(*w)).collect();
            parts.push(format!("{}=[{}]", d.name(), entries.join(", ")));
        } else {
            parts.push(format!("{}={}", d.name(), d.describe(state.word(id))));
        }
    }
    parts.join(" ")
}

/// Breadth-first search over committed states from the start state. Every
/// input is an evidence observation with one full set of frame
/// interpretations the recognizer can give its item (the empty set when it
/// holds no item). Each input goes through what the stepper does: the
/// interpretations limit, `gather`, `authorize` and `commit`. The
/// successor of a state is what commit gives; the end item's successors are
/// not followed, because at run time only `finish` steps the end item, as
/// the last step (`push` refuses it). Returns the number of states visited.
fn search(
    spec: &ProgramSpec,
    index: &ContractIndex,
    observations: &[Observation],
    frame_sets: &BTreeMap<u128, Vec<Vec<FrameFacts>>>,
    limits: VerifyLimits,
    found: &mut Vec<Finding>,
) -> usize {
    let item_dim = spec.item_dimension();
    let end_word = spec.end_word();
    let no_frames: Vec<Vec<FrameFacts>> = vec![Vec::new()];
    let permitted = usize::from(spec.containment.max_interpretations);
    let mut inputs: Vec<Input<'_>> = Vec::new();
    let mut ids: Vec<Vec<ContractId>> = Vec::new();
    for o in observations {
        let sets = item_dim
            .and_then(|d| o.word(d))
            .and_then(|item| frame_sets.get(&item))
            .unwrap_or(&no_frames);
        for frames in sets.iter().filter(|f| f.len() <= permitted) {
            inputs.push(Input {
                observation: o,
                frames,
            });
            ids.push(index.lookup(o));
        }
    }

    let start = ProjectionState::start(spec);
    let mut seen: HashSet<ProjectionState> = HashSet::from([start.clone()]);
    let mut order: Vec<ProjectionState> = Vec::new();
    let mut queue = VecDeque::from([start]);
    let mut committed = vec![false; spec.resolved.len()];
    let mut used: Vec<Vec<bool>> = spec
        .sequences()
        .iter()
        .map(|s| vec![false; s.items.len()])
        .collect();
    let mut conflicts: Vec<(&'static str, &'static str, String)> = Vec::new();
    let mut no_exit: Option<String> = None;
    let mut complete = true;
    let frame_describe = spec.frame_dim().map(|d| &spec.dims[d]);

    while let Some(state) = queue.pop_front() {
        let mut exit = false;
        for (input, ids) in inputs.iter().zip(&ids) {
            let candidates = gather(spec, &state, ids, input.observation, input.frames).candidates;
            match authorize(spec, &state, &candidates, 0, 0) {
                Err(crate::codes::Refusal::Conflict {
                    first,
                    second,
                    target,
                    ..
                }) => {
                    let key = (first, second, target.clone());
                    if !conflicts.contains(&key) {
                        conflicts.push(key);
                        found.push(Finding::Conflict {
                            first,
                            second,
                            target,
                            witness: format!(
                                "{} evidence={} frame={}",
                                render_state(spec, &state),
                                render_evidence(spec, input.observation),
                                render_frames(input.frames, frame_describe)
                            ),
                        });
                    }
                }
                Err(_) => {}
                Ok(transition) => {
                    let actualized = transition.contracts().to_vec();
                    let mut next = state.clone();
                    if commit(spec, &mut next, transition).is_ok() {
                        exit = true;
                        for c in &actualized {
                            committed[usize::from(c.0)] = true;
                        }
                        for c in candidates
                            .iter()
                            .filter(|c| actualized.contains(&c.contract))
                        {
                            if let Some(f) = c.frame {
                                used[usize::from(f.sequence.0)][usize::from(f.offset)] = true;
                            }
                        }
                        let end = item_dim.is_some_and(|d| input.observation.word(d) == end_word);
                        if !end && seen.insert(next.clone()) {
                            if seen.len() as u128 > limits.max_states {
                                complete = false;
                                break;
                            }
                            queue.push_back(next);
                        }
                    }
                }
            }
        }
        if !exit && no_exit.is_none() {
            no_exit = Some(render_state(spec, &state));
        }
        order.push(state);
        if !complete {
            break;
        }
    }

    if !complete {
        found.push(Finding::TooManyStates {
            explored: seen.len() as u128,
            permitted: limits.max_states,
        });
        return seen.len();
    }
    if let Some(witness) = no_exit {
        found.push(Finding::NoExit { witness });
    }
    for (c, done) in spec.resolved.iter().zip(&committed) {
        if !done {
            found.push(if c.emit.is_empty() {
                Finding::DeadContract { owner: c.name }
            } else {
                Finding::ImpossibleEffect { owner: c.name }
            });
        }
    }
    for (s, offsets) in spec.sequences().iter().zip(&used) {
        for (offset, done) in offsets.iter().enumerate() {
            if !done {
                found.push(Finding::DeadSequence {
                    owner: s.name,
                    offset,
                });
            }
        }
    }
    for (i, d) in spec.dims.iter().enumerate() {
        if d.role() != DimensionRole::State {
            continue;
        }
        let id = DimensionId(i as u16);
        let schema = d.schema();
        for label in schema.labels {
            let Some(field) = schema.field(label.field) else {
                continue;
            };
            let reached = order.iter().any(|state| {
                let words: Vec<u128> = if d.stack_depth().is_some() {
                    state.stack(id).to_vec()
                } else {
                    vec![state.word(id)]
                };
                words
                    .iter()
                    .any(|w| schema.holds(field.name, field_value(field, *w), label.value))
            });
            if !reached {
                found.push(Finding::Unreachable {
                    dimension: d.name(),
                    field: field.name,
                    label: label.label,
                });
            }
        }
    }
    order.len()
}
