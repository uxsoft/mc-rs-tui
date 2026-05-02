//! Suspend the TUI, spawn an external editor, restore on return.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Determine the editor command. Honors `$EDITOR`, falls back to `hx`, then `vi`.
#[must_use]
pub fn resolve_editor(configured: Option<&str>) -> String {
    if let Some(c) = configured {
        if !c.is_empty() {
            return c.to_owned();
        }
    }
    if let Ok(env) = std::env::var("EDITOR") {
        if !env.is_empty() {
            return env;
        }
    }
    if which::which("hx").is_ok() {
        return "hx".to_owned();
    }
    if cfg!(windows) {
        return "notepad.exe".to_owned();
    }
    "vi".to_owned()
}

/// Suspend crossterm raw/alt-screen state, spawn the editor synchronously, restore.
pub fn spawn_editor(editor: &str, file: &Path, line: Option<u32>) -> Result<ExitStatus> {
    suspend_terminal().context("suspend terminal")?;
    let result = run_editor(editor, file, line);
    let resume = resume_terminal();
    let status = result?;
    resume.context("resume terminal")?;
    Ok(status)
}

fn run_editor(editor: &str, file: &Path, line: Option<u32>) -> Result<ExitStatus> {
    // Split editor on whitespace so users can pass flags via $EDITOR (e.g. "code -w").
    let mut parts = editor.split_whitespace();
    let bin = parts.next().context("empty editor command")?;
    let mut cmd = Command::new(bin);
    for arg in parts {
        cmd.arg(arg);
    }
    if let Some(n) = line {
        cmd.arg(format!("+{n}"));
    }
    cmd.arg(file);
    let status = cmd.status().with_context(|| format!("spawn {editor}"))?;
    Ok(status)
}

fn suspend_terminal() -> io::Result<()> {
    let mut out = io::stdout();
    execute!(out, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn resume_terminal() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_configured_first() {
        assert_eq!(resolve_editor(Some("nano")), "nano");
    }
}
