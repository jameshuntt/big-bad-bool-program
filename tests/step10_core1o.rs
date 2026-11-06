//! Step 10: a shadow of core-1o's parser, written only as dimensions,
//! sequences and contracts, compared with the real parser on every published
//! fixture file: accepted, rejected, and entry structure.
//!
//! core-1o's two quiet behaviours are modelled, not hidden:
//! - inside a block, a token that is neither `}` nor a key is skipped
//!   (core-1o-0.2.0/src/engine/parser.rs:152-154): the `skip-in-block` contracts;
//! - at the top level, a token that is neither `{` nor a key ends the document
//!   (parser.rs:230): the `top-stop` contracts move to phase Done, and every
//!   later token is accepted and ignored.

use std::path::{Path, PathBuf};

use big_bad_bool::bitpack;
use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractSpec, DimensionId, FieldMatcher, FieldValue, ItemMatch,
    Observation, OffsetFacts, Program, RecognizerSpec, RefusalRule, SequenceSpec, StackOp, Stepper,
    VerifiedProgram, VerifyLimits, Write, clause, contract, dimension, pop, push, sequence, set,
    verify,
};
use core_1o::engine::ast::{Node, NodeValue};
use core_1o::engine::lexer::{Lexer, Token};
use core_1o::engine::parser::Parser;

// ---- dimensions ----

#[bitpack(storage = u8)]
pub struct OneToken {
    #[bits(3)]
    #[labels(LBrace = 0, RBrace = 1, Key = 2, Scalar = 3, Invalid = 4, End = 5)]
    #[invalid(6..=7)]
    pub kind: u8,
}

#[bitpack(storage = u16)]
pub struct OneFrame {
    #[bits(2)]
    #[labels(NoSequence = 0, ScalarEntry = 1, BlockEntry = 2)]
    pub sequence: u16,
    #[bits(1)]
    pub offset: u16,
    #[bits(3)]
    #[labels(None = 0, Begin = 1, Interior = 2, End = 3, Complete = 4)]
    #[invalid(5..=7)]
    pub position: u16,
    #[bits(1)]
    #[labels(boundary = 1)]
    #[flags(boundary = 0)]
    pub marks: u16,
    #[bits(2)]
    #[labels(None = 0, EntryKey = 1, BlockKey = 2)]
    pub role: u16,
}

#[bitpack(storage = u8)]
pub struct OneScope {
    #[bits(2)]
    #[labels(Top = 0, Group = 1, ValueBlock = 2)]
    #[invalid(3)]
    pub scope: u8,
}

#[bitpack(storage = u8)]
pub struct OnePhase {
    #[bits(2)]
    #[labels(Entry = 0, Value = 1, Done = 2)]
    #[invalid(3)]
    pub phase: u8,
}

#[bitpack(storage = u8)]
pub struct OneEffect {
    #[bits(3)]
    #[labels(
        None = 0,
        Key = 1,
        ScalarValue = 2,
        OpenValue = 3,
        CloseValue = 4,
        OpenGroup = 5,
        CloseGroup = 6
    )]
    #[invalid(7)]
    pub kind: u8,
}

const LBRACE: u128 = OneToken::LABEL_KIND_LBRACE.value;
const RBRACE: u128 = OneToken::LABEL_KIND_RBRACE.value;
const KEY: u128 = OneToken::LABEL_KIND_KEY.value;
const SCALAR: u128 = OneToken::LABEL_KIND_SCALAR.value;
const INVALID: u128 = OneToken::LABEL_KIND_INVALID.value;
const END: u128 = OneToken::LABEL_KIND_END.value;

const TOP: u128 = OneScope::LABEL_SCOPE_TOP.value;
const GROUP: u128 = OneScope::LABEL_SCOPE_GROUP.value;
const VALUE_BLOCK: u128 = OneScope::LABEL_SCOPE_VALUEBLOCK.value;
const ENTRY: u128 = OnePhase::LABEL_PHASE_ENTRY.value;
const VALUE: u128 = OnePhase::LABEL_PHASE_VALUE.value;
const DONE: u128 = OnePhase::LABEL_PHASE_DONE.value;

