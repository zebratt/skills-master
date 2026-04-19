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

use crate::error::{SkmError, SkmResult};
use crate::frontmatter::parse_skill_md;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Filename of the canonical state file inside the manager directory.
pub const STATE_FILE: &str = "state.json";
/// Filename of the single backup, written before every successful store.
pub const STATE_BAK_FILE: &str = "state.json.bak";
/// Filename of the advisory flock file, serializing concurrent stores.
pub const STATE_LOCK_FILE: &str = ".state.lock";

/// How long `store` polls `.state.lock` before giving up with `LockTimeout`.
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Path to the lock file for a given manager directory.
pub fn lock_path(manager_dir: &Path) -> PathBuf {
    manager_dir.join(STATE_LOCK_FILE)
}

/// Path to the primary state file.
pub fn state_path(manager_dir: &Path) -> PathBuf {
    manager_dir.join(STATE_FILE)
}

/// Path to the single backup.
pub fn state_bak_path(manager_dir: &Path) -> PathBuf {
    manager_dir.join(STATE_BAK_FILE)
}

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

/// Load state from `manager_dir`, applying the fallback chain defined in
/// `ceo-plans/2026-04-19-skillsmaster.md`:
///
/// 1. Read and parse `state.json`.
/// 2. On read or parse failure, read and parse `state.json.bak`.
/// 3. On both failing, rebuild a lossy state from `skills_root` by scanning
///    every `SKILL.md` frontmatter. `lastImportedFrom`, `lastSyncedSha`,
///    and `distribution` are lost on this path — they live only in state.json.
///
/// Returns `SkmError::StateCorrupted` only if every fallback also fails.
pub fn load(manager_dir: &Path, skills_root: &Path) -> SkmResult<State> {
    match read_json(&state_path(manager_dir)) {
        Ok(Some(state)) => return Ok(state),
        Ok(None) => {} // missing, fall through
        Err(_) => {}   // corrupt, fall through
    }
    match read_json(&state_bak_path(manager_dir)) {
        Ok(Some(state)) => return Ok(state),
        Ok(None) => {}
        Err(_) => {}
    }
    rebuild_from_filesystem(skills_root).map_err(|e| {
        SkmError::StateCorrupted(format!(
            "state.json and backup unreadable; filesystem rebuild failed: {e}"
        ))
    })
}

/// Atomic store: flock `.state.lock` (5s budget), `cp state.json state.json.bak`
/// if the primary exists, write `state.json.tmp`, fsync, rename over `state.json`.
pub fn store(state: &State, manager_dir: &Path) -> SkmResult<()> {
    std::fs::create_dir_all(manager_dir).map_err(|e| SkmError::Io {
        path: manager_dir.to_path_buf(),
        source: e,
    })?;

    let lock_file = acquire_lock(manager_dir)?;

    let primary = state_path(manager_dir);
    let backup = state_bak_path(manager_dir);
    let tmp = manager_dir.join(format!("{STATE_FILE}.tmp"));

    if primary.exists() {
        std::fs::copy(&primary, &backup).map_err(|e| SkmError::Io {
            path: backup.clone(),
            source: e,
        })?;
    }

    let bytes = serde_json::to_vec_pretty(state)?;
    {
        let mut f = File::create(&tmp).map_err(|e| SkmError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        f.write_all(&bytes).map_err(|e| SkmError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        f.sync_all().map_err(|e| SkmError::Io {
            path: tmp.clone(),
            source: e,
        })?;
    }
    std::fs::rename(&tmp, &primary).map_err(|e| SkmError::Io {
        path: primary.clone(),
        source: e,
    })?;

    // Explicit unlock: drop releases the fd, but being explicit documents intent.
    FileExt::unlock(&lock_file).map_err(|e| SkmError::Io {
        path: lock_path(manager_dir),
        source: e,
    })?;
    Ok(())
}

/// Scan `skills_root/*/SKILL.md` and synthesize a fresh `State`.
///
/// Lossy: `lastImportedFrom`, `lastSyncedSha`, `lastSyncedAt`, and `distribution`
/// records cannot be reconstructed from the filesystem alone. Callers should treat
/// this as an emergency fallback — the next `skm sync` will re-populate distribution.
pub fn rebuild_from_filesystem(skills_root: &Path) -> SkmResult<State> {
    let mut state = State::new();
    let entries = match std::fs::read_dir(skills_root) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(state),
        Err(e) => {
            return Err(SkmError::Io {
                path: skills_root.to_path_buf(),
                source: e,
            })
        }
    };
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
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        let fm = parse_skill_md(&skill_md)?;
        state.skills.insert(
            fm.name.clone(),
            SkillEntry {
                origin: Origin::User,
                last_synced_sha: None,
                last_synced_at: None,
                last_imported_from: None,
                distribution: BTreeMap::new(),
            },
        );
    }
    Ok(state)
}

