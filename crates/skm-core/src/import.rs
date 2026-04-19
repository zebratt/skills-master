//! `skm import` — promote a plugin skill into canonical `~/.agents/skills/`.
//!
//! Per ceo-plans/2026-04-19-skillsmaster.md:
//!
//!   `skm import <plugin-path> [--as <new-name>] [--strict|--force]`
//!
//! Flow:
//!   1. Validate plugin path exists and contains SKILL.md.
//!   2. Determine target name (`--as <n>` or plugin's own `name`).
//!   3. Self-containment scan of SKILL.md for `../`-style escapes.
//!      - default: warn + continue
//!      - `--strict`: warning → abort
//!      - `--force`: skip scan
//!   4. Copy plugin dir → `skills_root/<target>`.
//!   5. If renamed: rewrite `name:` in the copied SKILL.md so
//!      `frontmatter.name == dirname` invariant holds.
//!   6. Record `origin: plugin` + `lastImportedFrom` in state.json atomically.
//!
//! Post-import, `skm sync` is expected to distribute the new skill.

use crate::error::{SkmError, SkmResult};
use crate::frontmatter::{parse_skill_md, skill_md_path, Frontmatter};
use crate::init::ensure_initialized;
use crate::layout::Layout;
use crate::state::{load, store, ImportedFrom, Origin, SkillEntry};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Result of a completed import.
#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub name: String,
    pub target: PathBuf,
    pub warnings: Vec<String>,
    pub origin_marketplace: String,
    pub origin_path: String,
}

/// User-facing knobs for the self-containment check.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// `--strict`: any escape-reference warning aborts the import.
    pub strict: bool,
    /// `--force`: skip the self-containment scan entirely.
    pub force: bool,
}

/// Import `plugin_path` as a skill named `name_override` (or its own name
/// if `None`). On success, persists state.json and returns the outcome.
pub fn run(
    layout: &Layout,
    plugin_path: &Path,
    name_override: Option<&str>,
    opts: ImportOptions,
) -> SkmResult<ImportOutcome> {
    ensure_initialized(layout)?;

    // 1. Validate source dir + SKILL.md.
    if !plugin_path.is_dir() {
        return Err(SkmError::Io {
            path: plugin_path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "plugin path is not a directory",
            ),
        });
    }
    let src_skill_md = skill_md_path(plugin_path);
    if !src_skill_md.is_file() {
        return Err(SkmError::FrontmatterInvalid {
            path: src_skill_md.clone(),
            reason: "plugin directory has no SKILL.md".into(),
        });
    }

    // 2. Read the plugin's frontmatter directly — do NOT go through
    //    parse_skill_md yet because rename changes the name-vs-dirname
    //    invariant check.
    let raw = fs::read_to_string(&src_skill_md).map_err(|e| SkmError::Io {
        path: src_skill_md.clone(),
        source: e,
    })?;
    let (yaml, body_offset) =
        extract_frontmatter_with_offset(&raw).ok_or_else(|| SkmError::FrontmatterInvalid {
            path: src_skill_md.clone(),
            reason: "missing `---` frontmatter block".into(),
        })?;
    let plugin_fm: Frontmatter =
        serde_yaml::from_str(yaml).map_err(|e| SkmError::FrontmatterInvalid {
            path: src_skill_md.clone(),
            reason: e.to_string(),
        })?;

    // 3. Resolve target name.
    let target_name = name_override
        .map(str::to_owned)
        .unwrap_or_else(|| plugin_fm.name.clone());

    // 4. Target must not already exist.
    let target_dir = layout.skills_root().join(&target_name);
    if target_dir.exists() {
        return Err(SkmError::SkillConflict { name: target_name });
    }

    // 5. Self-containment scan.
    let warnings = if opts.force {
        Vec::new()
    } else {
        let w = scan_escape_refs(&raw);
        if opts.strict && !w.is_empty() {
            return Err(SkmError::ImportNonSelfContained {
                name: target_name,
                reference: w.join("; "),
            });
        }
        w
    };

    // 6. Copy plugin_path → target_dir.
    copy_dir_recursive(plugin_path, &target_dir)?;

    // 7. If renamed, rewrite `name:` in the copied SKILL.md so
    //    parse_skill_md accepts it (name must equal dirname).
    if target_name != plugin_fm.name {
        let dst_skill_md = skill_md_path(&target_dir);
        let rewritten = rewrite_name_field(&raw, &target_name, body_offset)?;
        fs::write(&dst_skill_md, rewritten).map_err(|e| SkmError::Io {
            path: dst_skill_md,
            source: e,
        })?;
    }

    // 8. Sanity-parse the installed copy. Any error here is a bug in
    //    rewrite_name_field, not a user error.
    let installed_md = skill_md_path(&target_dir);
    let _ = parse_skill_md(&installed_md)?;

    // 9. Persist state.json.
    let origin_marketplace = derive_marketplace_name(plugin_path);
    let origin_path = plugin_path
        .canonicalize()
        .unwrap_or_else(|_| plugin_path.to_path_buf())
        .display()
        .to_string();

    let mut state = load(&layout.manager_dir(), &layout.skills_root())?;
    state.skills.insert(
        target_name.clone(),
        SkillEntry {
            origin: Origin::Plugin,
            last_synced_sha: None,
            last_synced_at: Some(current_timestamp()),
            last_imported_from: Some(ImportedFrom {
                origin_marketplace: origin_marketplace.clone(),
                origin_path: origin_path.clone(),
            }),
            distribution: BTreeMap::new(),
        },
    );
    store(&state, &layout.manager_dir())?;

    Ok(ImportOutcome {
        name: target_name,
        target: target_dir,
        warnings,
        origin_marketplace,
        origin_path,
    })
}

