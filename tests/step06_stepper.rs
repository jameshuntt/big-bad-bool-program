//! Step 6: section 16, end to end. Both examples and the module-level
//! refusal, compared to golden step results and observer events.

mod common;

use big_bad_bool_contract::{
    ContainmentSpec, NoObserver, Observation, ProgramSpec, RefusalRule, StepResult, Stepper,
    VerifiedProgram, verify,
};
use common::{Recorder, golden, render_result};

fn verified(containment: ContainmentSpec) -> VerifiedProgram {
    verify(
        common::program_with(containment).build().unwrap(),
        common::LIMITS,
    )
    .expect("section 16 verifies")
}

/// Steps `kinds`, returning results and event lines.
fn run(program: &VerifiedProgram, kinds: &[u128]) -> (Vec<StepResult>, Vec<String>) {
    let mut recorder = Recorder {
        spec: program.spec().clone(),
        lines: Vec::new(),
    };
    let mut stepper = Stepper::new(program, &mut recorder);
    let mut results = Vec::new();
    for observation in common::tokens(0, kinds) {
        results.extend(stepper.push(observation));
    }
    results.extend(stepper.finish());
    (results, recorder.lines)
}

fn text(spec: &ProgramSpec, results: &[StepResult], events: &[String]) -> String {
    let mut out = String::from("# step results\n");
    for r in results {
        out.push_str(&render_result(spec, r));
        out.push('\n');
    }
    out.push_str("# observer events\n");
    for e in events {
        out.push_str(e);
        out.push('\n');
    }
    out
}

fn committed(spec: &ProgramSpec, results: &[StepResult]) -> Vec<(u32, &'static str)> {
    results
        .iter()
        .map(|r| {
            (
                r.position,
                r.committed
                    .map_or("-", |id| spec.contract(id).unwrap().name),
            )
        })
        .collect()
}

/// Events from `observed <position>` up to the next `observed`.
fn events_at(events: &[String], position: u32) -> Vec<String> {
    let start = format!("observed {position} ");
    let from = events.iter().position(|e| e.starts_with(&start)).unwrap();
    let len = events[from + 1..]
        .iter()
        .position(|e| e.starts_with("observed "))
        .map_or(events.len() - from, |n| n + 1);
    events[from..from + len].to_vec()
}

#[test]
fn async_fn_foo() {
    let program = verified(common::CONTAINMENT);
    let spec = program.spec();
    let (results, events) = run(&program, common::ASYNC_FN_FOO);
    assert_eq!(
        committed(spec, &results),
        [
            (0, "fn-head-begin"),
            (1, "fn-keyword-after-modifier"),
            (2, "declare-function-name"),
            (3, "open-params"),
            (4, "declare-parameter"),
            (5, "param-colon"),
            (6, "type-reference"),
            (7, "close-params"),
            (8, "open-fn-body"),
            (9, "close-fn-body"),
            (10, "end"),
        ]
    );
    // What observers see at position 2, in order (report section 16).
    assert_eq!(
        events_at(&events, 2),
        [
            "observed 2 {kind=Ident}",
            "framed [AsyncFnHead@2]",
            "candidate declare-function-name via AsyncFnHead",
            "authorized declare-function-name",
            "committed state {phase=FnHead, marks=can_declare} -> {phase=SignatureName, marks=can_declare}",
            "emitted effect {kind=DeclaresIdentity, class=Function} at 2",
        ]
    );
    // Position 3: two frames, one contract, counted once.
    assert_eq!(
        events_at(&events, 3)[1..4],
        [
            "framed [AsyncFnHead@3 boundary, ParamBinding@0 boundary]",
            "candidate open-params via AsyncFnHead",
            "candidate open-params via ParamBinding",
        ]
    );
    golden("step06_async_fn_foo.txt", &text(spec, &results, &events));
}

