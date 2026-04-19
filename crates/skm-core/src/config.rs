//! User-editable configuration at `~/.agents/skills-manager/config.yaml`.
//!
//! The CEO plan's "no runtime config" applied to the **tool set**: tools are
//! hardcoded and new ones require a release. But a tool's distribution **mode**
//! depends on how the user has configured that tool locally. Example: OpenClaw
//! and some Codex setups can be pointed at `~/.agents/skills/` directly, making
//! them `source-consumer` instead of `distribute`. We give users a way to say so.
//!
//! Schema (all fields optional):
//!
//! ```yaml
//! tool_modes:
//!   codex: source-consumer   # override: Codex reads skills_root directly
//!   openclaw: source-consumer
//! ```
//!
//! Accepted values: `distribute` | `source-consumer`. Unknown tool names or
//! modes are an error — we won't silently drop typos.
//!
//! Missing file is not an error; `load()` returns defaults.

use crate::error::{SkmError, SkmResult};
use crate::tools::{Mode, Tool};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    tool_modes: BTreeMap<String, ModeName>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ModeName {
    Distribute,
    SourceConsumer,
}

impl From<ModeName> for Mode {
    fn from(m: ModeName) -> Self {
        match m {
            ModeName::Distribute => Mode::Distribute,
            ModeName::SourceConsumer => Mode::SourceConsumer,
        }
    }
}

/// Resolved config: per-tool mode overrides, keyed by `Tool`.
#[derive(Debug, Clone, Default)]
pub struct Config {
    tool_modes: BTreeMap<Tool, Mode>,
}

impl Config {
    /// Load `manager_dir/config.yaml`. Missing file → empty config.
    pub fn load(manager_dir: &Path) -> SkmResult<Self> {
        let path = manager_dir.join("config.yaml");
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => {
                return Err(SkmError::Io {
                    path: path.clone(),
                    source: e,
                });
            }
        };
        let file: ConfigFile =
            serde_yaml::from_str(&raw).map_err(|e| SkmError::FrontmatterInvalid {
                path: path.clone(),
                reason: format!("config.yaml: {e}"),
            })?;
        Self::from_file(file, &path)
    }

    fn from_file(file: ConfigFile, path: &Path) -> SkmResult<Self> {
        let mut tool_modes = BTreeMap::new();
        for (k, v) in file.tool_modes {
            let tool: Tool = k.parse().map_err(|e| SkmError::FrontmatterInvalid {
                path: path.to_path_buf(),
                reason: format!("config.yaml tool_modes: {e}"),
            })?;
            tool_modes.insert(tool, v.into());
        }
        Ok(Self { tool_modes })
    }

    /// Resolve the distribution mode for `tool`. Returns the override if the user
    /// has set one, otherwise the hardcoded default from `Tool::mode()`.
    pub fn mode_for(&self, tool: Tool) -> Mode {
        self.tool_modes.get(&tool).copied().unwrap_or(tool.mode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_is_empty_config() {
        let tmp = tempdir().unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert_eq!(cfg.mode_for(Tool::Claude), Mode::Distribute);
        assert_eq!(cfg.mode_for(Tool::Hermes), Mode::SourceConsumer);
    }

    #[test]
    fn override_flips_codex_to_source_consumer() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.yaml"),
            "tool_modes:\n  codex: source-consumer\n",
        )
        .unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert_eq!(cfg.mode_for(Tool::Codex), Mode::SourceConsumer);
        // Others keep their defaults.
        assert_eq!(cfg.mode_for(Tool::Claude), Mode::Distribute);
        assert_eq!(cfg.mode_for(Tool::Openclaw), Mode::Distribute);
    }

    #[test]
    fn unknown_tool_name_errors() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.yaml"),
            "tool_modes:\n  cursor: distribute\n",
        )
        .unwrap();
        let err = Config::load(tmp.path()).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn unknown_mode_errors() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.yaml"),
            "tool_modes:\n  codex: bogus\n",
        )
        .unwrap();
        let err = Config::load(tmp.path()).unwrap_err();
        assert!(matches!(err, SkmError::FrontmatterInvalid { .. }));
    }

    #[test]
    fn empty_file_is_valid() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("config.yaml"), "").unwrap();
        let cfg = Config::load(tmp.path()).unwrap();
        assert_eq!(cfg.mode_for(Tool::Codex), Mode::Distribute);
    }
}
