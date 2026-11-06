//! Section 16 of the architecture report, written as test data: five
//! dimensions, five sequences, fifteen contracts. None of it is in the library.

#![allow(dead_code)]

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractSpec, FieldMatcher, FieldValue, ItemMatch, Observation,
    OffsetFacts, Program, ProgramBuilder, RecognizerSpec, RefusalRule, SequenceSpec, StackOp,
    Write, clause, contract, dimension, pop, push, sequence, set,
};

// ---- dimensions ----

#[bitpack(storage = u8)]
pub struct TokenWord {
    #[bits(4)]
    #[labels(
        KwAsync = 0,
        KwFn = 1,
        Ident = 2,
        LParen = 3,
        RParen = 4,
        Colon = 5,
        Comma = 6,
        LBrace = 7,
        RBrace = 8,
        End = 9
    )]
    #[invalid(10..=15)]
    pub kind: u8,
}

#[bitpack(storage = u32)]
pub struct FrameWord {
    #[bits(3)]
    #[labels(
        NoSequence = 0,
        AsyncFnHead = 1,
        FnHead = 2,
        ParamBinding = 3,
        TypeAfterColon = 4,
        CallWithArg = 5
    )]
    pub sequence: u32,
    #[bits(2)]
    pub offset: u32,
    #[bits(3)]
    #[labels(None = 0, Begin = 1, Interior = 2, End = 3, Complete = 4)]
    #[invalid(5..=7)]
    pub position: u32,
    #[bits(2)]
    #[labels(boundary = 1, params_follow = 2)]
    #[flags(boundary = 0, params_follow = 1)]
    pub marks: u32,
    #[bits(4)]
    #[labels(
        None = 0,
        FunctionDeclaration = 1,
        Parameter = 2,
        TypeRef = 3,
        CallCallee = 4,
        CallOpen = 5,
        CallArgument = 6,
        CallClose = 7
    )]
    pub construct: u32,
    #[bits(2)]
    #[labels(
        None = 0,
        DeclaredIdentity = 1,
        ReferencedIdentity = 2,
        TypeReference = 3
    )]
    pub role: u32,
    #[bits(1)]
    #[labels(None = 0, Async = 1)]
    pub modifier: u32,
}

#[bitpack(storage = u8)]
pub struct ContextWord {
    #[bits(2)]
    #[labels(Module = 0, FnParams = 1, FnBody = 2, CallArgs = 3)]
    pub scope: u8,
    #[bits(3)]
    #[labels(
        Item = 0,
        Pattern = 1,
        AfterPattern = 2,
        Type = 3,
        AfterType = 4,
        Statement = 5,
        Expr = 6,
        AfterExpr = 7
    )]
    pub expecting: u8,
    #[bits(1)]
    #[labels(const_expression = 1)]
    #[flags(const_expression = 0)]
    pub marks: u8,
}

#[bitpack(storage = u8)]
pub struct StateWord {
    #[bits(3)]
    #[labels(
        Items = 0,
        FnHead = 1,
        SignatureName = 2,
        Params = 3,
        SignatureEnd = 4,
        Body = 5
    )]
    #[invalid(6..=7)]
    pub phase: u8,
    #[bits(1)]
    #[labels(can_declare = 1)]
    #[flags(can_declare = 0)]
    #[default(1)]
    pub marks: u8,
}

#[bitpack(storage = u8)]
pub struct EffectWord {
    #[bits(3)]
    #[labels(
        None = 0,
        DeclaresIdentity = 1,
        ReferencesIdentity = 2,
        OpensScope = 3,
        ClosesScope = 4
    )]
    #[invalid(5..=7)]
    pub kind: u8,
    #[bits(3)]
    #[labels(None = 0, Function = 1, Binding = 2, Type = 3, Value = 4)]
    #[invalid(5..=7)]
    pub class: u8,
}

// ---- label values, read from the generated tables ----

