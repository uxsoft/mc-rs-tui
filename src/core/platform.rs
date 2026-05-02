//! Cross-platform helpers for environment lookups that differ between
//! Unix (HOME, /bin/sh) and Windows (USERPROFILE, cmd.exe).

use std::path::PathBuf;

/// Best-effort home directory: `HOME` on Unix, falls back to `USERPROFILE`
/// on Windows. Returns `None` if neither is set.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Resolve the shell to use for `<shell> <args> <command>` invocations.
/// Returns `(program, args_before_command)`.
///
/// Unix: `$SHELL` if set, otherwise `/bin/sh`; args are `["-c"]`.
/// Windows: `$SHELL` if set (Git Bash etc.) with no args; otherwise
/// `$COMSPEC` or `cmd.exe` with `["/C"]`.
#[must_use]
pub fn default_shell() -> (String, &'static [&'static str]) {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() {
            // If the user set $SHELL on Windows it's almost certainly a
            // POSIX-flavored shell (Git Bash, MSYS, WSL); pass `-c`.
            return (s, &["-c"]);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(s) = std::env::var("COMSPEC") {
            if !s.is_empty() {
                return (s, &["/C"]);
            }
        }
        ("cmd.exe".to_string(), &["/C"])
    }
    #[cfg(not(windows))]
    {
        ("/bin/sh".to_string(), &["-c"])
    }
}
