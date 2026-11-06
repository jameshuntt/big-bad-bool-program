//! Step 11: klepto as the oracle for declaration versus reference on real
//! Rust. The section 16 program steps each corpus file, tokenized with
//! proc-macro2; klepto reads the same file through `syn`.
//!
//! - Every `CapturedFn` name gets a DeclaresIdentity/Function effect, and the
//!   frame at that item carries modifier Async exactly when `is_async`.
//! - Every path occurrence with `arg_of` gets a ReferencesIdentity/Value
//!   effect, and every call's callee gets one too.
//! - klepto records a one-segment path only when it names an item
//!   (klepto-0.3.5/src/syntax/extract.rs:1727-1733); the shadow references
//!   every argument. Each argument klepto leaves out must be a parameter of
//!   its function or a name klepto knows no item for.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use big_bad_bool_contract::{Observation, Observer, StepEvent, Stepper, VerifiedProgram, verify};
use common::{EffectWord, FrameWord};
use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// One token with its kind and its (line, column) start.
struct Tok {
    kind: u128,
    text: String,
    at: (usize, usize),
}

fn flatten(stream: TokenStream, out: &mut Vec<Tok>) {
    for tree in stream {
        match tree {
            TokenTree::Group(g) => {
                let (open, close) = match g.delimiter() {
                    Delimiter::Parenthesis => (common::LPAREN, common::RPAREN),
                    Delimiter::Brace => (common::LBRACE, common::RBRACE),
                    other => panic!("the corpus uses only () and {{}}, not {other:?}"),
                };
                let s = g.span_open().start();
                out.push(Tok {
                    kind: open,
                    text: "(".into(),
                    at: (s.line, s.column),
                });
                flatten(g.stream(), out);
                let e = g.span_close().start();
                out.push(Tok {
                    kind: close,
                    text: ")".into(),
                    at: (e.line, e.column),
                });
            }
            TokenTree::Ident(i) => {
                let text = i.to_string();
                let kind = match text.as_str() {
                    "async" => common::KW_ASYNC,
                    "fn" => common::KW_FN,
                    _ => common::IDENT,
                };
                let s = i.span().start();
                out.push(Tok {
                    kind,
                    text,
                    at: (s.line, s.column),
                });
            }
            TokenTree::Punct(p) => {
                let kind = match p.as_char() {
                    ':' => common::COLON,
                    ',' => common::COMMA,
                    other => panic!("the corpus uses only `:` and `,`, not {other:?}"),
                };
                let s = p.span().start();
                out.push(Tok {
                    kind,
                    text: p.to_string(),
                    at: (s.line, s.column),
                });
            }
            TokenTree::Literal(l) => panic!("the corpus has no literals, found {l}"),
        }
    }
}

/// The frame words seen at each position.
#[derive(Default)]
struct Frames {
    position: u32,
    words: Vec<(u32, u128)>,
}

impl Observer for Frames {
    fn observe(&mut self, event: &StepEvent<'_>) {
        match event {
            StepEvent::Observed(o) => self.position = o.position,
            StepEvent::Framed(frames) => self
                .words
                .extend(frames.iter().map(|f| (self.position, f.word))),
            _ => {}
        }
    }
}

struct Stepped {
    tokens: Vec<Tok>,
    declares: Vec<usize>,
    references: BTreeSet<usize>,
    async_at: BTreeSet<usize>,
}

fn step(program: &VerifiedProgram, path: &Path) -> Stepped {
    let src = std::fs::read_to_string(path).unwrap();
    let mut tokens = Vec::new();
    flatten(TokenStream::from_str(&src).unwrap(), &mut tokens);
    let mut frames = Frames::default();
    let mut stepper = Stepper::new(program, &mut frames);
    let mut results = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        results.extend(stepper.push(Observation {
            position: i as u32,
            item: t.kind,
        }));
    }
    results.extend(stepper.finish());
    assert!(
        results.iter().all(|r| r.committed.is_some()),
        "{} steps clean: {:?}",
        path.display(),
        results.iter().find(|r| r.committed.is_none())
    );
    let mut declares = Vec::new();
    let mut references = BTreeSet::new();
    for e in results.iter().flat_map(|r| &r.effects) {
        let w = EffectWord::from_raw(e.word as u8);
        match (w.kind(), w.class()) {
            (1, 1) => declares.push(e.position as usize),
            (2, 4) => {
                references.insert(e.position as usize);
            }
            _ => {}
        }
    }
    let async_at = frames
        .words
        .iter()
        .filter(|(_, w)| FrameWord::from_raw(*w as u32).modifier() == 1)
        .map(|(p, _)| *p as usize)
        .collect();
    Stepped {
        tokens,
        declares,
        references,
        async_at,
    }
}

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/klepto");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