pub const KW_ASYNC: u128 = TokenWord::LABEL_KIND_KWASYNC.value;
pub const KW_FN: u128 = TokenWord::LABEL_KIND_KWFN.value;
pub const IDENT: u128 = TokenWord::LABEL_KIND_IDENT.value;
pub const LPAREN: u128 = TokenWord::LABEL_KIND_LPAREN.value;
pub const RPAREN: u128 = TokenWord::LABEL_KIND_RPAREN.value;
pub const COLON: u128 = TokenWord::LABEL_KIND_COLON.value;
pub const COMMA: u128 = TokenWord::LABEL_KIND_COMMA.value;
pub const LBRACE: u128 = TokenWord::LABEL_KIND_LBRACE.value;
pub const RBRACE: u128 = TokenWord::LABEL_KIND_RBRACE.value;
pub const END: u128 = TokenWord::LABEL_KIND_END.value;

const BEGIN: u128 = FrameWord::LABEL_POSITION_BEGIN.value;
const INTERIOR: u128 = FrameWord::LABEL_POSITION_INTERIOR.value;
const FUNCTION_DECLARATION: u128 = FrameWord::LABEL_CONSTRUCT_FUNCTIONDECLARATION.value;
const PARAMETER: u128 = FrameWord::LABEL_CONSTRUCT_PARAMETER.value;
const TYPE_REF: u128 = FrameWord::LABEL_CONSTRUCT_TYPEREF.value;
const CALL_CALLEE: u128 = FrameWord::LABEL_CONSTRUCT_CALLCALLEE.value;
const CALL_OPEN: u128 = FrameWord::LABEL_CONSTRUCT_CALLOPEN.value;
const CALL_ARGUMENT: u128 = FrameWord::LABEL_CONSTRUCT_CALLARGUMENT.value;
const CALL_CLOSE: u128 = FrameWord::LABEL_CONSTRUCT_CALLCLOSE.value;
const DECLARED_IDENTITY: u128 = FrameWord::LABEL_ROLE_DECLAREDIDENTITY.value;
const REFERENCED_IDENTITY: u128 = FrameWord::LABEL_ROLE_REFERENCEDIDENTITY.value;
const TYPE_REFERENCE: u128 = FrameWord::LABEL_ROLE_TYPEREFERENCE.value;
const ASYNC: u128 = FrameWord::LABEL_MODIFIER_ASYNC.value;
const PARAMS_FOLLOW: u128 = FrameWord::LABEL_MARKS_PARAMS_FOLLOW.value;

const MODULE: u128 = ContextWord::LABEL_SCOPE_MODULE.value;
const FN_PARAMS: u128 = ContextWord::LABEL_SCOPE_FNPARAMS.value;
const FN_BODY: u128 = ContextWord::LABEL_SCOPE_FNBODY.value;
const CALL_ARGS: u128 = ContextWord::LABEL_SCOPE_CALLARGS.value;
const ITEM: u128 = ContextWord::LABEL_EXPECTING_ITEM.value;
const PATTERN: u128 = ContextWord::LABEL_EXPECTING_PATTERN.value;
const AFTER_PATTERN: u128 = ContextWord::LABEL_EXPECTING_AFTERPATTERN.value;
const TYPE: u128 = ContextWord::LABEL_EXPECTING_TYPE.value;
const AFTER_TYPE: u128 = ContextWord::LABEL_EXPECTING_AFTERTYPE.value;
const STATEMENT: u128 = ContextWord::LABEL_EXPECTING_STATEMENT.value;
const EXPR: u128 = ContextWord::LABEL_EXPECTING_EXPR.value;
const AFTER_EXPR: u128 = ContextWord::LABEL_EXPECTING_AFTEREXPR.value;
const CONST_EXPRESSION: u128 = ContextWord::LABEL_MARKS_CONST_EXPRESSION.value;