/// Extract the YAML block between `---` fences. Returns `(yaml, body_byte_offset)`
/// where `body_byte_offset` is the index in `raw` where the post-frontmatter body
/// begins. Used by `rewrite_name_field`.
fn extract_frontmatter_with_offset(raw: &str) -> Option<(&str, usize)> {
    let rest = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))?;
    let prefix_len = raw.len() - rest.len();

    for (pat, after_len) in [("\n---\n", 5usize), ("\n---\r\n", 6), ("\n---", 4)] {
        if let Some(idx) = rest.find(pat) {
            let yaml = &rest[..idx];
            let body_start = prefix_len + idx + after_len;
            return Some((yaml, body_start));
        }
    }
    None
}

/// Rewrite the `name:` line in the YAML frontmatter so it reads `name: <new>`.
/// The rest of the file is preserved byte-for-byte.
fn rewrite_name_field(raw: &str, new_name: &str, body_offset: usize) -> SkmResult<String> {
    // The YAML block starts after `---\n` (either 4 or 5 bytes) and ends
    // `body_offset` bytes into `raw`.
    let header_len = if raw.starts_with("---\r\n") { 5 } else { 4 };
    let yaml_end = body_offset.saturating_sub(if raw[..body_offset].ends_with("\r\n") {
        6
    } else {
        if raw[..body_offset].ends_with("\n") {
            5
        } else {
            4
        }
    });

    let yaml = &raw[header_len..yaml_end];
    let rewritten_yaml =
        replace_name_line(yaml, new_name).ok_or_else(|| SkmError::FrontmatterInvalid {
            path: PathBuf::from("<imported>"),
            reason: "could not find `name:` line to rewrite".into(),
        })?;

    let mut out = String::new();
    out.push_str(&raw[..header_len]);
    out.push_str(&rewritten_yaml);
    out.push_str(&raw[yaml_end..]);
    Ok(out)
}

