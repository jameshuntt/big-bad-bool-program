//! The program spec: dimensions, clauses, writes, sequences, containment, and
//! the builder that resolves names once.
//!
//! Everything here is data. Nothing here evaluates a contract or writes state.
//! `build` resolves every name to an index and a field mask, checks every
//! value against its field, and checks every write against its dimension's
//! role. After `build`, the other modules use only indices and masks.

use big_bad_bool_core::{BitFieldSpec, PackedWord};
use big_bad_bool_interact::{FieldMatcher, FieldValue, MatchOp, ReconstructionError, patch_word};
use flaga_solver::Schema;

use crate::codes::Finding;

/// The longest sequence `build` accepts. The window radius is this minus one at most.
pub const MAX_SEQUENCE_LEN: usize = 16;

/// Index of a dimension, in the order it was added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DimensionId(pub u16);

/// Index of a contract, in the order it was added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContractId(pub u16);

/// Index of a sequence in the recognizer. Its frame key is the index plus one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SequenceId(pub u16);

impl SequenceId {
    /// The value the frame word's sequence field holds for this sequence. Zero means no sequence.
    #[must_use]
    pub const fn key(self) -> u128 {
        self.0 as u128 + 1
    }
}

/// Who may write a dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimensionRole {
    /// Written by the input and the recognizer. Contracts read it.
    Evidence,
    /// Written only by commit. Contracts read it.
    State,
    /// Written only by commit, once per step, through `emit`. Nobody reads it inside a step.
    Effect,
}

impl DimensionRole {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::State => "state",
            Self::Effect => "effect",
        }
    }
}

/// Field writes on one raw word, through interact's `patch_word` for the word's own type.
pub(crate) type PatchFn = fn(u128, &[FieldValue]) -> Result<u128, ReconstructionError>;

fn patch_raw<T: PackedWord>(raw: u128, values: &[FieldValue]) -> Result<u128, ReconstructionError> {
    let word = T::try_from_raw_u128_checked(raw)
        .map_err(|error| ReconstructionError::RawRejected { raw, error })?;
    patch_word(word, values).map(PackedWord::raw_u128)
}

/// One `#[bitpack]` type with a name and a role. Build it with [`dimension!`](crate::dimension!).
#[derive(Debug, Clone, Copy)]
pub struct DimensionSpec {
    name: &'static str,
    schema: Schema<'static>,
    role: DimensionRole,
    stack_depth: Option<u16>,
    patch: PatchFn,
}

/// A dimension over the packed word `T`. `schema` must be `schema_of!(T)`;
/// the [`dimension!`](crate::dimension!) macro passes both from one type.
#[must_use]
pub const fn dimension<T: PackedWord>(
    name: &'static str,
    schema: Schema<'static>,
    role: DimensionRole,
) -> DimensionSpec {
    DimensionSpec {
        name,
        schema,
        role,
        stack_depth: None,
        patch: patch_raw::<T>,
    }
}

impl DimensionSpec {
    /// A state dimension kept as a bounded stack. Contracts see the top entry.
    #[must_use]
    pub const fn stacked(mut self, max_depth: u16) -> Self {
        self.stack_depth = Some(max_depth);
        self
    }

    /// The dimension's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The word's generated tables.
    #[must_use]
    pub const fn schema(&self) -> &Schema<'static> {
        &self.schema
    }

    /// Who may write it.
    #[must_use]
    pub const fn role(&self) -> DimensionRole {
        self.role
    }

    /// The stack bound, for a stacked dimension.
    #[must_use]
    pub const fn stack_depth(&self) -> Option<u16> {
        self.stack_depth
    }

    /// Every field at its `#[default(n)]`, zero where none is given.
    #[must_use]
    pub fn default_word(&self) -> u128 {
        self.schema
            .fields
            .iter()
            .fold(0, |acc, f| acc | (f.default.unwrap_or(0) << f.shift))
    }

    pub(crate) fn patch(
        &self,
        raw: u128,
        values: &[FieldValue],
    ) -> Result<u128, ReconstructionError> {
        (self.patch)(raw, values)
    }

    /// The word as `field=label` pairs, in field order.
    #[must_use]
    pub fn describe(&self, word: u128) -> String {
        let parts: Vec<String> = self
            .schema
            .fields
            .iter()
            .map(|f| {
                format!(
                    "{}={}",
                    f.name,
                    render_value(&self.schema, f, field_value(f, word))
                )
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }
}

/// One condition: a dimension name and an interact matcher on one of its fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Clause {
    /// The dimension.
    pub dimension: &'static str,
    /// The field condition.
    pub matcher: FieldMatcher,
}

/// One field write on one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Write {
    /// The dimension.
    pub dimension: &'static str,
    /// The field and its new value.
    pub value: FieldValue,
}