const ITEMS: u128 = StateWord::LABEL_PHASE_ITEMS.value;
const FN_HEAD: u128 = StateWord::LABEL_PHASE_FNHEAD.value;
const SIGNATURE_NAME: u128 = StateWord::LABEL_PHASE_SIGNATURENAME.value;
const PARAMS: u128 = StateWord::LABEL_PHASE_PARAMS.value;
const SIGNATURE_END: u128 = StateWord::LABEL_PHASE_SIGNATUREEND.value;
const BODY: u128 = StateWord::LABEL_PHASE_BODY.value;
const CAN_DECLARE: u128 = StateWord::LABEL_MARKS_CAN_DECLARE.value;

const DECLARES_IDENTITY: u128 = EffectWord::LABEL_KIND_DECLARESIDENTITY.value;
const REFERENCES_IDENTITY: u128 = EffectWord::LABEL_KIND_REFERENCESIDENTITY.value;
const OPENS_SCOPE: u128 = EffectWord::LABEL_KIND_OPENSSCOPE.value;
const CLOSES_SCOPE: u128 = EffectWord::LABEL_KIND_CLOSESSCOPE.value;
const FUNCTION: u128 = EffectWord::LABEL_CLASS_FUNCTION.value;
const BINDING: u128 = EffectWord::LABEL_CLASS_BINDING.value;
const TYPE_CLASS: u128 = EffectWord::LABEL_CLASS_TYPE.value;
const VALUE: u128 = EffectWord::LABEL_CLASS_VALUE.value;

// ---- sequences ----

const fn kind(value: u128) -> ItemMatch {
    ItemMatch::One(FieldMatcher::eq("kind", value))
}

const fn construct(value: u128) -> FieldValue {
    FieldValue::new("construct", value)
}

const fn role(value: u128) -> FieldValue {
    FieldValue::new("role", value)
}

const F_ASYNC: FieldValue = FieldValue::new("modifier", ASYNC);
const F_PARAMS_FOLLOW: FieldValue = FieldValue::new("marks", PARAMS_FOLLOW);

pub const S1_ASYNC_FN_HEAD: SequenceSpec = sequence(
    "AsyncFnHead",
    &[kind(KW_ASYNC), kind(KW_FN), kind(IDENT), kind(LPAREN)],
)
.facts(&[
    OffsetFacts {
        offset: 0,
        values: &[construct(FUNCTION_DECLARATION), F_ASYNC],
    },
    OffsetFacts {
        offset: 1,
        values: &[construct(FUNCTION_DECLARATION), F_ASYNC],
    },
    OffsetFacts {
        offset: 2,
        values: &[
            construct(FUNCTION_DECLARATION),
            F_ASYNC,
            role(DECLARED_IDENTITY),
            F_PARAMS_FOLLOW,
        ],
    },
    OffsetFacts {
        offset: 3,
        values: &[construct(FUNCTION_DECLARATION), F_ASYNC],
    },
]);

pub const S2_FN_HEAD: SequenceSpec = sequence("FnHead", &[kind(KW_FN), kind(IDENT), kind(LPAREN)])
    .facts(&[
        OffsetFacts {
            offset: 0,
            values: &[construct(FUNCTION_DECLARATION)],
        },
        OffsetFacts {
            offset: 1,
            values: &[construct(FUNCTION_DECLARATION), role(DECLARED_IDENTITY)],
        },
        OffsetFacts {
            offset: 2,
            values: &[construct(FUNCTION_DECLARATION)],
        },
    ]);

pub const S3_PARAM_BINDING: SequenceSpec = sequence(
    "ParamBinding",
    &[
        ItemMatch::OneOf(&[
            FieldMatcher::eq("kind", LPAREN),
            FieldMatcher::eq("kind", COMMA),
        ]),
        kind(IDENT),
        kind(COLON),
    ],
)
.facts(&[OffsetFacts {
    offset: 1,
    values: &[construct(PARAMETER), role(DECLARED_IDENTITY)],
}]);

pub const S4_TYPE_AFTER_COLON: SequenceSpec =
    sequence("TypeAfterColon", &[kind(COLON), kind(IDENT)]).facts(&[OffsetFacts {
        offset: 1,
        values: &[construct(TYPE_REF), role(TYPE_REFERENCE)],
    }]);

