//! The evidence domain: the observed evidence words the verifier tries.
//!
//! At run time an observation may hold any word its schema accepts, labelled
//! or not. What the program does with a word depends only on which matchers
//! hold for it: the contract clauses on its dimension, the recognizer's item
//! matchers when it is the item dimension, and whether it is the end item.
//! Whether flaga_solver accepts it depends only on the field values its
//! invalid and reserved ranges and its requires and conflicts relations read.
//!
//! So each field's values fall into classes: two values are in one class when
//! every one of those predicates on the field gives the same answer for both.
//! A word's matcher answers and its legality are fixed by the class of each
//! of its fields. The domain keeps every labelled value of each field (the
//! default when it has none, as before) and adds one value the search below
//! finds in each class none of those is in, combines the fields, and keeps
//! the words flaga_solver accepts. That is at least one legal word for every
//! class combination a legal word can have, so every set of matcher answers
//! a legal runtime word can give is given by some word of the domain, and no
//! other.
//!
//! The classes are found without listing a field's values: each predicate
//! splits a region of values into the part where it holds and the part where
//! it does not, and a region is kept only when some value lies in it.
//!
//! The same classes answer whether a contract's state clauses can hold
//! (`witness`). There the predicates are the contract's positive and
//! FORBID clauses on the dimension and the legality rules, and no label is
//! added: one legal word for every class combination a legal word can have
//! is enough to decide whether some legal word meets the positive clauses and
//! none of the FORBID clauses. Past the bound there is no answer, never a
//! smaller domain.

use std::collections::BTreeMap;

use big_bad_bool_core::BitFieldSpec;
use big_bad_bool_interact::MatchOp;

use crate::model::{DimensionSpec, ProgramSpec, ResolvedClause, ResolvedItem, field_value};

/// One predicate on one field's value, decided as the runtime decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Pred {
    /// The value is exactly this.
    Eq(u128),
    /// The value is in `min..=max`.
    Range(u128, u128),
    /// Every bit of the mask is set.
    AllBits(u128),
    /// Some bit of the mask is set.
    AnyBit(u128),
}

impl Pred {
    /// The predicate a clause op decides on `field`, through the same rule as
    /// `ResolvedClause::holds`: `Eq` and `NotEq` go through `Schema::holds`,
    /// so a non-zero label on a flag field holds when all its bits are set.
    /// `Any` decides nothing about the value.
    fn of_op(dim: &DimensionSpec, field: &BitFieldSpec, op: MatchOp) -> Option<Self> {
        match op {
            MatchOp::Any => None,
            MatchOp::Eq(v) | MatchOp::NotEq(v) => Some(Self::label(dim, field, v)),
            MatchOp::RangeInclusive { min, max } => Some(Self::Range(min, max)),
            MatchOp::MaskAll { mask } => Some(Self::AllBits(mask)),
            MatchOp::MaskAny { mask } => Some(Self::AnyBit(mask)),
        }
    }

    /// `Schema::holds(field, value, label)` as a predicate on `value`.
    fn label(dim: &DimensionSpec, field: &BitFieldSpec, label: u128) -> Self {
        if dim.schema().is_flag_field(field.name) && label != 0 {
            Self::AllBits(label)
        } else {
            Self::Eq(label)
        }
    }

    pub(crate) const fn holds(self, value: u128) -> bool {
        match self {
            Self::Eq(v) => value == v,
            Self::Range(min, max) => value >= min && value <= max,
            Self::AllBits(mask) => value & mask == mask,
            Self::AnyBit(mask) => value & mask != 0,
        }
    }
}

/// Every predicate on `field` of `dim` that decides a matcher answer or the
/// word's legality: the given clause ops, the end item's value in the field,
/// the field's invalid and reserved ranges, and the labels its requires and
/// conflicts relations test. Sorted, without repeats.
pub(crate) fn field_preds(
    dim: &DimensionSpec,
    field: &BitFieldSpec,
    clauses: &[ResolvedClause],
    end: Option<u128>,
) -> Vec<Pred> {
    let schema = dim.schema();
    let mut preds: Vec<Pred> = clauses
        .iter()
        .filter(|c| c.field.name == field.name)
        .filter_map(|c| Pred::of_op(dim, field, c.op))
        .collect();
    preds.extend(end.map(|w| Pred::Eq(field_value(field, w))));
    preds.extend(
        schema
            .ranges
            .iter()
            .filter(|r| r.field == field.name)
            .map(|r| Pred::Range(r.min, r.max)),
    );
    for rel in schema.relations {
        for (on, label) in [
            (rel.field, rel.when_label),
            (rel.other_field, rel.other_label),
        ] {
            if on == field.name {
                if let Some(v) = schema.label_value(on, label) {
                    preds.push(Pred::label(dim, field, v));
                }
            }
        }
    }
    preds.sort_unstable();
    preds.dedup();
    preds
}

