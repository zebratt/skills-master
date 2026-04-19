//! `skm status` — skill × tool distribution matrix, read-only.
//!
//! Per ceo-plans/2026-04-19-skillsmaster.md (lines 124, 138-145):
//!
//! Mode enum (Phase 1): `symlink | source-consumer | missing | conflict | broken`.
//!
//! A row is emitted for every skill directory that physically exists under
//! `skills_root`. For each skill and each of the 4 hardcoded tools:
//!
//!   - `NotRequested` — skill's frontmatter `tools` field doesn't include this tool.
//!   - `SourceConsumer` — tool is Hermes (always healthy; tool reads skills_root directly).
//!   - `Symlink`       — target is a symlink resolving to `skills_root/<name>`.
//!   - `Missing`       — target absent.
//!   - `Broken`        — symlink exists but its target is gone.
//!   - `Conflict`      — target exists but is not our symlink (real file/dir, or a symlink
//!     pointing somewhere else).
//!
//! `status` never writes. Drift repairs are a `doctor` job (Phase 1.5).

use crate::config::Config;
use crate::error::SkmResult;
use crate::frontmatter::{parse_skill_md, skill_md_path, Frontmatter, Tools};
use crate::init::ensure_initialized;
use crate::layout::Layout;
use crate::state::{load, Origin, State};
use crate::tools::{Mode, Tool};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Per-tool cell in the status matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolStatus {
    /// Frontmatter `tools` did not request this tool.
    NotRequested,
    /// Hermes reads skills_root directly; always healthy when the skill exists.
    SourceConsumer,
    /// Symlink present and resolves to `<skills_root>/<name>`.
    Symlink,
    /// Symlink or file expected but nothing at the target path.
    Missing,
    /// Symlink present but its target is gone.
    Broken,
    /// Target exists but is not our symlink (real file, wrong target, etc.).
    Conflict,
}

/// Per-skill row in the status matrix.
#[derive(Debug, Clone, Serialize)]
pub struct SkillStatus {
    pub name: String,
    pub origin: Origin,
    /// Optional — skills with unparseable SKILL.md surface `frontmatter_error`
    /// and leave `tools` empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontmatter_error: Option<String>,
    pub tools: BTreeMap<String, ToolStatus>,
}

/// Whole-report payload; CLI converts to human or JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub skills: Vec<SkillStatus>,
}

/// Compute the status matrix.
///
/// `home` is used to resolve each tool's distribution directory (e.g.,
/// `home/.claude/skills/<name>`). In production, CLI passes `$HOME`;
/// tests pass a `tempdir()`.
///
/// Scans `layout.skills_root()` for skill directories (each must contain
/// `SKILL.md`). Unparseable skills still appear in the report with
/// `frontmatter_error` set — hiding them would hide real problems.
pub fn run(layout: &Layout, home: &Path) -> SkmResult<StatusReport> {
    ensure_initialized(layout)?;

    let state = load(&layout.manager_dir(), &layout.skills_root())?;
    let config = Config::load(&layout.manager_dir())?;

    let mut skills: Vec<SkillStatus> = Vec::new();
    for skill_name in list_skill_dirs(&layout.skills_root())? {
        let skill_dir = layout.skills_root().join(&skill_name);
        let skill_md = skill_md_path(&skill_dir);

        let parse_result = parse_skill_md(&skill_md);
        let origin = state
            .skills
            .get(&skill_name)
            .map(|e| e.origin)
            .unwrap_or(Origin::User);

        match parse_result {
            Ok(fm) => {
                let requested = requested_tools(&fm);
                let tools =
                    compute_tool_statuses(&skill_name, &skill_dir, home, &requested, &config);
                skills.push(SkillStatus {
                    name: skill_name,
                    origin,
                    frontmatter_error: None,
                    tools,
                });
            }
            Err(e) => {
                skills.push(SkillStatus {
                    name: skill_name,
                    origin,
                    frontmatter_error: Some(e.to_string()),
                    tools: BTreeMap::new(),
                });
            }
        }
    }

    // Also report skills that are in state but missing from disk — they're
    // either mid-rename or orphaned. Surfacing them is what `status` is for.
    report_disk_orphans_from_state(&state, &layout.skills_root(), &mut skills);

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(StatusReport { skills })
}

/// List every first-level directory under `skills_root` that contains a `SKILL.md`.
/// Non-directories and directories without SKILL.md are silently skipped.
fn list_skill_dirs(skills_root: &Path) -> SkmResult<Vec<String>> {
    let entries = match std::fs::read_dir(skills_root) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(crate::error::SkmError::Io {
                path: skills_root.to_path_buf(),
                source: e,
            })
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| crate::error::SkmError::Io {
            path: skills_root.to_path_buf(),
            source: e,
        })?;
        let ftype = entry.file_type().map_err(|e| crate::error::SkmError::Io {
            path: entry.path(),
            source: e,
        })?;
        if !ftype.is_dir() {
            continue;
        }
        if !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