pub const S5_CALL_WITH_ARG: SequenceSpec = sequence(
    "CallWithArg",
    &[
        kind(IDENT),
        kind(LPAREN),
        kind(IDENT),
        ItemMatch::OneOf(&[
            FieldMatcher::eq("kind", RPAREN),
            FieldMatcher::eq("kind", COMMA),
        ]),
    ],
)
.facts(&[
    OffsetFacts {
        offset: 0,
        values: &[construct(CALL_CALLEE), role(REFERENCED_IDENTITY)],
    },
    OffsetFacts {
        offset: 1,
        values: &[construct(CALL_OPEN)],
    },
    OffsetFacts {
        offset: 2,
        values: &[construct(CALL_ARGUMENT), role(REFERENCED_IDENTITY)],
    },
    OffsetFacts {
        offset: 3,
        values: &[construct(CALL_CLOSE)],
    },
]);

pub const SEQUENCES: &[SequenceSpec] = &[
    S1_ASYNC_FN_HEAD,
    S2_FN_HEAD,
    S3_PARAM_BINDING,
    S4_TYPE_AFTER_COLON,
    S5_CALL_WITH_ARG,
];

pub const RECOGNIZER: RecognizerSpec = RecognizerSpec {
    item_dimension: "token",
    frame_dimension: "frame",
    sequence_field: "sequence",
    offset_field: "offset",
    position_field: "position",
    boundary_flag: "boundary",
    end_item: FieldValue::new("kind", END),
    sequences: SEQUENCES,
};

// ---- clauses ----

const fn token(value: u128) -> Clause {
    clause("token", FieldMatcher::eq("kind", value))
}
const fn frame(field: &'static str, value: u128) -> Clause {
    clause("frame", FieldMatcher::eq(field, value))
}
const fn context(field: &'static str, value: u128) -> Clause {
    clause("context", FieldMatcher::eq(field, value))
}
const fn state(field: &'static str, value: u128) -> Clause {
    clause("state", FieldMatcher::eq(field, value))
}
const fn phase(value: u128) -> Write {
    set("state", "phase", value)
}
const fn expecting(value: u128) -> Write {
    set("context", "expecting", value)
}

const CAN_DECLARE_HOLDS: Clause = state("marks", CAN_DECLARE);
const IN_CONST_EXPRESSION: Clause = context("marks", CONST_EXPRESSION);

const DECLARES_FUNCTION: &[Write] = &[
    set("effect", "kind", DECLARES_IDENTITY),
    set("effect", "class", FUNCTION),
];
const DECLARES_BINDING: &[Write] = &[
    set("effect", "kind", DECLARES_IDENTITY),
    set("effect", "class", BINDING),
];
const REFERENCES_TYPE: &[Write] = &[
    set("effect", "kind", REFERENCES_IDENTITY),
    set("effect", "class", TYPE_CLASS),
];
const REFERENCES_VALUE: &[Write] = &[
    set("effect", "kind", REFERENCES_IDENTITY),
    set("effect", "class", VALUE),
];

const PUSH_FN_PARAMS: StackOp = push(
    "context",
    &[
        FieldValue::new("scope", FN_PARAMS),
        FieldValue::new("expecting", PATTERN),
    ],
);
const PUSH_FN_BODY: StackOp = push(
    "context",
    &[
        FieldValue::new("scope", FN_BODY),
        FieldValue::new("expecting", STATEMENT),
    ],
);
const PUSH_CALL_ARGS: StackOp = push(
    "context",
    &[
        FieldValue::new("scope", CALL_ARGS),
        FieldValue::new("expecting", EXPR),
    ],
);

// ---- contracts ----

pub const K1_FN_HEAD_BEGIN: ContractSpec = contract("fn-head-begin")
    .when(&[
        frame("construct", FUNCTION_DECLARATION),
        frame("position", BEGIN),
    ])
    .require(&[context("expecting", ITEM), CAN_DECLARE_HOLDS])
    .forbid(&[IN_CONST_EXPRESSION])
    .transition(&[phase(FN_HEAD)]);