#[test]
fn bar_foo_inside_the_body() {
    let program = verified(common::CONTAINMENT);
    let spec = program.spec();
    let (results, events) = run(&program, common::ASYNC_FN_FOO_CALLING_BAR);
    assert_eq!(
        committed(spec, &results)[9..],
        [
            (9, "callee-reference"),
            (10, "open-call-args"),
            (11, "argument-reference"),
            (12, "close-call"),
            (13, "close-fn-body"),
            (14, "end"),
        ]
    );
    // `foo` was declared at 2 and is referenced at 11.
    assert_eq!(
        render_result(spec, &results[2]),
        "2: commit declare-function-name effects [effect {kind=DeclaresIdentity, class=Function} at 2]"
    );
    assert_eq!(
        render_result(spec, &results[11]),
        "11: commit argument-reference effects [effect {kind=ReferencesIdentity, class=Value} at 11]"
    );
    // K4 is not applicable at `(` in the body: one candidate only.
    let at_10: Vec<String> = events_at(&events, 10)
        .into_iter()
        .filter(|e| e.starts_with("candidate"))
        .collect();
    assert_eq!(at_10, ["candidate open-call-args via CallWithArg"]);
    golden("step06_bar_foo_in_body.txt", &text(spec, &results, &events));
}

#[test]
fn bar_foo_at_module_level_stops() {
    let program = verified(common::CONTAINMENT);
    let spec = program.spec();
    let (results, events) = run(&program, common::BAR_FOO);
    assert_eq!(
        results
            .iter()
            .map(|r| render_result(spec, r))
            .collect::<Vec<_>>(),
        [
            "0: refused [[BBBC0102] `callee-reference` refused at position 0: require_any needs context.expecting in {Statement, Expr}, observed Item; [BBBC0101] no contract accepts position 0 (1 refused)]"
        ]
    );
    let report = big_bad_bool_contract::Liaise::liaise_report(&results[0].refusals[0]);
    assert_eq!(report.evidence_value("contract"), Some("callee-reference"));
    assert_eq!(
        report.evidence_value("missing"),
        Some("context.expecting in {Statement, Expr}")
    );
    assert_eq!(report.evidence_value("observed"), Some("Item"));
    golden(
        "step06_bar_foo_module_stop.txt",
        &text(spec, &results, &events),
    );
}

#[test]
fn bar_foo_at_module_level_skips() {
    let program = verified(ContainmentSpec {
        on_refusal: RefusalRule::SkipObservation,
        ..common::CONTAINMENT
    });
    let spec = program.spec();
    let (results, events) = run(&program, common::BAR_FOO);
    assert_eq!(
        committed(spec, &results),
        [(0, "-"), (1, "-"), (2, "-"), (3, "-"), (4, "end")]
    );
    golden(
        "step06_bar_foo_module_skip.txt",
        &text(spec, &results, &events),
    );
}

#[test]
fn a_position_is_stepped_once_its_window_is_full() {
    let program = verified(common::CONTAINMENT);
    let mut stepper = Stepper::new(&program, NoObserver);
    let observations = common::tokens(0, common::ASYNC_FN_FOO);
    for o in &observations[..3] {
        assert!(stepper.push(*o).is_empty());
    }
    let first = stepper.push(observations[3]);
    assert_eq!(first.iter().map(|r| r.position).collect::<Vec<_>>(), [0]);
    for o in &observations[4..] {
        assert_eq!(stepper.push(*o).len(), 1);
    }
    let rest = stepper.finish();
    assert_eq!(
        rest.iter().map(|r| r.position).collect::<Vec<_>>(),
        [7, 8, 9, 10]
    );
    assert!(stepper.finish().is_empty());
    assert!(
        stepper
            .push(Observation {
                position: 11,
                item: 0
            })
            .is_empty()
    );
}

#[test]
fn legal_next_is_read_only_and_follows_state() {
    let program = verified(common::CONTAINMENT);
    let mut stepper = Stepper::new(&program, NoObserver);
    assert_eq!(
        stepper.legal_next(),
        [common::KW_ASYNC, common::KW_FN, common::END]
    );
    let before = stepper.state().clone();
    let _ = stepper.legal_next();
    assert_eq!(stepper.state(), &before);
    for o in common::tokens(
        0,
        &[
            common::KW_ASYNC,
            common::KW_FN,
            common::IDENT,
            common::LPAREN,
        ],
    ) {
        stepper.push(o);
    }
    // After `async`: phase FnHead.
    assert_eq!(
        stepper.legal_next(),
        [common::KW_ASYNC, common::KW_FN, common::IDENT]
    );
}
