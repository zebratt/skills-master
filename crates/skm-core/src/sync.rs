//! `skm sync` — distribute `~/.agents/skills/*` to tool directories.
//!
//! Per ceo-plans/2026-04-19-skillsmaster.md:
//!   - For each skill, expand frontmatter `tools` to a concrete Tool set.
//!   - For each `Distribute` tool, ensure a symlink at `$HOME/.<tool>/skills/<name>`
//!     points at `<skills_root>/<name>`.
//!   - For `SourceConsumer` tools (Hermes), record "consumed" in state.json but
//!     write nothing to the filesystem.
//!
//! **Conflicts are never overwritten.** If a target exists and isn't our
//! symlink (real file, wrong symlink target), `sync` skips the cell and
//! surfaces it in the outcome. Phase 1.5 `doctor --fix` handles repair.
//!
//! `--dry-run` computes the outcome without touching the filesystem or
//! state.json.

use crate::config::Config;
use crate::error::{SkmError, SkmResult};
use crate::frontmatter::{parse_skill_md, skill_md_path, Tools};
use crate::init::ensure_initialized;
use crate::layout::Layout;
use crate::state::{load, store, DistributionMode, DistributionRecord, Origin, SkillEntry};
use crate::tools::{Mode, Tool};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Per-cell action that sync intends to take, or took.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SyncAction {
    /// Symlink will be (or was) created at `target` → `source`.
    CreateSymlink { target: PathBuf, source: PathBuf },
    /// Target already resolves to our canonical skill — no-op.
    AlreadyCorrect,
    /// Tool is `SourceConsumer`; state.json records consumption only.
    RecordSourceConsumer,
    /// Frontmatter didn't request this tool — nothing to do.
    NotRequested,
    /// Target exists but isn't our symlink; left untouched.
    Conflict { target: PathBuf, reason: String },
    /// Stale distribution removed (skill previously requested this tool but
    /// no longer does, and the old symlink was ours to remove).
    RemovedStale { target: PathBuf },
}

/// One row in the sync outcome: `(skill, tool, action)`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncCell {
    pub skill: String,
    pub tool: String,
    #[serde(flatten)]
    pub action: SyncAction,
}

/// Full outcome returned by `run`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    pub dry_run: bool,
    pub cells: Vec<SyncCell>,
    /// Skills removed from state because they no longer exist on disk.
    pub orphans_dropped: Vec<String>,
    /// SKILL.md that failed to parse — their rows are omitted from `cells`.
    pub parse_errors: Vec<ParseError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParseError {
    pub skill: String,
    pub error: String,
}

