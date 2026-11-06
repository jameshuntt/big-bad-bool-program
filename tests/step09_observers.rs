//! Step 9: observers see every event and change nothing. The same step
//! results with no observer, a counting observer, and the ob-serve-a-bowl
//! adapter.

mod common;

use big_bad_bool_contract::{
    NoObserver, Observer, ProjectionState, StepEvent, StepResult, Stepper, VerifiedProgram, verify,
};

fn program() -> VerifiedProgram {
    verify(common::program().build().unwrap(), common::LIMITS).unwrap()
}

fn run<O: Observer>(
    program: &VerifiedProgram,
    kinds: &[u128],
    observer: O,
) -> (Vec<StepResult>, ProjectionState, O) {
    let mut stepper = Stepper::new(program, observer);
    let mut results = Vec::new();
    for o in common::tokens(0, kinds) {
        results.extend(stepper.push(o));
    }
    results.extend(stepper.finish());
    let state = stepper.state().clone();
    (results, state, stepper.into_observer())
}

/// Counts events by kind, in order.
#[derive(Default)]
struct Counting {
    kinds: Vec<u8>,
}

impl Observer for Counting {
    fn observe(&mut self, event: &StepEvent<'_>) {
        self.kinds.push(match event {
            StepEvent::Observed(_) => 1,
            StepEvent::Framed(_) => 2,
            StepEvent::Candidate(_) => 3,
            StepEvent::Refused(_) => 4,
            StepEvent::Authorized(_) => 5,
            StepEvent::Committed { .. } => 6,
            StepEvent::Emitted(_) => 7,
        });
    }
}

const INPUTS: &[&[u128]] = &[
    common::ASYNC_FN_FOO,
    common::ASYNC_FN_FOO_CALLING_BAR,
    common::BAR_FOO,
];

#[test]
fn counting_observer_changes_nothing() {
    let program = program();
    for kinds in INPUTS {
        let (plain, plain_state, _) = run(&program, kinds, NoObserver);
        let (counted, counted_state, counting) = run(&program, kinds, Counting::default());
        assert_eq!(plain, counted);
        assert_eq!(plain_state, counted_state);
        // One Observed per stepped position.
        assert_eq!(
            counting.kinds.iter().filter(|k| **k == 1).count(),
            plain.len()
        );
    }
}

#[cfg(feature = "bowl")]
#[test]
fn bowl_adapter_sees_every_event_and_changes_nothing() {
    use big_bad_bool_contract::ob_serve_a_bowl::{
        ObservableWakeRuntime, ObserverConfig, WakeProfileCode,
    };
    use big_bad_bool_contract::{BowlObserver, EventKind};

    let program = program();
    for kinds in INPUTS {
        let (plain, plain_state, _) = run(&program, kinds, NoObserver);
        let (_, _, counting) = run(&program, kinds, Counting::default());

        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(ObserverConfig::new("trace", WakeProfileCode::InlineQueue));
        let (bowled, bowled_state, mut bowl) = run(&program, kinds, BowlObserver::new(runtime));
        assert_eq!(plain, bowled);
        assert_eq!(plain_state, bowled_state);
        assert_eq!(bowl.published(), counting.kinds.len());

        let delivered = bowl.runtime_mut().drain_inline();
        let phases: Vec<u8> = delivered.iter().map(|e| e.key.phase() as u8).collect();
        assert_eq!(phases, counting.kinds);
        assert!(
            delivered.iter().all(|e| e.key.reason() == 2),
            "reason is manual"
        );
        assert_eq!(EventKind::Committed as u8, 6);
    }
}

#[cfg(feature = "bowl")]
#[test]
fn bowl_packets_name_position_contract_and_changed_dimensions() {
    use big_bad_bool_contract::BowlObserver;
    use big_bad_bool_contract::ob_serve_a_bowl::{
        ObservableWakeRuntime, ObserverConfig, WakeProfileCode,
    };

    let program = program();
    let mut runtime = ObservableWakeRuntime::new();
    runtime.register(ObserverConfig::new("trace", WakeProfileCode::InlineQueue));
    let (_, _, mut bowl) = run(&program, common::ASYNC_FN_FOO, BowlObserver::new(runtime));
    let delivered = bowl.runtime_mut().drain_inline();
    // Position 2: Observed, Framed, Candidate under AsyncFnHead and under
    // FnHead (both kept since correction 4), Authorized, Committed, Emitted.
    let at_2: Vec<(u64, u64, u32, u32)> = delivered
        .iter()
        .filter(|e| e.payload.slot == 2)
        .map(|e| {
            (
                e.key.phase(),
                e.key.field_mask(),
                e.payload.generation,
                e.payload.offset,
            )
        })
        .collect();
    assert_eq!(
        at_2,
        [
            (1, 0, 2, u32::MAX),
            (2, 0, 2, u32::MAX),
            (3, 0, 2, 2),
            (3, 0, 2, 2),
            (5, 0, 2, 2),
            (6, 1 << 3, 2, 2),
            (7, 1 << 4, 2, 2),
        ]
    );
    // Position 1 authorizes two contracts together; `len` says how many.
    let authorized_1 = delivered
        .iter()
        .find(|e| e.payload.slot == 1 && e.key.phase() == 5)
        .unwrap();
    assert_eq!(
        (authorized_1.payload.offset, authorized_1.payload.len),
        (0, 2)
    );
    // Position 3 pushes `context` and writes `state`.
    let committed_3 = delivered
        .iter()
        .find(|e| e.payload.slot == 3 && e.key.phase() == 6)
        .unwrap();
    assert_eq!(committed_3.key.field_mask(), (1 << 2) | (1 << 3));
}