/// Field values inside `0..=max` meeting every constraint: an interval, bits
/// that must be set, bits that must be clear, values excluded, masks with
/// some bit set and masks with some bit clear.
#[derive(Debug, Clone)]
struct Region {
    max: u128,
    lo: u128,
    hi: u128,
    ones: u128,
    zeros: u128,
    not: Vec<u128>,
    some_one: Vec<u128>,
    some_zero: Vec<u128>,
}

impl Region {
    fn all(max: u128) -> Self {
        Self {
            max,
            lo: 0,
            hi: max,
            ones: 0,
            zeros: !max,
            not: Vec::new(),
            some_one: Vec::new(),
            some_zero: Vec::new(),
        }
    }

    /// The part of the region where `pred` gives `truth`: one region, or two
    /// for a value outside a range (below it and above it).
    fn split(&self, pred: Pred, truth: bool) -> Vec<Self> {
        let mut r = self.clone();
        match (pred, truth) {
            (Pred::Eq(v), true) => {
                r.lo = r.lo.max(v);
                r.hi = r.hi.min(v);
            }
            (Pred::Eq(v), false) => r.not.push(v),
            (Pred::Range(min, max), true) => {
                r.lo = r.lo.max(min);
                r.hi = r.hi.min(max);
            }
            (Pred::Range(min, max), false) => {
                let mut out = Vec::new();
                if let Some(below) = min.checked_sub(1) {
                    let mut b = self.clone();
                    b.hi = b.hi.min(below);
                    out.push(b);
                }
                if let Some(above) = max.checked_add(1) {
                    let mut a = self.clone();
                    a.lo = a.lo.max(above);
                    out.push(a);
                }
                return out;
            }
            (Pred::AllBits(mask), true) => r.ones |= mask,
            (Pred::AllBits(mask), false) => r.some_zero.push(mask),
            (Pred::AnyBit(mask), true) => r.some_one.push(mask),
            (Pred::AnyBit(mask), false) => r.zeros |= mask,
        }
        vec![r]
    }

    /// A value in the region, `Some(None)` when it is empty, `None` when the
    /// search spent `budget`.
    fn witness(&self, budget: &mut u128) -> Option<Option<u128>> {
        *budget = budget.checked_sub(1)?;
        if self.ones & self.zeros != 0 || self.lo > self.hi {
            return Some(None);
        }
        // A mask with no bit set yet: try each bit it may still set.
        if let Some(mask) = self.some_one.iter().find(|m| **m & self.ones == 0) {
            for bit in bits(mask & !self.zeros) {
                let mut r = self.clone();
                r.ones |= bit;
                if let Some(w) = r.witness(budget)? {
                    return Some(Some(w));
                }
            }
            return Some(None);
        }
        // A mask with no bit clear yet: try each bit it may still clear.
        if let Some(mask) = self.some_zero.iter().find(|m| **m & self.zeros == 0) {
            for bit in bits(mask & !self.ones) {
                let mut r = self.clone();
                r.zeros |= bit;
                if let Some(w) = r.witness(budget)? {
                    return Some(Some(w));
                }
            }
            return Some(None);
        }
        let mut from = self.lo;
        loop {
            let Some(x) = least_from(from, self.ones, self.zeros, self.max) else {
                return Some(None);
            };
            if x > self.hi {
                return Some(None);
            }
            if !self.not.contains(&x) {
                return Some(Some(x));
            }
            match x.checked_add(1) {
                Some(next) => from = next,
                None => return Some(None),
            }
        }
    }
}

/// Each set bit of `mask`, lowest first.
fn bits(mask: u128) -> impl Iterator<Item = u128> {
    (0..u128::BITS)
        .map(|b| 1u128 << b)
        .filter(move |b| mask & b != 0)
}

/// `c`'s low bits laid into the set bits of `mask`, lowest first.
fn deposit(mut c: u128, mask: u128) -> u128 {
    let mut out = 0;
    for bit in bits(mask) {
        if c & 1 == 1 {
            out |= bit;
        }
        c >>= 1;
    }
    out
}

