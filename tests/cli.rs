//! End-to-end tests that run the compiled binary. Only `--dry` and `--help`
//! are exercised so nothing is ever actually cleaned.

use std::{fs, path::Path, process::Command};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-recursive-clean"))
}

/// Removes ANSI escape sequences (`ESC [ ... <letter>`) from the output.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn write_package(dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
}

fn stdout(cmd: &mut Command) -> String {
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "exit {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    strip_ansi(&String::from_utf8(output.stdout).unwrap())
}

#[test]
fn help_lists_flags() {
    let out = stdout(bin().arg("--help"));
    for flag in ["--release", "--doc", "--dry", "--yes", "--ignored-patterns"] {
        assert!(out.contains(flag), "missing {flag} in:\n{out}");
    }
}

#[test]
fn release_and_doc_conflict() {
    let output = bin().args(["--release", "--doc"]).output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn dry_run_lists_projects_without_cleaning() {
    let tmp = tempfile::tempdir().unwrap();
    write_package(&tmp.path().join("a"), "a");
    write_package(&tmp.path().join("b"), "b");
    let marker = tmp.path().join("a/target/debug/marker");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    fs::write(&marker, "").unwrap();

    let out = stdout(bin().arg(tmp.path()).arg("--dry"));

    assert!(out.contains("Dry run"), "{out}");
    assert!(out.contains("2 project(s) would be cleaned"), "{out}");
    assert!(out.contains(&tmp.path().join("a").display().to_string()));
    assert!(out.contains(&tmp.path().join("b").display().to_string()));
    assert!(marker.exists(), "dry run must not delete anything");
}

#[test]
fn works_as_cargo_subcommand() {
    // `cargo recursive-clean ...` invokes the binary with `recursive-clean` as argv[1].
    let tmp = tempfile::tempdir().unwrap();
    write_package(&tmp.path().join("a"), "a");

    let out = stdout(bin().arg("recursive-clean").arg(tmp.path()).arg("--dry"));

    assert!(out.contains("1 project(s) would be cleaned"), "{out}");
}

#[test]
fn reports_when_no_projects_found() {
    let tmp = tempfile::tempdir().unwrap();

    let out = stdout(bin().arg(tmp.path()).arg("--dry"));

    assert!(out.contains("No projects found"), "{out}");
}

#[test]
fn empty_ignored_patterns_disables_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    write_package(&tmp.path().join("a"), "a");
    write_package(&tmp.path().join("a/target/package/a-0.1.0"), "a");

    let default = stdout(bin().arg(tmp.path()).arg("--dry"));
    let none = stdout(
        bin()
            .arg(tmp.path())
            .arg("--dry")
            .arg("--ignored-patterns="),
    );

    assert!(
        default.contains("1 project(s) would be cleaned"),
        "{default}"
    );
    assert!(none.contains("2 project(s) would be cleaned"), "{none}");
}

#[test]
fn dry_run_lists_glob_workspace_members() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(
        ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    write_package(&ws.join("crates/a"), "a");
    write_package(&ws.join("crates/b"), "b");

    let out = stdout(bin().arg(tmp.path()).arg("--dry"));

    assert!(out.contains("3 project(s) would be cleaned"), "{out}");
}
