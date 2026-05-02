//! Read-only CPIO backend (newc + odc).
//!
//! cpio archives are sequential by nature — there is no offset table — so we
//! materialize each file's bytes into memory on open. Suitable for typical
//! initramfs / RPM-payload sizes; very large cpios will hit the same memory
//! ceiling that compressed tars do.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use crate::core::{Entry, EntryKind, Error, Result, VPath};
use crate::vfs::trait_::{AsyncReader, Capabilities, Vfs};
use async_trait::async_trait;

use crate::vfs_archive::safe_archive_key;
use crate::vfs_archive::tar_vfs::AsyncSliceReader;

#[derive(Debug, Clone)]
struct CpioEntry {
    name: String,
    kind: EntryKind,
    size: u64,
    mode: Option<u32>,
    mtime: Option<std::time::SystemTime>,
    data: Option<Arc<[u8]>>,
}

pub struct CpioVfs {
    scheme: &'static str,
    entries: BTreeMap<String, CpioEntry>,
    children: BTreeMap<String, Vec<String>>,
}

impl CpioVfs {
    pub fn open(path: &Path, scheme: &'static str) -> Result<Self> {
        let f = std::fs::File::open(path).map_err(Error::Io)?;
        let mut reader = cpio_archive::reader(f).map_err(cpio_err)?;
        let mut entries = BTreeMap::new();
        loop {
            let hdr = match reader.read_next().map_err(cpio_err)? {
                Some(h) => h,
                None => break,
            };
            let raw_name = hdr.name().to_string();
            let Some(name) = safe_archive_key(&raw_name) else {
                tracing::warn!("cpio: skipping traversal entry {raw_name:?}");
                // Drain the data record so the reader stays aligned.
                let size = hdr.file_size();
                let mut sink = std::io::sink();
                let _ = std::io::copy(&mut Read::take(&mut reader, size), &mut sink);
                continue;
            };
            if name == "/" {
                // Skip "TRAILER!!!" or empty-name records.
                continue;
            }
            let mode_full = hdr.mode();
            let kind = kind_from_mode(mode_full);
            let size = hdr.file_size();
            let mtime = std::time::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(u64::from(hdr.mtime())));
            let data = if matches!(kind, EntryKind::File) && size > 0 {
                let cap = size.min(crate::vfs_archive::MAX_DECOMPRESSED) as usize;
                let mut buf = Vec::with_capacity(cap);
                Read::take(&mut reader, size)
                    .read_to_end(&mut buf)
                    .map_err(Error::Io)?;
                Some(Arc::<[u8]>::from(buf))
            } else {
                None
            };
            let leaf = name
                .rsplit_once('/')
                .map_or(name.as_str(), |(_, n)| n)
                .to_string();
            entries.insert(
                name,
                CpioEntry {
                    name: leaf,
                    kind,
                    size,
                    mode: Some(mode_full & 0o7777),
                    mtime,
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
        safe_archive_key(&layer.sub.to_string_lossy())
            .ok_or_else(|| Error::InvalidPath(format!("path traversal in {p}")))
    }
}

fn cpio_err(e: cpio_archive::Error) -> Error {
    Error::Vfs(format!("cpio: {e}"))
}

fn kind_from_mode(mode_full: u32) -> EntryKind {
    let file_type = mode_full & 0o170_000;
    match file_type {
        0o040_000 => EntryKind::Dir,
        0o120_000 => EntryKind::Symlink,
        0o100_000 => EntryKind::File,
        0o010_000 => EntryKind::Fifo,
        0o140_000 => EntryKind::Socket,
        0o060_000 => EntryKind::BlockDevice,
        0o020_000 => EntryKind::CharDevice,
        _ => EntryKind::Other,
    }
}

fn build_child_index(entries: &BTreeMap<String, CpioEntry>) -> BTreeMap<String, Vec<String>> {
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
impl Vfs for CpioVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT | Capabilities::RANDOM_READ
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        let key = self.key_for(p)?;
        if key == "/" {
            return Ok(root_entry());
        }
        if let Some(e) = self.entries.get(&key) {
            return Ok(cpio_to_entry(e));
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
                out.push(cpio_to_entry(e));
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

fn root_entry() -> Entry {
    Entry {
        name: "/".into(),
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

fn cpio_to_entry(e: &CpioEntry) -> Entry {
    Entry {
        name: e.name.clone(),
        kind: e.kind,
        size: e.size,
        mtime: e.mtime,
        atime: None,
        ctime: None,
        mode: e.mode,
        uid: None,
        gid: None,
        nlink: None,
        target: None,
    }
}
