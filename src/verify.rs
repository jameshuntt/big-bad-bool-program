//! Verification: every offline check, once, before any stepping.
//!
//! `verify` is the only way to get a [`VerifiedProgram`]. It compiles the
//! recognizer and the contract index, checks every word the spec fixes with
//! flaga_solver, and answers the coherence questions. The stepper never calls
//! this module; it only takes what it returns.

use std::collections::BTreeMap;

use big_bad_bool_core::BitFieldSpec;
use big_bad_bool_interact::MatchOp;

use crate::codes::Finding;
use crate::frame::Recognizer;
use crate::model::{ContractId, DimensionSpec, ProgramSpec, ResolvedStackOp, field_value};
use crate::step::ProjectionState;

/// Bounds on the verifier's searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifyLimits {
    /// Reachable states the search may visit before it reports `TooManyStates`.
    pub max_states: u128,
    /// Candidate words per dimension the witness search may try.
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
/// item field. Contracts with no such clause are checked for every item.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContractIndex {
    keyed: Vec<(BitFieldSpec, BTreeMap<u128, Vec<ContractId>>)>,
    unkeyed: Vec<ContractId>,
}

impl ContractIndex {
    fn new(spec: &ProgramSpec) -> Self {
        let item_schema = spec.dims[spec.item_dim].schema();
        let mut index = Self::default();
        for (i, c) in spec.resolved.iter().enumerate() {
            let id = ContractId(i as u16);
            let key = c.when.iter().find_map(|cl| match cl.op {
                MatchOp::Eq(v)
                    if cl.dim == spec.item_dim && !item_schema.is_flag_field(cl.field.name) =>
                {
                    Some((cl.field, v))
                }
                _ => None,
            });
            match key {
                Some((field, value)) => {
                    let slot = match index.keyed.iter().position(|(f, _)| f.name == field.name) {
                        Some(slot) => slot,
                        None => {
                            index.keyed.push((field, BTreeMap::new()));
                            index.keyed.len() - 1
                        }
                    };
                    index.keyed[slot].1.entry(value).or_default().push(id);
                }
                None => index.unkeyed.push(id),
            }
        }
        index
    }

    /// The contracts that can apply to `item`, in id order.
    pub(crate) fn lookup(&self, item: u128) -> Vec<ContractId> {
        let mut ids = self.unkeyed.clone();
        for (field, buckets) in &self.keyed {
            if let Some(found) = buckets.get(&field_value(field, item)) {
                ids.extend_from_slice(found);
            }
        }
        ids.sort_unstable();
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
    items: Vec<u128>,
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

    /// The report, holding any `info` findings.
    #[must_use]
    pub const fn report(&self) -> &VerificationReport {
        &self.report
    }

    /// Every labelled item word, the set `legal_next` and the state search draw from.
    #[must_use]
    pub fn item_words(&self) -> &[u128] {
        &self.items
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
        if d.role() == crate::model::DimensionRole::State {
            check_fixed(
                "start",
                d,
                start.word(crate::model::DimensionId(i as u16)),
                found,
            );
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
    let item_dim = &spec.dims[spec.item_dim];
    let items = match labelled_words(item_dim, limits.max_states) {
        Some(items) => items,
        None => {
            report.findings.push(Finding::TooManyStates {
                explored: 0,
                permitted: limits.max_states,
            });
            return Err(report);
        }
    };
    if !report.passed() {
        return Err(report);
    }
    let index = ContractIndex::new(&spec);
    Ok(VerifiedProgram {
        spec,
        recognizer,
        index,
        items,
        report,
    })
}
