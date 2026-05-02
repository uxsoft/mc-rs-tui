//! Shell-template macro substitution used by mc.ext.ini bindings, the user
//! menu, and the command line.
//!
//! Supported tokens (mc parity):
//!
//! * `%f` — current entry file name
//! * `%d` — active panel cwd (path display string)
//! * `%p` — full path to current entry (cwd + name)
//! * `%n` — current entry file name **without** extension
//! * `%x` — extension of current entry (no dot), or empty
//! * `%s` — list of marked entry names, space-separated; falls back to `%f`
//! * `%t` — alias for `%s`
//! * `%D` — *other* panel cwd
//! * `%F` — `%f` from the *other* panel
//! * `%%` — literal `%`
//!
//! Unknown `%X` tokens are passed through unchanged.

use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct MacroCtx {
    /// Active panel cwd as a display string (e.g. `/home/me`).
    pub cwd: String,
    /// Active entry filename (cursor row).
    pub current: String,
    /// Tagged entry names on active panel.
    pub marked: Vec<String>,
    /// Other panel cwd.
    pub other_cwd: String,
    /// Other panel cursor entry filename.
    pub other_current: String,
}

impl MacroCtx {
    #[must_use]
    pub fn current_full(&self) -> String {
        if self.current.is_empty() {
            self.cwd.clone()
        } else {
            join_path(&self.cwd, &self.current)
        }
    }

    #[must_use]
    pub fn marked_or_current(&self) -> Vec<String> {
        if self.marked.is_empty() {
            if self.current.is_empty() {
                Vec::new()
            } else {
                vec![self.current.clone()]
            }
        } else {
            self.marked.clone()
        }
    }
}

#[must_use]
pub fn substitute(template: &str, ctx: &MacroCtx) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('f') => out.push_str(&shell_quote(&ctx.current)),
            Some('d') => out.push_str(&shell_quote(&ctx.cwd)),
            Some('p') => out.push_str(&shell_quote(&ctx.current_full())),
            Some('n') => out.push_str(&shell_quote(file_stem(&ctx.current))),
            Some('x') => out.push_str(&shell_quote(file_ext(&ctx.current))),
            Some('s') | Some('t') => {
                let names = ctx.marked_or_current();
                let joined: Vec<String> = names.iter().map(|s| shell_quote(s)).collect();
                out.push_str(&joined.join(" "));
            }
            Some('D') => out.push_str(&shell_quote(&ctx.other_cwd)),
            Some('F') => out.push_str(&shell_quote(&ctx.other_current)),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn file_stem(name: &str) -> &str {
    Path::new(name)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(name)
}

fn file_ext(name: &str) -> &str {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_string();
    }
    if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// POSIX-shell-safe single-quote. `O'Brien` → `'O'\''Brien'`. Empty → `''`.
#[must_use]
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b','));
    if safe {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn ctx() -> MacroCtx {
        MacroCtx {
            cwd: "/tmp".into(),
            current: "foo.tar.gz".into(),
            marked: vec!["a.txt".into(), "b c.txt".into()],
            other_cwd: "/home".into(),
            other_current: "x".into(),
        }
    }

    #[test]
    fn basic_subs() {
        let c = ctx();
        assert_eq!(substitute("view %f", &c), "view foo.tar.gz");
        assert_eq!(substitute("ls %d", &c), "ls /tmp");
        assert_eq!(substitute("cat %p", &c), "cat /tmp/foo.tar.gz");
        assert_eq!(substitute("ext=%x", &c), "ext=gz");
        assert_eq!(substitute("stem=%n", &c), "stem=foo.tar");
    }

    #[test]
    fn marked_substitution_quotes() {
        let c = ctx();
        let out = substitute("rm %s", &c);
        // Spaces in "b c.txt" must be quoted.
        assert!(out.contains("a.txt"));
        assert!(out.contains("'b c.txt'"));
    }

    #[test]
    fn marked_falls_back_to_current() {
        let mut c = ctx();
        c.marked.clear();
        assert_eq!(substitute("rm %s", &c), "rm foo.tar.gz");
    }

    #[test]
    fn percent_literal() {
        let c = ctx();
        assert_eq!(substitute("100%% %f", &c), "100% foo.tar.gz");
    }

    #[test]
    fn unknown_token_pass_through() {
        let c = ctx();
        assert_eq!(substitute("a %Q b", &c), "a %Q b");
    }

    #[test]
    fn shell_quote_examples() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("foo.txt"), "foo.txt");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("O'Brien"), r#"'O'\''Brien'"#);
    }
}