const E_KEY: u128 = OneEffect::LABEL_KIND_KEY.value;
const E_SCALAR: u128 = OneEffect::LABEL_KIND_SCALARVALUE.value;
const E_OPEN_VALUE: u128 = OneEffect::LABEL_KIND_OPENVALUE.value;
const E_CLOSE_VALUE: u128 = OneEffect::LABEL_KIND_CLOSEVALUE.value;
const E_OPEN_GROUP: u128 = OneEffect::LABEL_KIND_OPENGROUP.value;
const E_CLOSE_GROUP: u128 = OneEffect::LABEL_KIND_CLOSEGROUP.value;

// ---- sequences: a key before a scalar, a key before a block ----

const fn kind(value: u128) -> ItemMatch {
    ItemMatch::One(FieldMatcher::eq("kind", value))
}

const SEQUENCES: &[SequenceSpec] = &[
    sequence("ScalarEntry", &[kind(KEY), kind(SCALAR)]).facts(&[OffsetFacts {
        offset: 0,
        values: &[FieldValue::new("role", 1)],
    }]),
    sequence("BlockEntry", &[kind(KEY), kind(LBRACE)]).facts(&[OffsetFacts {
        offset: 0,
        values: &[FieldValue::new("role", 2)],
    }]),
];

const RECOGNIZER: RecognizerSpec = RecognizerSpec {
    item_dimension: "token",
    frame_dimension: "frame",
    sequence_field: "sequence",
    offset_field: "offset",
    position_field: "position",
    boundary_flag: "boundary",
    end_item: FieldValue::new("kind", END),
    sequences: SEQUENCES,
};

// ---- contracts ----

const fn token(value: u128) -> Clause {
    clause("token", FieldMatcher::eq("kind", value))
}
const fn phase(value: u128) -> Clause {
    clause("state", FieldMatcher::eq("phase", value))
}
const fn scope(value: u128) -> Clause {
    clause("context", FieldMatcher::eq("scope", value))
}
const IN_A_BLOCK: Clause = clause("context", FieldMatcher::not_eq("scope", TOP));
const fn to(value: u128) -> Write {
    set("state", "phase", value)
}
const fn effect(value: u128) -> Write {
    set("effect", "kind", value)
}
const PUSH_GROUP: StackOp = push("context", &[FieldValue::new("scope", GROUP)]);
const PUSH_VALUE_BLOCK: StackOp = push("context", &[FieldValue::new("scope", VALUE_BLOCK)]);

const CONTRACTS: &[ContractSpec] = &[
    contract("entry-key")
        .when(&[token(KEY), phase(ENTRY)])
        .transition(&[to(VALUE)])
        .emit(&[effect(E_KEY)]),
    contract("scalar-value")
        .when(&[token(SCALAR), phase(VALUE)])
        .transition(&[to(ENTRY)])
        .emit(&[effect(E_SCALAR)]),
    contract("open-value-block")
        .when(&[token(LBRACE), phase(VALUE)])
        .stack(&[PUSH_VALUE_BLOCK])
        .transition(&[to(ENTRY)])
        .emit(&[effect(E_OPEN_VALUE)]),
    contract("open-group")
        .when(&[token(LBRACE), phase(ENTRY), scope(TOP)])
        .stack(&[PUSH_GROUP])
        .emit(&[effect(E_OPEN_GROUP)]),
    contract("close-value-block")
        .when(&[token(RBRACE), phase(ENTRY), scope(VALUE_BLOCK)])
        .stack(&[pop("context")])
        .emit(&[effect(E_CLOSE_VALUE)]),
    contract("close-group")
        .when(&[token(RBRACE), phase(ENTRY), scope(GROUP)])
        .stack(&[pop("context")])
        .emit(&[effect(E_CLOSE_GROUP)]),
    // parser.rs:152-154: inside a block, anything but `}`, a key or an invalid token is skipped.
    contract("skip-brace-in-block").when(&[token(LBRACE), phase(ENTRY), IN_A_BLOCK]),
    contract("skip-scalar-in-block").when(&[token(SCALAR), phase(ENTRY), IN_A_BLOCK]),
    // parser.rs:230: at the top level, anything but `{`, a key or an invalid token ends the document.
    contract("top-stop-brace")
        .when(&[token(RBRACE), phase(ENTRY), scope(TOP)])
        .transition(&[to(DONE)]),
    contract("top-stop-scalar")
        .when(&[token(SCALAR), phase(ENTRY), scope(TOP)])
        .transition(&[to(DONE)]),
    contract("after-done").when(&[phase(DONE)]),
    contract("end").when(&[token(END), phase(ENTRY), scope(TOP)]),
];

