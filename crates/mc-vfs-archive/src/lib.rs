//! Read-only archive backends mounted as virtual filesystems.

pub mod tar_vfs;
pub mod zip_vfs;

use std::path::Path;
use std::sync::Arc;

use mc_vfs::Vfs;

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
        } else {
            None
        }
    }
}

/// Mount a local archive at `host_path` as a virtual filesystem with the given
/// `scheme`. The scheme becomes the layer name in [`mc_core::VPath`].
pub fn mount_local(path: &Path, kind: ArchiveKind, scheme: &'static str) -> mc_core::Result<Arc<dyn Vfs>> {
    match kind {
        ArchiveKind::Tar => Ok(Arc::new(TarVfs::open_uncompressed(path, scheme)?)),
        ArchiveKind::TarGz => Ok(Arc::new(TarVfs::open_compressed(path, scheme, Compression::Gz)?)),
        ArchiveKind::TarBz2 => Ok(Arc::new(TarVfs::open_compressed(path, scheme, Compression::Bz2)?)),
        ArchiveKind::TarXz => Ok(Arc::new(TarVfs::open_compressed(path, scheme, Compression::Xz)?)),
        ArchiveKind::TarZst => Ok(Arc::new(TarVfs::open_compressed(path, scheme, Compression::Zst)?)),
        ArchiveKind::Zip => Ok(Arc::new(ZipVfs::open(path, scheme)?)),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Compression {
    Gz,
    Bz2,
    Xz,
    Zst,
}
