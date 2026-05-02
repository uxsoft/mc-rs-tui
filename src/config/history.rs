//! Tiny persistent ring-buffer history (cmdline + find patterns).
//!
//! Stored as a plain text file, one entry per line, oldest → newest. The
//! `push()` method de-duplicates the most-recent entry to mirror shell
//! behaviour, and trims to `max` entries on save.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct History {
    path: PathBuf,
    items: VecDeque<String>,
    max: usize,
}

impl History {
    /// Load from `path`; missing file → empty history.
    #[must_use]
    pub fn load(path: PathBuf, max: usize) -> Self {
        let items = read_file(&path).unwrap_or_default();
        let mut h = Self {
            path,
            items: items.into(),
            max,
        };
        h.trim();
        h
    }

    pub fn push(&mut self, entry: String) {
        let entry = entry.trim().to_string();
        if entry.is_empty() {
            return;
        }
        if self.items.back() == Some(&entry) {
            return;
        }
        self.items.push_back(entry);
        self.trim();
        if let Err(e) = self.save() {
            tracing::warn!("history save {}: {e}", self.path.display());
        }
    }

    fn trim(&mut self) {
        while self.items.len() > self.max {
            self.items.pop_front();
        }
    }

    fn save(&self) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(self.items.iter().map(|s| s.len() + 1).sum());
        for line in &self.items {
            writeln!(buf, "{line}")?;
        }
        crate::config::io::write_user_file_atomic(&self.path, &buf)
    }

    #[must_use]
    pub fn entries(&self) -> &VecDeque<String> {
        &self.items
    }

    /// Get the entry `n` steps back from the most-recent (n=0 → newest).
    #[must_use]
    pub fn nth_back(&self, n: usize) -> Option<&str> {
        let len = self.items.len();
        if n >= len {
            return None;
        }
        self.items.get(len - 1 - n).map(String::as_str)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn read_file(path: &Path) -> std::io::Result<Vec<String>> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            out.push(line);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_recall() {
        let td = tempfile::tempdir().unwrap();
        let mut h = History::load(td.path().join("hist"), 5);
        h.push("ls".into());
        h.push("pwd".into());
        h.push("ls".into()); // not adjacent; counted
        assert_eq!(h.len(), 3);
        assert_eq!(h.nth_back(0), Some("ls"));
        assert_eq!(h.nth_back(1), Some("pwd"));
        assert_eq!(h.nth_back(2), Some("ls"));
        assert!(h.nth_back(3).is_none());
    }

    #[test]
    fn dedup_adjacent() {
        let td = tempfile::tempdir().unwrap();
        let mut h = History::load(td.path().join("hist"), 5);
        h.push("ls".into());
        h.push("ls".into());
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn trims_to_max() {
        let td = tempfile::tempdir().unwrap();
        let mut h = History::load(td.path().join("hist"), 3);
        for i in 0..10 {
            h.push(format!("c{i}"));
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.nth_back(0), Some("c9"));
        assert_eq!(h.nth_back(2), Some("c7"));
    }

    #[test]
    fn round_trip() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("hist");
        let mut h = History::load(p.clone(), 10);
        h.push("a".into());
        h.push("b".into());
        let h2 = History::load(p, 10);
        assert_eq!(
            h2.entries().iter().cloned().collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}
