//! Step 3: code outside the crate (contracts, frames, observers, the
//! application) cannot get `&mut` state or build an authorized transition.
//!
//! The harness is big-bad-bool-test-suite's `tests/compile_fail.rs` pattern:
//! each case is a small crate that must fail `cargo check` with a given
//! message. Case crates and their build output live under this workspace's
//! `target/tmp`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Case {
    name: &'static str,
    source: &'static str,
    expected_stderr: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "build_authorized_transition",
        source: r#"
            use big_bad_bool_contract::{AuthorizedTransition, ContractId};
            pub fn forge() -> AuthorizedTransition {
                AuthorizedTransition { contract: ContractId(0), position: 0 }
            }
        "#,
        expected_stderr: "fields `contract` and `position` of struct `AuthorizedTransition` are private",
    },
    Case {
        name: "candidate_is_not_a_transition",
        source: r#"
            use big_bad_bool_contract::{AuthorizedTransition, Candidate};
            pub fn promote(c: Candidate) -> AuthorizedTransition {
                c.into()
            }
        "#,
        expected_stderr: "the trait bound `AuthorizedTransition: From<big_bad_bool_contract::Candidate>` is not satisfied",
    },
    Case {
        name: "write_state_words",
        source: r#"
            use big_bad_bool_contract::ProjectionState;
            pub fn write(state: &mut ProjectionState) {
                state.words[0] = 1;
            }
        "#,
        expected_stderr: "field `words` of struct `ProjectionState` is private",
    },
    Case {
        name: "start_state",
        source: r#"
            use big_bad_bool_contract::{ProgramSpec, ProjectionState};
            pub fn start(spec: &ProgramSpec) -> ProjectionState {
                ProjectionState::start(spec)
            }
        "#,
        expected_stderr: "associated function `start` is private",
    },
    Case {
        name: "call_commit",
        source: r#"
            pub fn reach() {
                let _ = big_bad_bool_contract::step::commit::commit;
            }
        "#,
        expected_stderr: "module `step` is private",
    },
    Case {
        name: "call_authorize",
        source: r#"
            pub fn reach() {
                let _ = big_bad_bool_contract::authorize;
            }
        "#,
        expected_stderr: "cannot find value `authorize` in crate `big_bad_bool_contract`",
    },
    Case {
        name: "fact_view_is_read_only",
        source: r#"
            use big_bad_bool_contract::{DimensionId, FactView};
            pub fn write(view: &FactView<'_>) {
                view.word_mut(DimensionId(0));
            }
        "#,
        expected_stderr: "no method named `word_mut` found",
    },
];

#[test]
fn outside_code_cannot_write_state_or_authorize() {
    let failures: Vec<String> = CASES.iter().filter_map(compile_fails).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// `None` when the case fails to compile with its expected message.
fn compile_fails(case: &Case) -> Option<String> {
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let crate_dir = make_case_crate(tmp, case.name, case.source);
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .arg("check")
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", tmp.join("compile_fail_target"))
        .output()
        .expect("cargo runs for the compile-fail case");
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return Some(format!("compile-fail case `{}` compiled.", case.name));
    }
    let errors: Vec<&str> = stderr.lines().filter(|l| l.starts_with("error")).collect();
    (!stderr.contains(case.expected_stderr)).then(|| {
        format!(
            "compile-fail case `{}` did not say `{}`. It said:\n{}",
            case.name,
            case.expected_stderr,
            errors.join("\n")
        )
    })
}

fn make_case_crate(tmp: &Path, name: &str, source: &str) -> PathBuf {
    let root = tmp.join("compile_fail").join(name);
    fs::create_dir_all(root.join("src")).expect("case directory");
    let lib = env!("CARGO_MANIFEST_DIR");
    fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "bbbc_compile_fail_{name}"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
big-bad-bool-contract = {{ path = "{lib}" }}

[workspace]
"#
        ),
    )
    .expect("case manifest");
    fs::write(root.join("src/lib.rs"), source).expect("case source");
    let lock = Path::new(lib).join("Cargo.lock");
    if lock.exists() {
        fs::copy(lock, root.join("Cargo.lock")).expect("case lock file");
    }
    root
}
