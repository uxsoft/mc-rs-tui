//! Local filesystem backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath, VPathBuf};
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::trait_::{AsyncReader, AsyncWriter, Capabilities, Vfs, WriteOpts};

#[derive(Debug, Default)]
pub struct LocalVfs;

impl LocalVfs {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn shared() -> Arc<dyn Vfs> {
        Arc::new(Self)
    }
}

fn local_path(p: &VPath) -> Result<PathBuf> {
    let layer = p
        .layers()
        .first()
        .ok_or_else(|| Error::InvalidPath("empty vpath".into()))?;
    if layer.scheme != "local" {
        return Err(Error::InvalidPath(format!(
            "local vfs cannot handle scheme {:?}",
            layer.scheme
        )));
    }
    Ok(layer.sub.clone())
}

fn entry_kind_from_metadata(md: &std::fs::Metadata) -> EntryKind {
    let ft = md.file_type();
    if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_file() {
        EntryKind::File
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if ft.is_fifo() {
                return EntryKind::Fifo;
            }
            if ft.is_socket() {
                return EntryKind::Socket;
            }
            if ft.is_block_device() {
                return EntryKind::BlockDevice;
            }
            if ft.is_char_device() {
                return EntryKind::CharDevice;
            }
        }
        EntryKind::Other
    }
}

fn build_entry(name: String, md: &std::fs::Metadata) -> Entry {
    let kind = entry_kind_from_metadata(md);
    let mtime = md.modified().ok();
    let atime = md.accessed().ok();
    let ctime = md.created().ok();

    #[cfg(unix)]
    let (mode, uid, gid, nlink) = {
        use std::os::unix::fs::MetadataExt;
        (
            Some(md.mode()),
            Some(md.uid()),
            Some(md.gid()),
            Some(md.nlink()),
        )
    };
    #[cfg(not(unix))]
    let (mode, uid, gid, nlink) = (None, None, None, None);

    Entry {
        name,
        kind,
        size: md.len(),
        mtime,
        atime,
        ctime,
        mode,
        uid,
        gid,
        nlink,
        target: None,
    }
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

#[async_trait]
impl Vfs for LocalVfs {
    fn scheme(&self) -> &'static str {
        "local"
    }

    fn capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::READ
            | Capabilities::WRITE
            | Capabilities::STAT
            | Capabilities::RANDOM_READ
            | Capabilities::WATCH;
        if cfg!(unix) {
            caps |= Capabilities::SYMLINK | Capabilities::CHMOD | Capabilities::CHOWN;
        }
        caps
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        let path = local_path(p)?;
        let md = fs::symlink_metadata(&path).await?;
        let mut entry = build_entry(name_of(&path), &md);
        if entry.is_symlink() {
            if let Ok(target) = fs::read_link(&path).await {
                entry.target = Some(target.to_string_lossy().into_owned());
            }
        }
        Ok(entry)
    }

    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>> {
        let path = local_path(p)?;
        let mut rd = fs::read_dir(&path).await?;
        let mut out = Vec::new();
        while let Some(child) = rd.next_entry().await? {
            let md = match child.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mut entry = build_entry(child.file_name().to_string_lossy().into_owned(), &md);
            if entry.is_symlink() {
                if let Ok(target) = fs::read_link(child.path()).await {
                    entry.target = Some(target.to_string_lossy().into_owned());
                }
            }
            out.push(entry);
        }
        Ok(out)
    }

    async fn open_read(&self, p: &VPath) -> Result<AsyncReader> {
        let path = local_path(p)?;
        let f = fs::File::open(&path).await?;
        Ok(Box::new(f))
    }

    async fn open_write(&self, p: &VPath, opts: WriteOpts) -> Result<AsyncWriter> {
        let path = local_path(p)?;
        let mut o = fs::OpenOptions::new();
        o.write(true)
            .create(opts.create)
            .truncate(opts.truncate)
            .append(opts.append);
        let f = o.open(&path).await?;
        Ok(Box::new(f))
    }

    async fn mkdir(&self, p: &VPath) -> Result<()> {
        let path = local_path(p)?;
        fs::create_dir(&path).await?;
        Ok(())
    }

    async fn rmdir(&self, p: &VPath) -> Result<()> {
        let path = local_path(p)?;
        fs::remove_dir(&path).await?;
        Ok(())
    }

    async fn unlink(&self, p: &VPath) -> Result<()> {
        let path = local_path(p)?;
        fs::remove_file(&path).await?;
        Ok(())
    }

    async fn rename(&self, from: &VPath, to: &VPath) -> Result<()> {
        let from = local_path(from)?;
        let to = local_path(to)?;
        fs::rename(&from, &to).await?;
        Ok(())
    }

    async fn readlink(&self, p: &VPath) -> Result<VPathBuf> {
        let path = local_path(p)?;
        let target = fs::read_link(&path).await?;
        Ok(VPath::local(target))
    }
}

#[allow(dead_code)]
async fn _read_to_string_for_test(p: &VPath) -> Result<String> {
    let mut r = LocalVfs.open_read(p).await?;
    let mut s = String::new();
    r.read_to_string(&mut s).await?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[tokio::test]
    async fn read_dir_lists_entries() {
        let td = tempdir();
        let p = td.path();
        std::fs::create_dir(p.join("sub")).unwrap();
        let mut f = std::fs::File::create(p.join("file.txt")).unwrap();
        f.write_all(b"hi").unwrap();
        drop(f);

        let vfs = LocalVfs;
        let entries = vfs.read_dir(&VPath::local(p.to_path_buf())).await.unwrap();
        let names: std::collections::BTreeSet<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains("sub"));
        assert!(names.contains("file.txt"));
    }

    #[tokio::test]
    async fn stat_reports_size() {
        let td = tempdir();
        let path = td.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let entry = LocalVfs.stat(&VPath::local(path)).await.unwrap();
        assert_eq!(entry.size, 5);
        assert!(matches!(entry.kind, EntryKind::File));
    }
}
