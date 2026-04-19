//! Integration tests for `skm sync`.

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
        format!("---\nname: {name}\ntools: {tools}\n---\n"),
    )
    .unwrap();
}

#[test]
fn sync_fails_when_not_initialized() {
    let tmp = tempdir().unwrap();
    skm(tmp.path())
        .arg("sync")
        .assert()
        .failure()
        .stderr(contains("not been initialized"));
}

#[test]
fn sync_creates_symlinks_for_requested_tools() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    let skills_root = tmp.path().join(".agents/skills");
    write_skill(&skills_root, "foo", "[claude, codex]");

    skm(tmp.path())
        .arg("sync")
        .assert()
        .success()
        .stdout(contains("foo/claude"))
        .stdout(contains("foo/codex"));

    let claude_link = tmp.path().join(".claude/skills/foo");
    let codex_link = tmp.path().join(".codex/skills/foo");
    assert!(fs::symlink_metadata(&claude_link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(&codex_link)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn sync_dry_run_does_not_modify_filesystem() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    write_skill(&tmp.path().join(".agents/skills"), "foo", "[claude]");

    skm(tmp.path())
        .args(["sync", "--dry-run"])
        .assert()
        .success()
        .stdout(contains("dry-run"));

    assert!(fs::symlink_metadata(tmp.path().join(".claude/skills/foo")).is_err());
}

#[test]
fn sync_json_output_is_machine_readable() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    write_skill(&tmp.path().join(".agents/skills"), "foo", "[hermes]");

    let output = skm(tmp.path())
        .args(["--json", "sync"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["dry_run"], false);
    let cells = parsed["cells"].as_array().unwrap();
    let hermes = cells
        .iter()
        .find(|c| c["skill"] == "foo" && c["tool"] == "hermes")
        .unwrap();
    assert_eq!(hermes["kind"], "record-source-consumer");
}