fn program() -> VerifiedProgram {
    let builder = Program::new()
        .add(dimension!(OneToken, "token", Evidence))
        .add(dimension!(OneFrame, "frame", Evidence))
        .add(dimension!(OneScope, "context", State).stacked(64))
        .add(dimension!(OnePhase, "state", State))
        .add(dimension!(OneEffect, "effect", Effect))
        .recognize(RECOGNIZER)
        .contain(ContainmentSpec {
            max_interpretations: 2,
            max_candidates: 2,
            max_stack_depth: 64,
            on_refusal: RefusalRule::Stop,
        });
    let spec = CONTRACTS
        .iter()
        .fold(builder, |b, c| b.contract(*c))
        .build()
        .expect("shadow builds");
    verify(
        spec,
        VerifyLimits {
            max_states: 10_000,
            max_witness_tries: 4_096,
        },
    )
    .expect("shadow verifies")
}

// ---- comparison ----

fn token_kind(t: &Token) -> u128 {
    match t {
        Token::LBrace(_) => LBRACE,
        Token::RBrace(_) => RBRACE,
        Token::Identifier(..) => KEY,
        Token::StringLiteral(..)
        | Token::Number(..)
        | Token::Float(..)
        | Token::Bool(..)
        | Token::EscapedString(..) => SCALAR,
        Token::Invalid(..) => INVALID,
    }
}

fn scalar_text(t: &Token) -> String {
    match t {
        Token::StringLiteral(s, _) | Token::EscapedString(s, _, _) => format!("{s:?}"),
        Token::Number(n, _) => n.to_string(),
        Token::Float(f, _) => f.to_string(),
        Token::Bool(b, _) => b.to_string(),
        other => format!("{other:?}"),
    }
}

