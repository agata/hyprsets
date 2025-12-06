mod config;
mod run;
mod state;
mod ui;

use anyhow::{Context, Result, anyhow};
use clap::{ArgAction, Parser, Subcommand, ValueHint};
use config::{AppConfig, Workset, default_config_path};
use run::run_workset as run_workset_launch;
use std::{
    path::{Path, PathBuf},
    process,
};
use ui::editor::{EditorExit, run_editor};
use ui::home::{HomeExit, run_home};

#[derive(Debug, Parser)]
#[command(
    name = "hyprsets",
    version,
    about = "HyprSets — Launch and edit Hyprland worksets",
    long_about = "No subcommand: open TUI list. run <id>: launch without TUI. edit <id>: jump into editor.",
    after_help = "Config is saved at ~/.config/hyprsets/hyprsets.toml (or the path provided via --config)."
)]
struct Cli {
    /// Path to config TOML. Defaults to ~/.config/hyprsets/hyprsets.toml
    #[arg(short, long, value_name = "PATH", global = true, value_hint = ValueHint::FilePath, help = "Path to config (default: ~/.config/hyprsets/hyprsets.toml)")]
    config: Option<PathBuf>,

    /// Increase log verbosity (-v, -vv)
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a workset directly without TUI
    Run { id: String },
    /// Open editor for a workset id
    Edit { id: String },
    /// Show the hyprsets version and exit
    Version,
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command.as_ref(), Some(Command::Version)) {
        print_version();
        return Ok(());
    }

    let config_path = cli.config.unwrap_or_else(default_config_path);
    let app_cfg = AppConfig::load_or_init(&config_path)
        .with_context(|| format!("failed to load config at {}", config_path.display()))?;

    let result = match cli.command {
        Some(Command::Run { id }) => run_workset(&app_cfg, &id, cli.verbose > 0, false),
        Some(Command::Edit { id }) => edit_workset(config_path.as_path(), &id),
        Some(Command::Version) => unreachable!("version is handled earlier"),
        None => run_interactive(app_cfg, config_path.as_path(), cli.verbose > 0),
    };

    match &result {
        Ok(_) => println!("hyprsets: exiting (normal)"),
        Err(err) => eprintln!("hyprsets: exiting with error: {err}"),
    }

    result
}

fn run_interactive(mut app_cfg: AppConfig, config_path: &Path, verbose: bool) -> Result<()> {
    let mut last_selected_id: Option<String> = None;
    loop {
        let initial_selection = last_selected_id.take();
        match run_home(app_cfg.clone(), config_path, initial_selection)? {
            HomeExit::Quit => return Ok(()),
            HomeExit::Run {
                id,
                preconfirm_clean,
            } => {
                let cfg = AppConfig::load_or_init(config_path)?;
                return run_workset(&cfg, &id, verbose, preconfirm_clean);
            }
            HomeExit::Edit(id) => {
                last_selected_id = Some(id.clone());
                edit_workset(config_path, &id)?;
                app_cfg = AppConfig::load_or_init(config_path)?;
            }
        }
    }
}

fn run_workset(cfg: &AppConfig, id: &str, verbose: bool, preconfirm_clean: bool) -> Result<()> {
    let ws = find_workset(cfg, id)?;
    run_workset_launch(ws, verbose, preconfirm_clean)
}

fn print_version() {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
}

fn edit_workset(config_path: &Path, id: &str) -> Result<()> {
    let cfg = AppConfig::load_or_init(config_path)?;
    let ws = cfg
        .worksets
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or_else(|| anyhow!("workset not found: {id}"))?;

    let EditorExit::Saved(updated) = run_editor(ws, config_path)?;
    println!("finished editing workset '{}'", updated.id);
    Ok(())
}

fn find_workset<'a>(cfg: &'a AppConfig, id: &str) -> Result<&'a Workset> {
    if let Some(ws) = cfg.worksets.iter().find(|w| w.id == id) {
        return Ok(ws);
    }

    let available = if cfg.worksets.is_empty() {
        "<none>".to_string()
    } else {
        cfg.worksets
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    Err(anyhow!("workset not found: {id}. available: {available}"))
}
