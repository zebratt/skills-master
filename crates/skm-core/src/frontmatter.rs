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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub tools: Tools,
    #[serde(default)]
    pub upstream: Option<Upstream>,
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

// TODO Phase 1: implement parse_skill_md(path) -> SkmResult<Frontmatter>
// that splits YAML frontmatter block from body and deserializes it.
