//! Integration tests for `skm import`.

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

fn write_plugin(parent: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ntools: [claude]\n---\n{body}"),
    )
    .unwrap();
    dir
}

#[test]
fn import_fails_when_not_initialized() {
    let tmp = tempdir().unwrap();
    let plugin = write_plugin(&tmp.path().join("market/skills"), "foo", "");
    skm(tmp.path())
        .arg("import")
        .arg(&plugin)
        .assert()
        .failure()
        .stderr(contains("not been initialized"));
}

#[test]
fn import_copies_skill_and_records_state() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    let plugin = write_plugin(&tmp.path().join("market/skills"), "foo", "# body\n");

    skm(tmp.path())
        .arg("import")
        .arg(&plugin)
        .assert()
        .success()
        .stdout(contains("imported foo"));

    assert!(tmp.path().join(".agents/skills/foo/SKILL.md").is_file());
}

#[test]
fn import_with_as_renames_dir_and_rewrites_name() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    let plugin = write_plugin(&tmp.path().join("market/skills"), "context7", "");

    skm(tmp.path())
        .arg("import")
        .arg(&plugin)
        .args(["--as", "ctx7"])
        .assert()
        .success();

    let installed = tmp.path().join(".agents/skills/ctx7/SKILL.md");
    assert!(installed.is_file());
    let contents = fs::read_to_string(&installed).unwrap();
    assert!(contents.contains("name: ctx7"));
    assert!(!contents.contains("name: context7"));
}

#[test]
fn import_strict_aborts_on_escape_reference() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    let plugin = write_plugin(
        &tmp.path().join("market/skills"),
        "risky",
        "see ../other/file\n",
    );

    skm(tmp.path())
        .arg("import")
        .arg(&plugin)
        .arg("--strict")
        .assert()
        .failure()
        .stderr(contains("references path outside"));

    assert!(!tmp.path().join(".agents/skills/risky").exists());
}

#[test]
fn import_force_bypasses_scan() {
    let tmp = tempdir().unwrap();
    skm(tmp.path()).arg("init").assert().success();
    let plugin = write_plugin(
        &tmp.path().join("market/skills"),
        "risky",
        "see ../other/file\n",
    );

    let output = skm(tmp.path())
        .arg("import")
        .arg(&plugin)
        .arg("--force")
        .args(["--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["warnings"].as_array().unwrap().len(), 0);
    assert!(tmp.path().join(".agents/skills/risky").exists());
}
