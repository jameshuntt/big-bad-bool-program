//! A program with no framing: two evidence dimensions, one state dimension,
//! one effect dimension, and the contracts between them.
//!
//! `cargo run --example thermostat`

use big_bad_bool::bitpack;
use big_bad_bool_program::{
    Clause, ContainmentSpec, ContractSpec, FieldMatcher, NoObserver, Observation, Program,
    RefusalRule, Stepper, VerifyLimits, clause, contract, dimension, set, verify,
};

#[bitpack(storage = u8)]
pub struct Reading {
    #[bits(2)]
    #[labels(Cold = 0, Mild = 1, Hot = 2)]
    #[invalid(3)]
    pub level: u8,
}

#[bitpack(storage = u8)]
pub struct Mode {
    #[bits(1)]
    #[labels(Auto = 0, Manual = 1)]
    pub kind: u8,
}

#[bitpack(storage = u8)]
pub struct Heater {
    #[bits(1)]
    #[labels(Off = 0, On = 1)]
    pub state: u8,
}

#[bitpack(storage = u8)]
pub struct Command {
    #[bits(2)]
    #[labels(None = 0, StartHeat = 1, StopHeat = 2)]
    #[invalid(3)]
    pub kind: u8,
}

const COLD: u128 = Reading::LABEL_LEVEL_COLD.value;
const MILD: u128 = Reading::LABEL_LEVEL_MILD.value;
const HOT: u128 = Reading::LABEL_LEVEL_HOT.value;
const AUTO: u128 = Mode::LABEL_KIND_AUTO.value;
const MANUAL: u128 = Mode::LABEL_KIND_MANUAL.value;
const OFF: u128 = Heater::LABEL_STATE_OFF.value;
const ON: u128 = Heater::LABEL_STATE_ON.value;

const fn reading(level: u128) -> Clause {
    clause("reading", FieldMatcher::eq("level", level))
}
const fn mode(kind: u128) -> Clause {
    clause("mode", FieldMatcher::eq("kind", kind))
}
const fn heater(state: u128) -> Clause {
    clause("heater", FieldMatcher::eq("state", state))
}

const CONTRACTS: &[ContractSpec] = &[
    contract("heat-when-cold")
        .when(&[reading(COLD), mode(AUTO)])
        .require(&[heater(OFF)])
        .transition(&[set("heater", "state", ON)])
        .emit(&[set("command", "kind", Command::LABEL_KIND_STARTHEAT.value)]),
    contract("stop-when-hot")
        .when(&[reading(HOT), mode(AUTO)])
        .require(&[heater(ON)])
        .transition(&[set("heater", "state", OFF)])
        .emit(&[set("command", "kind", Command::LABEL_KIND_STOPHEAT.value)]),
    contract("hold").when(&[reading(MILD)]),
    contract("manual").when(&[mode(MANUAL)]),
];

fn main() {
    let spec = CONTRACTS
        .iter()
        .fold(
            Program::new()
                .add(dimension!(Reading, "reading", Evidence))
                .add(dimension!(Mode, "mode", Evidence))
                .add(dimension!(Heater, "heater", State))
                .add(dimension!(Command, "command", Effect))
                .contain(ContainmentSpec {
                    max_interpretations: 1,
                    max_candidates: 4,
                    max_actualized: 4,
                    max_stack_depth: 1,
                    on_refusal: RefusalRule::SkipObservation,
                }),
            |program, c| program.contract(*c),
        )
        .build()
        .expect("every name resolves");

    // Verified once, before any input: coherence, satisfiability, and a
    // bounded search of the reachable states.
    let program = verify(
        spec,
        VerifyLimits {
            max_states: 1_000,
            max_witness_tries: 1_024,
        },
    )
    .expect("the program verifies");
    let spec = program.spec();
    let (reading, mode) = (
        spec.dimension_id("reading").unwrap(),
        spec.dimension_id("mode").unwrap(),
    );
    let heater = spec.dimension_id("heater").unwrap();

    let input = [
        (COLD, AUTO),
        (MILD, AUTO),
        (HOT, AUTO),
        (HOT, AUTO),
        (COLD, MANUAL),
    ];
    let mut stepper = Stepper::new(&program, NoObserver);
    for (position, (level, kind)) in (0..).zip(input) {
        let observation = Observation::new(position)
            .with(reading, level)
            .with(mode, kind);
        for result in stepper.push(observation) {
            let committed: Vec<&str> = result
                .committed
                .iter()
                .map(|id| spec.contract(*id).unwrap().name)
                .collect();
            let effects: Vec<String> = result
                .effects
                .iter()
                .map(|e| spec.dimension(e.dimension).unwrap().describe(e.word))
                .collect();
            // Position 3: the heater is already off, so stop-when-hot is
            // refused and no contract accepts the observation (default deny).
            match result.refusals.last() {
                None => {
                    let line = format!(
                        "{}: {} {}",
                        result.position,
                        committed.join("+"),
                        effects.join(" ")
                    );
                    println!("{}", line.trim_end());
                }
                Some(refusal) => println!("{}: refused {refusal}", result.position),
            }
        }
    }
    let state = spec
        .dimension(heater)
        .unwrap()
        .describe(stepper.state().word(heater));
    println!("heater at the end: {state}");
}
