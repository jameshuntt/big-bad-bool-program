//! Step 5: offsets, positions, boundary, contained matches and end padding.
//!
//! Since correction 4 a match inside a longer one is kept beside it, not
//! dropped: `FnHead` coexists with `AsyncFnHead` and adds no contradiction.

mod common;

use big_bad_bool_contract::{
    FieldMatcher, FieldValue, FrameFacts, ItemMatch, Position, Program, ProgramSpec, Recognizer,
    RecognizerSpec, SequenceId, SequenceSpec, dimension, sequence,
};
use common::{COLON, COMMA, FrameWord, IDENT, KW_ASYNC, KW_FN, LPAREN, RPAREN, TokenWord};

fn spec() -> ProgramSpec {
    common::program().build().unwrap()
}

fn items(kinds: &[u128]) -> Vec<u128> {
    common::tokens(0, kinds)
        .iter()
        .map(|o| o.word(common::TOKEN).unwrap())
        .collect()
}

/// (sequence name, offset) pairs, in recognizer order.
fn frames(spec: &ProgramSpec, kinds: &[u128], focal: usize) -> Vec<(&'static str, u8)> {
    let recognizer = Recognizer::new(spec).unwrap();
    recognizer
        .recognize(&items(kinds), focal)
        .iter()
        .map(|f| (spec.sequence(f.sequence).unwrap().name, f.offset))
        .collect()
}

fn frame_word(f: &FrameFacts) -> FrameWord {
    FrameWord::from_raw(f.word as u32)
}

#[test]
fn async_fn_foo_gives_foo_s1_offset_2() {
    let spec = spec();
    let kinds = [KW_ASYNC, KW_FN, IDENT, LPAREN];
    // FnHead offset 1 is kept beside AsyncFnHead offset 2 (correction 4).
    assert_eq!(
        frames(&spec, &kinds, 2),
        [("AsyncFnHead", 2), ("FnHead", 1)]
    );
    let found = Recognizer::new(&spec).unwrap().recognize(&items(&kinds), 2);
    let word = frame_word(&found[0]);
    assert_eq!(word.sequence(), 1);
    assert_eq!(word.offset(), 2);
    assert_eq!(u128::from(word.position()), Position::Interior.value());
    assert_eq!(word.construct(), 1, "FunctionDeclaration");
    assert_eq!(word.role(), 1, "DeclaredIdentity");
    assert_eq!(word.modifier(), 1, "Async");
    assert_eq!(word.marks(), 0b10, "params_follow, no boundary");
    let frame = spec.dimension(spec.frame_dimension().unwrap()).unwrap();
    assert_eq!(
        frame.describe(found[0].word),
        "{sequence=AsyncFnHead, offset=2, position=Interior, marks=params_follow, construct=FunctionDeclaration, role=DeclaredIdentity, modifier=Async}"
    );
}

#[test]
fn fn_foo_gives_s2_offset_1() {
    assert_eq!(frames(&spec(), &[KW_FN, IDENT, LPAREN], 1), [("FnHead", 1)]);
}

#[test]
fn bar_foo_gives_s5_offset_2() {
    assert_eq!(
        frames(&spec(), &[IDENT, LPAREN, IDENT, RPAREN], 2),
        [("CallWithArg", 2)]
    );
}

#[test]
fn fn_bar_foo_colon_gives_s3_not_s5() {
    // fn bar ( foo : i32 )
    let kinds = [KW_FN, IDENT, LPAREN, IDENT, COLON, IDENT, RPAREN];
    assert_eq!(frames(&spec(), &kinds, 3), [("ParamBinding", 1)]);
}

#[test]
fn positions_and_contained_matches() {
    let spec = spec();
    let kinds = [KW_ASYNC, KW_FN, IDENT, LPAREN, IDENT, COLON, IDENT, RPAREN];
    // `async`: S1 begins.
    assert_eq!(frames(&spec, &kinds, 0), [("AsyncFnHead", 0)]);
    // `fn`: S2 offset 0 lies inside S1 and is kept beside S1 offset 1.
    assert_eq!(
        frames(&spec, &kinds, 1),
        [("AsyncFnHead", 1), ("FnHead", 0)]
    );
    let found = Recognizer::new(&spec).unwrap().recognize(&items(&kinds), 0);
    assert_eq!(
        u128::from(frame_word(&found[0]).position()),
        Position::Begin.value()
    );
}