/// Execute sync.
///
/// Reads skills_root + state, computes actions, (optionally) applies them to
/// the filesystem, then persists state.json. When `dry_run` is true the
/// filesystem and state are both untouched.
pub fn run(layout: &Layout, home: &Path, dry_run: bool) -> SkmResult<SyncOutcome> {
    ensure_initialized(layout)?;

    let mut state = load(&layout.manager_dir(), &layout.skills_root())?;
    let config = Config::load(&layout.manager_dir())?;
    let skills_root = layout.skills_root();

    let disk_skills = list_skill_dirs(&skills_root)?;
    let disk_set: BTreeSet<String> = disk_skills.iter().cloned().collect();

    let mut cells: Vec<SyncCell> = Vec::new();
    let mut parse_errors: Vec<ParseError> = Vec::new();

    for skill in &disk_skills {
        let skill_dir = skills_root.join(skill);
        let skill_md = skill_md_path(&skill_dir);
        let fm = match parse_skill_md(&skill_md) {
            Ok(fm) => fm,
            Err(e) => {
                parse_errors.push(ParseError {
                    skill: skill.clone(),
                    error: e.to_string(),
                });
                continue;
            }
        };

        let requested: BTreeSet<Tool> = match fm.tools {
            Tools::All(_) => Tool::all().iter().copied().collect(),
            Tools::List(v) => v.into_iter().collect(),
        };

        // Pull prior distribution record so we can retire stale entries.
        let prior = state
            .skills
            .get(skill)
            .map(|e| e.distribution.clone())
            .unwrap_or_default();

        let mut new_distribution: BTreeMap<String, DistributionRecord> = BTreeMap::new();

        for tool in Tool::all() {
            if !requested.contains(tool) {
                // If we previously distributed to this tool, retire the symlink
                // (only if it's still ours — never touch conflicts).
                if let Some(old) = prior.get(tool.name()) {
                    if matches!(old.mode, DistributionMode::Symlink) {
                        let target = PathBuf::from(&old.path);
                        match retire_symlink(&target, &skill_dir, dry_run) {
                            Retire::Removed => cells.push(SyncCell {
                                skill: skill.clone(),
                                tool: tool.name().into(),
                                action: SyncAction::RemovedStale { target },
                            }),
                            Retire::NotOurs => cells.push(SyncCell {
                                skill: skill.clone(),
                                tool: tool.name().into(),
                                action: SyncAction::Conflict {
                                    target,
                                    reason: "stale target no longer ours; left in place".into(),
                                },
                            }),
                            Retire::Absent => {}
                        }
                    }
                }
                cells.push(SyncCell {
                    skill: skill.clone(),
                    tool: tool.name().into(),
                    action: SyncAction::NotRequested,
                });
                continue;
            }

            match config.mode_for(*tool) {
                Mode::SourceConsumer => {
                    // If we previously created a symlink for this tool (prior
                    // config said Distribute), retire it — the tool now reads
                    // skills_root directly and the symlink is obsolete.
                    if let Some(old) = prior.get(tool.name()) {
                        if matches!(old.mode, DistributionMode::Symlink) {
                            let target = PathBuf::from(&old.path);
                            match retire_symlink(&target, &skill_dir, dry_run) {
                                Retire::Removed => cells.push(SyncCell {
                                    skill: skill.clone(),
                                    tool: tool.name().into(),
                                    action: SyncAction::RemovedStale {
                                        target: target.clone(),
                                    },
                                }),
                                Retire::NotOurs => cells.push(SyncCell {
                                    skill: skill.clone(),
                                    tool: tool.name().into(),
                                    action: SyncAction::Conflict {
                                        target: target.clone(),
                                        reason: "prior symlink no longer ours; left in place"
                                            .into(),
                                    },
                                }),
                                Retire::Absent => {}
                            }
                        }
                    }
                    new_distribution.insert(
                        tool.name().into(),
                        DistributionRecord {
                            mode: DistributionMode::SourceConsumer,
                            path: skills_root.display().to_string(),
                        },
                    );
                    cells.push(SyncCell {
                        skill: skill.clone(),
                        tool: tool.name().into(),
                        action: SyncAction::RecordSourceConsumer,
                    });
                }
                Mode::Distribute => {
                    let target = tool
                        .dist_path(home, skill)
                        .expect("Distribute has dist_path");
                    let source = skill_dir.clone();
                    match plan_symlink(&target, &source) {
                        SymlinkPlan::AlreadyCorrect => {
                            new_distribution.insert(
                                tool.name().into(),
                                DistributionRecord {
                                    mode: DistributionMode::Symlink,
                                    path: target.display().to_string(),
                                },
                            );
                            cells.push(SyncCell {
                                skill: skill.clone(),
                                tool: tool.name().into(),
                                action: SyncAction::AlreadyCorrect,
                            });
                        }
                        SymlinkPlan::Create => {
                            if !dry_run {
                                if let Some(parent) = target.parent() {
                                    fs::create_dir_all(parent).map_err(|e| SkmError::Io {
                                        path: parent.to_path_buf(),
                                        source: e,
                                    })?;
                                }
                                std::os::unix::fs::symlink(&source, &target).map_err(|e| {
                                    SkmError::Io {
                                        path: target.clone(),
                                        source: e,
                                    }
                                })?;
                            }
                            new_distribution.insert(
                                tool.name().into(),
                                DistributionRecord {
                                    mode: DistributionMode::Symlink,
                                    path: target.display().to_string(),
                                },
                            );
                            cells.push(SyncCell {
                                skill: skill.clone(),
                                tool: tool.name().into(),
                                action: SyncAction::CreateSymlink { target, source },
                            });
                        }
                        SymlinkPlan::Conflict(reason) => {
                            cells.push(SyncCell {
                                skill: skill.clone(),
                                tool: tool.name().into(),
                                action: SyncAction::Conflict { target, reason },
                            });
                        }
                    }
                }
            }
        }

        // Update state's skill entry.
        let entry = state
            .skills
            .entry(skill.clone())
            .or_insert_with(|| SkillEntry {
                origin: Origin::User,
                last_synced_sha: None,
                last_synced_at: None,
                last_imported_from: None,
                distribution: BTreeMap::new(),
            });
        entry.distribution = new_distribution;
    }

    // Drop state entries for skills no longer on disk.
    let existing: Vec<String> = state.skills.keys().cloned().collect();
    let mut orphans_dropped = Vec::new();
    for name in existing {
        if !disk_set.contains(&name) {
            state.skills.remove(&name);
            orphans_dropped.push(name);
        }
    }

    if !dry_run {
        store(&state, &layout.manager_dir())?;
    }

    Ok(SyncOutcome {
        dry_run,
        cells,
        orphans_dropped,
        parse_errors,
    })
}