/// A push or pop on a stacked state dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackOp {
    /// Push the dimension's default word with these fields set.
    Push {
        /// The stacked dimension.
        dimension: &'static str,
        /// Fields of the new entry.
        values: &'static [FieldValue],
    },
    /// Pop the top entry.
    Pop {
        /// The stacked dimension.
        dimension: &'static str,
    },
}

/// A contract: conditions over a fact view, and the writes it proposes.
///
/// WHEN decides whether it applies. REQUIRE, REQUIRE-ANY and FORBID decide
/// whether it is refused. TRANSITION, STACK and EMIT are applied only by commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractSpec {
    /// The contract's name.
    pub name: &'static str,
    /// All must hold, or the contract does not apply.
    pub when: &'static [Clause],
    /// All must hold, or the contract is refused.
    pub require: &'static [Clause],
    /// One group must hold in full, or the contract is refused.
    pub require_any: &'static [&'static [Clause]],
    /// None may hold, or the contract is refused.
    pub forbid: &'static [Clause],
    /// State field writes.
    pub transition: &'static [Write],
    /// Stack pushes and pops.
    pub stack: &'static [StackOp],
    /// Effect field writes.
    pub emit: &'static [Write],
}

/// A condition on one dimension.
#[must_use]
pub const fn clause(dimension: &'static str, matcher: FieldMatcher) -> Clause {
    Clause { dimension, matcher }
}

/// A write of `value` to `dimension.field`.
#[must_use]
pub const fn set(dimension: &'static str, field: &'static str, value: u128) -> Write {
    Write {
        dimension,
        value: FieldValue::new(field, value),
    }
}

/// Push an entry on a stacked dimension.
#[must_use]
pub const fn push(dimension: &'static str, values: &'static [FieldValue]) -> StackOp {
    StackOp::Push { dimension, values }
}

/// Pop the top entry of a stacked dimension.
#[must_use]
pub const fn pop(dimension: &'static str) -> StackOp {
    StackOp::Pop { dimension }
}

/// A contract with nothing in it yet.
#[must_use]
pub const fn contract(name: &'static str) -> ContractSpec {
    ContractSpec {
        name,
        when: &[],
        require: &[],
        require_any: &[],
        forbid: &[],
        transition: &[],
        stack: &[],
        emit: &[],
    }
}

impl ContractSpec {
    /// Sets the WHEN clauses.
    #[must_use]
    pub const fn when(mut self, clauses: &'static [Clause]) -> Self {
        self.when = clauses;
        self
    }

    /// Sets the REQUIRE clauses.
    #[must_use]
    pub const fn require(mut self, clauses: &'static [Clause]) -> Self {
        self.require = clauses;
        self
    }

    /// Sets the REQUIRE-ANY groups.
    #[must_use]
    pub const fn require_any(mut self, groups: &'static [&'static [Clause]]) -> Self {
        self.require_any = groups;
        self
    }

    /// Sets the FORBID clauses.
    #[must_use]
    pub const fn forbid(mut self, clauses: &'static [Clause]) -> Self {
        self.forbid = clauses;
        self
    }

    /// Sets the state writes.
    #[must_use]
    pub const fn transition(mut self, writes: &'static [Write]) -> Self {
        self.transition = writes;
        self
    }

    /// Sets the stack ops.
    #[must_use]
    pub const fn stack(mut self, ops: &'static [StackOp]) -> Self {
        self.stack = ops;
        self
    }

    /// Sets the effect writes.
    #[must_use]
    pub const fn emit(mut self, writes: &'static [Write]) -> Self {
        self.emit = writes;
        self
    }
}

/// What one item of a sequence accepts, as matchers on the item dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemMatch {
    /// Any item, including the end item.
    Any,
    /// An item this matcher holds for.
    One(FieldMatcher),
    /// An item one of these matchers holds for.
    OneOf(&'static [FieldMatcher]),
}

/// Frame word fields the recognizer sets for one offset of a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OffsetFacts {
    /// The offset inside the sequence.
    pub offset: u8,
    /// Frame word fields for an item at that offset.
    pub values: &'static [FieldValue],
}

/// A declared sequence of item classes. Only declared sequences become frame words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceSpec {
    /// The sequence's name.
    pub name: &'static str,
    /// One matcher per item.
    pub items: &'static [ItemMatch],
    /// Facts per offset. Several entries for one offset are applied in order.
    pub facts: &'static [OffsetFacts],
}

/// A sequence with no facts yet.
#[must_use]
pub const fn sequence(name: &'static str, items: &'static [ItemMatch]) -> SequenceSpec {
    SequenceSpec {
        name,
        items,
        facts: &[],
    }
}

impl SequenceSpec {
    /// Sets the facts per offset.
    #[must_use]
    pub const fn facts(mut self, facts: &'static [OffsetFacts]) -> Self {
        self.facts = facts;
        self
    }
}