fn requested_tools(fm: &Frontmatter) -> Vec<Tool> {
    match &fm.tools {
        Tools::All(_) => Tool::all().to_vec(),
        Tools::List(v) => v.clone(),
    }
}

fn compute_tool_statuses(
    skill_name: &str,
    skill_dir: &Path,
    home: &Path,
    requested: &[Tool],
    config: &Config,
) -> BTreeMap<String, ToolStatus> {
    let mut out = BTreeMap::new();
    for tool in Tool::all() {
        let status = if !requested.contains(tool) {
            ToolStatus::NotRequested
        } else {
            match config.mode_for(*tool) {
                Mode::SourceConsumer => ToolStatus::SourceConsumer,
                Mode::Distribute => {
                    let target = tool
                        .dist_path(home, skill_name)
                        .expect("Distribute tools always have a dist_path");
                    classify_symlink(&target, skill_dir)
                }
            }
        };
        out.insert(tool.name().to_string(), status);
    }
    out
}

/// Inspect `target` and return the appropriate `ToolStatus`.
///
/// - No such path → `Missing`
/// - Symlink → read link, canonicalize both sides, compare → `Symlink` / `Conflict` / `Broken`
/// - Anything else → `Conflict`
fn classify_symlink(target: &Path, expected_skill_dir: &Path) -> ToolStatus {
    let meta = match std::fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return ToolStatus::Missing,
        // Permission errors etc. → treat as Conflict so the user investigates.
        Err(_) => return ToolStatus::Conflict,
    };
    if !meta.file_type().is_symlink() {
        return ToolStatus::Conflict;
    }
    // Symlink exists. Does its target exist?
    let link_target = match std::fs::read_link(target) {
        Ok(p) => p,
        Err(_) => return ToolStatus::Conflict,
    };
    // Absolute-path comparison: the Distribute contract is that symlinks point
    // at `skills_root/<name>`. Accept either literal match or canonical match.
    let link_abs = if link_target.is_absolute() {
        link_target.clone()
    } else {
        target
            .parent()
            .map(|p| p.join(&link_target))
            .unwrap_or(link_target.clone())
    };
    match (
        std::fs::canonicalize(&link_abs),
        std::fs::canonicalize(expected_skill_dir),
    ) {
        (Ok(actual), Ok(expected)) if actual == expected => ToolStatus::Symlink,
        (Ok(_), Ok(_)) => ToolStatus::Conflict,
        // Target directory is gone (link dangles).
        (Err(ref e), _) if e.kind() == ErrorKind::NotFound => ToolStatus::Broken,
        _ => ToolStatus::Conflict,
    }
}

/// For skills that exist in state.distribution but *not* on disk, emit a row
/// so the user sees the orphan. `tools` cells mirror the state record where
/// possible; we can't re-check symlink health without the source, so we
/// classify existing-on-disk distribution targets with `Conflict` (the
/// skill they point at is gone).
fn report_disk_orphans_from_state(state: &State, skills_root: &Path, out: &mut Vec<SkillStatus>) {
    for (name, entry) in &state.skills {
        let on_disk = skills_root.join(name).join("SKILL.md").is_file();
        if on_disk {
            continue;
        }
        if out.iter().any(|s| &s.name == name) {
            continue; // already reported
        }
        let mut tools = BTreeMap::new();
        for tool in Tool::all() {
            // If state recorded a distribution for this tool, mark Conflict (dangling).
            // Otherwise NotRequested.
            let status = if entry.distribution.contains_key(tool.name()) {
                ToolStatus::Conflict
            } else {
                ToolStatus::NotRequested
            };
            tools.insert(tool.name().to_string(), status);
        }
        out.push(SkillStatus {
            name: name.clone(),
            origin: entry.origin,
            frontmatter_error: Some("skill directory missing from disk".to_string()),
            tools,
        });
    }
}

