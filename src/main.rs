use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use mc_rs::config::{AppConfig, ConfigPaths};
use mc_rs::tui::{self, App};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "mc-rs",
    version,
    about = "Rust + Ratatui port of GNU Midnight Commander"
)]
struct Args {
    /// Initial directory for both panels.
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,
    /// Skin name or path.
    #[arg(short = 'S', long)]
    skin: Option<String>,
    /// Keymap file path.
    #[arg(short = 'K', long)]
    keymap: Option<PathBuf>,
    /// Print final cwd to this file on exit.
    #[arg(short = 'P', long, value_name = "FILE")]
    print_cwd: Option<PathBuf>,
}

fn main() -> Result<()> {
    color_eyre::install().ok();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let args = Args::parse();
    let start = match args.dir {
        Some(d) => d,
        None => std::env::current_dir().context("current_dir")?,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;

    let print_cwd = args.print_cwd.clone();
    let exit_info = runtime.block_on(async move {
        let paths = ConfigPaths::discover();
        let config = AppConfig::load(&paths.main_config()).unwrap_or_else(|e| {
            tracing::warn!("config load: {e}; using defaults");
            AppConfig::default()
        });
        let (app, job_rx) = App::new(config, start);
        tui::run(app, job_rx).await
    })?;

    if let (Some(file), Some(cwd)) = (print_cwd, exit_info.final_cwd) {
        if let Err(e) = std::fs::write(&file, format!("{}\n", cwd.display())) {
            tracing::warn!("write {}: {e}", file.display());
        }
    }

    Ok(())
}