fn render_nodes(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| {
            let value = match &n.value {
                NodeValue::Block(children) => format!("{{{}}}", render_nodes(children)),
                NodeValue::String(s) => format!("{s:?}"),
                NodeValue::Number(v) => v.to_string(),
                NodeValue::Float(v) => v.to_string(),
                NodeValue::Bool(v) => v.to_string(),
            };
            format!("{}={value}", n.key)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The shadow's verdict: `Some(structure)` when every position committed.
fn shadow(program: &VerifiedProgram, tokens: &[Token]) -> Option<String> {
    let mut stepper = Stepper::new(program, big_bad_bool_contract::NoObserver);
    let mut results = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        results
            .extend(stepper.push(Observation::new(i as u32).with(DimensionId(0), token_kind(t))));
    }
    results.extend(stepper.finish());
    if results.iter().any(|r| r.committed.is_none()) {
        return None;
    }
    // Rebuild the entry tree from effects; the key text comes from the token at the effect's position.
    let mut levels: Vec<Vec<String>> = vec![Vec::new()];
    let mut keys: Vec<String> = Vec::new();
    for effect in results.iter().flat_map(|r| &r.effects) {
        let at = &tokens[effect.position as usize];
        match effect.word {
            E_KEY => match at {
                Token::Identifier(k, _) => keys.push(k.clone()),
                other => panic!("key effect on {other:?}"),
            },
            E_SCALAR => {
                let key = keys.pop().expect("a key before a value");
                levels
                    .last_mut()
                    .unwrap()
                    .push(format!("{key}={}", scalar_text(at)));
            }
            E_OPEN_VALUE => levels.push(Vec::new()),
            E_CLOSE_VALUE => {
                let children = levels.pop().unwrap();
                let key = keys.pop().expect("a key before a block");
                levels
                    .last_mut()
                    .unwrap()
                    .push(format!("{key}={{{}}}", children.join(", ")));
            }
            E_OPEN_GROUP | E_CLOSE_GROUP => {}
            other => panic!("unknown effect {other}"),
        }
    }
    Some(levels.pop().unwrap().join(", "))
}

fn core_1o(tokens: &[Token]) -> Option<String> {
    Parser::new(tokens.to_vec())
        .parse()
        .ok()
        .map(|nodes| render_nodes(&nodes))
}

fn fixtures_dir() -> PathBuf {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&std::env::var_os("HOME").expect("HOME")).join(".cargo"));
    let src = home.join("registry/src");
    std::fs::read_dir(&src)
        .expect("cargo registry sources")
        .filter_map(Result::ok)
        .map(|e| e.path().join("core-1o-0.2.0/fixtures"))
        .find(|p| p.is_dir())
        .expect("core-1o 0.2.0 fixtures in the cargo registry")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn shadow_matches_core_1o_on_every_published_fixture() {
    let program = program();
    let mut paths = Vec::new();
    files(&fixtures_dir(), &mut paths);
    paths.sort();
    assert_eq!(paths.len(), 37, "core-1o 0.2.0 publishes 37 fixture files");
    let mut accepted = 0;
    for path in &paths {
        let src = std::fs::read_to_string(path).unwrap();
        let tokens = Lexer::new(&src).tokenize();
        let theirs = core_1o(&tokens);
        let ours = shadow(&program, &tokens);
        assert_eq!(ours, theirs, "{}", path.display());
        accepted += usize::from(theirs.is_some());
    }
    assert_eq!(accepted, 37);
}

#[test]
fn shadow_matches_core_1o_on_edge_documents() {
    let program = program();
    let documents = [
        // plain, nested, and two top-level forms in one document
        "{ a 1 b { c \"x\" d true } }",
        "name { inner { deep 1.5 } } other 2",
        // skipped inside a block (parser.rs:152-154)
        "{ a 1 \"stray\" { b 2 } }",
        // the top level ends at a stray token (parser.rs:230), whatever follows
        "a 1 } b 2",
        "a 1 \"stop\" { never",
        // rejected
        "{ a 1",
        "a",
        "a b",
        "{ a }",
        "a \"open",
        "{ a \"bad \\q escape\" }",
        "",
        "{ }",
    ];
    for src in documents {
        let tokens = Lexer::new(src).tokenize();
        assert_eq!(shadow(&program, &tokens), core_1o(&tokens), "{src:?}");
    }
    assert_eq!(
        shadow(
            &program,
            &Lexer::new("{ a 1 \"stray\" { b 2 } }").tokenize()
        )
        .as_deref(),
        Some("a=1, b=2")
    );
    assert_eq!(
        shadow(&program, &Lexer::new("a 1 } b 2").tokenize()).as_deref(),
        Some("a=1")
    );
    assert_eq!(shadow(&program, &Lexer::new("{ a 1").tokenize()), None);
}

#[test]
fn shadow_spec_verifies_with_info_only() {
    let program = program();
    assert!(program.report().passed());
    println!(
        "{:?} states {}",
        program.report().findings,
        program.report().states
    );
}