/// Convenience for the CLI: compute the report under `home = $HOME` or a
/// test-provided override. Returns `NotInitialized` (via `run`) if layout
/// is incomplete.
pub fn run_for_home(layout: &Layout, home_override: Option<PathBuf>) -> SkmResult<StatusReport> {
    let home = home_override.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    });
    run(layout, &home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use std::fs;
    use tempfile::tempdir;

    /// Build an initialized layout under tempdir with `$HOME` equal to tempdir.
    fn setup() -> (tempfile::TempDir, Layout, PathBuf) {
        let tmp = tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let layout = Layout::default_for_home(&home);
        init::run(&layout).unwrap();
        (tmp, layout, home)
    }

    fn write_skill(layout: &Layout, name: &str, tools: &str) {
        let dir = layout.skills_root().join(name);
        fs::create_dir_all(&dir).unwrap();
        let body = format!("---\nname: {name}\ntools: {tools}\n---\n# {name}\n");
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    fn make_tool_symlink(home: &Path, tool: Tool, layout: &Layout, name: &str) {
        let dir = tool.dist_root(home).unwrap();
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(layout.skills_root().join(name), dir.join(name)).unwrap();
    }

    #[test]
    fn reports_missing_when_no_symlinks_yet() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude, codex]");

        let report = run(&layout, &home).unwrap();
        let foo = report.skills.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.tools["claude"], ToolStatus::Missing);
        assert_eq!(foo.tools["codex"], ToolStatus::Missing);
        assert_eq!(foo.tools["openclaw"], ToolStatus::NotRequested);
        assert_eq!(foo.tools["hermes"], ToolStatus::NotRequested);
    }

    #[test]
    fn reports_symlink_when_link_resolves_to_canonical_skill() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        make_tool_symlink(&home, Tool::Claude, &layout, "foo");

        let report = run(&layout, &home).unwrap();
        let foo = report.skills.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.tools["claude"], ToolStatus::Symlink);
    }

    #[test]
    fn reports_broken_when_symlink_target_gone() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        make_tool_symlink(&home, Tool::Claude, &layout, "foo");
        // Remove the canonical target.
        fs::remove_dir_all(layout.skills_root().join("foo")).unwrap();

        let report = run(&layout, &home).unwrap();
        // Skill is gone from disk — report_disk_orphans handles that path.
        let foo = report.skills.iter().find(|s| s.name == "foo");
        if let Some(s) = foo {
            // If the orphan reporter caught it, fine.
            assert!(s.frontmatter_error.is_some());
        }

        // More directly test Broken: create a symlink pointing at a non-existent
        // sibling to exercise the dangling-link branch without removing the skill.
        let (_tmp2, layout2, home2) = setup();
        write_skill(&layout2, "bar", "[claude]");
        let claude_dir = Tool::Claude.dist_root(&home2).unwrap();
        fs::create_dir_all(&claude_dir).unwrap();
        std::os::unix::fs::symlink(
            layout2.skills_root().join("does-not-exist"),
            claude_dir.join("bar"),
        )
        .unwrap();

        let report = run(&layout2, &home2).unwrap();
        let bar = report.skills.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.tools["claude"], ToolStatus::Broken);
    }

    #[test]
    fn reports_conflict_when_target_is_regular_file() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        let claude_dir = Tool::Claude.dist_root(&home).unwrap();
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("foo"), b"not a symlink").unwrap();

        let report = run(&layout, &home).unwrap();
        let foo = report.skills.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.tools["claude"], ToolStatus::Conflict);
    }

    #[test]
    fn reports_conflict_when_symlink_points_elsewhere() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");

        let decoy = layout.skills_root().join("decoy");
        fs::create_dir_all(&decoy).unwrap();
        let claude_dir = Tool::Claude.dist_root(&home).unwrap();
        fs::create_dir_all(&claude_dir).unwrap();
        std::os::unix::fs::symlink(&decoy, claude_dir.join("foo")).unwrap();

        let report = run(&layout, &home).unwrap();
        let foo = report.skills.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.tools["claude"], ToolStatus::Conflict);
    }

    #[test]
    fn hermes_always_source_consumer_when_requested() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "all");

        let report = run(&layout, &home).unwrap();
        let foo = report.skills.iter().find(|s| s.name == "foo").unwrap();
        assert_eq!(foo.tools["hermes"], ToolStatus::SourceConsumer);
        // The three Distribute tools should be Missing (no symlinks yet).
        assert_eq!(foo.tools["claude"], ToolStatus::Missing);
        assert_eq!(foo.tools["codex"], ToolStatus::Missing);
        assert_eq!(foo.tools["openclaw"], ToolStatus::Missing);
    }

    #[test]
    fn frontmatter_error_does_not_abort_the_report() {
        let (_tmp, layout, home) = setup();
        // Good skill.
        write_skill(&layout, "good", "[claude]");
        // Bad skill — SKILL.md with no frontmatter.
        let bad = layout.skills_root().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("SKILL.md"), b"no frontmatter here").unwrap();

        let report = run(&layout, &home).unwrap();
        let bad_row = report.skills.iter().find(|s| s.name == "bad").unwrap();
        assert!(bad_row.frontmatter_error.is_some());
        assert!(bad_row.tools.is_empty());
        let good_row = report.skills.iter().find(|s| s.name == "good").unwrap();
        assert!(good_row.frontmatter_error.is_none());
    }

    #[test]
    fn fails_cleanly_when_not_initialized() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));
        let err = run(&layout, tmp.path()).unwrap_err();
        assert!(matches!(err, crate::error::SkmError::NotInitialized));
    }
}