/// Where the recognizer reads items, where it writes frame words, and what it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecognizerSpec {
    /// Evidence dimension holding each observation's item word.
    pub item_dimension: &'static str,
    /// Evidence dimension the frame words belong to.
    pub frame_dimension: &'static str,
    /// Frame field for the sequence key (sequence index plus one).
    pub sequence_field: &'static str,
    /// Frame field for the offset inside the sequence.
    pub offset_field: &'static str,
    /// Frame field for [`Position`](crate::Position).
    pub position_field: &'static str,
    /// Frame flag set when the item ends one match and begins another.
    pub boundary_flag: &'static str,
    /// The item past the last observation. The stepper pads with it.
    pub end_item: FieldValue,
    /// The declared sequences.
    pub sequences: &'static [SequenceSpec],
}

/// What the stepper does after a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalRule {
    /// Stop stepping.
    Stop,
    /// Leave state as it was and go on with the next position.
    SkipObservation,
}

/// Containment limits, applied at every position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContainmentSpec {
    /// Frame interpretations per position, after longest-match.
    pub max_interpretations: u8,
    /// Candidates per position: contract and frame pairs, before merging.
    pub max_candidates: u8,
    /// Distinct contracts that actualize together at one position.
    pub max_actualized: u8,
    /// The deepest any stacked dimension may be declared.
    pub max_stack_depth: u16,
    /// What happens after a refusal.
    pub on_refusal: RefusalRule,
}

/// Where the focal item sits in one matched sequence. The values are fixed:
/// the frame dimension's position field holds them, and zero means no sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Position {
    /// Offset zero of a sequence longer than one.
    Begin = 1,
    /// Neither the first nor the last item.
    Interior = 2,
    /// The last item of a sequence longer than one.
    End = 3,
    /// The only item of a one-item sequence.
    Complete = 4,
}

impl Position {
    /// The position field value.
    #[must_use]
    pub const fn value(self) -> u128 {
        self as u128
    }

    /// The position of `offset` in a sequence of `len` items.
    #[must_use]
    pub const fn of(offset: usize, len: usize) -> Self {
        if len == 1 {
            Self::Complete
        } else if offset == 0 {
            Self::Begin
        } else if offset + 1 == len {
            Self::End
        } else {
            Self::Interior
        }
    }
}

/// One input position and the evidence words observed there.
///
/// An observation holds at most one word per evidence dimension, in
/// dimension order. A dimension it does not hold reads as nothing: no clause
/// on it holds, as with an empty stack. With a recognizer, the item word is
/// the item dimension's word, and the frame dimension is written by the
/// recognizer, never observed. The stepper checks every word with
/// flaga_solver before it is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Observation {
    position: u32,
    evidence: Vec<(DimensionId, u128)>,
}

impl Observation {
    /// An observation at `position` holding no evidence yet.
    #[must_use]
    pub const fn new(position: u32) -> Self {
        Self {
            position,
            evidence: Vec::new(),
        }
    }

    /// Sets `dimension`'s raw word. A second word for one dimension replaces the first.
    #[must_use]
    pub fn with(mut self, dimension: DimensionId, word: u128) -> Self {
        match self.evidence.binary_search_by_key(&dimension, |(d, _)| *d) {
            Ok(i) => self.evidence[i].1 = word,
            Err(i) => self.evidence.insert(i, (dimension, word)),
        }
        self
    }

    /// Sets `dimension`'s word from a packed value of its `#[bitpack]` type.
    #[must_use]
    pub fn with_word<T: PackedWord>(self, dimension: DimensionId, word: T) -> Self {
        self.with(dimension, word.raw_u128())
    }

    /// The input position. The application maps it back to its own text.
    #[must_use]
    pub const fn position(&self) -> u32 {
        self.position
    }

    /// `dimension`'s word, when the observation holds one.
    #[must_use]
    pub fn word(&self, dimension: DimensionId) -> Option<u128> {
        self.evidence
            .binary_search_by_key(&dimension, |(d, _)| *d)
            .ok()
            .map(|i| self.evidence[i].1)
    }

    /// Every word held, in dimension order.
    #[must_use]
    pub fn evidence(&self) -> &[(DimensionId, u128)] {
        &self.evidence
    }
}

/// The value of `field` inside `word`.
pub(crate) const fn field_value(field: &BitFieldSpec, word: u128) -> u128 {
    (word & field.mask) >> field.shift
}

/// A field value by its label, its set flag names, or its number.
pub(crate) fn render_value(schema: &Schema<'_>, field: &BitFieldSpec, value: u128) -> String {
    if schema.is_flag_field(field.name) {
        let names: Vec<&str> = schema
            .flags
            .iter()
            .filter(|f| f.field == field.name && value & f.mask != 0)
            .map(|f| f.name)
            .collect();
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join("|")
        }
    } else {
        schema
            .label_for(field.name, value)
            .map_or_else(|| value.to_string(), str::to_owned)
    }
}

/// A clause with its dimension index and field spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedClause {
    pub(crate) dim: usize,
    pub(crate) dim_name: &'static str,
    pub(crate) field: BitFieldSpec,
    pub(crate) op: MatchOp,
}

