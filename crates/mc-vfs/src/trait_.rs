use std::sync::Arc;

use async_trait::async_trait;
use bitflags::bitflags;
use mc_core::{Entry, Error, Result, VPath, VPathBuf};
use tokio::io::{AsyncRead, AsyncWrite};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Capabilities: u32 {
        const READ        = 1 << 0;
        const WRITE       = 1 << 1;
        const STAT        = 1 << 2;
        const SYMLINK     = 1 << 3;
        const CHMOD       = 1 << 4;
        const CHOWN       = 1 << 5;
        const RANDOM_READ = 1 << 6;
        const WATCH       = 1 << 7;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOpts {
    pub create: bool,
    pub truncate: bool,
    pub append: bool,
}

pub type AsyncReader = Box<dyn AsyncRead + Send + Unpin>;
pub type AsyncWriter = Box<dyn AsyncWrite + Send + Unpin>;

#[async_trait]
pub trait Vfs: Send + Sync {
    fn scheme(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    async fn stat(&self, p: &VPath) -> Result<Entry>;
    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>>;
    async fn open_read(&self, p: &VPath) -> Result<AsyncReader>;
    async fn open_write(&self, _p: &VPath, _opts: WriteOpts) -> Result<AsyncWriter> {
        Err(Error::NotSupported)
    }

    async fn mkdir(&self, _p: &VPath) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn rmdir(&self, _p: &VPath) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn unlink(&self, _p: &VPath) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn rename(&self, _from: &VPath, _to: &VPath) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn chmod(&self, _p: &VPath, _mode: u32) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn chown(&self, _p: &VPath, _uid: u32, _gid: u32) -> Result<()> {
        Err(Error::NotSupported)
    }
    async fn readlink(&self, _p: &VPath) -> Result<VPathBuf> {
        Err(Error::NotSupported)
    }
    async fn symlink(&self, _target: &VPath, _link: &VPath) -> Result<()> {
        Err(Error::NotSupported)
    }

    /// Open this entry as a sub-VFS (e.g., a `tar` file → ArchiveVfs).
    async fn mount_as_vfs(&self, _p: &VPath) -> Result<Arc<dyn Vfs>> {
        Err(Error::NotSupported)
    }
}
