//! Every refusal and every finding, as liaise codes.
//!
//! [`Finding`] is what `build` and `verify` report about a spec. [`Refusal`] is
//! what the stepper reports about one input position. Both carry their facts as
//! liaise evidence, so a report can be read without the message text.
//!
//! This module names no other module of the crate. Names arrive as the
//! `&'static str` the spec was written with.

use liaise::LiaiseCodes;

/// A problem with a spec, found by `build` (names, ranges, roles) or by
/// `verify` (coherence). Severity `info` findings do not fail verification.
#[derive(Debug, Clone, PartialEq, Eq, LiaiseCodes)]
#[liaise(prefix = "BBBC", domain = "big-bad-bool-contract")]
pub enum Finding {
    /// A contract with no WHEN clause. interact's `EmptyRule`, for contracts.
    #[liaise(code = 1, msg = "`{owner}` has no WHEN clause", evidence(owner))]
    EmptyWhen {
        /// The contract.
        owner: &'static str,
    },
    /// A name that is not a dimension of the program.
    #[liaise(
        code = 2,
        msg = "`{owner}` names unknown dimension `{dimension}`",
        evidence(owner, dimension)
    )]
    UnknownDimension {
        /// The contract, sequence or `recognizer`.
        owner: &'static str,
        /// The name given.
        dimension: &'static str,
    },
    /// A field (or flag) the dimension's schema does not have.
    #[liaise(
        code = 3,
        msg = "`{owner}` names unknown field `{dimension}.{field}`",
        evidence(owner, dimension, field)
    )]
    UnknownField {
        /// The contract, sequence or `recognizer`.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The name given.
        field: &'static str,
    },
    /// A value larger than the field can hold.
    #[liaise(
        code = 4,
        msg = "`{owner}`: `{dimension}.{field}` value {value} exceeds max {max}",
        evidence(owner, dimension, field, value, max)
    )]
    ValueOutOfRange {
        /// The contract, sequence or `recognizer`.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The field.
        field: &'static str,
        /// The value given.
        value: u128,
        /// The field's largest value.
        max: u128,
    },
    /// A range whose minimum is above its maximum.
    #[liaise(
        code = 5,
        msg = "`{owner}`: `{dimension}.{field}` range {min}..={max} is empty",
        evidence(owner, dimension, field, min, max)
    )]
    InvalidRange {
        /// The contract or sequence.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The field.
        field: &'static str,
        /// Range start.
        min: u128,
        /// Range end.
        max: u128,
    },
    /// A range that reaches past the field's largest value.
    #[liaise(
        code = 6,
        msg = "`{owner}`: `{dimension}.{field}` range {min}..={max} exceeds max {field_max}",
        evidence(owner, dimension, field, min, max, field_max)
    )]
    RangeOutOfRange {
        /// The contract or sequence.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The field.
        field: &'static str,
        /// Range start.
        min: u128,
        /// Range end.
        max: u128,
        /// The field's largest value.
        field_max: u128,
    },
    /// A mask with bits the field does not have.
    #[liaise(
        code = 7,
        msg = "`{owner}`: `{dimension}.{field}` mask {mask:#x} exceeds max {field_max:#x}",
        evidence(owner, dimension, field, mask, field_max)
    )]
    MaskOutOfRange {
        /// The contract or sequence.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The field.
        field: &'static str,
        /// The mask given.
        mask: u128,
        /// The field's largest value.
        field_max: u128,
    },
    /// Two dimensions, sequences or contracts with one name.
    #[liaise(code = 8, msg = "two {kind}s are named `{name}`", evidence(kind, name))]
    DuplicateName {
        /// `dimension`, `sequence` or `contract`.
        kind: &'static str,
        /// The name.
        name: &'static str,
    },
    /// A write the dimension's role does not allow.
    #[liaise(
        code = 9,
        msg = "`{owner}` may not write `{dimension}`: {rule}",
        evidence(owner, dimension, rule)
    )]
    ForbiddenWrite {
        /// The contract, dimension or `recognizer`.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// The role rule that refuses it.
        rule: &'static str,
    },
    /// A clause on an effect dimension. Effect words exist only inside one step.
    #[liaise(
        code = 10,
        msg = "`{owner}` reads effect dimension `{dimension}`",
        evidence(owner, dimension)
    )]
    ForbiddenRead {
        /// The contract.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
    },
    /// A part every program needs.
    #[liaise(code = 11, msg = "the program has no {part}", evidence(part))]
    Missing {
        /// `recognizer` or `containment`.
        part: &'static str,
    },
    /// A sequence with no items.
    #[liaise(code = 12, msg = "sequence `{owner}` has no items", evidence(owner))]
    EmptySequence {
        /// The sequence.
        owner: &'static str,
    },
    /// A sequence longer than the window allows.
    #[liaise(
        code = 13,
        msg = "sequence `{owner}` has {len} items, permitted {permitted}",
        evidence(owner, len, permitted)
    )]
    SequenceTooLong {
        /// The sequence.
        owner: &'static str,
        /// Its length.
        len: usize,
        /// The limit.
        permitted: usize,
    },
    /// A stacked dimension deeper than the containment limit.
    #[liaise(
        code = 14,
        msg = "`{dimension}` stacks {depth} deep, permitted {permitted}",
        evidence(dimension, depth, permitted)
    )]
    StackBound {
        /// The dimension.
        dimension: &'static str,
        /// Its declared depth.
        depth: u16,
        /// `ContainmentSpec::max_stack_depth`.
        permitted: u16,
    },
    /// A word the spec fixes (start, frame, push or emit word) that its schema refuses.
    #[liaise(
        code = 15,
        msg = "`{owner}` makes an illegal `{dimension}` word: {verdict}",
        evidence(owner, dimension, verdict)
    )]
    IllegalWord {
        /// What makes the word.
        owner: &'static str,
        /// The dimension.
        dimension: &'static str,
        /// flaga_solver's verdict.
        verdict: String,
    },
    /// A contract whose conditions no words can meet.
    #[liaise(
        code = 20,
        msg = "`{owner}` can never hold: `{dimension}.{field}`",
        evidence(owner, dimension, field)
    )]
    Unsatisfiable {
        /// The contract.
        owner: &'static str,
        /// The dimension with no witness word.
        dimension: &'static str,
        /// The field whose clauses contradict, or `(joint)`.
        field: &'static str,
    },
    /// Two contracts with different writes that both hold in one reachable state.
    #[liaise(
        code = 21,
        msg = "`{first}` and `{second}` both hold with different writes at {witness}",
        evidence(first, second, witness)
    )]
    Ambiguous {
        /// The lower contract.
        first: &'static str,
        /// The higher contract.
        second: &'static str,
        /// State and input where both hold.
        witness: String,
    },
    /// A labelled state value no reachable state holds.
    #[liaise(
        code = 22,
        severity = "info",
        msg = "`{dimension}.{field}` = {label} is never reached",
        evidence(dimension, field, label)
    )]
    Unreachable {
        /// The state dimension.
        dimension: &'static str,
        /// The field.
        field: &'static str,
        /// The label.
        label: &'static str,
    },
    /// A reachable state where no input commits.
    #[liaise(
        code = 23,
        msg = "no input leaves reachable state {witness}",
        evidence(witness)
    )]
    NoExit {
        /// The state.
        witness: String,
    },
    /// A contract that commits in no reachable state.
    #[liaise(
        code = 24,
        msg = "`{owner}` commits in no reachable state",
        evidence(owner)
    )]
    DeadContract {
        /// The contract.
        owner: &'static str,
    },
    /// An emitting contract that commits in no reachable state.
    #[liaise(
        code = 25,
        msg = "`{owner}` emits, but commits in no reachable state",
        evidence(owner)
    )]
    ImpossibleEffect {
        /// The contract.
        owner: &'static str,
    },
    /// A sequence offset no contract accepts in any reachable state.
    #[liaise(
        code = 26,
        msg = "sequence `{owner}` offset {offset} is accepted in no reachable state",
        evidence(owner, offset)
    )]
    DeadSequence {
        /// The sequence.
        owner: &'static str,
        /// The offset.
        offset: usize,
    },
    /// Two sequences of one length that can match one window with different facts.
    #[liaise(
        code = 27,
        msg = "sequences `{first}` and `{second}` can match one window",
        evidence(first, second)
    )]
    OverlappingSequences {
        /// The lower sequence.
        first: &'static str,
        /// The higher sequence.
        second: &'static str,
    },
    /// A sequence that can match inside a longer one. Longest-match drops it there.
    #[liaise(
        code = 28,
        severity = "info",
        msg = "sequence `{inner}` can match inside `{outer}` at offset {at}",
        evidence(inner, outer, at)
    )]
    ContainedSequence {
        /// The shorter sequence.
        inner: &'static str,
        /// The longer sequence.
        outer: &'static str,
        /// Offset of `inner` inside `outer`.
        at: usize,
    },
    /// The reachable-state search hit its limit. Nothing past it was checked.
    #[liaise(
        code = 29,
        msg = "state search stopped at {explored} states, permitted {permitted}",
        evidence(explored, permitted)
    )]
    TooManyStates {
        /// States seen.
        explored: u128,
        /// `VerifyLimits::max_states`.
        permitted: u128,
    },
}

