//! Canonical directory layout for SkillsMaster.
//!
//! Per ceo-plans/2026-04-19-skillsmaster.md:
//! ```text
//! ~/.agents/skills/                 # canonical skills root (the source of truth)
//! ~/.agents/skills-manager/         # internal bookkeeping
//!   ├── state.json                  # sync metadata
//!   ├── state.json.bak              # single backup
//!   ├── .state.lock                 # advisory flock
//!   └── backups/                    # pre-sync filesystem snapshots (Phase 1.5)
//! ```
//!
//! Paths are computed lazily at call time (no global state) so tests can inject
//! arbitrary roots via `Layout::new(root)`.

use std::path::{Path, PathBuf};

/// Default parent directory under `$HOME` that holds both `skills/` and
/// `skills-manager/`. Kept as a constant so tests and CLI agree.
pub const DEFAULT_ROOT_DIR: &str = ".agents";

/// Subdirectory under the root that holds the canonical skills.
pub const SKILLS_SUBDIR: &str = "skills";

/// Subdirectory under the root that holds skm's internal state.
pub const MANAGER_SUBDIR: &str = "skills-manager";

/// Subdirectory under `manager_dir` that holds pre-sync snapshots (Phase 1.5).
pub const BACKUPS_SUBDIR: &str = "backups";

/// A resolved SkillsMaster layout rooted at a parent directory.
///
/// Typical production use: `Layout::default_for_home(&home)` where `home` is
/// `$HOME`. Tests pass a `tempdir()` root directly via `Layout::new(root)`.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// Build a layout rooted at `<root>` (typically `$HOME/.agents`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Build the default layout given `$HOME`: `<home>/.agents`.
    pub fn default_for_home(home: &Path) -> Self {
        Self::new(home.join(DEFAULT_ROOT_DIR))
    }

    /// Parent directory holding both skills/ and skills-manager/.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Canonical skills directory — the one every tool points at or syncs from.
    pub fn skills_root(&self) -> PathBuf {
        self.root.join(SKILLS_SUBDIR)
    }

    /// Internal bookkeeping directory (state.json, locks, etc.).
    pub fn manager_dir(&self) -> PathBuf {
        self.root.join(MANAGER_SUBDIR)
    }

    /// Pre-sync snapshots directory (Phase 1.5 `doctor --fix`).
    pub fn backups_dir(&self) -> PathBuf {
        self.manager_dir().join(BACKUPS_SUBDIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn default_for_home_uses_dot_agents() {
        let layout = Layout::default_for_home(Path::new("/home/matt"));
        assert_eq!(layout.root(), PathBuf::from("/home/matt/.agents"));
        assert_eq!(
            layout.skills_root(),
            PathBuf::from("/home/matt/.agents/skills")
        );
        assert_eq!(
            layout.manager_dir(),
            PathBuf::from("/home/matt/.agents/skills-manager")
        );
        assert_eq!(
            layout.backups_dir(),
            PathBuf::from("/home/matt/.agents/skills-manager/backups")
        );
    }

    #[test]
    fn new_accepts_arbitrary_root() {
        let layout = Layout::new("/tmp/skm-test");
        assert_eq!(layout.skills_root(), PathBuf::from("/tmp/skm-test/skills"));
        assert_eq!(
            layout.manager_dir(),
            PathBuf::from("/tmp/skm-test/skills-manager")
        );
    }
}
