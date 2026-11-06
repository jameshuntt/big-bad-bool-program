//! Verification: every offline check, once, before any stepping.
//!
//! `verify` is the only way to get a [`VerifiedProgram`]. It compiles the
//! recognizer and the contract index, checks every word the spec fixes with
//! flaga_solver, and answers the coherence questions. The stepper never calls
//! this module; it only takes what it returns.

use std::collections::{BTreeMap, HashSet, VecDeque};

use big_bad_bool_core::BitFieldSpec;
use big_bad_bool_interact::MatchOp;
use stable_digest::StableDigest;

use crate::codes::Finding;
use crate::contract::{FactView, Outcome, evaluate};
use crate::digest::{TRACE_FORMAT, TraceHeader, spec_digest};
use crate::frame::Recognizer;
use crate::model::{
    ContractId, DimensionId, DimensionRole, DimensionSpec, Observation, ProgramSpec,
    ResolvedClause, ResolvedItem, ResolvedStackOp, field_value,
};
use crate::step::ProjectionState;
use crate::step::authorize::authorize;
use crate::step::commit::commit;

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
    recognizer: Recognizer,
    index: ContractIndex,
    inputs: Vec<Observation>,
    digest: StableDigest,
    report: VerificationReport,
}

impl VerifiedProgram {
    /// The spec.
    #[must_use]
    pub const fn spec(&self) -> &ProgramSpec {
        &self.spec
    }

