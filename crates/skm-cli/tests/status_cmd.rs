//! Integration tests for `skm status`.
//!
//! These tests drive the real binary and set both `HOME` and `SKM_ROOT` to a
//! tempdir so the test is fully hermetic — nothing under the user's real home
//! is read or written.

use assert_cmd::Command;
use predicates::str::contains;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn skm(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("skm").expect("skm binary builds");
    cmd.env("HOME", home);
    cmd.env_remove("SKM_ROOT");
    cmd
}

fn write_skill(skills_root: &Path, name: &str, tools: &str) {
    let dir = skills_root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ntools: {tools}\n---\n# body\n"),
    )
    .unwrap();
}

#[test]
fn status_fails_when_not_initialized() {
    let tmp = tempdir().unwrap();
    skm(tmp.path())
        .arg("status")
        .assert()
        .failure()
        .stderr(contains("not been initialized"));
}

#[test]
fn status_prints_matrix_after_init() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();

    let skills_root = tmp.path().join(".agents/skills");
    write_skill(&skills_root, "foo", "[claude]");

    skm(tmp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(contains("foo"))
        .stdout(contains("miss"));
}

#[test]
fn status_json_is_machine_readable() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();

    let skills_root = tmp.path().join(".agents/skills");
    write_skill(&skills_root, "foo", "all");

    let output = skm(tmp.path())
        .args(["--json", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let foo = parsed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "foo")
        .expect("foo present");
    assert_eq!(foo["tools"]["hermes"], "source-consumer");
    assert_eq!(foo["tools"]["claude"], "missing");
}

#[test]
fn status_detects_healthy_symlink() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();

    let skills_root = tmp.path().join(".agents/skills");
    write_skill(&skills_root, "foo", "[claude]");

    let claude_dir = tmp.path().join(".claude/skills");
    fs::create_dir_all(&claude_dir).unwrap();
    std::os::unix::fs::symlink(skills_root.join("foo"), claude_dir.join("foo")).unwrap();

    let output = skm(tmp.path())
        .args(["--json", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let foo = parsed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "foo")
        .unwrap();
    assert_eq!(foo["tools"]["claude"], "symlink");
}
