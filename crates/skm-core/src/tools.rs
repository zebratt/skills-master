use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Hardcoded per CEO plan: no runtime config, new tools require a new release.
///
/// The `subdir` for a Distribute tool is relative to `$HOME`, e.g. `.claude/skills`
/// → `$HOME/.claude/skills/<skill>`. Hermes is `SourceConsumer`: it reads from
/// `~/.agents/skills/` directly, so it has no per-tool distribution directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Openclaw,
    Hermes,
}

/// Distribution strategy. Phase 1 has only two; Phase 1.5 adds `Copy`/`Native`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Create a symlink in the tool's skills directory.
    Distribute,
    /// Tool reads from canonical skills_root directly; no per-tool artifact.
    SourceConsumer,
}

impl Tool {
    pub fn all() -> &'static [Tool] {
        &[Tool::Claude, Tool::Codex, Tool::Openclaw, Tool::Hermes]
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
            Tool::Openclaw => "openclaw",
            Tool::Hermes => "hermes",
        }
    }

    pub fn mode(self) -> Mode {
        match self {
            Tool::Claude | Tool::Codex | Tool::Openclaw => Mode::Distribute,
            Tool::Hermes => Mode::SourceConsumer,
        }
    }

    /// Subdirectory under `$HOME` where this tool's skills are symlinked.
    /// `SourceConsumer` tools return `None` — they have no per-tool dir.
    pub fn subdir(self) -> Option<&'static str> {
        match self {
            Tool::Claude => Some(".claude/skills"),
            Tool::Codex => Some(".codex/skills"),
            Tool::Openclaw => Some(".openclaw/skills"),
            Tool::Hermes => None,
        }
    }

    /// Directory where `skm sync` installs symlinks for this tool, given `$HOME`.
    /// Returns `None` for `SourceConsumer` tools.
    pub fn dist_root(self, home: &Path) -> Option<PathBuf> {
        self.subdir().map(|s| home.join(s))
    }

    /// Full path of this tool's distribution entry for a given skill.
    /// Returns `None` for `SourceConsumer` tools.
    pub fn dist_path(self, home: &Path, skill: &str) -> Option<PathBuf> {
        self.dist_root(home).map(|r| r.join(skill))
    }
}

/// Parse a tool name. Matches `Tool::name()` output exactly (lowercase).
impl std::str::FromStr for Tool {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Tool::Claude),
            "codex" => Ok(Tool::Codex),
            "openclaw" => Ok(Tool::Openclaw),
            "hermes" => Ok(Tool::Hermes),
            other => Err(format!("unknown tool {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn dist_path_composes_under_home() {
        let home = Path::new("/home/matt");
        assert_eq!(
            Tool::Claude.dist_path(home, "foo").unwrap(),
            PathBuf::from("/home/matt/.claude/skills/foo")
        );
        assert_eq!(
            Tool::Codex.dist_path(home, "foo").unwrap(),
            PathBuf::from("/home/matt/.codex/skills/foo")
        );
    }

    #[test]
    fn hermes_has_no_dist_path() {
        assert!(Tool::Hermes
            .dist_path(Path::new("/home/matt"), "foo")
            .is_none());
        assert_eq!(Tool::Hermes.mode(), Mode::SourceConsumer);
    }

    #[test]
    fn modes_line_up_with_ceo_plan() {
        assert_eq!(Tool::Claude.mode(), Mode::Distribute);
        assert_eq!(Tool::Codex.mode(), Mode::Distribute);
        assert_eq!(Tool::Openclaw.mode(), Mode::Distribute);
        assert_eq!(Tool::Hermes.mode(), Mode::SourceConsumer);
    }

    #[test]
    fn parse_roundtrip() {
        for t in Tool::all() {
            let parsed: Tool = t.name().parse().unwrap();
            assert_eq!(parsed, *t);
        }
        assert!("bogus".parse::<Tool>().is_err());
    }
}