impl ResolvedClause {
    /// Whether the clause holds for `word`. `Eq` and `NotEq` go through
    /// `Schema::holds`, so a flag-field label holds when its bits are set.
    /// Every other op is interact's `MatchOp::matches`.
    pub(crate) fn holds(&self, schema: &Schema<'_>, word: u128) -> bool {
        let value = field_value(&self.field, word);
        match self.op {
            MatchOp::Eq(label) => schema.holds(self.field.name, value, label),
            MatchOp::NotEq(label) => !schema.holds(self.field.name, value, label),
            op => op.matches(value),
        }
    }

    pub(crate) fn render(&self, schema: &Schema<'_>) -> String {
        let op = match self.op {
            MatchOp::Any => "any".to_owned(),
            MatchOp::Eq(v) => format!("= {}", render_value(schema, &self.field, v)),
            MatchOp::NotEq(v) => format!("!= {}", render_value(schema, &self.field, v)),
            MatchOp::RangeInclusive { min, max } => format!("in {min}..={max}"),
            MatchOp::MaskAll { mask } => format!("has all {mask:#x}"),
            MatchOp::MaskAny { mask } => format!("has any {mask:#x}"),
        };
        format!("{}.{} {op}", self.dim_name, self.field.name)
    }
}

/// Writes to one dimension, in the order given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DimWrites {
    pub(crate) dim: usize,
    pub(crate) values: Vec<FieldValue>,
}

/// A stack op with its dimension index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedStackOp {
    Push { dim: usize, values: Vec<FieldValue> },
    Pop { dim: usize },
}

/// A contract with every name resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedContract {
    pub(crate) name: &'static str,
    pub(crate) when: Vec<ResolvedClause>,
    pub(crate) require: Vec<ResolvedClause>,
    pub(crate) require_any: Vec<Vec<ResolvedClause>>,
    pub(crate) forbid: Vec<ResolvedClause>,
    pub(crate) transition: Vec<DimWrites>,
    pub(crate) stack: Vec<ResolvedStackOp>,
    pub(crate) emit: Vec<DimWrites>,
}

impl ResolvedContract {
    /// The stack ops on `dim`, in order.
    pub(crate) fn stack_on(&self, dim: usize) -> Vec<ResolvedStackOp> {
        self.stack
            .iter()
            .filter(|op| match op {
                ResolvedStackOp::Push { dim: d, .. } | ResolvedStackOp::Pop { dim: d } => *d == dim,
            })
            .cloned()
            .collect()
    }

    /// The fields of `dim` any of the contract's clauses read, in clause order.
    pub(crate) fn reads(&self, dim: usize) -> Vec<&'static str> {
        let mut fields: Vec<&'static str> = Vec::new();
        let all = self
            .when
            .iter()
            .chain(&self.require)
            .chain(self.require_any.iter().flatten())
            .chain(&self.forbid);
        for c in all.filter(|c| c.dim == dim) {
            if !fields.contains(&c.field.name) {
                fields.push(c.field.name);
            }
        }
        fields
    }

    /// True when the contract pushes, pops or writes fields of `dim`.
    pub(crate) fn touches_stack(&self, dim: usize) -> bool {
        !self.stack_on(dim).is_empty() || self.transition.iter().any(|w| w.dim == dim)
    }
}

/// What one sequence item accepts, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedItem {
    Any,
    OneOf(Vec<ResolvedClause>),
}

impl ResolvedItem {
    pub(crate) fn matches(&self, schema: &Schema<'_>, item: u128) -> bool {
        match self {
            Self::Any => true,
            Self::OneOf(options) => options.iter().any(|c| c.holds(schema, item)),
        }
    }
}

/// A sequence with its matchers resolved and its facts grouped per offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSequence {
    pub(crate) name: &'static str,
    pub(crate) items: Vec<ResolvedItem>,
    pub(crate) facts: Vec<Vec<FieldValue>>,
}

impl ResolvedSequence {
    /// The facts declared at `offset`, one value per field: the last one given.
    pub(crate) fn declared(&self, offset: usize) -> Vec<FieldValue> {
        let mut out: Vec<FieldValue> = Vec::new();
        for v in &self.facts[offset] {
            out.retain(|o| o.field != v.field);
            out.push(*v);
        }
        out
    }
}

/// The frame word fields the recognizer owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameLayout {
    pub(crate) sequence: BitFieldSpec,
    pub(crate) offset: BitFieldSpec,
    pub(crate) position: BitFieldSpec,
    pub(crate) boundary: BitFieldSpec,
    pub(crate) boundary_bit: u128,
}

/// A recognizer with every name resolved.
#[derive(Debug, Clone)]
pub(crate) struct Framing {
    pub(crate) recognizer: RecognizerSpec,
    pub(crate) item_dim: usize,
    pub(crate) frame_dim: usize,
    pub(crate) layout: FrameLayout,
    pub(crate) end_word: u128,
    pub(crate) sequences: Vec<ResolvedSequence>,
}

