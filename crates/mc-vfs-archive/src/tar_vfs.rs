//! Read-only TAR backend.
//!
//! For uncompressed `.tar`, we keep an offset table over the source file and
//! seek for `open_read`. For gzipped `.tar.gz`, we decompress the entire archive
//! into memory on open (Phase 7 keeps this simple; Phase 9 will spool large
//! gzipped archives to a tempfile).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath};
use mc_vfs::trait_::{AsyncReader, Capabilities, Vfs};
use tokio::io::AsyncRead;

#[derive(Debug, Clone)]
struct ArchEntry {
    name: String,
    kind: EntryKind,
    size: u64,
    /// Byte offset in source data. Only meaningful for uncompressed mode.
    data_offset: u64,
    /// In-memory data for compressed-mode entries.
    data: Option<Arc<[u8]>>,
    mode: Option<u32>,
    mtime: Option<std::time::SystemTime>,
    /// Symlink target (for `EntryKind::Symlink`).
    link_target: Option<String>,
}

#[derive(Debug)]
enum Source {
    File(PathBuf),
    /// Decompressed bytes held in memory (kept alive while the VFS exists).
    Memory,
}

pub struct TarVfs {
    scheme: &'static str,
    source: Source,
    /// Map of "/parent/dir/name" → entry metadata. Always rooted at "/".
    entries: BTreeMap<String, ArchEntry>,
    /// Map of "/parent/dir" → list of immediate child names.
    children: BTreeMap<String, Vec<String>>,
}

impl TarVfs {
    pub fn open_uncompressed(path: &Path, scheme: &'static str) -> Result<Self> {
        let f = std::fs::File::open(path).map_err(Error::Io)?;
        let mut a = tar::Archive::new(f);
        let mut entries = BTreeMap::new();
        for hdr in a.entries_with_seek().map_err(Error::Io)? {
            let h = hdr.map_err(Error::Io)?;
            if let Some(e) = build_entry_for_header(&h, /*compressed*/ false)? {
                entries.insert(e.0, e.1);
            }
        }
        let children = build_child_index(&entries);
        Ok(Self {
            scheme,
            source: Source::File(path.to_path_buf()),
            entries,
            children,
        })
    }

    pub fn open_compressed(
        path: &Path,
        scheme: &'static str,
        c: crate::Compression,
    ) -> Result<Self> {
        let f = std::fs::File::open(path).map_err(Error::Io)?;
        let mut buf = Vec::new();
        match c {
            crate::Compression::Gz => {
                let mut d = flate2::read::GzDecoder::new(f);
                d.read_to_end(&mut buf).map_err(Error::Io)?;
            }
            crate::Compression::Bz2 => {
                let mut d = bzip2::read::BzDecoder::new(f);
                d.read_to_end(&mut buf).map_err(Error::Io)?;
            }
            crate::Compression::Xz => {
                let mut d = xz2::read::XzDecoder::new(f);
                d.read_to_end(&mut buf).map_err(Error::Io)?;
            }
            crate::Compression::Zst => {
                let mut d = zstd::Decoder::new(f).map_err(Error::Io)?;
                d.read_to_end(&mut buf).map_err(Error::Io)?;
            }
        }
        let arc: Arc<[u8]> = Arc::from(buf);
        let mut entries = BTreeMap::new();
        let mut a = tar::Archive::new(Cursor::new(arc));
        for hdr in a.entries().map_err(Error::Io)? {
            let mut h = hdr.map_err(Error::Io)?;
            if let Some((name, mut e)) = build_entry_for_header(&h, /*compressed*/ true)? {
                if matches!(e.kind, EntryKind::File) {
                    let mut data = Vec::with_capacity(e.size as usize);
                    h.read_to_end(&mut data).map_err(Error::Io)?;
                    e.data = Some(Arc::from(data));
                }
                entries.insert(name, e);
            }
        }
        let children = build_child_index(&entries);
        Ok(Self {
            scheme,
            source: Source::Memory,
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
    let mut out = String::with_capacity(s.len() + 1);
    if !s.starts_with('/') {
        out.push('/');
    }
    out.push_str(s.trim_end_matches('/'));
    if out.is_empty() {
        out.push('/');
    }
    out
}

fn build_entry_for_header(
    h: &tar::Entry<'_, impl Read>,
    _compressed: bool,
) -> Result<Option<(String, ArchEntry)>> {
    let path = match h.path() {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    let p_str = path.to_string_lossy().to_string();
    if p_str.is_empty() || p_str == "/" || p_str == "./" {
        return Ok(None);
    }
    let key = normalize_key(p_str.trim_end_matches('/'));
    let kind = match h.header().entry_type() {
        tar::EntryType::Directory => EntryKind::Dir,
        tar::EntryType::Symlink => EntryKind::Symlink,
        tar::EntryType::Regular | tar::EntryType::Continuous => EntryKind::File,
        _ => EntryKind::Other,
    };
    let size = h.header().size().unwrap_or(0);
    let mode = h.header().mode().ok();
    let mtime = h
        .header()
        .mtime()
        .ok()
        .and_then(|s| std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s)));
    let data_offset = h.raw_file_position();
    let link_target = h
        .link_name()
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned());
    let name = key.rsplit_once('/').map_or(key.as_str(), |(_, n)| n).to_string();
    let entry = ArchEntry {
        name,
        kind,
        size,
        data_offset,
        data: None,
        mode,
        mtime,
        link_target,
    };
    Ok(Some((key, entry)))
}

fn build_child_index(entries: &BTreeMap<String, ArchEntry>) -> BTreeMap<String, Vec<String>> {
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    children.insert("/".into(), Vec::new());
    // Synthesize directories for paths whose ancestors aren't explicitly listed.
    let mut dirs_to_add: Vec<String> = Vec::new();
    for key in entries.keys() {
        let mut cur = key.as_str();
        while let Some((parent, _)) = cur.rsplit_once('/') {
            let p = if parent.is_empty() { "/" } else { parent };
            if !entries.contains_key(p) && p != "/" {
                dirs_to_add.push(p.to_string());
            }
            cur = p;
            if cur == "/" {
                break;
            }
        }
    }
    let mut all_keys: Vec<String> = entries.keys().cloned().collect();
    for d in dirs_to_add {
        all_keys.push(d);
    }
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
impl Vfs for TarVfs {
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
            return Ok(arch_to_entry(e));
        }
        // Synthetic directory.
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
                out.push(arch_to_entry(e));
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
        match (&self.source, &e.data) {
            (Source::Memory, Some(bytes)) => Ok(Box::new(AsyncSliceReader::new(bytes.clone()))),
            (Source::File(path), _) => {
                let mut f = std::fs::File::open(path).map_err(Error::Io)?;
                f.seek(SeekFrom::Start(e.data_offset)).map_err(Error::Io)?;
                let mut data = vec![0u8; e.size as usize];
                f.read_exact(&mut data).map_err(Error::Io)?;
                Ok(Box::new(AsyncSliceReader::new(Arc::from(data))))
            }
            (Source::Memory, None) => Err(Error::Vfs("missing decompressed data".into())),
        }
    }
}

