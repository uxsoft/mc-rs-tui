//! Subshell drop-to-shell with cwd-sync.
//!
//! Suspends the TUI, runs `$SHELL` interactively in `cwd`, and on shell exit
//! reads the directory the user was in (captured via a small per-shell hook
//! that writes `$PWD` on exit). The panel cwd is then re-pointed at that path.
//!
//! Supported shells: `bash`, `zsh`, `fish`, plain `sh`. Other shells fall back
//! to the no-sync drop.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Sh,
}

impl ShellKind {
    fn from_path(path: &str) -> Self {
        let leaf = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        match leaf {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "fish" => Self::Fish,
            _ => Self::Sh,
        }
    }
}

/// Run the subshell. Returns the directory the user ended up in on exit (when
/// we could determine it), or `None` on error or unsupported-shell paths.
pub fn drop_to_shell_with_sync(cwd: &Path) -> Result<Option<PathBuf>> {
    let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let kind = ShellKind::from_path(&shell_path);

    // Per-process tempfile path for the cwd dump.
    let pwd_file = std::env::temp_dir().join(format!(
        "mc-rs-pwd-{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&pwd_file);

    // Build a per-shell hook file that sources the user's startup, then sets
    // an EXIT trap (or fish equivalent) that writes $PWD into our tempfile.
    let hook = match kind {
        ShellKind::Bash | ShellKind::Sh => {
            let body = format!(
                "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n\
                 trap 'pwd > {pwd}' EXIT\n",
                pwd = shell_quote(&pwd_file.display().to_string()),
            );
            Some(write_hook("bashrc", &body)?)
        }
        ShellKind::Zsh => {
            let body = format!(
                "[ -f \"$HOME/.zshrc\" ] && . \"$HOME/.zshrc\"\n\
                 TRAPEXIT() {{ pwd > {pwd}; }}\n",
                pwd = shell_quote(&pwd_file.display().to_string()),
            );
            Some(write_hook("zshrc", &body)?)
        }
        ShellKind::Fish => {
            let body = format!(
                "function __mc_rs_on_exit --on-event fish_exit\n  pwd > {pwd}\nend\n",
                pwd = shell_quote(&pwd_file.display().to_string()),
            );
            Some(write_hook("fish_init", &body)?)
        }
    };

    // Suspend our TUI.
    execute!(io::stdout(), LeaveAlternateScreen).context("leave alt-screen")?;
    disable_raw_mode().context("disable raw mode")?;

    eprintln!(
        "[mc-rs] drop to {shell_path} (Ctrl-D / exit returns to mc-rs; cwd will sync)"
    );

    // Build the command. For bash we use `--rcfile` to inject the hook; for
    // zsh we set $ZDOTDIR to a directory containing only our `.zshrc`; for
    // fish we use `--init-command`.
    let status = match (kind, hook.as_deref()) {
        (ShellKind::Bash, Some(rc)) | (ShellKind::Sh, Some(rc)) => Command::new(&shell_path)
            .arg("--rcfile")
            .arg(rc)
            .arg("-i")
            .current_dir(cwd)
            .status(),
        (ShellKind::Zsh, Some(rc)) => {
            // ZDOTDIR must point to a directory; `rc` already lives in a per-
            // shell tempdir whose layout is `<dir>/.zshrc`.
            let zdotdir = rc.parent().unwrap_or_else(|| Path::new("."));
            Command::new(&shell_path)
                .env("ZDOTDIR", zdotdir)
                .current_dir(cwd)
                .status()
        }
        (ShellKind::Fish, Some(rc)) => Command::new(&shell_path)
            .arg("--init-command")
            .arg(format!("source {}", shell_quote(&rc.display().to_string())))
            .current_dir(cwd)
            .status(),
        _ => Command::new(&shell_path).current_dir(cwd).status(),
    };
    if let Err(e) = status {
        tracing::warn!("subshell spawn: {e}");
    }

    enable_raw_mode().context("enable raw mode")?;
    execute!(io::stdout(), EnterAlternateScreen).context("enter alt-screen")?;

    // Read the captured PWD (if any).
    let new_cwd = std::fs::read_to_string(&pwd_file)
        .ok()
        .map(|s| PathBuf::from(s.trim_end_matches(['\r', '\n']).to_string()));
    let _ = std::fs::remove_file(&pwd_file);
    Ok(new_cwd)
}

fn write_hook(name: &str, body: &str) -> io::Result<PathBuf> {
    // Each invocation gets a fresh tempdir to keep ZDOTDIR clean.
    let dir = std::env::temp_dir().join(format!(
        "mc-rs-shell-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(match name {
        "zshrc" => ".zshrc",
        _ => name,
    });
    std::fs::write(&path, body)?;
    Ok(path)
}

fn shell_quote(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/')) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 4);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_detect() {
        assert_eq!(ShellKind::from_path("/usr/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_path("/usr/local/bin/zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::from_path("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(ShellKind::from_path("/bin/sh"), ShellKind::Sh);
        assert_eq!(ShellKind::from_path("/opt/anything"), ShellKind::Sh);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/tmp/foo"), "/tmp/foo");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("O'Brien"), r#"'O'\''Brien'"#);
    }
}
