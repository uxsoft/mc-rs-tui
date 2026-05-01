//! Read-only archive backends mounted as virtual filesystems.

pub mod cpio_vfs;
#[cfg(feature = "rar")]
pub mod rar_vfs;
pub mod sevenz_vfs;
pub mod tar_vfs;
pub mod zip_vfs;

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use mc_core::Error;
use mc_vfs::Vfs;

/// Maximum decompressed-archive size we will buffer in memory. Anything larger
/// is rejected with [`Error::Vfs`] so a "compression bomb" cannot OOM the
/// process.
pub const MAX_DECOMPRESSED: u64 = 2 * 1024 * 1024 * 1024;

/// Sanitise an archive entry path so traversal components (`..`, absolute
/// references, leading slashes) cannot escape the VFS root.
///
/// Returns `None` for any entry that *did* contain `..` — the entire entry
/// should then be skipped by the caller. Empty / `.`-only paths normalise to
/// `"/"`. The result is always rooted at `/`, with no trailing slash, and
/// uses `/` as the only separator (callers are expected to pre-normalise
/// `\` if their format permits it).
#[must_use]
pub fn safe_archive_key(s: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in s.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            return None;
        }
        parts.push(c);
    }
    if parts.is_empty() {
        return Some("/".into());
    }
    Some(format!("/{}", parts.join("/")))
}

/// Decompress `r` fully into a `Vec<u8>` while enforcing
/// [`MAX_DECOMPRESSED`]. Wraps the reader in `take(MAX + 1)` so we can
/// distinguish "exactly at the cap" from "over the cap".
pub fn decompress_capped<R: Read>(r: R) -> mc_core::Result<Vec<u8>> {
    decompress_capped_with(r, MAX_DECOMPRESSED)
}

fn decompress_capped_with<R: Read>(r: R, limit: u64) -> mc_core::Result<Vec<u8>> {
    let mut limited = r.take(limit + 1);
    let mut buf = Vec::new();
    limited.read_to_end(&mut buf).map_err(Error::Io)?;
    if buf.len() as u64 > limit {
        return Err(Error::Vfs(format!(
            "archive exceeds {limit}-byte decompressed cap",
        )));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_key_normal() {
        assert_eq!(
            safe_archive_key("dir/file.txt"),
            Some("/dir/file.txt".into())
        );
        assert_eq!(safe_archive_key("/abs/path"), Some("/abs/path".into()));
        assert_eq!(safe_archive_key("trailing/"), Some("/trailing".into()));
        assert_eq!(safe_archive_key(""), Some("/".into()));
        assert_eq!(safe_archive_key("/"), Some("/".into()));
        assert_eq!(
            safe_archive_key("./hidden/in/the/middle"),
            Some("/hidden/in/the/middle".into())
        );
    }

    #[test]
    fn safe_key_rejects_traversal() {
        assert_eq!(safe_archive_key("../etc/passwd"), None);
        assert_eq!(safe_archive_key("a/../../b"), None);
        assert_eq!(safe_archive_key("a/b/.."), None);
        assert_eq!(safe_archive_key(".."), None);
    }

    #[test]
    fn cap_rejects_oversize() {
        // Use a tiny limit so the test allocates a few KB, not 2 GiB.
        let r = std::io::repeat(0u8);
        let err = decompress_capped_with(r, 1024).unwrap_err();
        assert!(matches!(err, Error::Vfs(_)));
    }

    #[test]
    fn cap_accepts_within() {
        let bytes = vec![0u8; 100];
        let r = std::io::Cursor::new(bytes);
        let out = decompress_capped_with(r, 1024).unwrap();
        assert_eq!(out.len(), 100);
    }
}

pub use cpio_vfs::CpioVfs;
#[cfg(feature = "rar")]
pub use rar_vfs::RarVfs;
pub use sevenz_vfs::SevenZVfs;
pub use tar_vfs::TarVfs;
pub use zip_vfs::ZipVfs;

/// Detect an archive type from a local file path's extension. Used by the
/// registry to decide whether to mount when a user presses Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Zip,
    Cpio,
    SevenZ,
    #[cfg(feature = "rar")]
    Rar,
}

impl ArchiveKind {
    /// Best-effort detection by lowercased filename suffix.
    #[must_use]
    pub fn detect(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") || name.ends_with(".tbz") {
            Some(Self::TarBz2)
        } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            Some(Self::TarXz)
        } else if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
            Some(Self::TarZst)
        } else if name.ends_with(".tar") {
            Some(Self::Tar)
        } else if name.ends_with(".zip") {
            Some(Self::Zip)
        } else if name.ends_with(".cpio") {
            Some(Self::Cpio)
        } else if name.ends_with(".7z") {
            Some(Self::SevenZ)
        } else if name.ends_with(".rar") {
            #[cfg(feature = "rar")]
            {
                Some(Self::Rar)
            }
            #[cfg(not(feature = "rar"))]
            {
                None
            }
        } else {
            None
        }
    }
}

/// Mount a local archive at `host_path` as a virtual filesystem with the given
/// `scheme`. The scheme becomes the layer name in [`mc_core::VPath`].
pub fn mount_local(
    path: &Path,
    kind: ArchiveKind,
    scheme: &'static str,
) -> mc_core::Result<Arc<dyn Vfs>> {
    match kind {
        ArchiveKind::Tar => Ok(Arc::new(TarVfs::open_uncompressed(path, scheme)?)),
        ArchiveKind::TarGz => Ok(Arc::new(TarVfs::open_compressed(
            path,
            scheme,
            Compression::Gz,
        )?)),
        ArchiveKind::TarBz2 => Ok(Arc::new(TarVfs::open_compressed(
            path,
            scheme,
            Compression::Bz2,
        )?)),
        ArchiveKind::TarXz => Ok(Arc::new(TarVfs::open_compressed(
            path,
            scheme,
            Compression::Xz,
        )?)),
        ArchiveKind::TarZst => Ok(Arc::new(TarVfs::open_compressed(
            path,
            scheme,
            Compression::Zst,
        )?)),
        ArchiveKind::Zip => Ok(Arc::new(ZipVfs::open(path, scheme)?)),
        ArchiveKind::Cpio => Ok(Arc::new(CpioVfs::open(path, scheme)?)),
        ArchiveKind::SevenZ => Ok(Arc::new(SevenZVfs::open(path, scheme)?)),
        #[cfg(feature = "rar")]
        ArchiveKind::Rar => Ok(Arc::new(RarVfs::open(path, scheme)?)),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Compression {
    Gz,
    Bz2,
    Xz,
    Zst,
}
