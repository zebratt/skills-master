use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use skm_core::init;
use skm_core::layout::Layout;
use std::path::PathBuf;

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
    let layout = resolve_layout()?;

    match cli.cmd {
        Cmd::Init => run_init(&layout, cli.json),
        Cmd::Sync { dry_run } => {
            anyhow::bail!("sync: not yet implemented (Phase 1, dry_run={dry_run})")
        }
        Cmd::Status => {
            anyhow::bail!("status: not yet implemented (Phase 1, json={})", cli.json)
        }
        Cmd::Import {
            path,
            r#as,
            strict,
            force,
        } => {
            anyhow::bail!(
                "import: not yet implemented (Phase 1, path={path}, as={:?}, strict={strict}, force={force})",
                r#as
            )
        }
    }
}

/// Build the `Layout` from the environment.
///
/// Precedence:
///   1. `SKM_ROOT` env var — absolute override. Used by integration tests and
///      power users who want `skm` to manage a non-default directory.
///   2. `$HOME/.agents` — default production layout.
fn resolve_layout() -> Result<Layout> {
    if let Some(root) = std::env::var_os("SKM_ROOT") {
        return Ok(Layout::new(PathBuf::from(root)));
    }
    let home = std::env::var_os("HOME").context("$HOME is not set; cannot locate ~/.agents")?;
    Ok(Layout::default_for_home(&PathBuf::from(home)))
}

fn run_init(layout: &Layout, json: bool) -> Result<()> {
    let status = init::run(layout).context("skm init failed")?;
    if json {
        let key = match status {
            init::Status::Created => "created",
            init::Status::AlreadyInitialized => "already-initialized",
        };
        let payload = serde_json::json!({
            "status": key,
            "root": layout.root(),
            "skillsRoot": layout.skills_root(),
            "managerDir": layout.manager_dir(),
        });
        println!("{}", serde_json::to_string(&payload)?);
    } else {
        match status {
            init::Status::Created => {
                println!("skm: initialized {}", layout.root().display());
            }
            init::Status::AlreadyInitialized => {
                println!("skm: already initialized at {}", layout.root().display());
            }
        }
    }
    Ok(())
}
