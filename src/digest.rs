//! The spec's stable material and its digest.
//!
//! The material is plain text in liaise's `key.len=N` / `key=value` form:
//! every dimension with its whole schema, the recognizer when there is one,
//! every contract and the containment limits, in declaration order.
//! stable_digest hashes it.
//! Nothing here runs at step time.

use big_bad_bool_core::{FieldRelationKind, ValueRangeKind};
use big_bad_bool_interact::{FieldMatcher, FieldValue, MatchOp};
use stable_digest::StableDigest;

use crate::model::{Clause, ItemMatch, ProgramSpec, RefusalRule, StackOp, Write};

/// The material's first line. Changes when the material's form changes.
pub const SPEC_FORMAT: &str = "big-bad-bool-contract.spec.v1";

/// The trace format a [`TraceHeader`] names.
pub const TRACE_FORMAT: &str = "big-bad-bool-contract.trace.v1";

/// The head of a trace: which spec produced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TraceHeader {
    /// [`TRACE_FORMAT`].
    pub format: &'static str,
    /// The verified program's spec digest.
    pub spec: StableDigest,
}

impl TraceHeader {
    /// `big-bad-bool-contract.trace.v1 spec=<hex>`.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{} spec={}", self.format, self.spec)
    }
}

struct Material(String);

impl Material {
    fn field(&mut self, key: &str, value: &str) {
        use std::fmt::Write as _;
        let _ = writeln!(self.0, "{key}.len={}", value.len());
        let _ = writeln!(self.0, "{key}={value}");
    }
}

fn op(op: MatchOp) -> String {
    match op {
        MatchOp::Any => "any".to_owned(),
        MatchOp::Eq(v) => format!("eq:{v}"),
        MatchOp::NotEq(v) => format!("ne:{v}"),
        MatchOp::RangeInclusive { min, max } => format!("range:{min}:{max}"),
        MatchOp::MaskAll { mask } => format!("mask_all:{mask}"),
        MatchOp::MaskAny { mask } => format!("mask_any:{mask}"),
    }
}

fn matcher(m: &FieldMatcher) -> String {
    format!("{} {}", m.field, op(m.op))
}

fn clause(c: &Clause) -> String {
    format!("{}.{}", c.dimension, matcher(&c.matcher))
}

fn value(v: &FieldValue) -> String {
    format!("{}={}", v.field, v.value)
}

fn write(w: &Write) -> String {
    format!("{}.{}", w.dimension, value(&w.value))
}

fn values(vs: &[FieldValue]) -> String {
    vs.iter().map(value).collect::<Vec<_>>().join(",")
}

