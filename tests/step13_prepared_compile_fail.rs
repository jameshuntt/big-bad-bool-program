//! Step 13: outside code cannot forge a prepared step, copy one, or commit
//! one twice. Same harness as `step03_compile_fail.rs`: each case is a small
//! crate that must fail `cargo check` with a given message, built under this
//! workspace's `target/tmp`.

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
        name: "forge_prepared_step",
        source: r#"
            use big_bad_bool_program::PreparedStep;
            pub fn forge() -> PreparedStep {
                PreparedStep { version: 0 }
            }
        "#,
        expected_stderr: "cannot construct `PreparedStep` with struct literal syntax due to private fields",
    },
    Case {
        name: "prepared_step_is_not_clone",
        source: r#"
            use big_bad_bool_program::{NoObserver, Observation, Stepper};
            pub fn copy(stepper: &mut Stepper<'_, NoObserver>) {
                let prepared = stepper.prepare(Observation::new(0));
                let again = prepared.clone();
                let _ = stepper.commit(prepared);
                let _ = stepper.commit(again);
            }
        "#,
        expected_stderr: "no method named `clone` found for struct `PreparedStep`",
    },
    Case {
        name: "commit_twice",
        source: r#"
            use big_bad_bool_program::{NoObserver, Observation, Stepper};
            pub fn twice(stepper: &mut Stepper<'_, NoObserver>) {
                let prepared = stepper.prepare(Observation::new(0));
                let _ = stepper.commit(prepared);
                let _ = stepper.commit(prepared);
            }
        "#,
        expected_stderr: "use of moved value: `prepared`",
    },
    Case {
        name: "prepare_cannot_write",
        source: r#"
            use big_bad_bool_program::{NoObserver, Observation, PreparedStep, Stepper};
            pub fn hold(stepper: &mut Stepper<'_, NoObserver>) -> PreparedStep {
                let reader: &Stepper<'_, NoObserver> = stepper;
                let prepared = reader.prepare(Observation::new(0));
                let _ = reader.commit(prepared);
                reader.prepare(Observation::new(1))
            }
        "#,
        expected_stderr: "cannot borrow `*reader` as mutable, as it is behind a `&` reference",
    },
];

#[test]
fn outside_code_cannot_forge_copy_or_reuse_a_prepared_step() {
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
        .env("CARGO_TARGET_DIR", tmp.join("prepared_compile_fail_target"))
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
    let root = tmp.join("prepared_compile_fail").join(name);
    fs::create_dir_all(root.join("src")).expect("case directory");
    let lib = env!("CARGO_MANIFEST_DIR");
    fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "bbbc_prepared_compile_fail_{name}"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
big-bad-bool-program = {{ path = "{lib}" }}

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