/// Replace the first `name:` line in a YAML block. Preserves surrounding lines.
fn replace_name_line(yaml: &str, new_name: &str) -> Option<String> {
    let mut out = String::new();
    let mut replaced = false;
    for line in yaml.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("name:") {
            // Preserve leading whitespace.
            let indent_len = line.len() - trimmed.len();
            out.push_str(&line[..indent_len]);
            out.push_str(&format!("name: {new_name}"));
            // Preserve trailing newline if present.
            if line.ends_with("\r\n") {
                out.push_str("\r\n");
            } else if line.ends_with('\n') {
                out.push('\n');
            }
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    if replaced {
        Some(out)
    } else {
        None
    }
}

/// Scan raw SKILL.md text for relative references that escape the skill root.
/// Returns a list of human-readable warning strings with line numbers.
///
/// Heuristic: any substring matching `../` is flagged. False positives are
/// possible (e.g., in prose describing other tools) but per CEO plan the
/// default behavior is warn-only — user can run with `--force` to skip.
fn scan_escape_refs(raw: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        if line.contains("../") {
            warnings.push(format!(
                "line {}: escape reference `../` → {}",
                idx + 1,
                line.trim()
            ));
        }
    }
    warnings
}

/// Best-effort: grandparent directory name (e.g., `marketplace-x/skills/foo` →
/// `marketplace-x`). Falls back to parent name, then `"unknown"`.
fn derive_marketplace_name(plugin_path: &Path) -> String {
    plugin_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .or_else(|| plugin_path.parent().and_then(|p| p.file_name()))
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

fn current_timestamp() -> String {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal ISO-ish: state.json consumers treat this opaquely in Phase 1.
    format!("{epoch}")
}

/// Recursive copy. Follows files, not symlinks — symlinked resources in plugin
/// dirs are intentionally out of scope in Phase 1 (caught by scan, user handles).
fn copy_dir_recursive(src: &Path, dst: &Path) -> SkmResult<()> {
    fs::create_dir_all(dst).map_err(|e| SkmError::Io {
        path: dst.to_path_buf(),
        source: e,
    })?;
    for entry in fs::read_dir(src).map_err(|e| SkmError::Io {
        path: src.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| SkmError::Io {
            path: src.to_path_buf(),
            source: e,
        })?;
        let ftype = entry.file_type().map_err(|e| SkmError::Io {
            path: entry.path(),
            source: e,
        })?;
        let target = dst.join(entry.file_name());
        if ftype.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ftype.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| SkmError::Io {
                path: target.clone(),
                source: e,
            })?;
        }
        // Skip symlinks — see above.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Layout) {
        let tmp = tempdir().unwrap();
        let layout = Layout::default_for_home(tmp.path());
        init::run(&layout).unwrap();
        (tmp, layout)
    }

    fn write_plugin_skill(parent: &Path, name: &str, body: &str) -> PathBuf {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).unwrap();
        let md = format!("---\nname: {name}\ntools: [claude]\n---\n{body}");
        fs::write(dir.join("SKILL.md"), md).unwrap();
        dir
    }

    #[test]
    fn imports_plugin_preserving_name() {
        let (tmp, layout) = setup();
        let src_root = tmp.path().join("market/skills");
        let plugin = write_plugin_skill(&src_root, "context7", "# body\n");

        let out = run(&layout, &plugin, None, ImportOptions::default()).unwrap();
        assert_eq!(out.name, "context7");
        assert!(out.target.join("SKILL.md").is_file());

        let state = load(&layout.manager_dir(), &layout.skills_root()).unwrap();
        let entry = state.skills.get("context7").unwrap();
        assert_eq!(entry.origin, Origin::Plugin);
        assert!(entry.last_imported_from.is_some());
        assert_eq!(
            entry
                .last_imported_from
                .as_ref()
                .unwrap()
                .origin_marketplace,
            "market"
        );
    }

    #[test]
    fn rename_rewrites_name_field_so_parse_accepts_it() {
        let (tmp, layout) = setup();
        let src = write_plugin_skill(&tmp.path().join("market/skills"), "context7", "# body\n");

        run(&layout, &src, Some("ctx7"), ImportOptions::default()).unwrap();

        let installed = layout.skills_root().join("ctx7/SKILL.md");
        let fm = parse_skill_md(&installed).unwrap();
        assert_eq!(fm.name, "ctx7");
    }

    #[test]
    fn target_collision_returns_skill_conflict() {
        let (tmp, layout) = setup();
        // Seed a canonical skill with the same name.
        let existing = layout.skills_root().join("foo");
        fs::create_dir_all(&existing).unwrap();
        fs::write(
            existing.join("SKILL.md"),
            "---\nname: foo\ntools: all\n---\n",
        )
        .unwrap();

        let plugin = write_plugin_skill(&tmp.path().join("market/skills"), "foo", "");
        let err = run(&layout, &plugin, None, ImportOptions::default()).unwrap_err();
        assert!(matches!(err, SkmError::SkillConflict { .. }));
    }

    #[test]
    fn escape_ref_warning_does_not_abort_by_default() {
        let (tmp, layout) = setup();
        let src = write_plugin_skill(
            &tmp.path().join("market/skills"),
            "risky",
            "See ../outside/helper.md for more.\n",
        );
        let out = run(&layout, &src, None, ImportOptions::default()).unwrap();
        assert!(!out.warnings.is_empty());
        assert!(out.warnings[0].contains("../outside"));
    }

    #[test]
    fn strict_mode_aborts_on_escape_ref() {
        let (tmp, layout) = setup();
        let src = write_plugin_skill(
            &tmp.path().join("market/skills"),
            "risky",
            "bad ../escape here\n",
        );
        let err = run(
            &layout,
            &src,
            None,
            ImportOptions {
                strict: true,
                force: false,
            },
        )
        .unwrap_err();
        assert!(matches!(err, SkmError::ImportNonSelfContained { .. }));

        // Target should NOT have been created.
        assert!(!layout.skills_root().join("risky").exists());
    }

    #[test]
    fn force_skips_scan_entirely() {
        let (tmp, layout) = setup();
        let src = write_plugin_skill(
            &tmp.path().join("market/skills"),
            "risky",
            "bad ../escape here\n",
        );
        let out = run(
            &layout,
            &src,
            None,
            ImportOptions {
                strict: false,
                force: true,
            },
        )
        .unwrap();
        assert!(out.warnings.is_empty());
        assert!(layout.skills_root().join("risky").exists());
    }

    #[test]
    fn missing_plugin_path_returns_error() {
        let (tmp, layout) = setup();
        let err = run(
            &layout,
            &tmp.path().join("does-not-exist"),
            None,
            ImportOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, SkmError::Io { .. }));
    }

    #[test]
    fn plugin_without_skill_md_returns_error() {
        let (tmp, layout) = setup();
        let src = tmp.path().join("empty");
        fs::create_dir_all(&src).unwrap();
        let err = run(&layout, &src, None, ImportOptions::default()).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn rewrite_name_line_preserves_other_keys() {
        let yaml = "name: foo\ndescription: hi\ntools: [claude]\n";
        let out = replace_name_line(yaml, "bar").unwrap();
        assert!(out.starts_with("name: bar\n"));
        assert!(out.contains("description: hi"));
        assert!(out.contains("tools: [claude]"));
    }

    #[test]
    fn copies_nested_assets() {
        let (tmp, layout) = setup();
        let src = tmp.path().join("market/skills/deep");
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(
            src.join("SKILL.md"),
            "---\nname: deep\ntools: [claude]\n---\n",
        )
        .unwrap();
        fs::write(src.join("assets/icon.svg"), b"<svg/>").unwrap();

        run(&layout, &src, None, ImportOptions::default()).unwrap();
        assert!(layout.skills_root().join("deep/assets/icon.svg").is_file());
    }
}