/// Why one input position did not commit.
#[derive(Debug, Clone, PartialEq, Eq, LiaiseCodes)]
#[liaise(prefix = "BBBC", domain = "big-bad-bool-contract")]
pub enum Refusal {
    /// No contract gave a candidate. Default deny.
    #[liaise(
        code = 101,
        msg = "no contract accepts position {position} ({refused} refused)",
        evidence(position, refused)
    )]
    NoContract {
        /// Input position.
        position: u32,
        /// How many contracts held WHEN but were refused.
        refused: usize,
    },
    /// A contract whose WHEN held, refused by REQUIRE, REQUIRE-ANY or FORBID.
    #[liaise(
        code = 102,
        msg = "`{contract}` refused at position {position}: {clause} needs {missing}, observed {observed}",
        evidence(position, contract, clause, missing, observed)
    )]
    ContractRefused {
        /// Input position.
        position: u32,
        /// The contract.
        contract: &'static str,
        /// `require`, `require_any` or `forbid`.
        clause: &'static str,
        /// What the clause needs.
        missing: String,
        /// What the fact view holds.
        observed: String,
    },
    /// Two candidates with different writes.
    #[liaise(
        code = 103,
        msg = "`{first}` and `{second}` both hold at position {position} with different writes",
        evidence(position, first, second)
    )]
    Ambiguous {
        /// Input position.
        position: u32,
        /// The lower contract.
        first: &'static str,
        /// The higher contract.
        second: &'static str,
    },
    /// A containment count over its limit.
    #[liaise(
        code = 104,
        msg = "{limit} limit exceeded at position {position}: observed {observed}, permitted {permitted}",
        evidence(position, limit, observed, permitted)
    )]
    LimitExceeded {
        /// Input position.
        position: u32,
        /// `interpretations` or `candidates`.
        limit: &'static str,
        /// The count.
        observed: usize,
        /// The limit.
        permitted: usize,
    },
    /// A committed or observed word its schema refuses. Commit rolled back, or the observation was not taken.
    #[liaise(
        code = 105,
        msg = "illegal `{dimension}` word at position {position}: {verdict}",
        evidence(position, dimension, verdict)
    )]
    IllegalWord {
        /// Input position.
        position: u32,
        /// The dimension.
        dimension: &'static str,
        /// flaga_solver's verdict.
        verdict: String,
    },
    /// A push past the dimension's depth.
    #[liaise(
        code = 106,
        msg = "`{dimension}` stack overflow at position {position}: depth {depth}, permitted {permitted}",
        evidence(position, dimension, depth, permitted)
    )]
    StackOverflow {
        /// Input position.
        position: u32,
        /// The dimension.
        dimension: &'static str,
        /// Depth the push would reach.
        depth: usize,
        /// The dimension's depth.
        permitted: usize,
    },
    /// A pop from an empty stack.
    #[liaise(
        code = 107,
        msg = "`{dimension}` stack underflow at position {position}",
        evidence(position, dimension)
    )]
    StackUnderflow {
        /// Input position.
        position: u32,
        /// The dimension.
        dimension: &'static str,
    },
    /// An observation word the program does not take. The observation
    /// counts as holding no evidence, for its own step and for its neighbours' windows.
    #[liaise(
        code = 108,
        msg = "observation at position {position} may not hold `{dimension}`: {rule}",
        evidence(position, dimension, rule)
    )]
    Evidence {
        /// Input position.
        position: u32,
        /// The dimension's name, or `#<id>` when there is no such dimension.
        dimension: String,
        /// The rule that refuses it.
        rule: &'static str,
    },
}
