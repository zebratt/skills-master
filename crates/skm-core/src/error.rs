use std::path::PathBuf;
use thiserror::Error;

pub type SkmResult<T> = Result<T, SkmError>;

#[derive(Debug, Error)]
pub enum SkmError {
    #[error("state.json is corrupt and backup unreadable: {0}")]
    StateCorrupted(String),

    #[error(
        "could not acquire state lock within {timeout_secs}s (another skm process may be running)"
    )]
    LockTimeout { timeout_secs: u64 },

    #[error("frontmatter at {path:?} is invalid: {reason}")]
    FrontmatterInvalid { path: PathBuf, reason: String },

    #[error("skill name {name:?} does not match directory {dirname:?}")]
    NameDirnameMismatch { name: String, dirname: String },

    #[error("skill {name:?} references path outside its directory: {reference}")]
    ImportNonSelfContained { name: String, reference: String },

    #[error("conflict: skill {name:?} exists in multiple sources")]
    SkillConflict { name: String },

    #[error("skm has not been initialized; run `skm init` first")]
    NotInitialized,

    #[error("io error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
