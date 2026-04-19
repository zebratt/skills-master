//! SKILL.md frontmatter parser.
//!
//! Phase 1 schema:
//!
//! ```yaml
//! ---
//! name: <string>   # must equal dirname
//! description: <string>
//! tools: [claude, codex, openclaw, hermes] | 'all'
//! upstream:
//!   type: git | plugin | none
//!   # type=git:    url (required), ref (default 'main')
//!   # type=plugin: originMarketplace, originPath
//! ---
//! ```

use crate::error::{SkmError, SkmResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level SKILL.md frontmatter. Unknown fields are tolerated so we can
/// coexist with Claude Code's `allowed-tools` and other upstream conventions.
/// Missing `tools` defaults to `all` — the friendly choice, since distribution
/// to a tool is just a symlink with no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_tools_all")]
    pub tools: Tools,
    #[serde(default)]
    pub upstream: Option<Upstream>,
}

fn default_tools_all() -> Tools {
    Tools::All(AllMarker::All)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Tools {
    All(AllMarker),
    List(Vec<crate::tools::Tool>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AllMarker {
    #[serde(rename = "all")]
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Upstream {
    Git {
        url: String,
        #[serde(default = "default_git_ref")]
        r#ref: String,
    },
    Plugin {
        #[serde(rename = "originMarketplace")]
        origin_marketplace: String,
        #[serde(rename = "originPath")]
        origin_path: String,
    },
    None,
}

fn default_git_ref() -> String {
    "main".to_string()
}

/// Parse a SKILL.md file: split the YAML frontmatter block from the body,
/// deserialize the YAML, and verify `name` matches the parent directory name.
///
/// Returns the parsed `Frontmatter`. Body content is discarded — Phase 1 only
/// needs the metadata.
pub fn parse_skill_md(path: &Path) -> SkmResult<Frontmatter> {
    let raw = std::fs::read_to_string(path).map_err(|e| SkmError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let yaml = extract_frontmatter(&raw).ok_or_else(|| SkmError::FrontmatterInvalid {
        path: path.to_path_buf(),
        reason: "missing `---` frontmatter block at top of file".into(),
    })?;

    let fm: Frontmatter = serde_yaml::from_str(yaml).map_err(|e| SkmError::FrontmatterInvalid {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    let dirname = skill_dirname(path).ok_or_else(|| SkmError::FrontmatterInvalid {
        path: path.to_path_buf(),
        reason: "cannot derive skill directory name from path".into(),
    })?;

    if fm.name != dirname {
        return Err(SkmError::NameDirnameMismatch {
            name: fm.name,
            dirname,
        });
    }

    Ok(fm)
}

/// Return the content between the first `---\n` and the next `\n---` line.
/// Accepts optional leading BOM / whitespace-only lines before the opening fence.
fn extract_frontmatter(raw: &str) -> Option<&str> {
    let s = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let rest = s
        .strip_prefix("---\n")
        .or_else(|| s.strip_prefix("---\r\n"))?;

    let end_patterns = ["\n---\n", "\n---\r\n", "\n---"];
    for pat in end_patterns {
        if let Some(idx) = rest.find(pat) {
            let yaml = &rest[..idx];
            // The closing fence must start at a line boundary, which `\n---` guarantees.
            // If trailing data follows `\n---` without newline, that's the last line, fine.
            return Some(yaml);
        }
    }
    None
}

/// For `.../<skill>/SKILL.md`, return `<skill>`.
fn skill_dirname(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let name = parent.file_name()?;
    Some(name.to_string_lossy().into_owned())
}

/// Return the canonical SKILL.md path given a skill directory.
pub fn skill_md_path(skill_dir: &Path) -> PathBuf {
    skill_dir.join("SKILL.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_skill(dir: &Path, name: &str, body: &str) -> PathBuf {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn parses_minimal_tools_list() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "foo",
            "---\nname: foo\ntools: [claude, codex]\n---\n# body\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert_eq!(fm.name, "foo");
        match fm.tools {
            Tools::List(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected list"),
        }
        assert!(fm.description.is_none());
        assert!(fm.upstream.is_none());
    }

    #[test]
    fn parses_tools_all_marker() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "bar",
            "---\nname: bar\ndescription: hi\ntools: all\n---\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert!(matches!(fm.tools, Tools::All(AllMarker::All)));
        assert_eq!(fm.description.as_deref(), Some("hi"));
    }

    #[test]
    fn parses_upstream_git_with_default_ref() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "baz",
            "---\nname: baz\ntools: [hermes]\nupstream:\n  type: git\n  url: https://example.com/x.git\n---\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        match fm.upstream {
            Some(Upstream::Git { url, r#ref }) => {
                assert_eq!(url, "https://example.com/x.git");
                assert_eq!(r#ref, "main");
            }
            _ => panic!("expected git upstream"),
        }
    }

    #[test]
    fn parses_upstream_plugin() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "plg",
            "---\nname: plg\ntools: [claude]\nupstream:\n  type: plugin\n  originMarketplace: mkt\n  originPath: a/b\n---\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        match fm.upstream {
            Some(Upstream::Plugin {
                origin_marketplace,
                origin_path,
            }) => {
                assert_eq!(origin_marketplace, "mkt");
                assert_eq!(origin_path, "a/b");
            }
            _ => panic!("expected plugin upstream"),
        }
    }

    #[test]
    fn rejects_name_dirname_mismatch() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "dirname",
            "---\nname: other\ntools: [claude]\n---\n",
        );
        let err = parse_skill_md(&path).unwrap_err();
        match err {
            SkmError::NameDirnameMismatch { name, dirname } => {
                assert_eq!(name, "other");
                assert_eq!(dirname, "dirname");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_frontmatter_fence() {
        let tmp = tempdir().unwrap();
        let path = write_skill(tmp.path(), "nofence", "name: nofence\ntools: [claude]\n");
        let err = parse_skill_md(&path).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn rejects_unclosed_frontmatter() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "unclosed",
            "---\nname: unclosed\ntools: [claude]\n",
        );
        let err = parse_skill_md(&path).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn tolerates_unknown_top_level_field() {
        // Claude Code SKILL.md files carry `allowed-tools:` — we must not reject them.
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "extra",
            "---\nname: extra\ntools: [claude]\nallowed-tools: Bash(ls:*)\nbogus: 1\n---\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert_eq!(fm.name, "extra");
    }

    #[test]
    fn missing_tools_defaults_to_all() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "notools",
            "---\nname: notools\ndescription: hi\n---\n# body\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert!(matches!(fm.tools, Tools::All(AllMarker::All)));
    }

    #[test]
    fn rejects_unknown_tool() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "badtool",
            "---\nname: badtool\ntools: [cursor]\n---\n",
        );
        let err = parse_skill_md(&path).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn accepts_crlf_frontmatter() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "crlf",
            "---\r\nname: crlf\r\ntools: [claude]\r\n---\r\nbody\r\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert_eq!(fm.name, "crlf");
    }

    #[test]
    fn accepts_bom() {
        let tmp = tempdir().unwrap();
        let path = write_skill(
            tmp.path(),
            "bomskill",
            "\u{feff}---\nname: bomskill\ntools: [claude]\n---\n",
        );
        let fm = parse_skill_md(&path).unwrap();
        assert_eq!(fm.name, "bomskill");
    }
}