fn position_at(tokens: &[Tok], line: u32, column: u32) -> usize {
    tokens
        .iter()
        .position(|t| t.at == (line as usize, column as usize))
        .unwrap_or_else(|| panic!("a token at {line}:{column}"))
}

#[test]
fn klepto_agrees_on_declarations_and_references() {
    let program = verify(common::program().build().unwrap(), common::LIMITS).unwrap();
    let files = corpus();
    assert_eq!(files.len(), 3);
    let mut fns = 0;
    let mut args = 0;
    let mut kinds = [0usize; 3];
    for path in &files {
        let stepped = step(&program, path);
        let k = klepto::Klepto::new("corpus")
            .scan_file(path)
            .parse()
            .expect("klepto parses");

        // Declarations: each CapturedFn's name is the first token named so at or after its item start.
        let mut expected_declares = Vec::new();
        let mut expected_declares_in_order = Vec::new();
        for f in &k.functions {
            let start = position_at(
                &stepped.tokens,
                f.location.line.unwrap(),
                f.location.column.unwrap(),
            );
            let name = start
                + stepped.tokens[start..]
                    .iter()
                    .position(|t| t.text == f.name)
                    .expect("the name follows the item start");
            expected_declares.push(name);
            expected_declares_in_order.push(name);
            assert_eq!(
                stepped.async_at.contains(&name),
                f.is_async,
                "{}: `{}` async in klepto and in the frame",
                path.display(),
                f.name
            );
        }
        expected_declares.sort_unstable();
        assert_eq!(stepped.declares, expected_declares, "{}", path.display());
        fns += k.functions.len();

        // References. klepto records every call, so every callee is a reference.
        // It records a one-segment path only when it names an item; a
        // parameter is a local value and an unknown name is dropped. So each
        // klepto argument path must be a shadow reference, and each other
        // argument the shadow references must be a parameter of its function
        // or a name klepto knows no item for.
        let callees: BTreeSet<usize> = k
            .calls
            .iter()
            .map(|c| {
                position_at(
                    &stepped.tokens,
                    c.location.line.unwrap(),
                    c.location.column.unwrap(),
                )
            })
            .collect();
        let item_args: BTreeSet<usize> = k
            .paths
            .iter()
            .filter(|p| p.arg_of.is_some())
            .map(|p| {
                position_at(
                    &stepped.tokens,
                    p.location.line.unwrap(),
                    p.location.column.unwrap(),
                )
            })
            .collect();
        assert!(callees.is_subset(&stepped.references), "{}", path.display());
        assert!(
            item_args.is_subset(&stepped.references),
            "{}",
            path.display()
        );
        for &at in stepped.references.difference(&callees) {
            args += 1;
            let text = &stepped.tokens[at].text;
            if item_args.contains(&at) {
                kinds[0] += 1;
                continue;
            }
            let enclosing = k
                .functions
                .iter()
                .zip(&expected_declares_in_order)
                .filter(|(_, name_at)| **name_at < at)
                .map(|(f, _)| f)
                .last()
                .expect("an argument sits in a function");
            let is_param = enclosing
                .args
                .iter()
                .any(|a| a.split(" :").next() == Some(text.as_str()));
            let is_item = k.functions.iter().any(|f| &f.name == text);
            assert!(is_param || !is_item, "{}: `{text}` at {at}", path.display());
            kinds[if is_param { 1 } else { 2 }] += 1;
        }
    }
    // 9 functions; 5 call arguments: 1 item reference klepto records, 3 parameters, 1 unknown name.
    assert_eq!(fns, 9);
    assert_eq!((args, kinds), (5, [1, 3, 1]));
}