fn arch_to_entry(e: &ArchEntry) -> Entry {
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
        target: e.link_target.clone(),
    }
}

/// Async reader over a shared byte slice — used for archive entry reads.
pub(crate) struct AsyncSliceReader {
    data: Arc<[u8]>,
    pos: usize,
}

impl AsyncSliceReader {
    pub(crate) fn new(data: Arc<[u8]>) -> Self {
        Self { data, pos: 0 }
    }
}

impl AsyncRead for AsyncSliceReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let me = self.get_mut();
        let remaining = &me.data[me.pos..];
        let n = remaining.len().min(buf.remaining());
        if n == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        buf.put_slice(&remaining[..n]);
        me.pos += n;
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tar::{Builder, Header};

    fn make_tar(path: &Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut b = Builder::new(f);
        let mut h = Header::new_gnu();
        h.set_path("dir/").unwrap();
        h.set_size(0);
        h.set_entry_type(tar::EntryType::Directory);
        h.set_mode(0o755);
        h.set_cksum();
        b.append(&h, std::io::empty()).unwrap();

        let body = b"hello world";
        let mut h = Header::new_gnu();
        h.set_path("dir/file.txt").unwrap();
        h.set_size(body.len() as u64);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_cksum();
        b.append(&h, body.as_ref()).unwrap();
        b.into_inner().unwrap().flush().unwrap();
    }

    #[tokio::test]
    async fn open_and_list_uncompressed_tar() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("a.tar");
        make_tar(&p);
        let vfs = TarVfs::open_uncompressed(&p, "tar").unwrap();
        let root = VPath::new([mc_core::path::Layer {
            scheme: "tar".into(),
            location: String::new(),
            sub: "/".into(),
        }]);
        let entries = vfs.read_dir(&root).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "dir");
        let inside = VPath::new([mc_core::path::Layer {
            scheme: "tar".into(),
            location: String::new(),
            sub: "/dir".into(),
        }]);
        let inner = vfs.read_dir(&inside).await.unwrap();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].name, "file.txt");

        let file_path = VPath::new([mc_core::path::Layer {
            scheme: "tar".into(),
            location: String::new(),
            sub: "/dir/file.txt".into(),
        }]);
        let mut reader = vfs.open_read(&file_path).await.unwrap();
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf)
            .await
            .unwrap();
        assert_eq!(&buf, b"hello world");
    }
}