/// A program whose names all resolve and whose values all fit. Built by
/// [`ProgramBuilder::build`]; checked for coherence by [`verify`](crate::verify).
///
/// The recognizer is optional. Without one, a program is dimensions,
/// contracts and containment: observations bring every evidence word, the
/// window radius is zero, and there is no end item.
#[derive(Debug, Clone)]
pub struct ProgramSpec {
    pub(crate) dims: Vec<DimensionSpec>,
    pub(crate) framing: Option<Framing>,
    pub(crate) contracts: Vec<ContractSpec>,
    pub(crate) resolved: Vec<ResolvedContract>,
    pub(crate) containment: ContainmentSpec,
}

impl ProgramSpec {
    /// The dimensions, in the order added.
    #[must_use]
    pub fn dimensions(&self) -> &[DimensionSpec] {
        &self.dims
    }

    /// A dimension by id.
    #[must_use]
    pub fn dimension(&self, id: DimensionId) -> Option<&DimensionSpec> {
        self.dims.get(usize::from(id.0))
    }

    /// A dimension's id by name.
    #[must_use]
    pub fn dimension_id(&self, name: &str) -> Option<DimensionId> {
        self.dims
            .iter()
            .position(|d| d.name == name)
            .map(|i| DimensionId(i as u16))
    }

    /// The contracts, in the order added.
    #[must_use]
    pub fn contracts(&self) -> &[ContractSpec] {
        &self.contracts
    }

    /// A contract by id.
    #[must_use]
    pub fn contract(&self, id: ContractId) -> Option<&ContractSpec> {
        self.contracts.get(usize::from(id.0))
    }

    /// A contract's id by name.
    #[must_use]
    pub fn contract_id(&self, name: &str) -> Option<ContractId> {
        self.contracts
            .iter()
            .position(|c| c.name == name)
            .map(|i| ContractId(i as u16))
    }

    /// The recognizer, when the program has one.
    #[must_use]
    pub fn recognizer(&self) -> Option<&RecognizerSpec> {
        self.framing.as_ref().map(|f| &f.recognizer)
    }

    /// A sequence by id.
    #[must_use]
    pub fn sequence(&self, id: SequenceId) -> Option<&SequenceSpec> {
        self.recognizer()?.sequences.get(usize::from(id.0))
    }

    /// A sequence's id by name.
    #[must_use]
    pub fn sequence_id(&self, name: &str) -> Option<SequenceId> {
        self.recognizer()?
            .sequences
            .iter()
            .position(|s| s.name == name)
            .map(|i| SequenceId(i as u16))
    }

    /// The containment limits.
    #[must_use]
    pub const fn containment(&self) -> &ContainmentSpec {
        &self.containment
    }

    /// Items of context on each side of the focal item: the longest
    /// sequence minus one. Zero without a recognizer.
    #[must_use]
    pub fn window_radius(&self) -> usize {
        self.sequences()
            .iter()
            .map(|s| s.items.len().saturating_sub(1))
            .max()
            .unwrap_or(0)
    }

    /// The item word past the last observation, when there is a recognizer.
    #[must_use]
    pub fn end_word(&self) -> Option<u128> {
        self.framing.as_ref().map(|f| f.end_word)
    }

    /// The recognizer's item dimension.
    #[must_use]
    pub fn item_dimension(&self) -> Option<DimensionId> {
        self.item_dim().map(|i| DimensionId(i as u16))
    }

    /// The recognizer's frame dimension.
    #[must_use]
    pub fn frame_dimension(&self) -> Option<DimensionId> {
        self.frame_dim().map(|i| DimensionId(i as u16))
    }

    pub(crate) fn item_dim(&self) -> Option<usize> {
        self.framing.as_ref().map(|f| f.item_dim)
    }

    pub(crate) fn frame_dim(&self) -> Option<usize> {
        self.framing.as_ref().map(|f| f.frame_dim)
    }

    /// The resolved sequences; none without a recognizer.
    pub(crate) fn sequences(&self) -> &[ResolvedSequence] {
        self.framing.as_ref().map_or(&[], |f| &f.sequences)
    }

    /// True for an evidence dimension an observation may hold: every
    /// evidence dimension except the one the recognizer writes.
    #[must_use]
    pub fn is_observed(&self, dimension: DimensionId) -> bool {
        let i = usize::from(dimension.0);
        self.dims
            .get(i)
            .is_some_and(|d| d.role == DimensionRole::Evidence && Some(i) != self.frame_dim())
    }

    /// The dimensions an observation may hold, in order.
    pub(crate) fn observed(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.dims.len()).filter(|i| self.is_observed(DimensionId(*i as u16)))
    }
}

/// The assembly surface. `Program::new()` starts a builder.
#[derive(Debug, Clone, Copy)]
pub struct Program;

