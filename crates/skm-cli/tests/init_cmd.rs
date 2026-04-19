//! Integration tests for `skm init`.
//!
//! Exercises the real binary via `assert_cmd`, using `SKM_ROOT` to redirect
//! the layout into a `tempdir()` so the user's `~/.agents` is never touched.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;

fn skm() -> Command {
    Command::cargo_bin("skm").expect("skm binary builds")
}

#[test]
fn init_creates_layout_and_state_json() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("agents");

    skm()
        .env("SKM_ROOT", &root)
        .arg("init")
        .assert()
        .success()
        .stdout(contains("initialized"));

    assert!(root.join("skills").is_dir());
    assert!(root.join("skills-manager").is_dir());
    assert!(root.join("skills-manager/backups").is_dir());
    assert!(root.join("skills-manager/state.json").is_file());
}

#[test]
fn init_is_idempotent() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("agents");

    skm().env("SKM_ROOT", &root).arg("init").assert().success();

    skm()
        .env("SKM_ROOT", &root)
        .arg("init")
        .assert()
        .success()
        .stdout(contains("already initialized"));
}

#[test]
fn init_json_emits_machine_readable_output() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("agents");

    let output = skm()
        .env("SKM_ROOT", &root)
        .args(["--json", "init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert_eq!(parsed["status"], "created");
    assert!(parsed["skillsRoot"].as_str().unwrap().ends_with("skills"));
    assert!(parsed["managerDir"]
        .as_str()
        .unwrap()
        .ends_with("skills-manager"));

    // Second call flips the status.
    let output2 = skm()
        .env("SKM_ROOT", &root)
        .args(["--json", "init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed2: serde_json::Value = serde_json::from_slice(&output2).unwrap();
    assert_eq!(parsed2["status"], "already-initialized");
}
