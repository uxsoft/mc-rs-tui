//! Recursive filename + content search across any [`mc_vfs::Vfs`].
//!
//! The [`run`] function spawns a tokio task that traverses `start` and emits
//! [`Match`] values onto an mpsc channel. Cooperative cancellation via a
//! [`tokio_util::sync::CancellationToken`].

use std::sync::Arc;

use mc_core::{Entry, EntryKind, VPath};
use mc_vfs::Vfs;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct Query {
    pub start: VPath,
    /// Glob pattern matched against entry filenames (case-insensitive).
    pub name_glob: Option<globset::GlobMatcher>,
    /// Substring or regex (depending on `content_regex`) searched inside files.
    pub content: Option<ContentQuery>,
    /// Substring case-insensitive matches on names of dirs to skip.
    pub ignore_dirs: Vec<String>,
    /// Maximum number of matches before stopping the search.
    pub max_matches: usize,
}

#[derive(Debug, Clone)]
pub struct ContentQuery {
    pub regex: regex::bytes::Regex,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub path: VPath,
    /// 1-based line number of the first content match, if any.
    pub line: Option<u32>,
}

#[derive(Debug)]
pub enum FindEvent {
    Scanned(VPath),
    Matched(Match),
    Done,
}

pub fn run(
    vfs: Arc<dyn Vfs>,
    query: Query,
    cancel: CancellationToken,
) -> mpsc::Receiver<FindEvent> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        if let Err(e) = walk(&*vfs, &query, &tx, &cancel, 0).await {
            tracing::warn!("find: {e}");
        }
        let _ = tx.send(FindEvent::Done).await;
    });
    rx
}

async fn walk(
    vfs: &dyn Vfs,
    q: &Query,
    tx: &mpsc::Sender<FindEvent>,
    cancel: &CancellationToken,
    matches_emitted_so_far: usize,
) -> mc_core::Result<usize> {
    if cancel.is_cancelled() {
        return Ok(matches_emitted_so_far);
    }
    let mut count = matches_emitted_so_far;
    let entries = vfs.read_dir(&q.start).await?;
    for entry in entries {
        if cancel.is_cancelled() || count >= q.max_matches {
            break;
        }
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let child = match child_of(&q.start, &entry.name) {
            Some(c) => c,
            None => continue,
        };
        let _ = tx.send(FindEvent::Scanned(child.clone())).await;

        if entry.is_dir() {
            // Apply ignore-dirs filter.
            let n_lower = entry.name.to_ascii_lowercase();
            if q
                .ignore_dirs
                .iter()
                .any(|d| n_lower == d.to_ascii_lowercase())
            {
                continue;
            }
            let sub_q = Query {
                start: child.clone(),
                name_glob: q.name_glob.clone(),
                content: q.content.clone(),
                ignore_dirs: q.ignore_dirs.clone(),
                max_matches: q.max_matches,
            };
            count = Box::pin(walk(vfs, &sub_q, tx, cancel, count)).await?;
            continue;
        }

        // Filename filter.
        if let Some(g) = &q.name_glob {
            if !g.is_match(&entry.name) {
                continue;
            }
        }
        // Content filter.
        let line = if let Some(cq) = &q.content {
            match scan_content(vfs, &child, &entry, cq, cancel).await? {
                Some(n) => Some(n),
                None => continue,
            }
        } else {
            None
        };
        if tx
            .send(FindEvent::Matched(Match {
                path: child,
                line,
            }))
            .await
            .is_err()
        {
            break;
        }
        count += 1;
    }
    Ok(count)
}