    /// The compiled recognizer.
    #[must_use]
    pub const fn recognizer(&self) -> &Recognizer {
        &self.recognizer
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
    /// one of its labelled words.
    #[must_use]
    pub fn inputs(&self) -> &[Observation] {
        &self.inputs
    }

    pub(crate) const fn index(&self) -> &ContractIndex {
        &self.index
    }
}

/// Every combination of each field's labelled values (a field with no labels
/// keeps its default), filtered by flaga_solver. `None` past `cap` words.
fn labelled_words(dim: &DimensionSpec, cap: u128) -> Option<Vec<u128>> {
    let schema = dim.schema();
    let default = dim.default_word();
    let mut words = vec![default];
    for field in schema.fields {
        let mut values: Vec<u128> = schema
            .labels
            .iter()
            .filter(|l| l.field == field.name)
            .map(|l| l.value)
            .collect();
        if values.is_empty() {
            values.push(field_value(field, default));
        }
        values.sort_unstable();
        values.dedup();
        if (words.len() as u128).saturating_mul(values.len() as u128) > cap {
            return None;
        }
        words = words
            .iter()
            .flat_map(|w| {
                values
                    .iter()
                    .map(move |v| (w & !field.mask) | (v << field.shift))
            })
            .collect();
    }
    words.retain(|w| flaga_solver::check(schema, *w).is_ok_ignoring_deprecated());
    Some(words)
}

/// Every observation the search tries: each observed evidence dimension
/// absent or at one of its labelled words, at position 0. `None` past `cap`.
fn evidence_inputs(
    spec: &ProgramSpec,
    labelled: &[Vec<u128>],
    cap: u128,
) -> Option<Vec<Observation>> {
    let mut out = vec![Observation::new(0)];
    for d in spec.observed() {
        let words = &labelled[d];
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
    check_fixed(
        "recognizer",
        &spec.dims[spec.item_dim],
        spec.end_word,
        found,
    );
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
    let recognizer = match Recognizer::new(&spec) {
        Ok(r) => r,
        Err(found) => {
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
    let mut labelled = vec![Vec::new(); spec.dims.len()];
    for d in spec.observed() {
        match labelled_words(&spec.dims[d], tries) {
            Some(words) => labelled[d] = words,
            None => return too_many(report),
        }
    }
    let Some(inputs) = evidence_inputs(&spec, &labelled, tries) else {
        return too_many(report);
    };
    let items = labelled[spec.item_dim].clone();
    let index = ContractIndex::new(&spec);
    sequences(&spec, &items, &mut report.findings);
    let frames: Vec<u128> = std::iter::once(0)
        .chain(recognizer.universe().iter().map(|f| f.word))
        .collect();
    for c in 0..spec.resolved.len() {
        satisfiable(&spec, c, &labelled, &frames, limits, &mut report.findings);
    }
    report.states = search(
        &spec,
        &recognizer,
        &index,
        &inputs,
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

/// Equal-length sequences that can match one window with different facts
/// (error), and sequences that can match inside a longer one (info).
fn sequences(spec: &ProgramSpec, items: &[u128], found: &mut Vec<Finding>) {
    let schema = spec.dims[spec.item_dim].schema();
    let seqs = &spec.sequences;
    for (i, a) in seqs.iter().enumerate() {
        for (j, b) in seqs.iter().enumerate() {
            if i == j || a.items.is_empty() || b.items.is_empty() {
                continue;
            }
            if a.items.len() == b.items.len() {
                let same_window = a
                    .items
                    .iter()
                    .zip(&b.items)
                    .all(|(x, y)| meet(x, y, schema, items));
                if i < j && same_window && a.facts != b.facts {
                    found.push(Finding::OverlappingSequences {
                        first: a.name,
                        second: b.name,
                    });
                }
            } else if a.items.len() < b.items.len() {
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
    }
}

/// Values worth trying in one field: zero, the max, the default, every label,
/// all labels together, and each clause constant with its neighbours.
fn field_candidates(dim: &DimensionSpec, clauses: &[&ResolvedClause]) -> Vec<u128> {
    let field = &clauses[0].field;
    let max = field.max_value();
    let schema = dim.schema();
    let mut values = vec![0, max, field.default.unwrap_or(0)];
    let labels: Vec<u128> = schema
        .labels
        .iter()
        .filter(|l| l.field == field.name)
        .map(|l| l.value)
        .collect();
    values.push(labels.iter().fold(0, |acc, v| acc | v));
    values.extend(labels);
    for c in clauses {
        match c.op {
            MatchOp::Any => {}
            MatchOp::Eq(v) | MatchOp::NotEq(v) => {
                values.extend([v, v.saturating_add(1), v.saturating_sub(1)])
            }
            MatchOp::RangeInclusive { min, max } => {
                values.extend([min, max, min.saturating_sub(1), max.saturating_add(1)]);
            }
            MatchOp::MaskAll { mask } | MatchOp::MaskAny { mask } => values.push(mask),
        }
    }
    values.retain(|v| *v <= max);
    values.sort_unstable();
    values.dedup();
    values
}

/// Words of a state dimension worth trying: every combination of candidate
/// values over the fields the clauses name, other fields at default, legal
/// under flaga_solver. `None` past `cap`.
fn state_witnesses(
    dim: &DimensionSpec,
    clauses: &[&ResolvedClause],
    cap: u32,
) -> Option<Vec<u128>> {
    let mut words = vec![dim.default_word()];
    let mut fields: Vec<&str> = clauses.iter().map(|c| c.field.name).collect();
    fields.sort_unstable();
    fields.dedup();
    for name in fields {
        let on_field: Vec<&ResolvedClause> = clauses
            .iter()
            .copied()
            .filter(|c| c.field.name == name)
            .collect();
        let spec = on_field[0].field;
        let values = field_candidates(dim, &on_field);
        if (words.len() as u128).saturating_mul(values.len() as u128) > u128::from(cap) {
            return None;
        }
        words = words
            .iter()
            .flat_map(|w| {
                values
                    .iter()
                    .map(move |v| (w & !spec.mask) | (v << spec.shift))
            })
            .collect();
    }
    words.retain(|w| flaga_solver::check(dim.schema(), *w).is_ok_ignoring_deprecated());
    Some(words)
}

/// Finds a contract none of whose REQUIRE-ANY choices has a witness word in
/// every dimension it reads. Observed evidence words come from each
/// dimension's labelled words, frame words from what the recognizer can
/// produce, state words from the fields the clauses name.
fn satisfiable(
    spec: &ProgramSpec,
    contract: usize,
    labelled: &[Vec<u128>],
    frames: &[u128],
    limits: VerifyLimits,
    found: &mut Vec<Finding>,
) {
    let c = &spec.resolved[contract];
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
                labelled[d].iter().any(|w| fits(*w))
            } else if d == spec.frame_dim {
                frames.iter().any(|w| fits(*w))
            } else {
                let named: Vec<&ResolvedClause> = pos.iter().chain(neg.iter()).copied().collect();
                match state_witnesses(dim, &named, limits.max_witness_tries) {
                    Some(words) => words.iter().any(|w| fits(*w)),
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
                let field = pos
                    .iter()
                    .map(|cl| cl.field.name)
                    .find(|name| {
                        let only: Vec<&&ResolvedClause> =
                            pos.iter().filter(|cl| cl.field.name == *name).collect();
                        let values =
                            field_candidates(dim, &only.iter().map(|c| **c).collect::<Vec<_>>());
                        !values.iter().any(|v| {
                            let w = (dim.default_word() & !only[0].field.mask)
                                | (v << only[0].field.shift);
                            only.iter().all(|cl| cl.holds(dim.schema(), w))
                        })
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

/// One input of the state search: an observation and the frame word it is seen under.
struct Input {
    observation: Observation,
    frame: u128,
    sequence: Option<(usize, usize)>,
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
/// input is an evidence observation seen with no frame, or, when it holds an
/// item word, seen under a frame word whose focal offset accepts it. The
/// successor of a state is what commit gives; the end item's successors are
/// not followed. Returns the number of states visited.
fn search(
    spec: &ProgramSpec,
    recognizer: &Recognizer,
    index: &ContractIndex,
    observations: &[Observation],
    limits: VerifyLimits,
    found: &mut Vec<Finding>,
) -> usize {
    let item_dim = DimensionId(spec.item_dim as u16);
    let mut inputs: Vec<Input> = observations
        .iter()
        .map(|o| Input {
            observation: o.clone(),
            frame: 0,
            sequence: None,
        })
        .collect();
    for f in recognizer.universe() {
        for o in observations {
            if o.word(item_dim)
                .is_some_and(|item| recognizer.accepts(&f, item))
            {
                inputs.push(Input {
                    observation: o.clone(),
                    frame: f.word,
                    sequence: Some((usize::from(f.sequence.0), usize::from(f.offset))),
                });
            }
        }
    }

    let start = ProjectionState::start(spec);
    let mut seen: HashSet<ProjectionState> = HashSet::from([start.clone()]);
    let mut order: Vec<ProjectionState> = Vec::new();
    let mut queue = VecDeque::from([start]);
    let mut committed = vec![false; spec.resolved.len()];
    let mut used: Vec<Vec<bool>> = spec
        .sequences
        .iter()
        .map(|s| vec![false; s.items.len()])
        .collect();
    let mut ambiguous: Vec<(ContractId, ContractId)> = Vec::new();
    let mut no_exit: Option<String> = None;
    let mut complete = true;
    let frame_describe = &spec.dims[spec.frame_dim];

    while let Some(state) = queue.pop_front() {
        let mut exit = false;
        for input in &inputs {
            let words = state.fact_words(spec, &input.observation, input.frame);
            let view = FactView::new(
                spec,
                &words,
                0,
                input
                    .sequence
                    .map(|(s, _)| crate::model::SequenceId(s as u16)),
            );
            let candidates: Vec<_> = index
                .lookup(&input.observation)
                .into_iter()
                .filter_map(|id| match evaluate(id, &view) {
                    Outcome::Candidate(c) => Some(c),
                    _ => None,
                })
                .collect();
            match authorize(spec, &state, &candidates, 0, 0) {
                Err(crate::codes::Refusal::Ambiguous { first, second, .. }) => {
                    let ids = (
                        spec.contract_id(first).unwrap_or(ContractId(0)),
                        spec.contract_id(second).unwrap_or(ContractId(0)),
                    );
                    if !ambiguous.contains(&ids) {
                        ambiguous.push(ids);
                        found.push(Finding::Ambiguous {
                            first,
                            second,
                            witness: format!(
                                "{} evidence={} frame={}",
                                render_state(spec, &state),
                                render_evidence(spec, &input.observation),
                                if input.sequence.is_some() {
                                    frame_describe.describe(input.frame)
                                } else {
                                    "none".to_owned()
                                }
                            ),
                        });
                    }
                }
                Err(_) => {}
                Ok(transition) => {
                    let contract = usize::from(transition.contract().0);
                    let mut next = state.clone();
                    if commit(spec, &mut next, transition).is_ok() {
                        exit = true;
                        committed[contract] = true;
                        if let Some((s, o)) = input.sequence {
                            used[s][o] = true;
                        }
                        let end = input.observation.word(item_dim) == Some(spec.end_word);
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
    for (s, offsets) in spec.sequences.iter().zip(&used) {
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
