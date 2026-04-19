use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Openclaw,
    Hermes,
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

    pub fn distribution(self) -> Distribution {
        match self {
            Tool::Claude => Distribution::Symlink { dir: "~/.claude/skills" },
            Tool::Codex => Distribution::Symlink { dir: "~/.codex/skills" },
            Tool::Openclaw => Distribution::Symlink { dir: "~/.openclaw/skills" },
            Tool::Hermes => Distribution::SourceConsumer { source: "~/.agents/skills" },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Distribution {
    Symlink { dir: &'static str },
    SourceConsumer { source: &'static str },
}
