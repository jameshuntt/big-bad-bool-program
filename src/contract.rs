//! Evaluates one contract against one fact view.
//!
//! `evaluate` takes shared references only, so it cannot write. Its result is
//! not applicable, refused (naming the clause that failed), or a candidate.

use crate::codes::Refusal;
use crate::frame::FrameFacts;
use crate::model::{
    ContractId, DimensionId, ProgramSpec, ResolvedClause, field_value, render_value,
};
use big_bad_bool_interact::MatchOp;

/// The current word of every dimension at one input position. Read only.
///
/// `words` holds one entry per dimension, by index: each observed evidence
/// word (`None` when the observation does not hold it), the frame word (zero
/// when no sequence matched), each state word (the top entry for a stacked
/// dimension), and `None` for an empty stack or an effect dimension. `frame`
/// is the sequence placement the frame word comes from.
#[derive(Debug, Clone, Copy)]
pub struct FactView<'a> {
    spec: &'a ProgramSpec,
    words: &'a [Option<u128>],
    position: u32,
    frame: Option<FrameFacts>,
}

impl<'a> FactView<'a> {
    /// A view over `words`, one per dimension of `spec`.
    ///
    /// # Panics
    ///
    /// When `words` does not have one entry per dimension.
    #[must_use]
    pub fn new(
        spec: &'a ProgramSpec,
        words: &'a [Option<u128>],
        position: u32,
        frame: Option<FrameFacts>,
    ) -> Self {
        assert_eq!(words.len(), spec.dims.len(), "one word per dimension");
        Self {
            spec,
            words,
            position,
            frame,
        }
    }

    /// A dimension's word, if it has one here.
    #[must_use]
    pub fn word(&self, dimension: DimensionId) -> Option<u128> {
        self.words.get(usize::from(dimension.0)).copied().flatten()
    }

    /// The input position.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position
    }

    /// The sequence placement whose frame word this view carries.
    #[must_use]
    pub const fn frame(&self) -> Option<FrameFacts> {
        self.frame
    }

    fn holds(&self, clause: &ResolvedClause) -> bool {
        self.words[clause.dim].is_some_and(|w| clause.holds(self.spec.dims[clause.dim].schema(), w))
    }

    fn render(&self, clause: &ResolvedClause) -> String {
        clause.render(self.spec.dims[clause.dim].schema())
    }

    fn observed(&self, clause: &ResolvedClause) -> String {
        match self.words[clause.dim] {
            Some(w) => render_value(
                self.spec.dims[clause.dim].schema(),
                &clause.field,
                field_value(&clause.field, w),
            ),
            None => "nothing".to_owned(),
        }
    }

    /// What `clauses` read, as `value` when they read one field, else
    /// `dim.field = value` for each field.
    fn observed_all(&self, clauses: &[&ResolvedClause]) -> String {
        let mut fields: Vec<&ResolvedClause> = Vec::new();
        for c in clauses {
            if !fields
                .iter()
                .any(|f| f.dim == c.dim && f.field.name == c.field.name)
            {
                fields.push(c);
            }
        }
        if let [one] = fields.as_slice() {
            return self.observed(one);
        }
        fields
            .iter()
            .map(|c| format!("{}.{} = {}", c.dim_name, c.field.name, self.observed(c)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A contract that holds at one position, under one frame interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Candidate {
    /// The contract.
    pub contract: ContractId,
    /// The input position.
    pub position: u32,
    /// The frame interpretation it held under.
    pub frame: Option<FrameFacts>,
}

/// The result of evaluating one contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// WHEN did not hold.
    NotApplicable,
    /// WHEN held; REQUIRE, REQUIRE-ANY or FORBID refused. Always `Refusal::ContractRefused`.
    Refused(Refusal),
    /// Everything held.
    Candidate(Candidate),
}

/// Evaluates `contract` against `view`: WHEN, then REQUIRE, then REQUIRE-ANY, then FORBID.
///
/// `Eq` and `NotEq` clauses hold through flaga_solver's `Schema::holds`, so a
/// flag-field label holds when its bits are set.
///
/// # Panics
///
/// When `contract` is not a contract of the view's program.
#[must_use]
pub fn evaluate(contract: ContractId, view: &FactView<'_>) -> Outcome {
    let c = &view.spec.resolved[usize::from(contract.0)];
    if !c.when.iter().all(|cl| view.holds(cl)) {
        return Outcome::NotApplicable;
    }
    let refused = |clause: &'static str, missing: String, observed: String| {
        Outcome::Refused(Refusal::ContractRefused {
            position: view.position,
            contract: c.name,
            clause,
            missing,
            observed,
        })
    };
    if let Some(cl) = c.require.iter().find(|cl| !view.holds(cl)) {
        return refused("require", view.render(cl), view.observed(cl));
    }
    if !c.require_any.is_empty()
        && !c
            .require_any
            .iter()
            .any(|g| g.iter().all(|cl| view.holds(cl)))
    {
        let all: Vec<&ResolvedClause> = c.require_any.iter().flatten().collect();
        return refused(
            "require_any",
            missing_any(view, &c.require_any),
            view.observed_all(&all),
        );
    }
    if let Some(cl) = c.forbid.iter().find(|cl| view.holds(cl)) {
        return refused(
            "forbid",
            format!("not {}", view.render(cl)),
            view.observed(cl),
        );
    }
    Outcome::Candidate(Candidate {
        contract,
        position: view.position,
        frame: view.frame,
    })
}

/// REQUIRE-ANY groups as text: `dim.field in {A, B}` when every group is one
/// `Eq` on the same field, else the groups joined by `or`.
fn missing_any(view: &FactView<'_>, groups: &[Vec<ResolvedClause>]) -> String {
    let first = &groups[0][..];
    let one_field = groups.iter().all(|g| {
        matches!(g.as_slice(), [c] if matches!(c.op, MatchOp::Eq(_))
            && c.dim == first[0].dim && c.field.name == first[0].field.name)
    });
    if one_field {
        let schema = view.spec.dims[first[0].dim].schema();
        let labels: Vec<String> = groups
            .iter()
            .filter_map(|g| match g[0].op {
                MatchOp::Eq(v) => Some(render_value(schema, &g[0].field, v)),
                _ => None,
            })
            .collect();
        return format!(
            "{}.{} in {{{}}}",
            first[0].dim_name,
            first[0].field.name,
            labels.join(", ")
        );
    }
    groups
        .iter()
        .map(|g| {
            g.iter()
                .map(|c| view.render(c))
                .collect::<Vec<_>>()
                .join(" and ")
        })
        .collect::<Vec<_>>()
        .join(" or ")
}
