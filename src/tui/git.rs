//! Git working-tree status lookup for panel rendering.
//!
//! Shells out to `git -C <cwd> status --porcelain=v1 -z --untracked-files=all`
//! when the user has enabled `[options] git_status = true` and the panel cwd
//! is inside a local git repo. The result maps top-level entry names (the
//! ones the panel actually displays) to a single-character glyph.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Single-character glyph summarizing the git status of one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitGlyph {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
    Conflict,
}

impl GitGlyph {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Ignored => "!",
            Self::Conflict => "U",
        }
    }
}

/// Resolve `<cwd>` to its git toplevel via `git rev-parse --show-toplevel`.
/// Returns `None` if the directory is not inside a repo or git isn't on PATH.
async fn toplevel(cwd: &Path) -> Option<String> {
    let out = timeout(
        Duration::from_millis(500),
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    Some(s.trim().to_string())
}

/// Build a map from panel-visible entry name (top-level child of `cwd`
/// inside the repo) to its summary glyph. Untracked files and explicit
/// per-file changes appear directly; deeper changes bubble up to the
/// top-level subdirectory's glyph (Modified by default).
pub async fn status_for_cwd(cwd: &Path) -> Option<HashMap<String, GitGlyph>> {
    let top = toplevel(cwd).await?;
    let top_path = Path::new(&top);
    let prefix = pathdiff::diff_paths(cwd, top_path).unwrap_or_default();
    let prefix_str = prefix.to_string_lossy().replace('\\', "/");
    let prefix_str = prefix_str.trim_matches('/');

    let out = timeout(
        Duration::from_secs(2),
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args([
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=matching",
            ])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }

    let mut map: HashMap<String, GitGlyph> = HashMap::new();
    let bytes = out.stdout;
    let mut i = 0;
    while i + 3 < bytes.len() {
        // porcelain v1 -z record:  XY SP path NUL  (renames have an extra NUL+orig)
        let xy = &bytes[i..i + 2];
        // skip XY + space
        let mut j = i + 3;
        while j < bytes.len() && bytes[j] != 0 {
            j += 1;
        }
        let path = String::from_utf8_lossy(&bytes[i + 3..j]).into_owned();
        let glyph = classify(xy[0], xy[1]);
        i = j + 1;
        // For 'R'/'C' there's an additional NUL-terminated original path.
        if xy[0] == b'R' || xy[0] == b'C' {
            while i < bytes.len() && bytes[i] != 0 {
                i += 1;
            }
            i += 1;
        }
        let key = top_relative_to_panel_entry(&path, prefix_str);
        if let Some(name) = key {
            // First writer wins; deeper changes promote to Modified for the
            // top-level dir, but explicit Conflict/Untracked beats it.
            map.entry(name)
                .and_modify(|g| {
                    if matches!(glyph, GitGlyph::Conflict) {
                        *g = glyph;
                    } else if matches!(g, GitGlyph::Untracked | GitGlyph::Ignored) {
                        // keep as-is
                    } else if !matches!(g, GitGlyph::Conflict) {
                        *g = GitGlyph::Modified;
                    }
                })
                .or_insert(glyph);
        }
    }
    Some(map)
}

fn classify(x: u8, y: u8) -> GitGlyph {
    if x == b'U' || y == b'U' || (x == b'A' && y == b'A') || (x == b'D' && y == b'D') {
        return GitGlyph::Conflict;
    }
    if x == b'?' && y == b'?' {
        return GitGlyph::Untracked;
    }
    if x == b'!' && y == b'!' {
        return GitGlyph::Ignored;
    }
    if x == b'R' || y == b'R' {
        return GitGlyph::Renamed;
    }
    if x == b'A' || y == b'A' {
        return GitGlyph::Added;
    }
    if x == b'D' || y == b'D' {
        return GitGlyph::Deleted;
    }
    GitGlyph::Modified
}

/// `path` is repo-relative (forward slashes). `prefix` is the
/// repo-relative path of the panel's cwd (or empty if cwd == toplevel).
/// Returns the panel-visible top-level entry name, or `None` if `path`
/// doesn't fall under `prefix`.
fn top_relative_to_panel_entry(path: &str, prefix: &str) -> Option<String> {
    let rel = if prefix.is_empty() {
        path
    } else {
        let p = path.strip_prefix(prefix)?;
        p.strip_prefix('/').unwrap_or(p)
    };
    if rel.is_empty() {
        return None;
    }
    Some(match rel.split_once('/') {
        Some((head, _)) => head.to_string(),
        None => rel.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_basic() {
        assert!(matches!(classify(b'?', b'?'), GitGlyph::Untracked));
        assert!(matches!(classify(b'M', b' '), GitGlyph::Modified));
        assert!(matches!(classify(b' ', b'M'), GitGlyph::Modified));
        assert!(matches!(classify(b'A', b' '), GitGlyph::Added));
        assert!(matches!(classify(b'U', b'U'), GitGlyph::Conflict));
    }

    #[test]
    fn top_relative() {
        assert_eq!(
            top_relative_to_panel_entry("src/foo.rs", ""),
            Some("src".into())
        );
        assert_eq!(
            top_relative_to_panel_entry("src/tui/mod.rs", "src"),
            Some("tui".into())
        );
        assert_eq!(
            top_relative_to_panel_entry("README", ""),
            Some("README".into())
        );
        assert_eq!(top_relative_to_panel_entry("other/file", "crates"), None);
    }
}
