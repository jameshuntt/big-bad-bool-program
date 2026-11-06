//! Step 7: the spec digest is stable, changes when any clause changes, and a
//! trace header carries it.

mod common;

use big_bad_bool_contract::{
    Clause, ContainmentSpec, ContractSpec, FieldMatcher, ProgramBuilder, RefusalRule, SPEC_FORMAT,
    TRACE_FORMAT, clause, spec_digest, spec_material, verify,
};

fn with_contracts(contracts: &[ContractSpec]) -> ProgramBuilder {
    contracts
        .iter()
        .fold(
            common::dimensions(big_bad_bool_contract::Program::new()).recognize(common::RECOGNIZER),
            |b, c| b.contract(*c),
        )
        .contain(common::CONTAINMENT)
}

#[test]
fn section_16_digest_is_fixed() {
    let spec = common::program().build().unwrap();
    let digest = spec_digest(&spec);
    common::golden("step07_section16.digest", &format!("{digest}\n"));
    assert_eq!(digest, spec_digest(&common::program().build().unwrap()));
    let material = spec_material(&spec);
    assert!(material.starts_with(SPEC_FORMAT));
    assert!(material.contains("contract[2].name=declare-function-name\n"));
    assert!(material.contains("contract[2].forbid[0]=context.marks eq:1\n"));
}

#[test]
fn one_changed_clause_gives_a_new_digest() {
    let base = spec_digest(&common::program().build().unwrap());
    let mut contracts = common::CONTRACTS.to_vec();
    // K3 forbids `context.marks = const_expression`; make it forbid the empty field instead.
    const FORBID_EMPTY: &[Clause] = &[clause("context", FieldMatcher::eq("marks", 0))];
    contracts[2] = contracts[2].forbid(FORBID_EMPTY);
    let changed = spec_digest(&with_contracts(&contracts).build().unwrap());
    assert_ne!(base, changed);
    // Order is meaning too: swapping two contracts changes the digest.
    let mut swapped = common::CONTRACTS.to_vec();
    swapped.swap(0, 1);
    assert_ne!(
        base,
        spec_digest(&with_contracts(&swapped).build().unwrap())
    );
    // And the containment limits.
    let skip = common::program_with(ContainmentSpec {
        on_refusal: RefusalRule::SkipObservation,
        ..common::CONTAINMENT
    });
    assert_ne!(base, spec_digest(&skip.build().unwrap()));
}

#[test]
fn a_trace_header_carries_the_digest() {
    let program = verify(common::program().build().unwrap(), common::LIMITS).unwrap();
    let header = program.trace_header();
    assert_eq!(header.format, TRACE_FORMAT);
    assert_eq!(&header.spec, program.digest());
    assert_eq!(header.spec, spec_digest(program.spec()));
    assert_eq!(
        header.render(),
        format!("big-bad-bool-contract.trace.v1 spec={}", program.digest())
    );
    assert_eq!(program.digest().as_str().len(), 64);
}
