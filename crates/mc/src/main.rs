use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use mc_config::AppConfig;
use mc_tui::App;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "mc-rs", version, about = "Rust + Ratatui port of GNU Midnight Commander")]
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
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
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

    runtime.block_on(async move {
        let config = AppConfig::default();
        let app = App::new(config, start);
        mc_tui::run(app).await
    })?;

    Ok(())
}
