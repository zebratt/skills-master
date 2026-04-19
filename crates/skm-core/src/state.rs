//! state.json — external sync metadata, atomic rename + flock protected.
//!
//! Phase 1 schema matches ceo-plans/2026-04-19-skillsmaster.md:
//!
//! ```jsonc
//! {
//!   "version": 1,
//!   "skills": {
//!     "<name>": {
//!       "origin": "user" | "plugin",
//!       "lastSyncedSha": "abc...",           // git/plugin upstream only
//!       "lastSyncedAt": "2026-04-19T...",
//!       "lastImportedFrom": { ... },          // import-only
//!       "distribution": {
//!         "claude":   { "mode": "symlink",         "path": "~/.claude/skills/foo" },
//!         "codex":    { "mode": "symlink",         "path": "~/.codex/skills/foo" },
//!         "openclaw": { "mode": "symlink",         "path": "~/.openclaw/skills/foo" },
//!         "hermes":   { "mode": "source-consumer", "path": "~/.agents/skills/foo" }
//!       }
//!     }
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub skills: BTreeMap<String, SkillEntry>,
}

impl State {
    pub fn new() -> Self {
        Self {
            version: 1,
            skills: BTreeMap::new(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub origin: Origin,
    #[serde(rename = "lastSyncedSha", skip_serializing_if = "Option::is_none")]
    pub last_synced_sha: Option<String>,
    #[serde(rename = "lastSyncedAt", skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(rename = "lastImportedFrom", skip_serializing_if = "Option::is_none")]
    pub last_imported_from: Option<ImportedFrom>,
    pub distribution: BTreeMap<String, DistributionRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    User,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedFrom {
    #[serde(rename = "originMarketplace")]
    pub origin_marketplace: String,
    #[serde(rename = "originPath")]
    pub origin_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionRecord {
    pub mode: DistributionMode,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DistributionMode {
    Symlink,
    SourceConsumer,
    Missing,
    Conflict,
    Broken,
}

// TODO Phase 1:
// - fn load(dir: &Path) -> SkmResult<State>  (with .bak fallback + filesystem rebuild)
// - fn store(&self, dir: &Path) -> SkmResult<()>  (atomic rename + flock guard)
// - fn lock_path(dir: &Path) -> PathBuf
