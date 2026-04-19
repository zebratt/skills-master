use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "skm",
    about = "SkillsMaster — cross-tool AI skills distribution & upgrade manager",
    version,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Emit machine-readable JSON output where applicable.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Initialize `~/.agents/skills/` and `~/.agents/skills-manager/`.
    Init,

    /// Distribute skills to tool directories based on frontmatter `tools` field.
    Sync {
        /// Show what would happen without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show skill × tool distribution matrix.
    Status,

    /// Promote a plugin skill into `~/.agents/skills/`.
    Import {
        /// Path to the plugin skill directory.
        path: String,

        /// Rename on import. Rewrites both dirname and frontmatter.name atomically.
        #[arg(long)]
        r#as: Option<String>,

        /// Abort on any reference that escapes the skill directory.
        #[arg(long)]
        strict: bool,

        /// Skip self-containment check entirely. User accepts risk.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Init => {
            anyhow::bail!("init: not yet implemented (Phase 1)")
        }
        Cmd::Sync { dry_run } => {
            anyhow::bail!("sync: not yet implemented (Phase 1, dry_run={dry_run})")
        }
        Cmd::Status => {
            anyhow::bail!("status: not yet implemented (Phase 1, json={})", cli.json)
        }
        Cmd::Import { path, r#as, strict, force } => {
            anyhow::bail!(
                "import: not yet implemented (Phase 1, path={path}, as={:?}, strict={strict}, force={force})",
                r#as
            )
        }
    }
}