#[test]
fn an_item_that_ends_one_match_and_begins_another_is_a_boundary() {
    let spec = spec();
    let kinds = [KW_ASYNC, KW_FN, IDENT, LPAREN, IDENT, COLON, IDENT, RPAREN];
    let recognizer = Recognizer::new(&spec).unwrap();
    let at_paren = recognizer.recognize(&items(&kinds), 3);
    let shapes: Vec<(u16, u8, u32, u32)> = at_paren
        .iter()
        .map(|f| {
            let w = frame_word(f);
            (f.sequence.0, f.offset, w.position(), w.marks() & 1)
        })
        .collect();
    // S1 offset 3 End, S2 offset 2 End and S3 offset 0 Begin, all with the boundary flag.
    assert_eq!(shapes, [(0, 3, 3, 1), (1, 2, 3, 1), (2, 0, 1, 1)]);
    let at_colon = recognizer.recognize(&items(&kinds), 5);
    let shapes: Vec<(u16, u8, u32, u32)> = at_colon
        .iter()
        .map(|f| {
            let w = frame_word(f);
            (f.sequence.0, f.offset, w.position(), w.marks() & 1)
        })
        .collect();
    assert_eq!(shapes, [(2, 2, 3, 1), (3, 0, 1, 1)]);
    // `foo` inside S1 is not a boundary.
    assert_eq!(
        frame_word(&recognizer.recognize(&items(&kinds), 2)[0]).marks() & 1,
        0
    );
}

#[test]
fn nothing_matches_before_the_first_item_and_comma_works_like_rparen() {
    let spec = spec();
    // `( x :` at the very start: S3 has no item before offset 0, and needs none.
    assert_eq!(
        frames(&spec, &[LPAREN, IDENT, COLON], 1),
        [("ParamBinding", 1)]
    );
    // `f(a, b)`: `a` is S5 offset 2 with a comma after it, and S3 offset 1 needs a colon.
    assert_eq!(
        frames(&spec, &[IDENT, LPAREN, IDENT, COMMA, IDENT, RPAREN], 2),
        [("CallWithArg", 2)]
    );
}

const fn kind(value: u128) -> ItemMatch {
    ItemMatch::One(FieldMatcher::eq("kind", value))
}

const LAST_IDENT: SequenceSpec = sequence("LastIdent", &[kind(IDENT), kind(common::END)]);
const LONE_COLON: SequenceSpec = sequence("LoneColon", &[kind(COLON)]);

#[test]
fn end_padding_and_complete() {
    let spec = Program::new()
        .add(dimension!(TokenWord, "token", Evidence))
        .add(dimension!(FrameWord, "frame", Evidence))
        .recognize(RecognizerSpec {
            sequences: &[LAST_IDENT, LONE_COLON],
            end_item: FieldValue::new("kind", common::END),
            ..common::RECOGNIZER
        })
        .contain(common::CONTAINMENT)
        .build()
        .unwrap();
    let recognizer = Recognizer::new(&spec).unwrap();
    let input = items(&[COLON, IDENT]);
    // The last item sees the end item after it.
    let last = recognizer.recognize(&input, 1);
    assert_eq!((last[0].sequence, last[0].offset), (SequenceId(0), 0));
    // The end item itself, one past the input, is offset 1 of the same match.
    let end = recognizer.recognize(&input, 2);
    assert_eq!((end[0].sequence, end[0].offset), (SequenceId(0), 1));
    assert_eq!(
        u128::from(FrameWord::from_raw(end[0].word as u32).position()),
        Position::End.value()
    );
    // A one-item sequence is Complete.
    let colon = recognizer.recognize(&input, 0);
    assert_eq!(
        u128::from(FrameWord::from_raw(colon[0].word as u32).position()),
        Position::Complete.value()
    );
    // An identifier that is not last sees no end item.
    assert!(recognizer.recognize(&items(&[IDENT, IDENT]), 0).is_empty());
}