pub const K2_FN_KEYWORD_AFTER_MODIFIER: ContractSpec = contract("fn-keyword-after-modifier")
    .when(&[token(KW_FN), frame("position", INTERIOR)])
    .require(&[state("phase", FN_HEAD)]);

pub const K3_DECLARE_FUNCTION_NAME: ContractSpec = contract("declare-function-name")
    .when(&[
        frame("construct", FUNCTION_DECLARATION),
        frame("role", DECLARED_IDENTITY),
        token(IDENT),
    ])
    .require(&[CAN_DECLARE_HOLDS, state("phase", FN_HEAD)])
    .forbid(&[IN_CONST_EXPRESSION])
    .transition(&[phase(SIGNATURE_NAME)])
    .emit(DECLARES_FUNCTION);

pub const K4_OPEN_PARAMS: ContractSpec = contract("open-params")
    .when(&[token(LPAREN), state("phase", SIGNATURE_NAME)])
    .transition(&[phase(PARAMS)])
    .stack(&[PUSH_FN_PARAMS]);

pub const K5_DECLARE_PARAMETER: ContractSpec = contract("declare-parameter")
    .when(&[
        frame("construct", PARAMETER),
        frame("role", DECLARED_IDENTITY),
        token(IDENT),
    ])
    .require(&[context("scope", FN_PARAMS), context("expecting", PATTERN)])
    .transition(&[expecting(AFTER_PATTERN)])
    .emit(DECLARES_BINDING);

pub const K6_PARAM_COLON: ContractSpec = contract("param-colon")
    .when(&[token(COLON), context("scope", FN_PARAMS)])
    .require(&[context("expecting", AFTER_PATTERN)])
    .transition(&[expecting(TYPE)]);

pub const K7_TYPE_REFERENCE: ContractSpec = contract("type-reference")
    .when(&[frame("role", TYPE_REFERENCE)])
    .require(&[context("expecting", TYPE)])
    .transition(&[expecting(AFTER_TYPE)])
    .emit(REFERENCES_TYPE);

pub const K8_CLOSE_PARAMS: ContractSpec = contract("close-params")
    .when(&[token(RPAREN), context("scope", FN_PARAMS)])
    .require_any(&[
        &[context("expecting", AFTER_TYPE)],
        &[context("expecting", PATTERN)],
    ])
    .stack(&[pop("context")])
    .transition(&[phase(SIGNATURE_END)]);

pub const K9_OPEN_FN_BODY: ContractSpec = contract("open-fn-body")
    .when(&[token(LBRACE), state("phase", SIGNATURE_END)])
    .stack(&[PUSH_FN_BODY])
    .transition(&[phase(BODY)])
    .emit(&[set("effect", "kind", OPENS_SCOPE)]);

pub const K10_CLOSE_FN_BODY: ContractSpec = contract("close-fn-body")
    .when(&[token(RBRACE), context("scope", FN_BODY)])
    .stack(&[pop("context")])
    .transition(&[phase(ITEMS)])
    .emit(&[set("effect", "kind", CLOSES_SCOPE)]);

pub const K11_END: ContractSpec = contract("end")
    .when(&[token(END)])
    .require(&[context("scope", MODULE), state("phase", ITEMS)]);

pub const K12_CALLEE_REFERENCE: ContractSpec = contract("callee-reference")
    .when(&[
        frame("construct", CALL_CALLEE),
        frame("role", REFERENCED_IDENTITY),
        token(IDENT),
    ])
    .require_any(&[
        &[context("expecting", STATEMENT)],
        &[context("expecting", EXPR)],
    ])
    .transition(&[expecting(AFTER_EXPR)])
    .emit(REFERENCES_VALUE);

pub const K13_OPEN_CALL_ARGS: ContractSpec = contract("open-call-args")
    .when(&[token(LPAREN), frame("construct", CALL_OPEN)])
    .require(&[context("expecting", AFTER_EXPR)])
    .stack(&[PUSH_CALL_ARGS]);