enum SymlinkPlan {
    AlreadyCorrect,
    Create,
    Conflict(String),
}

fn plan_symlink(target: &Path, source: &Path) -> SymlinkPlan {
    match fs::symlink_metadata(target) {
        Err(e) if e.kind() == ErrorKind::NotFound => SymlinkPlan::Create,
        Err(e) => SymlinkPlan::Conflict(format!("stat error: {e}")),
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return SymlinkPlan::Conflict("target is not a symlink".into());
            }
            // Compare canonical paths. If link target is absent, it's a broken
            // symlink we'd normally overwrite — but to stay conservative in
            // Phase 1 we call it a conflict and let the user intervene.
            match (fs::canonicalize(target), fs::canonicalize(source)) {
                (Ok(a), Ok(b)) if a == b => SymlinkPlan::AlreadyCorrect,
                (Ok(_), Ok(_)) => SymlinkPlan::Conflict("symlink points elsewhere".into()),
                (Err(_), _) => SymlinkPlan::Conflict("symlink target missing".into()),
                (_, Err(e)) => SymlinkPlan::Conflict(format!("source missing: {e}")),
            }
        }
    }
}

enum Retire {
    Removed,
    NotOurs,
    Absent,
}

/// Remove a symlink at `target` iff it currently resolves to `expected_source`.
/// Never deletes regular files or foreign symlinks.
fn retire_symlink(target: &Path, expected_source: &Path, dry_run: bool) -> Retire {
    match fs::symlink_metadata(target) {
        Err(e) if e.kind() == ErrorKind::NotFound => Retire::Absent,
        Err(_) => Retire::NotOurs,
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Retire::NotOurs;
            }
            let ours = match (fs::canonicalize(target), fs::canonicalize(expected_source)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            if !ours {
                return Retire::NotOurs;
            }
            if !dry_run && fs::remove_file(target).is_err() {
                return Retire::NotOurs;
            }
            Retire::Removed
        }
    }
}