/// Read a JSON file, returning:
///   - `Ok(Some(state))` on success,
///   - `Ok(None)` if the file is absent,
///   - `Err(_)` on any read/parse error (corruption).
fn read_json(path: &Path) -> SkmResult<Option<State>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let state: State = serde_json::from_slice(&bytes)?;
            Ok(Some(state))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SkmError::Io {
            path: path.to_path_buf(),
            source: e,
        }),
    }
}

/// Poll `try_lock_exclusive` with a short backoff until `LOCK_TIMEOUT` elapses.
///
/// `fs2::FileExt::try_lock_exclusive` maps to `flock(LOCK_EX|LOCK_NB)` on Unix,
/// which is per-file-descriptor advisory locking — safe for multi-process /ship
/// and safe for same-process testing via distinct handles.
fn acquire_lock(manager_dir: &Path) -> SkmResult<File> {
    let path = lock_path(manager_dir);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| SkmError::Io {
            path: path.clone(),
            source: e,
        })?;
    let deadline = Instant::now() + LOCK_TIMEOUT;
    let backoff = Duration::from_millis(50);
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(_) => {
                if Instant::now() >= deadline {
                    return Err(SkmError::LockTimeout {
                        timeout_secs: LOCK_TIMEOUT.as_secs(),
                    });
                }
                sleep(backoff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn sample_state() -> State {
        let mut state = State::new();
        let mut distribution = BTreeMap::new();
        distribution.insert(
            "claude".into(),
            DistributionRecord {
                mode: DistributionMode::Symlink,
                path: "~/.claude/skills/foo".into(),
            },
        );
        distribution.insert(
            "hermes".into(),
            DistributionRecord {
                mode: DistributionMode::SourceConsumer,
                path: "~/.agents/skills/foo".into(),
            },
        );
        state.skills.insert(
            "foo".into(),
            SkillEntry {
                origin: Origin::User,
                last_synced_sha: Some("abc1234".into()),
                last_synced_at: Some("2026-04-19T12:00:00Z".into()),
                last_imported_from: None,
                distribution,
            },
        );
        state
    }

    fn write_skill_dir(root: &Path, name: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ntools: [claude]\n---\nbody\n"),
        )
        .unwrap();
    }

    #[test]
    fn roundtrip_empty_state() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path();
        let skills = tmp.path().join("skills"); // empty, unused

        let state = State::new();
        store(&state, mgr).unwrap();
        let loaded = load(mgr, &skills).unwrap();
        assert_eq!(loaded.version, 1);
        assert!(loaded.skills.is_empty());
    }

    #[test]
    fn roundtrip_full_state_preserves_fields() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path();
        let skills = tmp.path().join("skills");

        let state = sample_state();
        store(&state, mgr).unwrap();

        let loaded = load(mgr, &skills).unwrap();
        let entry = loaded.skills.get("foo").expect("foo present");
        assert_eq!(entry.origin, Origin::User);
        assert_eq!(entry.last_synced_sha.as_deref(), Some("abc1234"));
        assert_eq!(entry.distribution.len(), 2);
        assert_eq!(
            entry.distribution.get("claude").unwrap().mode,
            DistributionMode::Symlink
        );
        assert_eq!(
            entry.distribution.get("hermes").unwrap().mode,
            DistributionMode::SourceConsumer
        );
    }

    #[test]
    fn store_writes_backup_on_second_call() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path();

        let first = State::new();
        store(&first, mgr).unwrap();
        assert!(state_path(mgr).exists());
        assert!(!state_bak_path(mgr).exists());

        let second = sample_state();
        store(&second, mgr).unwrap();
        assert!(state_bak_path(mgr).exists());

        // .bak holds the previous (empty) state, not the current one.
        let bak_bytes = fs::read(state_bak_path(mgr)).unwrap();
        let bak: State = serde_json::from_slice(&bak_bytes).unwrap();
        assert!(bak.skills.is_empty());
    }

    #[test]
    fn load_falls_back_to_bak_when_primary_corrupt() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path();
        let skills = tmp.path().join("skills");

        // Lay down a good .bak, then a garbage primary.
        fs::create_dir_all(mgr).unwrap();
        let good = sample_state();
        fs::write(
            state_bak_path(mgr),
            serde_json::to_vec_pretty(&good).unwrap(),
        )
        .unwrap();
        fs::write(state_path(mgr), b"{ not valid json").unwrap();

        let loaded = load(mgr, &skills).unwrap();
        assert!(loaded.skills.contains_key("foo"));
    }

    #[test]
    fn load_rebuilds_from_filesystem_when_both_corrupt() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path().join("manager");
        let skills = tmp.path().join("skills");
        fs::create_dir_all(&mgr).unwrap();
        fs::create_dir_all(&skills).unwrap();

        write_skill_dir(&skills, "alpha");
        write_skill_dir(&skills, "beta");

        fs::write(state_path(&mgr), b"{ broken").unwrap();
        fs::write(state_bak_path(&mgr), b"also broken").unwrap();

        let loaded = load(&mgr, &skills).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.skills.len(), 2);
        let alpha = loaded.skills.get("alpha").unwrap();
        assert_eq!(alpha.origin, Origin::User);
        assert!(alpha.last_synced_sha.is_none());
        assert!(alpha.last_imported_from.is_none());
        assert!(alpha.distribution.is_empty());
    }

    #[test]
    fn load_returns_empty_state_when_nothing_exists() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path().join("manager");
        fs::create_dir_all(&mgr).unwrap();
        let skills = tmp.path().join("skills"); // intentionally missing

        let loaded = load(&mgr, &skills).unwrap();
        assert!(loaded.skills.is_empty());
    }

    #[test]
    fn store_creates_manager_dir_if_missing() {
        let tmp = tempdir().unwrap();
        let mgr = tmp.path().join("fresh");
        assert!(!mgr.exists());

        store(&State::new(), &mgr).unwrap();
        assert!(state_path(&mgr).exists());
    }

    #[test]
    fn store_times_out_when_lock_held() {
        // Simulate a contending process holding the flock. We open the lock file
        // via an independent file descriptor (flock is per-fd on macOS/Linux),
        // lock it, then call store() which should fail with LockTimeout.
        let tmp = tempdir().unwrap();
        let mgr = tmp.path();
        std::fs::create_dir_all(mgr).unwrap();

        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path(mgr))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        // LOCK_TIMEOUT is 5s; this must finish in ~5s.
        let start = Instant::now();
        let err = store(&State::new(), mgr).unwrap_err();
        let elapsed = start.elapsed();
        assert!(
            matches!(err, SkmError::LockTimeout { .. }),
            "expected LockTimeout, got {err:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(4),
            "store returned too quickly ({elapsed:?})"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "store took too long ({elapsed:?})"
        );

        FileExt::unlock(&lock).unwrap();
    }
}
