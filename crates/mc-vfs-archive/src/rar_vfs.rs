//! Read-only RAR backend (feature-gated; requires `unrar`'s restrictive
//! upstream licence — opt-in via `--features rar`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath};
use mc_vfs::trait_::{AsyncReader, Capabilities, Vfs};

use crate::tar_vfs::AsyncSliceReader;

#[derive(Debug, Clone)]
struct RarEntry {
    name: String,
    kind: EntryKind,
    size: u64,
    data: Option<Arc<[u8]>>,
}

pub struct RarVfs {
    scheme: &'static str,
    entries: BTreeMap<String, RarEntry>,
    children: BTreeMap<String, Vec<String>>,
}

impl RarVfs {
    pub fn open(path: &Path, scheme: &'static str) -> Result<Self> {
        let mut entries: BTreeMap<String, RarEntry> = BTreeMap::new();

        // Process mode lets us call .read() on each header to extract bytes.
        let mut archive = unrar::Archive::new(path)
            .open_for_processing()
            .map_err(|e| Error::Vfs(format!("rar open: {e}")))?;
        loop {
            let header = match archive
                .read_header()
                .map_err(|e| Error::Vfs(format!("rar header: {e}")))?
            {
                Some(h) => h,
                None => break,
            };
            let name = header.entry().filename.to_string_lossy().to_string();
            let key = normalize_key(&name);
            let is_dir = header.entry().is_directory();
            let size = header.entry().unpacked_size as u64;
            let kind = if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            // Read body (or skip if dir/zero-size).
            let (data, next) = if !is_dir && size > 0 {
                let (bytes, next) = header
                    .read()
                    .map_err(|e| Error::Vfs(format!("rar read: {e}")))?;
                (Some(Arc::<[u8]>::from(bytes)), next)
            } else {
                let next = header
                    .skip()
                    .map_err(|e| Error::Vfs(format!("rar skip: {e}")))?;
                (None, next)
            };
            archive = next;
            let leaf = key
                .rsplit_once('/')
                .map_or(key.as_str(), |(_, n)| n)
                .to_string();
            entries.insert(
                key,
                RarEntry {
                    name: leaf,
                    kind,
                    size,
                    data,
                },
            );
        }
        let children = build_child_index(&entries);
        Ok(Self {
            scheme,
            entries,
            children,
        })
    }

    fn key_for(&self, p: &VPath) -> Result<String> {
        let layer = p
            .layers()
            .iter()
            .rev()
            .find(|l| l.scheme == self.scheme)
            .ok_or_else(|| Error::InvalidPath(format!("vpath has no {} layer", self.scheme)))?;
        Ok(normalize_key(&layer.sub.to_string_lossy()))
    }
}

fn normalize_key(s: &str) -> String {
    let trimmed = s.trim_end_matches('/').replace('\\', "/");
    if trimmed.is_empty() {
        return "/".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed
    } else {
        format!("/{trimmed}")
    }
}

fn build_child_index(entries: &BTreeMap<String, RarEntry>) -> BTreeMap<String, Vec<String>> {
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    children.insert("/".into(), Vec::new());
    let mut to_add: Vec<String> = Vec::new();
    for key in entries.keys() {
        let mut cur = key.as_str();
        while let Some((parent, _)) = cur.rsplit_once('/') {
            let p = if parent.is_empty() { "/" } else { parent };
            if p != "/" && !entries.contains_key(p) {
                to_add.push(p.to_string());
            }
            cur = p;
            if cur == "/" {
                break;
            }
        }
    }
    let mut all_keys: Vec<String> = entries.keys().cloned().collect();
    all_keys.extend(to_add);
    all_keys.sort();
    all_keys.dedup();
    for key in &all_keys {
        if key == "/" {
            continue;
        }
        let (parent, name) = key.rsplit_once('/').unwrap_or(("", key.as_str()));
        let parent = if parent.is_empty() { "/" } else { parent };
        children
            .entry(parent.to_string())
            .or_default()
            .push(name.to_string());
    }
    for v in children.values_mut() {
        v.sort();
        v.dedup();
    }
    children
}

#[async_trait]
impl Vfs for RarVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT | Capabilities::RANDOM_READ
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        let key = self.key_for(p)?;
        if key == "/" {
            return Ok(synthetic_dir("/"));
        }
        if let Some(e) = self.entries.get(&key) {
            return Ok(entry_to_core(e));
        }
        if self.children.contains_key(&key) {
            return Ok(synthetic_dir(&key));
        }
        Err(Error::Vfs(format!("not found: {key}")))
    }

    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>> {
        let key = self.key_for(p)?;
        let kids = self
            .children
            .get(&key)
            .ok_or_else(|| Error::Vfs(format!("not a directory: {key}")))?;
        let mut out = Vec::with_capacity(kids.len());
        for name in kids {
            let child_key = if key == "/" {
                format!("/{name}")
            } else {
                format!("{key}/{name}")
            };
            if let Some(e) = self.entries.get(&child_key) {
                out.push(entry_to_core(e));
            } else {
                out.push(synthetic_dir(&child_key));
            }
        }
        Ok(out)
    }

    async fn open_read(&self, p: &VPath) -> Result<AsyncReader> {
        let key = self.key_for(p)?;
        let e = self
            .entries
            .get(&key)
            .ok_or_else(|| Error::Vfs(format!("not found: {key}")))?;
        if !matches!(e.kind, EntryKind::File) {
            return Err(Error::Vfs(format!("not a regular file: {key}")));
        }
        let bytes = e
            .data
            .clone()
            .ok_or_else(|| Error::Vfs(format!("no data for {key}")))?;
        Ok(Box::new(AsyncSliceReader::new(bytes)))
    }
}

fn synthetic_dir(key: &str) -> Entry {
    Entry {
        name: key.rsplit('/').next().unwrap_or("").to_string(),
        kind: EntryKind::Dir,
        size: 0,
        mtime: None,
        atime: None,
        ctime: None,
        mode: None,
        uid: None,
        gid: None,
        nlink: None,
        target: None,
    }
}

fn entry_to_core(e: &RarEntry) -> Entry {
    Entry {
        name: e.name.clone(),
        kind: e.kind,
        size: e.size,
        mtime: None,
        atime: None,
        ctime: None,
        mode: None,
        uid: None,
        gid: None,
        nlink: None,
        target: None,
    }
}