fn list_skill_dirs(skills_root: &Path) -> SkmResult<Vec<String>> {
    let entries = match fs::read_dir(skills_root) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(SkmError::Io {
                path: skills_root.to_path_buf(),
                source: e,
            })
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| SkmError::Io {
            path: skills_root.to_path_buf(),
            source: e,
        })?;
        let ftype = entry.file_type().map_err(|e| SkmError::Io {
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
    names.sort();
    Ok(names)
}

/// CLI helper: resolve `home` from `$HOME` (or caller override) and dispatch.
pub fn run_for_home(
    layout: &Layout,
    home_override: Option<PathBuf>,
    dry_run: bool,
) -> SkmResult<SyncOutcome> {
    let home = home_override.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    });
    run(layout, &home, dry_run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use crate::state::state_path;
    use tempfile::tempdir;

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
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ntools: {tools}\n---\nbody\n"),
        )
        .unwrap();
    }

    fn cell<'a>(outcome: &'a SyncOutcome, skill: &str, tool: &str) -> &'a SyncCell {
        outcome
            .cells
            .iter()
            .find(|c| c.skill == skill && c.tool == tool)
            .unwrap_or_else(|| panic!("no cell for {skill}/{tool}"))
    }

    #[test]
    fn creates_symlink_for_requested_tool() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");

        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "claude");
        assert!(matches!(c.action, SyncAction::CreateSymlink { .. }));

        let target = Tool::Claude.dist_path(&home, "foo").unwrap();
        let meta = fs::symlink_metadata(&target).unwrap();
        assert!(meta.file_type().is_symlink());
        assert_eq!(
            fs::canonicalize(&target).unwrap(),
            fs::canonicalize(layout.skills_root().join("foo")).unwrap()
        );
    }

    #[test]
    fn records_source_consumer_for_hermes() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[hermes]");
        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "hermes");
        assert!(matches!(c.action, SyncAction::RecordSourceConsumer));

        // state.json recorded the mode.
        let state = load(&layout.manager_dir(), &layout.skills_root()).unwrap();
        let entry = state.skills.get("foo").unwrap();
        assert_eq!(
            entry.distribution.get("hermes").unwrap().mode,
            DistributionMode::SourceConsumer
        );
    }

    #[test]
    fn dry_run_does_not_touch_filesystem_or_state() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");

        let out = run(&layout, &home, true).unwrap();
        assert!(out.dry_run);
        let target = Tool::Claude.dist_path(&home, "foo").unwrap();
        assert!(fs::symlink_metadata(&target).is_err());

        // state.json should still reflect the empty post-init state.
        let state = load(&layout.manager_dir(), &layout.skills_root()).unwrap();
        assert!(!state.skills.contains_key("foo"));
        assert!(state_path(&layout.manager_dir()).exists());
    }

    #[test]
    fn idempotent_second_run_reports_already_correct() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        run(&layout, &home, false).unwrap();

        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "claude");
        assert!(matches!(c.action, SyncAction::AlreadyCorrect));
    }

    #[test]
    fn skips_conflict_without_overwriting() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        let claude_dir = Tool::Claude.dist_root(&home).unwrap();
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("foo"), b"hand-placed file").unwrap();

        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "claude");
        assert!(matches!(c.action, SyncAction::Conflict { .. }));

        // Original file untouched.
        let bytes = fs::read(claude_dir.join("foo")).unwrap();
        assert_eq!(bytes, b"hand-placed file");

        // state.json should NOT record this tool (we didn't succeed).
        let state = load(&layout.manager_dir(), &layout.skills_root()).unwrap();
        let entry = state.skills.get("foo").unwrap();
        assert!(!entry.distribution.contains_key("claude"));
    }

    #[test]
    fn retires_stale_symlink_when_tool_no_longer_requested() {
        let (_tmp, layout, home) = setup();

        // Round 1: request [claude, codex], sync both.
        write_skill(&layout, "foo", "[claude, codex]");
        run(&layout, &home, false).unwrap();
        assert!(Tool::Codex.dist_path(&home, "foo").unwrap().exists());

        // Round 2: edit frontmatter to drop codex.
        let path = layout.skills_root().join("foo/SKILL.md");
        fs::write(&path, "---\nname: foo\ntools: [claude]\n---\nbody\n").unwrap();

        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "codex");
        assert!(matches!(c.action, SyncAction::RemovedStale { .. }));
        assert!(fs::symlink_metadata(Tool::Codex.dist_path(&home, "foo").unwrap()).is_err());
    }

    #[test]
    fn drops_orphan_state_entry_when_skill_deleted_from_disk() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[claude]");
        run(&layout, &home, false).unwrap();

        // Remove skill from disk.
        fs::remove_dir_all(layout.skills_root().join("foo")).unwrap();

        let out = run(&layout, &home, false).unwrap();
        assert!(out.orphans_dropped.contains(&"foo".to_string()));

        let state = load(&layout.manager_dir(), &layout.skills_root()).unwrap();
        assert!(!state.skills.contains_key("foo"));
    }

    #[test]
    fn unparseable_skill_surfaces_in_parse_errors_but_does_not_abort() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "good", "[claude]");

        let bad = layout.skills_root().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("SKILL.md"), b"no frontmatter").unwrap();

        let out = run(&layout, &home, false).unwrap();
        assert_eq!(out.parse_errors.len(), 1);
        assert_eq!(out.parse_errors[0].skill, "bad");
        // good still got synced.
        let c = cell(&out, "good", "claude");
        assert!(matches!(c.action, SyncAction::CreateSymlink { .. }));
    }

    #[test]
    fn config_override_flips_codex_to_source_consumer() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[codex]");
        fs::write(
            layout.manager_dir().join("config.yaml"),
            "tool_modes:\n  codex: source-consumer\n",
        )
        .unwrap();

        let out = run(&layout, &home, false).unwrap();
        let c = cell(&out, "foo", "codex");
        assert!(matches!(c.action, SyncAction::RecordSourceConsumer));

        // No symlink at ~/.codex/skills/foo.
        let target = Tool::Codex.dist_path(&home, "foo").unwrap();
        assert!(fs::symlink_metadata(&target).is_err());
    }

    #[test]
    fn switching_distribute_to_source_consumer_retires_old_symlink() {
        let (_tmp, layout, home) = setup();
        write_skill(&layout, "foo", "[codex]");

        // Round 1: default mode = Distribute → symlink created.
        run(&layout, &home, false).unwrap();
        let target = Tool::Codex.dist_path(&home, "foo").unwrap();
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());

        // Round 2: user flips codex to source-consumer in config.yaml.
        fs::write(
            layout.manager_dir().join("config.yaml"),
            "tool_modes:\n  codex: source-consumer\n",
        )
        .unwrap();

        let out = run(&layout, &home, false).unwrap();
        // Expect one cell reporting the removal, one recording source-consumer.
        let removed = out.cells.iter().any(|c| {
            c.skill == "foo"
                && c.tool == "codex"
                && matches!(c.action, SyncAction::RemovedStale { .. })
        });
        assert!(removed, "expected RemovedStale cell for codex/foo");
        assert!(fs::symlink_metadata(&target).is_err());
    }

    #[test]
    fn fails_when_not_initialized() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));
        let err = run(&layout, tmp.path(), false).unwrap_err();
        assert!(matches!(err, SkmError::NotInitialized));
    }
}