/// The least value at or above `lo`, inside `max`, with every bit of `ones`
/// set and every bit of `zeros` clear.
fn least_from(lo: u128, ones: u128, zeros: u128, max: u128) -> Option<u128> {
    let free = max & !ones & !zeros;
    let top = match free.count_ones() {
        128 => u128::MAX,
        n => (1u128 << n) - 1,
    };
    // `ones | deposit(c, free)` grows with `c`: find the least `c` reaching `lo`.
    let at = |c: u128| ones | deposit(c, free);
    if at(top) < lo {
        return None;
    }
    let (mut low, mut high) = (0u128, top);
    while low < high {
        let mid = low + (high - low) / 2;
        if at(mid) >= lo {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    Some(at(low))
}

/// One value per class of `0..=max` under `preds`, keyed by the answers of
/// `preds` in order. `None` past `cap` classes or past the search budget.
pub(crate) fn classes(preds: &[Pred], max: u128, cap: u128) -> Option<BTreeMap<Vec<bool>, u128>> {
    fn walk(
        region: &Region,
        preds: &[Pred],
        answers: &mut Vec<bool>,
        out: &mut BTreeMap<Vec<bool>, u128>,
        budget: &mut u128,
        cap: u128,
    ) -> Option<()> {
        let Some((&pred, rest)) = preds.split_first() else {
            let w = region.witness(budget)??;
            out.entry(answers.clone()).or_insert(w);
            return (out.len() as u128 <= cap).then_some(());
        };
        for truth in [true, false] {
            for part in region.split(pred, truth) {
                if part.witness(budget)?.is_some() {
                    answers.push(truth);
                    walk(&part, rest, answers, out, budget, cap)?;
                    answers.pop();
                }
            }
        }
        Some(())
    }
    let mut out = BTreeMap::new();
    // Each class is reached by at most three regions per predicate (a range
    // split), each asked for a witness once; the rest is mask branching.
    let mut budget = cap
        .saturating_mul(preds.len() as u128 + 1)
        .saturating_mul(8);
    walk(
        &Region::all(max),
        preds,
        &mut Vec::new(),
        &mut out,
        &mut budget,
        cap,
    )?;
    Some(out)
}

/// The values `field` takes in the domain: with `labelled`, its labelled
/// values (its default when it has none); then one more value for each class
/// no such value is in. Without `labelled`, one value per class.
pub(crate) fn field_domain(
    dim: &DimensionSpec,
    field: &BitFieldSpec,
    preds: &[Pred],
    labelled: bool,
    cap: u128,
) -> Option<Vec<u128>> {
    let schema = dim.schema();
    let mut values: Vec<u128> = Vec::new();
    if labelled {
        values.extend(
            schema
                .labels
                .iter()
                .filter(|l| l.field == field.name)
                .map(|l| l.value),
        );
        if values.is_empty() {
            values.push(field_value(field, dim.default_word()));
        }
    }
    let answers = |v: u128| -> Vec<bool> { preds.iter().map(|p| p.holds(v)).collect() };
    let covered: Vec<Vec<bool>> = values.iter().map(|v| answers(*v)).collect();
    for (key, w) in classes(preds, field.max_value(), cap)? {
        if !covered.contains(&key) {
            values.push(w);
        }
    }
    values.sort_unstable();
    values.dedup();
    Some(values)
}

/// Every clause that reads dimension `d`: the contracts' clauses and, on the
/// item dimension, the recognizer's item matchers.
pub(crate) fn clauses_on(spec: &ProgramSpec, d: usize) -> Vec<ResolvedClause> {
    let contracts = spec.resolved.iter().flat_map(|c| {
        c.when
            .iter()
            .chain(&c.require)
            .chain(c.require_any.iter().flatten())
            .chain(&c.forbid)
    });
    let items = spec
        .sequences()
        .iter()
        .flat_map(|s| &s.items)
        .flat_map(|i| match i {
            ResolvedItem::Any => &[][..],
            ResolvedItem::OneOf(options) => options.as_slice(),
        });
    contracts
        .chain(items)
        .filter(|c| c.dim == d)
        .copied()
        .collect()
}

/// The domain of one dimension: every combination of each field's domain
/// values (with or without its labelled values), kept when flaga_solver
/// accepts it. `None` past `cap` words.
pub(crate) fn dimension_words(
    dim: &DimensionSpec,
    clauses: &[ResolvedClause],
    end: Option<u128>,
    labelled: bool,
    cap: u128,
) -> Option<Vec<u128>> {
    let schema = dim.schema();
    let mut words = vec![dim.default_word()];
    for field in schema.fields {
        let preds = field_preds(dim, field, clauses, end);
        let values = field_domain(dim, field, &preds, labelled, cap)?;
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

/// The domain of observed evidence dimension `d` of `spec`.
pub(crate) fn evidence_words(spec: &ProgramSpec, d: usize, cap: u128) -> Option<Vec<u128>> {
    let end = spec.end_word().filter(|_| spec.item_dim() == Some(d));
    dimension_words(&spec.dims[d], &clauses_on(spec, d), end, true, cap)
}

/// A legal word of `dim` on which every clause of `pos` holds and no clause
/// of `neg` does: `Some(None)` when there is none, `None` when the domain
/// passes `cap` words. The domain is one legal word for every combination
/// of classes a legal word can have under these clauses and the legality
/// rules, so it holds such a word exactly when some legal word is one.
pub(crate) fn witness(
    dim: &DimensionSpec,
    pos: &[&ResolvedClause],
    neg: &[&ResolvedClause],
    cap: u128,
) -> Option<Option<u128>> {
    let schema = dim.schema();
    let clauses: Vec<ResolvedClause> = pos.iter().chain(neg).map(|c| **c).collect();
    let words = dimension_words(dim, &clauses, None, false, cap)?;
    Some(words.into_iter().find(|w| {
        pos.iter().all(|c| c.holds(schema, *w)) && !neg.iter().any(|c| c.holds(schema, *w))
    }))
}

#[cfg(test)]
mod tests {
    //! The domain against brute force, on schemas small enough to list every
    //! word: for many clause sets, the matcher answers the domain gives are
    //! exactly the answers some legal word gives.

    use std::collections::BTreeSet;

    use big_bad_bool::bitpack;

    use super::*;

    /// Eight bits: a labelled field with an invalid value, a requires and a
    /// conflicts relation, a flag field, and an unlabelled field with a
    /// reserved value.
    #[bitpack(storage = u8)]
    pub struct Tiny {
        #[bits(3)]
        #[labels(low = 1, high = 5)]
        #[requires(high => marks.hot)]
        #[invalid(7)]
        pub level: u8,
        #[bits(3)]
        #[labels(hot = 1, cold = 2, wet = 4)]
        #[flags(hot = 0, cold = 1, wet = 2)]
        #[conflicts(hot, wet)]
        pub marks: u8,
        #[bits(2)]
        #[reserved(3)]
        pub spare: u8,
    }

    /// Ten bits: a wide field with labels only at its ends and an invalid
    /// run, and an unlabelled field with a default.
    #[bitpack(storage = u16)]
    pub struct Wide {
        #[bits(6)]
        #[labels(zero = 0, top = 63)]
        #[invalid(40..=47)]
        pub n: u16,
        #[bits(4)]
        #[default(2)]
        pub m: u16,
    }

    /// A small deterministic generator, so a failure names its seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u128) -> u128 {
            u128::from(self.next()) % n
        }
    }

    fn legal(dim: &DimensionSpec, w: u128) -> bool {
        flaga_solver::check(dim.schema(), w).is_ok_ignoring_deprecated()
    }

    /// Every word the layout can hold.
    fn every_word(dim: &DimensionSpec) -> impl Iterator<Item = u128> {
        let layout = dim.schema().fields.iter().fold(0, |acc, f| acc | f.mask);
        0..=layout
    }

    /// A clause on a random field with a random op and in-range constants.
    fn random_clause(dim: &DimensionSpec, rng: &mut Rng) -> ResolvedClause {
        let fields = dim.schema().fields;
        let field = fields[rng.below(fields.len() as u128) as usize];
        let max = field.max_value();
        let labels: Vec<u128> = dim
            .schema()
            .labels
            .iter()
            .filter(|l| l.field == field.name)
            .map(|l| l.value)
            .collect();
        let value = |rng: &mut Rng| {
            if !labels.is_empty() && rng.below(3) == 0 {
                labels[rng.below(labels.len() as u128) as usize]
            } else {
                rng.below(max + 1)
            }
        };
        let op = match rng.below(6) {
            0 => MatchOp::Any,
            1 => MatchOp::Eq(value(rng)),
            2 => MatchOp::NotEq(value(rng)),
            3 => {
                let (a, b) = (rng.below(max + 1), rng.below(max + 1));
                MatchOp::RangeInclusive {
                    min: a.min(b),
                    max: a.max(b),
                }
            }
            4 => MatchOp::MaskAll {
                mask: rng.below(max + 1),
            },
            _ => MatchOp::MaskAny {
                mask: rng.below(max + 1),
            },
        };
        ResolvedClause {
            dim: 0,
            dim_name: dim.name(),
            field,
            op,
        }
    }

    /// What every clause answers for `w`, and whether `w` is the end word.
    fn answers(
        dim: &DimensionSpec,
        clauses: &[ResolvedClause],
        end: Option<u128>,
        w: u128,
    ) -> Vec<bool> {
        clauses
            .iter()
            .map(|c| c.holds(dim.schema(), w))
            .chain(end.map(|e| e == w))
            .collect()
    }

    /// The domain as it was before matcher classes: every combination of
    /// labelled values (the default where a field has none), legal ones only.
    fn labelled_only(dim: &DimensionSpec) -> Vec<u128> {
        let mut words = vec![dim.default_word()];
        for field in dim.schema().fields {
            let mut values: Vec<u128> = dim
                .schema()
                .labels
                .iter()
                .filter(|l| l.field == field.name)
                .map(|l| l.value)
                .collect();
            if values.is_empty() {
                values.push(field_value(field, dim.default_word()));
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
        words.retain(|w| legal(dim, *w));
        words
    }

    /// Brute force and the domain give one set of answers; every domain
    /// word is legal; every labelled word is still in the domain.
    fn check(dim: &DimensionSpec, clauses: &[ResolvedClause], end: Option<u128>, seed: u64) {
        let brute: BTreeSet<Vec<bool>> = every_word(dim)
            .filter(|w| legal(dim, *w))
            .map(|w| answers(dim, clauses, end, w))
            .collect();
        let domain = dimension_words(dim, clauses, end, true, 1 << 20).expect("within the cap");
        assert!(
            domain.iter().all(|w| legal(dim, *w)),
            "seed {seed}: an illegal domain word"
        );
        let modelled: BTreeSet<Vec<bool>> = domain
            .iter()
            .map(|w| answers(dim, clauses, end, *w))
            .collect();
        assert_eq!(
            brute, modelled,
            "seed {seed}: clauses {clauses:?}, end {end:?}, domain {domain:?}"
        );
        for w in labelled_only(dim) {
            assert!(
                domain.contains(&w),
                "seed {seed}: labelled word {w} dropped"
            );
        }
    }

    fn sweep(dim: &DimensionSpec, runs: u64) {
        let every: Vec<u128> = every_word(dim).collect();
        for seed in 1..=runs {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let n = 1 + rng.below(6) as usize;
            let clauses: Vec<ResolvedClause> =
                (0..n).map(|_| random_clause(dim, &mut rng)).collect();
            let end = (rng.below(2) == 0).then(|| every[rng.below(every.len() as u128) as usize]);
            check(dim, &clauses, end, seed);
        }
    }

    #[test]
    fn tiny_schema_domain_gives_every_answer_set_brute_force_gives() {
        sweep(&crate::dimension!(Tiny, "tiny", Evidence), 1_500);
    }

    #[test]
    fn wide_schema_domain_gives_every_answer_set_brute_force_gives() {
        sweep(&crate::dimension!(Wide, "wide", Evidence), 400);
    }

    /// Over every legal word of `dim`: some word meets `pos` and none of `neg`.
    fn brute_witness(
        dim: &DimensionSpec,
        pos: &[&ResolvedClause],
        neg: &[&ResolvedClause],
    ) -> bool {
        every_word(dim).filter(|w| legal(dim, *w)).any(|w| {
            pos.iter().all(|c| c.holds(dim.schema(), w))
                && !neg.iter().any(|c| c.holds(dim.schema(), w))
        })
    }

    /// `witness`, as `satisfiable` asks it for a state dimension, against
    /// brute force over every legal word, for random positive and FORBID
    /// clause sets. A found word is legal and meets the clauses.
    fn witness_sweep(dim: &DimensionSpec, runs: u64) {
        let mut found = 0;
        for seed in 1..=runs {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
            let pos: Vec<ResolvedClause> = (0..1 + rng.below(4))
                .map(|_| random_clause(dim, &mut rng))
                .collect();
            let neg: Vec<ResolvedClause> = (0..rng.below(4))
                .map(|_| random_clause(dim, &mut rng))
                .collect();
            let pos: Vec<&ResolvedClause> = pos.iter().collect();
            let neg: Vec<&ResolvedClause> = neg.iter().collect();
            let got = witness(dim, &pos, &neg, 1 << 20).expect("within the cap");
            assert_eq!(
                got.is_some(),
                brute_witness(dim, &pos, &neg),
                "seed {seed}: pos {pos:?}, neg {neg:?}, got {got:?}"
            );
            if let Some(w) = got {
                found += 1;
                assert!(legal(dim, w), "seed {seed}: illegal witness {w}");
                assert!(pos.iter().all(|c| c.holds(dim.schema(), w)));
                assert!(!neg.iter().any(|c| c.holds(dim.schema(), w)));
            }
        }
        // Both answers occur, so the sweep tests both directions.
        assert!(found > 0 && found < runs, "{found} of {runs}");
    }

    #[test]
    fn tiny_state_witness_agrees_with_brute_force() {
        witness_sweep(&crate::dimension!(Tiny, "tiny", State), 3_000);
    }

    #[test]
    fn wide_state_witness_agrees_with_brute_force() {
        witness_sweep(&crate::dimension!(Wide, "wide", State), 1_000);
    }

    #[test]
    fn no_clauses_leave_the_labelled_domain_as_it_was() {
        for dim in [
            crate::dimension!(Tiny, "tiny", Evidence),
            crate::dimension!(Wide, "wide", Evidence),
        ] {
            let domain = dimension_words(&dim, &[], None, true, 1 << 20).unwrap();
            let mut labelled = labelled_only(&dim);
            labelled.sort_unstable();
            let mut sorted = domain.clone();
            sorted.sort_unstable();
            // Legality rules alone can add a class no label is in, e.g. the
            // unlabelled values of `Tiny.level` next to its invalid 7.
            assert!(labelled.iter().all(|w| sorted.contains(w)));
        }
        // With no rule and no clause on a field, its labels are its domain.
        let wide = crate::dimension!(Wide, "wide", Evidence);
        let m = wide.schema().field("m").unwrap();
        assert_eq!(field_domain(&wide, m, &[], true, 64).unwrap(), [2]);
    }

    #[test]
    fn item_matchers_and_the_end_item_split_the_item_dimension() {
        use big_bad_bool_interact::FieldMatcher;

        use crate::model::{ItemMatch, RecognizerSpec, SequenceSpec, sequence};
        use crate::test_support::{RECOGNIZER, builder};

        const SEQUENCES: &[SequenceSpec] = &[sequence(
            "pair",
            &[
                ItemMatch::One(FieldMatcher::range("kind", 1, 2)),
                ItemMatch::Any,
            ],
        )];
        let spec = builder()
            .recognize(RecognizerSpec {
                sequences: SEQUENCES,
                ..RECOGNIZER
            })
            .build()
            .unwrap();
        let item = spec.item_dim().unwrap();
        let dim = &spec.dims[item];
        let clauses = clauses_on(&spec, item);
        assert_eq!(clauses.len(), 1);
        let preds = field_preds(dim, &dim.schema().fields[0], &clauses, spec.end_word());
        assert_eq!(preds, [Pred::Eq(3), Pred::Range(1, 2)]);
        // The frame dimension is written by the recognizer: no clause reads it here.
        assert!(clauses_on(&spec, spec.frame_dim().unwrap()).is_empty());
    }

    #[test]
    fn classes_split_ranges_and_masks_without_listing_values() {
        // 0..=2^100: two overlapping ranges and a mask, three predicates.
        let max = (1u128 << 100) - 1;
        let preds = [
            Pred::Range(10, 1 << 90),
            Pred::Range(1 << 80, max),
            Pred::AllBits(1 << 85 | 1),
        ];
        let found = classes(&preds, max, 64).unwrap();
        for (key, w) in &found {
            let got: Vec<bool> = preds.iter().map(|p| p.holds(*w)).collect();
            assert_eq!(&got, key);
        }
        // Below 10 and below 2^80 the mask's bit 85 cannot be set: one class
        // each. Inside both ranges and above the first, with and without the
        // mask: two classes each.
        assert_eq!(found.len(), 6);
        assert!(found.contains_key(&vec![true, true, true]));
        assert!(!found.contains_key(&vec![true, false, true]));
        assert_eq!(least_from(11, 1, 0, max), Some(11));
        assert_eq!(least_from(12, 1, 0, max), Some(13));
        assert_eq!(least_from(0, 1 << 99, 0, max), Some(1 << 99));
        assert_eq!(least_from(1, 0, max, max), None);
    }
}