impl Program {
    /// A builder with nothing in it.
    #[allow(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new() -> ProgramBuilder {
        ProgramBuilder::default()
    }
}

/// Collects dimensions, a recognizer, contracts and containment.
#[derive(Debug, Clone, Default)]
pub struct ProgramBuilder {
    dims: Vec<DimensionSpec>,
    recognizer: Option<RecognizerSpec>,
    contracts: Vec<ContractSpec>,
    containment: Option<ContainmentSpec>,
}

impl ProgramBuilder {
    /// Adds a dimension. The name is the report's assembly verb, not `ops::Add`.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(mut self, dimension: DimensionSpec) -> Self {
        self.dims.push(dimension);
        self
    }

    /// Sets the recognizer. A program without one is contracts over observed evidence only.
    #[must_use]
    pub fn recognize(mut self, recognizer: RecognizerSpec) -> Self {
        self.recognizer = Some(recognizer);
        self
    }

    /// Adds a contract.
    #[must_use]
    pub fn contract(mut self, contract: ContractSpec) -> Self {
        self.contracts.push(contract);
        self
    }

    /// Sets the containment limits.
    #[must_use]
    pub fn contain(mut self, containment: ContainmentSpec) -> Self {
        self.containment = Some(containment);
        self
    }

    /// Resolves every name and checks every value, field and role.
    ///
    /// # Errors
    ///
    /// Every finding, in the order found, when any is found.
    pub fn build(self) -> Result<ProgramSpec, Vec<Finding>> {
        let mut out = Vec::new();
        let dims = self.dims;

        for (i, d) in dims.iter().enumerate() {
            if dims[..i].iter().any(|e| e.name == d.name) {
                out.push(Finding::DuplicateName {
                    kind: "dimension",
                    name: d.name,
                });
            }
        }

        let containment = self.containment;
        if containment.is_none() {
            out.push(Finding::Missing {
                part: "containment",
            });
        }
        for d in &dims {
            let Some(depth) = d.stack_depth else { continue };
            if d.role != DimensionRole::State {
                out.push(Finding::ForbiddenWrite {
                    owner: d.name,
                    dimension: d.name,
                    rule: "only a state dimension can be stacked",
                });
            }
            let permitted = containment.map_or(u16::MAX, |c| c.max_stack_depth);
            if depth == 0 || depth > permitted {
                out.push(Finding::StackBound {
                    dimension: d.name,
                    depth,
                    permitted,
                });
            }
        }

        let resolver = Resolver { dims: &dims };
        let asked = self.recognizer.is_some();
        let framing = self
            .recognizer
            .and_then(|r| resolver.recognizer(&r, &mut out));

        let mut resolved = Vec::with_capacity(self.contracts.len());
        for (i, c) in self.contracts.iter().enumerate() {
            if self.contracts[..i].iter().any(|e| e.name == c.name) {
                out.push(Finding::DuplicateName {
                    kind: "contract",
                    name: c.name,
                });
            }
            resolved.push(resolver.contract(c, &mut out));
        }

        match containment {
            Some(containment) if out.is_empty() && asked == framing.is_some() => Ok(ProgramSpec {
                dims,
                framing,
                contracts: self.contracts,
                resolved,
                containment,
            }),
            _ => Err(out),
        }
    }
}

struct Resolver<'a> {
    dims: &'a [DimensionSpec],
}