async fn scan_content(
    vfs: &dyn Vfs,
    p: &VPath,
    entry: &Entry,
    q: &ContentQuery,
    cancel: &CancellationToken,
) -> mc_core::Result<Option<u32>> {
    if !matches!(entry.kind, EntryKind::File) {
        return Ok(None);
    }
    // Cap to keep memory bounded; large files just won't match content but are
    // still listed when only filename matches.
    const MAX: usize = 16 * 1024 * 1024;
    let mut reader = vfs.open_read(p).await?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = vec![0u8; 64 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        if buf.len() + n > MAX {
            buf.extend_from_slice(&tmp[..MAX - buf.len()]);
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    if let Some(m) = q.regex.find(&buf) {
        // Compute 1-based line number from byte offset.
        let prefix = &buf[..m.start()];
        let line = 1 + prefix.iter().filter(|&&b| b == b'\n').count() as u32;
        Ok(Some(line))
    } else {
        Ok(None)
    }
}

fn child_of(parent: &VPath, name: &str) -> Option<VPath> {
    let layer = parent.last().cloned()?;
    let mut new_layer = layer;
    new_layer.sub.push(name);
    let mut new = parent.clone();
    new.pop_layer();
    new.push_layer(new_layer);
    Some(new)
}

/// Build a case-insensitive [`globset::GlobMatcher`] from a user pattern.
pub fn build_name_glob(pattern: &str) -> Result<globset::GlobMatcher, globset::Error> {
    let mut b = globset::GlobBuilder::new(pattern);
    b.case_insensitive(true);
    let g = b.build()?;
    Ok(g.compile_matcher())
}

/// Build a content regex from a user pattern. `whole_word` wraps the pattern
/// in `\b…\b`; `case_insensitive` flips the regex flag.
pub fn build_content_regex(
    pattern: &str,
    whole_word: bool,
    case_insensitive: bool,
) -> Result<regex::bytes::Regex, regex::Error> {
    let mut p = String::new();
    if case_insensitive {
        p.push_str("(?i)");
    }
    if whole_word {
        p.push_str(r"\b");
    }
    p.push_str(pattern);
    if whole_word {
        p.push_str(r"\b");
    }
    regex::bytes::Regex::new(&p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_vfs::local::LocalVfs;
    use std::collections::HashSet;
    use std::fs;

    fn populate(root: &std::path::Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("src/lib.rs"), b"fn main() { println!(\"hi\"); }").unwrap();
        fs::write(root.join("src/util.rs"), b"// utility helpers\n").unwrap();
        fs::write(root.join("README.md"), b"# project\n").unwrap();
        fs::write(root.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    }

    async fn collect(mut rx: mpsc::Receiver<FindEvent>) -> Vec<VPath> {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                FindEvent::Matched(m) => out.push(m.path),
                FindEvent::Done => break,
                _ => {}
            }
        }
        out
    }

    #[tokio::test]
    async fn finds_by_extension_skipping_ignored_dir() {
        let td = tempfile::tempdir().unwrap();
        populate(td.path());
        let vfs = LocalVfs::shared();
        let q = Query {
            start: VPath::local(td.path().to_path_buf()),
            name_glob: Some(build_name_glob("*.rs").unwrap()),
            content: None,
            ignore_dirs: vec![".git".into()],
            max_matches: 100,
        };
        let cancel = CancellationToken::new();
        let rx = run(vfs, q, cancel);
        let names: HashSet<String> = collect(rx)
            .await
            .into_iter()
            .map(|p| p.to_string())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("src/lib.rs")));
        assert!(names.iter().any(|n| n.ends_with("src/util.rs")));
        assert!(!names.iter().any(|n| n.contains(".git")));
    }

    #[tokio::test]
    async fn content_filter_matches_substring() {
        let td = tempfile::tempdir().unwrap();
        populate(td.path());
        let vfs = LocalVfs::shared();
        let q = Query {
            start: VPath::local(td.path().to_path_buf()),
            name_glob: None,
            content: Some(ContentQuery {
                regex: build_content_regex("println", false, false).unwrap(),
            }),
            ignore_dirs: vec![".git".into()],
            max_matches: 100,
        };
        let rx = run(vfs, q, CancellationToken::new());
        let paths = collect(rx).await;
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string().ends_with("src/lib.rs"));
    }

    #[tokio::test]
    async fn cancel_stops_promptly() {
        let td = tempfile::tempdir().unwrap();
        for i in 0..200 {
            std::fs::write(td.path().join(format!("f{i:03}.txt")), b"x").unwrap();
        }
        let vfs = LocalVfs::shared();
        let cancel = CancellationToken::new();
        let q = Query {
            start: VPath::local(td.path().to_path_buf()),
            name_glob: Some(build_name_glob("*.txt").unwrap()),
            content: None,
            ignore_dirs: vec![],
            max_matches: 1000,
        };
        cancel.cancel();
        let rx = run(vfs, q, cancel);
        let paths = collect(rx).await;
        // Cancelled before scan started — no matches.
        assert!(paths.len() < 200);
    }
}