pub const K14_ARGUMENT_REFERENCE: ContractSpec = contract("argument-reference")
    .when(&[
        frame("construct", CALL_ARGUMENT),
        frame("role", REFERENCED_IDENTITY),
        token(IDENT),
    ])
    .require(&[context("scope", CALL_ARGS), context("expecting", EXPR)])
    .transition(&[expecting(AFTER_EXPR)])
    .emit(REFERENCES_VALUE);

pub const K15_CLOSE_CALL: ContractSpec = contract("close-call")
    .when(&[token(RPAREN), context("scope", CALL_ARGS)])
    .require(&[context("expecting", AFTER_EXPR)])
    .stack(&[pop("context")]);

pub const CONTRACTS: &[ContractSpec] = &[
    K1_FN_HEAD_BEGIN,
    K2_FN_KEYWORD_AFTER_MODIFIER,
    K3_DECLARE_FUNCTION_NAME,
    K4_OPEN_PARAMS,
    K5_DECLARE_PARAMETER,
    K6_PARAM_COLON,
    K7_TYPE_REFERENCE,
    K8_CLOSE_PARAMS,
    K9_OPEN_FN_BODY,
    K10_CLOSE_FN_BODY,
    K11_END,
    K12_CALLEE_REFERENCE,
    K13_OPEN_CALL_ARGS,
    K14_ARGUMENT_REFERENCE,
    K15_CLOSE_CALL,
];

pub const CONTAINMENT: ContainmentSpec = ContainmentSpec {
    max_interpretations: 4,
    max_candidates: 4,
    max_stack_depth: 32,
    on_refusal: RefusalRule::Stop,
};

/// The five dimensions, in report order.
pub fn dimensions(builder: ProgramBuilder) -> ProgramBuilder {
    builder
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(FrameWord, "frame", Evidence))
        .add(dimension!(ContextWord, "context", State).stacked(32))
        .add(dimension!(StateWord, "state", State))
        .add(dimension!(EffectWord, "effect", Effect))
}

/// The section 16 program, every contract and the given containment.
pub fn program_with(containment: ContainmentSpec) -> ProgramBuilder {
    CONTRACTS
        .iter()
        .fold(dimensions(Program::new()).recognize(RECOGNIZER), |b, c| {
            b.contract(*c)
        })
        .contain(containment)
}

/// The section 16 program.
pub fn program() -> ProgramBuilder {
    program_with(CONTAINMENT)
}

/// The token dimension: the first one added, and the recognizer's item dimension.
pub const TOKEN: big_bad_bool_contract::DimensionId = big_bad_bool_contract::DimensionId(0);

/// One observation per token kind, positions from `first`.
pub fn tokens(first: u32, kinds: &[u128]) -> Vec<Observation> {
    kinds
        .iter()
        .zip(first..)
        .map(|(k, position)| Observation::new(position).with_word(TOKEN, TokenWord::new(*k as u8)))
        .collect()
}

/// `async fn foo(x: i32) {}`
pub const ASYNC_FN_FOO: &[u128] = &[
    KW_ASYNC, KW_FN, IDENT, LPAREN, IDENT, COLON, IDENT, RPAREN, LBRACE, RBRACE,
];

/// `async fn foo(x: i32) { bar(foo) }`
pub const ASYNC_FN_FOO_CALLING_BAR: &[u128] = &[
    KW_ASYNC, KW_FN, IDENT, LPAREN, IDENT, COLON, IDENT, RPAREN, LBRACE, IDENT, LPAREN, IDENT,
    RPAREN, RBRACE,
];

/// `bar(foo)` at module level.
pub const BAR_FOO: &[u128] = &[IDENT, LPAREN, IDENT, RPAREN];

// ---- rendering for golden files ----

use big_bad_bool_contract::{
    DimensionId, DimensionRole, FrameFacts, Observer, ProgramSpec, ProjectionState, StepEvent,
    StepResult,
};