impl Resolver<'_> {
    fn dim(
        &self,
        owner: &'static str,
        name: &'static str,
        out: &mut Vec<Finding>,
    ) -> Option<usize> {
        let found = self.dims.iter().position(|d| d.name == name);
        if found.is_none() {
            out.push(Finding::UnknownDimension {
                owner,
                dimension: name,
            });
        }
        found
    }

    fn field(
        &self,
        owner: &'static str,
        dim: usize,
        name: &'static str,
        out: &mut Vec<Finding>,
    ) -> Option<BitFieldSpec> {
        let d = &self.dims[dim];
        let found = d.schema.field(name).copied();
        if found.is_none() {
            out.push(Finding::UnknownField {
                owner,
                dimension: d.name,
                field: name,
            });
        }
        found
    }

    /// interact's `validate_rule_table` checks for one matcher, one for one, over `BitFieldSpec`.
    fn matcher(
        &self,
        owner: &'static str,
        dim: usize,
        matcher: FieldMatcher,
        out: &mut Vec<Finding>,
    ) -> Option<ResolvedClause> {
        let field = self.field(owner, dim, matcher.field, out)?;
        let dimension = self.dims[dim].name;
        let field_max = field.max_value();
        let before = out.len();
        match matcher.op {
            MatchOp::Any => {}
            MatchOp::Eq(value) | MatchOp::NotEq(value) => {
                if value > field_max {
                    out.push(Finding::ValueOutOfRange {
                        owner,
                        dimension,
                        field: field.name,
                        value,
                        max: field_max,
                    });
                }
            }
            MatchOp::RangeInclusive { min, max } => {
                if min > max {
                    out.push(Finding::InvalidRange {
                        owner,
                        dimension,
                        field: field.name,
                        min,
                        max,
                    });
                } else if max > field_max {
                    out.push(Finding::RangeOutOfRange {
                        owner,
                        dimension,
                        field: field.name,
                        min,
                        max,
                        field_max,
                    });
                }
            }
            MatchOp::MaskAll { mask } | MatchOp::MaskAny { mask } => {
                if mask > field_max {
                    out.push(Finding::MaskOutOfRange {
                        owner,
                        dimension,
                        field: field.name,
                        mask,
                        field_max,
                    });
                }
            }
        }
        (out.len() == before).then_some(ResolvedClause {
            dim,
            dim_name: dimension,
            field,
            op: matcher.op,
        })
    }

    fn clause(
        &self,
        owner: &'static str,
        clause: &Clause,
        out: &mut Vec<Finding>,
    ) -> Option<ResolvedClause> {
        let dim = self.dim(owner, clause.dimension, out)?;
        if self.dims[dim].role == DimensionRole::Effect {
            out.push(Finding::ForbiddenRead {
                owner,
                dimension: clause.dimension,
            });
            return None;
        }
        self.matcher(owner, dim, clause.matcher, out)
    }

    fn clauses(
        &self,
        owner: &'static str,
        clauses: &[Clause],
        out: &mut Vec<Finding>,
    ) -> Vec<ResolvedClause> {
        clauses
            .iter()
            .filter_map(|c| self.clause(owner, c, out))
            .collect()
    }

    fn value(&self, owner: &'static str, dim: usize, value: FieldValue, out: &mut Vec<Finding>) {
        let Some(field) = self.field(owner, dim, value.field, out) else {
            return;
        };
        if value.value > field.max_value() {
            out.push(Finding::ValueOutOfRange {
                owner,
                dimension: self.dims[dim].name,
                field: field.name,
                value: value.value,
                max: field.max_value(),
            });
        }
    }

    fn writes(
        &self,
        owner: &'static str,
        writes: &[Write],
        role: DimensionRole,
        rule: &'static str,
        out: &mut Vec<Finding>,
    ) -> Vec<DimWrites> {
        let mut grouped: Vec<DimWrites> = Vec::new();
        for w in writes {
            let Some(dim) = self.dim(owner, w.dimension, out) else {
                continue;
            };
            if self.dims[dim].role != role {
                out.push(Finding::ForbiddenWrite {
                    owner,
                    dimension: w.dimension,
                    rule,
                });
                continue;
            }
            self.value(owner, dim, w.value, out);
            match grouped.iter_mut().find(|g| g.dim == dim) {
                Some(g) => g.values.push(w.value),
                None => grouped.push(DimWrites {
                    dim,
                    values: vec![w.value],
                }),
            }
        }
        grouped
    }

    fn stack_dim(
        &self,
        owner: &'static str,
        name: &'static str,
        out: &mut Vec<Finding>,
    ) -> Option<usize> {
        let dim = self.dim(owner, name, out)?;
        let d = &self.dims[dim];
        if d.role != DimensionRole::State || d.stack_depth.is_none() {
            out.push(Finding::ForbiddenWrite {
                owner,
                dimension: name,
                rule: "stack ops need a stacked state dimension",
            });
            return None;
        }
        Some(dim)
    }

    fn contract(&self, c: &ContractSpec, out: &mut Vec<Finding>) -> ResolvedContract {
        let owner = c.name;
        if c.when.is_empty() {
            out.push(Finding::EmptyWhen { owner });
        }
        let when = self.clauses(owner, c.when, out);
        let require = self.clauses(owner, c.require, out);
        let require_any = c
            .require_any
            .iter()
            .map(|group| self.clauses(owner, group, out))
            .collect();
        let forbid = self.clauses(owner, c.forbid, out);
        let transition = self.writes(
            owner,
            c.transition,
            DimensionRole::State,
            "transition writes only a state dimension",
            out,
        );
        let mut stack = Vec::new();
        for op in c.stack {
            match *op {
                StackOp::Push { dimension, values } => {
                    if let Some(dim) = self.stack_dim(owner, dimension, out) {
                        for v in values {
                            self.value(owner, dim, *v, out);
                        }
                        stack.push(ResolvedStackOp::Push {
                            dim,
                            values: values.to_vec(),
                        });
                    }
                }
                StackOp::Pop { dimension } => {
                    if let Some(dim) = self.stack_dim(owner, dimension, out) {
                        stack.push(ResolvedStackOp::Pop { dim });
                    }
                }
            }
        }
        let emit = self.writes(
            owner,
            c.emit,
            DimensionRole::Effect,
            "emit writes only an effect dimension",
            out,
        );
        ResolvedContract {
            name: owner,
            when,
            require,
            require_any,
            forbid,
            transition,
            stack,
            emit,
        }
    }

    fn recognizer(&self, r: &RecognizerSpec, out: &mut Vec<Finding>) -> Option<Framing> {
        const OWNER: &str = "recognizer";
        let item_dim = self.dim(OWNER, r.item_dimension, out);
        let frame_dim = self.dim(OWNER, r.frame_dimension, out);
        for dim in [item_dim, frame_dim].into_iter().flatten() {
            if self.dims[dim].role != DimensionRole::Evidence {
                out.push(Finding::ForbiddenWrite {
                    owner: OWNER,
                    dimension: self.dims[dim].name,
                    rule: "the recognizer reads and writes only evidence dimensions",
                });
            }
        }
        let (item_dim, frame_dim) = (item_dim?, frame_dim?);
        let frame_schema = &self.dims[frame_dim].schema;
        let frame_name = self.dims[frame_dim].name;

        let sequence = self.field(OWNER, frame_dim, r.sequence_field, out);
        let offset = self.field(OWNER, frame_dim, r.offset_field, out);
        let position = self.field(OWNER, frame_dim, r.position_field, out);
        let flag = frame_schema
            .flags
            .iter()
            .find(|f| f.name == r.boundary_flag);
        if flag.is_none() {
            out.push(Finding::UnknownField {
                owner: OWNER,
                dimension: frame_name,
                field: r.boundary_flag,
            });
        }
        let boundary = flag.and_then(|f| frame_schema.field(f.field).copied());

        if let Some(sequence) = sequence {
            let count = r.sequences.len() as u128;
            if count > sequence.max_value() {
                out.push(Finding::ValueOutOfRange {
                    owner: OWNER,
                    dimension: frame_name,
                    field: sequence.name,
                    value: count,
                    max: sequence.max_value(),
                });
            }
        }
        if let Some(position) = position {
            let highest = Position::Complete.value();
            if highest > position.max_value() {
                out.push(Finding::ValueOutOfRange {
                    owner: OWNER,
                    dimension: frame_name,
                    field: position.name,
                    value: highest,
                    max: position.max_value(),
                });
            }
        }
        self.value(OWNER, item_dim, r.end_item, out);

        let mut sequences = Vec::with_capacity(r.sequences.len());
        for (i, s) in r.sequences.iter().enumerate() {
            if r.sequences[..i].iter().any(|e| e.name == s.name) {
                out.push(Finding::DuplicateName {
                    kind: "sequence",
                    name: s.name,
                });
            }
            if s.items.is_empty() {
                out.push(Finding::EmptySequence { owner: s.name });
            }
            if s.items.len() > MAX_SEQUENCE_LEN {
                out.push(Finding::SequenceTooLong {
                    owner: s.name,
                    len: s.items.len(),
                    permitted: MAX_SEQUENCE_LEN,
                });
            }
            if let Some(offset) = offset {
                let last = s.items.len().saturating_sub(1) as u128;
                if last > offset.max_value() {
                    out.push(Finding::ValueOutOfRange {
                        owner: s.name,
                        dimension: frame_name,
                        field: offset.name,
                        value: last,
                        max: offset.max_value(),
                    });
                }
            }
            let items = s
                .items
                .iter()
                .map(|m| match *m {
                    ItemMatch::Any => ResolvedItem::Any,
                    ItemMatch::One(m) => ResolvedItem::OneOf(
                        self.matcher(s.name, item_dim, m, out).into_iter().collect(),
                    ),
                    ItemMatch::OneOf(ms) => ResolvedItem::OneOf(
                        ms.iter()
                            .filter_map(|m| self.matcher(s.name, item_dim, *m, out))
                            .collect(),
                    ),
                })
                .collect();
            let mut facts = vec![Vec::new(); s.items.len()];
            for f in s.facts {
                let at = usize::from(f.offset);
                if at >= s.items.len() {
                    out.push(Finding::ValueOutOfRange {
                        owner: s.name,
                        dimension: frame_name,
                        field: r.offset_field,
                        value: u128::from(f.offset),
                        max: s.items.len().saturating_sub(1) as u128,
                    });
                    continue;
                }
                for v in f.values {
                    self.value(s.name, frame_dim, *v, out);
                    facts[at].push(*v);
                }
            }
            sequences.push(ResolvedSequence {
                name: s.name,
                items,
                facts,
            });
        }

        let item = &self.dims[item_dim];
        let end_word = match item.patch(item.default_word(), &[r.end_item]) {
            Ok(word) => word,
            Err(e) => {
                out.push(Finding::IllegalWord {
                    owner: OWNER,
                    dimension: item.name,
                    verdict: e.to_string(),
                });
                0
            }
        };

        Some(Framing {
            recognizer: *r,
            item_dim,
            frame_dim,
            layout: FrameLayout {
                sequence: sequence?,
                offset: offset?,
                position: position?,
                boundary: boundary?,
                boundary_bit: flag?.mask,
            },
            end_word,
            sequences,
        })
    }
}
