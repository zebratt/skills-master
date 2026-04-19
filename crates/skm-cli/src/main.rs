use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use skm_core::import;
use skm_core::init;
use skm_core::layout::Layout;
use skm_core::status;
use skm_core::sync;
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
        Cmd::Sync { dry_run } => run_sync(&layout, cli.json, dry_run),
        Cmd::Status => run_status(&layout, cli.json),
        Cmd::Import {
            path,
            r#as,
            strict,
            force,
        } => run_import(&layout, cli.json, &path, r#as.as_deref(), strict, force),
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

fn run_status(layout: &Layout, json: bool) -> Result<()> {
    let report = status::run_for_home(layout, None).context("skm status failed")?;
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        render_status_human(&report);
    }
    Ok(())
}

/// Human-readable matrix. One row per skill, columns for each tool.
///
/// Cell glyphs:
///   ✔ symlink   ◉ source-consumer   · not-requested
///   ✗ missing   ⚠ conflict          ✗ broken (dangling)
fn render_status_human(report: &status::StatusReport) {
    use skm_core::tools::Tool;
    if report.skills.is_empty() {
        println!("(no skills under skills_root)");
        return;
    }

    let name_col = report
        .skills
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    // Header.
    print!("{:<width$}  ", "SKILL", width = name_col);
    for tool in Tool::all() {
        print!("{:<10}", tool.name());
    }
    println!("ORIGIN");

    for row in &report.skills {
        print!("{:<width$}  ", row.name, width = name_col);
        if let Some(err) = &row.frontmatter_error {
            println!("ERROR  {err}");
            continue;
        }
        for tool in Tool::all() {
            let cell = row.tools.get(tool.name()).map(status_glyph).unwrap_or("·");
            print!("{:<10}", cell);
        }
        let origin = match row.origin {
            skm_core::state::Origin::User => "user",
            skm_core::state::Origin::Plugin => "plugin",
        };
        println!("{origin}");
    }
}

fn status_glyph(s: &status::ToolStatus) -> &'static str {
    match s {
        status::ToolStatus::Symlink => "✔ sym",
        status::ToolStatus::SourceConsumer => "◉ src",
        status::ToolStatus::Missing => "✗ miss",
        status::ToolStatus::Conflict => "⚠ conflict",
        status::ToolStatus::Broken => "✗ broken",
        status::ToolStatus::NotRequested => "·",
    }
}

fn run_sync(layout: &Layout, json: bool, dry_run: bool) -> Result<()> {
    let outcome = sync::run_for_home(layout, None, dry_run).context("skm sync failed")?;
    if json {
        println!("{}", serde_json::to_string(&outcome)?);
    } else {
        render_sync_human(&outcome);
    }
    Ok(())
}

fn render_sync_human(outcome: &sync::SyncOutcome) {
    if outcome.dry_run {
        println!("skm sync (dry-run):");
    } else {
        println!("skm sync:");
    }

    let mut created = 0usize;
    let mut already = 0usize;
    let mut consumed = 0usize;
    let mut removed = 0usize;
    let mut conflicts: Vec<&sync::SyncCell> = Vec::new();
    for cell in &outcome.cells {
        match &cell.action {
            sync::SyncAction::CreateSymlink { target, .. } => {
                created += 1;
                println!("  + {}/{} → {}", cell.skill, cell.tool, target.display());
            }
            sync::SyncAction::AlreadyCorrect => already += 1,
            sync::SyncAction::RecordSourceConsumer => {
                consumed += 1;
            }
            sync::SyncAction::RemovedStale { target } => {
                removed += 1;
                println!("  - {}/{} ({})", cell.skill, cell.tool, target.display());
            }
            sync::SyncAction::Conflict { .. } => conflicts.push(cell),
            sync::SyncAction::NotRequested => {}
        }
    }

    if !conflicts.is_empty() {
        println!("\nconflicts (not modified):");
        for cell in conflicts {
            if let sync::SyncAction::Conflict { target, reason } = &cell.action {
                println!(
                    "  ⚠ {}/{} at {}: {}",
                    cell.skill,
                    cell.tool,
                    target.display(),
                    reason
                );
            }
        }
    }

    for err in &outcome.parse_errors {
        println!("  ! {} frontmatter: {}", err.skill, err.error);
    }

    for name in &outcome.orphans_dropped {
        println!("  (dropped orphan state entry: {name})");
    }

    println!(
        "\ncreated: {created}  already-ok: {already}  source-consumer: {consumed}  removed: {removed}  conflicts: {}",
        outcome.cells.iter().filter(|c| matches!(c.action, sync::SyncAction::Conflict { .. })).count()
    );
}

fn run_import(
    layout: &Layout,
    json: bool,
    path: &str,
    as_name: Option<&str>,
    strict: bool,
    force: bool,
) -> Result<()> {
    let outcome = import::run(
        layout,
        std::path::Path::new(path),
        as_name,
        import::ImportOptions { strict, force },
    )
    .context("skm import failed")?;
    if json {
        println!("{}", serde_json::to_string(&outcome)?);
    } else {
        println!(
            "skm: imported {} → {}",
            outcome.name,
            outcome.target.display()
        );
        if !outcome.warnings.is_empty() {
            println!("warnings:");
            for w in &outcome.warnings {
                println!("  ⚠ {w}");
            }
        }
        println!(
            "origin: marketplace={} path={}",
            outcome.origin_marketplace, outcome.origin_path
        );
        println!("next: run `skm sync` to distribute to tools.");
    }
    Ok(())
}