/// The stable material of `spec`.
#[must_use]
pub fn spec_material(spec: &ProgramSpec) -> String {
    let mut m = Material(String::new());
    m.0.push_str(SPEC_FORMAT);
    m.0.push('\n');

    for (i, d) in spec.dimensions().iter().enumerate() {
        let k = format!("dimension[{i}]");
        m.field(&format!("{k}.name"), d.name());
        m.field(&format!("{k}.role"), d.role().as_str());
        m.field(
            &format!("{k}.stack_depth"),
            &d.stack_depth().map_or_else(String::new, |n| n.to_string()),
        );
        let s = d.schema();
        for (j, f) in s.fields.iter().enumerate() {
            m.field(
                &format!("{k}.field[{j}]"),
                &format!(
                    "{} id={} shift={} width={} default={}",
                    f.name,
                    f.id,
                    f.shift,
                    f.width,
                    f.default.map_or_else(String::new, |v| v.to_string())
                ),
            );
        }
        for (j, l) in s.labels.iter().enumerate() {
            m.field(
                &format!("{k}.label[{j}]"),
                &format!("{} {}={}", l.field, l.label, l.value),
            );
        }
        for (j, r) in s.ranges.iter().enumerate() {
            let kind = match r.kind {
                ValueRangeKind::Reserved => "reserved",
                ValueRangeKind::Invalid => "invalid",
            };
            m.field(
                &format!("{k}.range[{j}]"),
                &format!("{} {kind} {}..={}", r.field, r.min, r.max),
            );
        }
        for (j, f) in s.flags.iter().enumerate() {
            m.field(
                &format!("{k}.flag[{j}]"),
                &format!("{} {}={}", f.field, f.name, f.bit),
            );
        }
        for (j, r) in s.relations.iter().enumerate() {
            let kind = match r.kind {
                FieldRelationKind::Requires => "requires",
                FieldRelationKind::Conflicts => "conflicts",
            };
            m.field(
                &format!("{k}.relation[{j}]"),
                &format!(
                    "{kind} {}={} {}={}",
                    r.field, r.when_label, r.other_field, r.other_label
                ),
            );
        }
        for (j, dep) in s.deprecated.iter().enumerate() {
            m.field(
                &format!("{k}.deprecated[{j}]"),
                &format!("{} {}={}", dep.field, dep.label, dep.value),
            );
        }
    }

    if let Some(r) = spec.recognizer() {
        m.field("recognizer.item_dimension", r.item_dimension);
        m.field("recognizer.frame_dimension", r.frame_dimension);
        m.field("recognizer.sequence_field", r.sequence_field);
        m.field("recognizer.offset_field", r.offset_field);
        m.field("recognizer.position_field", r.position_field);
        m.field("recognizer.boundary_flag", r.boundary_flag);
        m.field("recognizer.end_item", &value(&r.end_item));
        for (i, s) in r.sequences.iter().enumerate() {
            let k = format!("sequence[{i}]");
            m.field(&format!("{k}.name"), s.name);
            for (j, item) in s.items.iter().enumerate() {
                let text = match item {
                    ItemMatch::Any => "any".to_owned(),
                    ItemMatch::One(one) => matcher(one),
                    ItemMatch::OneOf(many) => {
                        many.iter().map(matcher).collect::<Vec<_>>().join(" | ")
                    }
                };
                m.field(&format!("{k}.item[{j}]"), &text);
            }
            for (j, f) in s.facts.iter().enumerate() {
                m.field(
                    &format!("{k}.facts[{j}]"),
                    &format!("{} {}", f.offset, values(f.values)),
                );
            }
        }
    }

    for (i, c) in spec.contracts().iter().enumerate() {
        let k = format!("contract[{i}]");
        m.field(&format!("{k}.name"), c.name);
        for (j, cl) in c.when.iter().enumerate() {
            m.field(&format!("{k}.when[{j}]"), &clause(cl));
        }
        for (j, cl) in c.require.iter().enumerate() {
            m.field(&format!("{k}.require[{j}]"), &clause(cl));
        }
        for (g, group) in c.require_any.iter().enumerate() {
            for (j, cl) in group.iter().enumerate() {
                m.field(&format!("{k}.require_any[{g}][{j}]"), &clause(cl));
            }
            if group.is_empty() {
                m.field(&format!("{k}.require_any[{g}]"), "");
            }
        }
        for (j, cl) in c.forbid.iter().enumerate() {
            m.field(&format!("{k}.forbid[{j}]"), &clause(cl));
        }
        for (j, w) in c.transition.iter().enumerate() {
            m.field(&format!("{k}.transition[{j}]"), &write(w));
        }
        for (j, op) in c.stack.iter().enumerate() {
            let text = match op {
                StackOp::Push {
                    dimension,
                    values: vs,
                } => format!("push {dimension} {}", values(vs)),
                StackOp::Pop { dimension } => format!("pop {dimension}"),
            };
            m.field(&format!("{k}.stack[{j}]"), &text);
        }
        for (j, w) in c.emit.iter().enumerate() {
            m.field(&format!("{k}.emit[{j}]"), &write(w));
        }
    }

    let c = spec.containment();
    m.field(
        "containment.max_interpretations",
        &c.max_interpretations.to_string(),
    );
    m.field("containment.max_candidates", &c.max_candidates.to_string());
    m.field(
        "containment.max_stack_depth",
        &c.max_stack_depth.to_string(),
    );
    m.field(
        "containment.on_refusal",
        match c.on_refusal {
            RefusalRule::Stop => "stop",
            RefusalRule::SkipObservation => "skip_observation",
        },
    );
    m.0
}

/// SHA-256 of the material, through stable_digest.
#[must_use]
pub fn spec_digest(spec: &ProgramSpec) -> StableDigest {
    StableDigest::sha256(spec_material(spec))
}