fn frames_text(spec: &ProgramSpec, frames: &[FrameFacts]) -> String {
    let parts: Vec<String> = frames
        .iter()
        .map(|f| {
            let boundary = if FrameWord::from_raw(f.word as u32).marks() & 1 == 1 {
                " boundary"
            } else {
                ""
            };
            format!(
                "{}@{}{boundary}",
                spec.sequence(f.sequence).unwrap().name,
                f.offset
            )
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

fn state_changes(spec: &ProgramSpec, before: &ProjectionState, after: &ProjectionState) -> String {
    let mut parts = Vec::new();
    for (i, d) in spec.dimensions().iter().enumerate() {
        let id = DimensionId(i as u16);
        if d.role() != DimensionRole::State {
            continue;
        }
        let show = |s: &ProjectionState| {
            if d.stack_depth().is_some() {
                let entries: Vec<String> = s.stack(id).iter().map(|w| d.describe(*w)).collect();
                format!("[{}]", entries.join(", "))
            } else {
                d.describe(s.word(id))
            }
        };
        let (b, a) = (show(before), show(after));
        if b != a {
            parts.push(format!("{} {b} -> {a}", d.name()));
        }
    }
    if parts.is_empty() {
        "nothing".to_owned()
    } else {
        parts.join("; ")
    }
}

/// One line per event.
pub fn render_event(spec: &ProgramSpec, event: &StepEvent<'_>) -> String {
    match event {
        StepEvent::Observed(o) => {
            let words: Vec<String> = o
                .evidence()
                .iter()
                .map(|(d, w)| spec.dimension(*d).unwrap().describe(*w))
                .collect();
            format!("observed {} {}", o.position(), words.join(" "))
        }
        StepEvent::Framed(frames) => format!("framed {}", frames_text(spec, frames)),
        StepEvent::Candidate(c) => format!(
            "candidate {} via {}",
            spec.contract(c.contract).unwrap().name,
            c.frame
                .map_or("no frame", |s| spec.sequence(s).unwrap().name)
        ),
        StepEvent::Refused(r) => format!("refused {r}"),
        StepEvent::Authorized(id) => format!("authorized {}", spec.contract(*id).unwrap().name),
        StepEvent::Committed { before, after } => {
            format!("committed {}", state_changes(spec, before, after))
        }
        StepEvent::Emitted(e) => {
            let d = spec.dimension(e.dimension).unwrap();
            format!(
                "emitted {} {} at {}",
                d.name(),
                d.describe(e.word),
                e.position
            )
        }
    }
}

/// One line per step result.
pub fn render_result(spec: &ProgramSpec, r: &StepResult) -> String {
    match r.committed {
        Some(id) => {
            let effects: Vec<String> = r
                .effects
                .iter()
                .map(|e| {
                    let d = spec.dimension(e.dimension).unwrap();
                    format!("{} {} at {}", d.name(), d.describe(e.word), e.position)
                })
                .collect();
            format!(
                "{}: commit {} effects [{}]",
                r.position,
                spec.contract(id).unwrap().name,
                effects.join(", ")
            )
        }
        None => {
            let refusals: Vec<String> = r.refusals.iter().map(ToString::to_string).collect();
            format!("{}: refused [{}]", r.position, refusals.join("; "))
        }
    }
}

/// Records every event as a line.
pub struct Recorder {
    pub spec: ProgramSpec,
    pub lines: Vec<String>,
}

impl Observer for Recorder {
    fn observe(&mut self, event: &StepEvent<'_>) {
        let line = render_event(&self.spec, event);
        self.lines.push(line);
    }
}

/// Compares `actual` with `tests/golden/<name>`. With `BLESS_GOLDEN` set, writes it instead.
pub fn golden(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("BLESS_GOLDEN").is_some() {
        std::fs::write(&path, actual).expect("write golden file");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("golden file exists");
    assert_eq!(actual, expected, "golden file {name} differs");
}

/// Default limits for the section 16 program.
pub const LIMITS: big_bad_bool_contract::VerifyLimits = big_bad_bool_contract::VerifyLimits {
    max_states: 100_000,
    max_witness_tries: 4_096,
};
