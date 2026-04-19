//! `skm init` — create the canonical directory layout and seed state.json.
//!
//! Idempotent: re-running against an initialized layout is a no-op (returns
//! `Status::AlreadyInitialized`). Never overwrites an existing state.json.
//!
//! Per ceo-plans/2026-04-19-skillsmaster.md: init is an *explicit* command
//! (Codex tension #9) — other commands that detect a missing layout return
//! `SkmError::NotInitialized` instead of silently initializing.

use crate::error::{SkmError, SkmResult};
use crate::layout::Layout;
use crate::state::{state_path, store, State};

/// Outcome of `run` — callers (CLI) render this to human or JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Layout didn't exist (or was incomplete) and was freshly created.
    Created,
    /// Layout already fully present; no write occurred.
    AlreadyInitialized,
}

/// Create `skills_root`, `manager_dir`, `backups_dir` and seed an empty
/// `state.json` if missing. Safe to call repeatedly.
pub fn run(layout: &Layout) -> SkmResult<Status> {
    let was_initialized = is_initialized(layout);

    for dir in [
        layout.skills_root(),
        layout.manager_dir(),
        layout.backups_dir(),
    ] {
        std::fs::create_dir_all(&dir).map_err(|e| SkmError::Io {
            path: dir,
            source: e,
        })?;
    }

    if !state_path(&layout.manager_dir()).exists() {
        store(&State::new(), &layout.manager_dir())?;
    }

    Ok(if was_initialized {
        Status::AlreadyInitialized
    } else {
        Status::Created
    })
}

/// Guard used by other commands (sync/status/import) per ceo-plan: if layout is
/// incomplete they fail with `NotInitialized` rather than silently bootstrap.
pub fn ensure_initialized(layout: &Layout) -> SkmResult<()> {
    if is_initialized(layout) {
        Ok(())
    } else {
        Err(SkmError::NotInitialized)
    }
}

/// Layout is "initialized" iff all three dirs exist AND state.json is present.
fn is_initialized(layout: &Layout) -> bool {
    layout.skills_root().is_dir()
        && layout.manager_dir().is_dir()
        && layout.backups_dir().is_dir()
        && state_path(&layout.manager_dir()).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn fresh_dir_gets_created() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));

        let status = run(&layout).unwrap();
        assert_eq!(status, Status::Created);
        assert!(layout.skills_root().is_dir());
        assert!(layout.manager_dir().is_dir());
        assert!(layout.backups_dir().is_dir());
        assert!(state_path(&layout.manager_dir()).is_file());
    }

    #[test]
    fn second_run_reports_already_initialized() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));

        assert_eq!(run(&layout).unwrap(), Status::Created);
        assert_eq!(run(&layout).unwrap(), Status::AlreadyInitialized);
    }

    #[test]
    fn does_not_overwrite_existing_state() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));

        // Seed a state.json with a skill entry so we can detect overwrite.
        std::fs::create_dir_all(layout.manager_dir()).unwrap();
        let marker = br#"{"version":1,"skills":{"marker":{"origin":"user","distribution":{}}}}"#;
        fs::write(state_path(&layout.manager_dir()), marker).unwrap();

        // Missing skills/backups dirs → first run still treats as not
        // initialized, but the existing state.json must survive.
        let status = run(&layout).unwrap();
        assert_eq!(status, Status::Created);

        let bytes = fs::read(state_path(&layout.manager_dir())).unwrap();
        assert!(
            std::str::from_utf8(&bytes).unwrap().contains("marker"),
            "existing state.json was overwritten"
        );
    }

    #[test]
    fn ensure_initialized_passes_after_run() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));
        run(&layout).unwrap();
        ensure_initialized(&layout).unwrap();
    }

    #[test]
    fn ensure_initialized_fails_without_run() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));
        let err = ensure_initialized(&layout).unwrap_err();
        assert!(matches!(err, SkmError::NotInitialized));
    }

    #[test]
    fn ensure_initialized_fails_when_state_missing() {
        let tmp = tempdir().unwrap();
        let layout = Layout::new(tmp.path().join("agents"));
        // Create all dirs but no state.json — simulates partial init or manual tampering.
        std::fs::create_dir_all(layout.skills_root()).unwrap();
        std::fs::create_dir_all(layout.backups_dir()).unwrap();
        let err = ensure_initialized(&layout).unwrap_err();
        assert!(matches!(err, SkmError::NotInitialized));
    }
}
