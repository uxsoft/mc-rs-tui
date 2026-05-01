//! Read-only ZIP backend.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::safe_archive_key;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(path: &Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.add_directory("dir/", opts).unwrap();
        w.start_file("dir/hello.txt", opts).unwrap();
        w.write_all(b"zhello").unwrap();
        w.finish().unwrap();
    }

    fn make_evil_zip(path: &Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file("../../etc/passwd", opts).unwrap();
        w.write_all(b"pwn").unwrap();
        w.finish().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zip_rejects_traversal() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("evil.zip");
        make_evil_zip(&p);
        let vfs = ZipVfs::open(&p, "zip").unwrap();
        let root = VPath::new([mc_core::path::Layer {
            scheme: "zip".into(),
            location: String::new(),
            sub: "/".into(),
        }]);
        let entries = vfs.read_dir(&root).await.unwrap();
        assert!(
            entries.is_empty(),
            "evil entry must not surface, got {entries:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_and_list_zip() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("z.zip");
        make_zip(&p);
        let vfs = ZipVfs::open(&p, "zip").unwrap();

        let root = VPath::new([mc_core::path::Layer {
            scheme: "zip".into(),
            location: String::new(),
            sub: "/".into(),
        }]);
        let entries = vfs.read_dir(&root).await.unwrap();
        assert!(entries.iter().any(|e| e.name == "dir"));

        let inside = VPath::new([mc_core::path::Layer {
            scheme: "zip".into(),
            location: String::new(),
            sub: "/dir".into(),
        }]);
        let inner = vfs.read_dir(&inside).await.unwrap();
        assert!(inner.iter().any(|e| e.name == "hello.txt"));

        let file_path = VPath::new([mc_core::path::Layer {
            scheme: "zip".into(),
            location: String::new(),
            sub: "/dir/hello.txt".into(),
        }]);
        let mut r = vfs.open_read(&file_path).await.unwrap();
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut r, &mut buf)
            .await
            .unwrap();
        assert_eq!(&buf, b"zhello");
    }
}

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath};
use mc_vfs::trait_::{AsyncReader, Capabilities, Vfs};

#[derive(Debug, Clone)]
struct ZipEntry {
    name: String,
    kind: EntryKind,
    size: u64,
    mode: Option<u32>,
}

pub struct ZipVfs {
    scheme: &'static str,
    entries: BTreeMap<String, ZipEntry>,
    children: BTreeMap<String, Vec<String>>,
    /// Shared zip archive guarded by a mutex (zip crate is sync).
    archive: Mutex<zip::ZipArchive<std::fs::File>>,
}

impl ZipVfs {
    pub fn open(path: &Path, scheme: &'static str) -> Result<Self> {
        let f = std::fs::File::open(path).map_err(Error::Io)?;
        let archive = zip::ZipArchive::new(f).map_err(zip_to_err)?;
        let mut entries = BTreeMap::new();
        let n = archive.len();
        // We need to access by index without holding the archive mutex inside open.
        let mut a = archive;
        for i in 0..n {
            let zf = a.by_index(i).map_err(zip_to_err)?;
            let name_raw = zf.name().to_string();
            if name_raw.is_empty() {
                continue;
            }
            let trimmed = name_raw.trim_end_matches('/');
            if trimmed.is_empty() {
                continue;
            }
            let Some(key) = safe_archive_key(trimmed) else {
                tracing::warn!("zip: skipping traversal entry {name_raw:?}");
                continue;
            };
            if key == "/" {
                continue;
            }
            let kind = if zf.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            let leaf = key
                .rsplit_once('/')
                .map_or(key.as_str(), |(_, n)| n)
                .to_string();
            entries.insert(
                key,
                ZipEntry {
                    name: leaf,
                    kind,
                    size: zf.size(),
                    mode: zf.unix_mode(),
                },
            );
        }
        let children = build_child_index(&entries);
        let _ = path; // archive owns the file; path kept for potential future re-open
        Ok(Self {
            scheme,
            entries,
            children,
            archive: Mutex::new(a),
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

fn zip_to_err(e: zip::result::ZipError) -> Error {
    Error::Vfs(format!("zip: {e}"))
}

fn build_child_index(entries: &BTreeMap<String, ZipEntry>) -> BTreeMap<String, Vec<String>> {
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    children.insert("/".into(), Vec::new());
    let mut all_keys: Vec<String> = entries.keys().cloned().collect();
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
impl Vfs for ZipVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT | Capabilities::RANDOM_READ
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        let key = self.key_for(p)?;
        if key == "/" {
            return Ok(Entry {
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
            });
        }
        if let Some(e) = self.entries.get(&key) {
            return Ok(zip_to_entry(e));
        }
        if self.children.contains_key(&key) {
            return Ok(Entry {
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
            });
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
                out.push(zip_to_entry(e));
            } else {
                out.push(Entry {
                    name: name.clone(),
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
                });
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
        // We must not hold the mutex across an await boundary. Read fully into a
        // buffer synchronously, then return an async reader over the buffer.
        let bytes = tokio::task::block_in_place(|| -> Result<Vec<u8>> {
            let mut a = self.archive.lock().expect("zip archive mutex poisoned");
            let zip_name = key.trim_start_matches('/');
            let mut zf = a.by_name(zip_name).map_err(zip_to_err)?;
            let mut buf = Vec::with_capacity(zf.size() as usize);
            std::io::copy(&mut zf, &mut buf).map_err(Error::Io)?;
            Ok(buf)
        })?;
        let arc: Arc<[u8]> = Arc::from(bytes);
        Ok(Box::new(crate::tar_vfs::AsyncSliceReader::new(arc)))
    }
}

fn zip_to_entry(e: &ZipEntry) -> Entry {
    Entry {
        name: e.name.clone(),
        kind: e.kind,
        size: e.size,
        mtime: None,
        atime: None,
        ctime: None,
        mode: e.mode,
        uid: None,
        gid: None,
        nlink: None,
        target: None,
    }
}
