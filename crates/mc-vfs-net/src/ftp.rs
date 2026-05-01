//! Read-only FTP backend.
//!
//! Single control connection guarded by a mutex; data transfers serialise
//! through it. Suitable for browsing and viewing; not parallel-throughput.
//!
//! Auth: anonymous if `user` is empty, otherwise `user@host:port` with the
//! password taken from `$MC_RS_FTP_PASS` (a Phase-9 stopgap until we have a
//! UI password prompt).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath};
use mc_vfs::trait_::{AsyncReader, Capabilities, Vfs};
use suppaftp::list::File as FtpFile;
use suppaftp::tokio::AsyncFtpStream;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct FtpEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl FtpEndpoint {
    pub fn parse(loc: &str) -> Result<Self> {
        let (user, host_port) = match loc.rsplit_once('@') {
            Some((u, hp)) => (u.to_string(), hp),
            None => (String::new(), loc),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| Error::InvalidPath(format!("bad port in {loc:?}")))?,
            ),
            None => (host_port.to_string(), 21u16),
        };
        if host.is_empty() {
            return Err(Error::InvalidPath(format!("empty host in {loc:?}")));
        }
        Ok(Self { user, host, port })
    }
}

pub struct FtpVfs {
    scheme: &'static str,
    endpoint: FtpEndpoint,
    stream: Arc<Mutex<AsyncFtpStream>>,
}

impl FtpVfs {
    pub async fn connect_with_password(
        scheme: &'static str,
        endpoint: FtpEndpoint,
        password: &str,
    ) -> Result<Self> {
        let mut stream = AsyncFtpStream::connect(format!("{}:{}", endpoint.host, endpoint.port))
            .await
            .map_err(|e| Error::Vfs(format!("ftp connect: {e}")))?;
        let user = if endpoint.user.is_empty() {
            "anonymous"
        } else {
            &endpoint.user
        };
        stream
            .login(user, password)
            .await
            .map_err(|e| Error::Vfs(format!("ftp login: {e}")))?;
        Ok(Self {
            scheme,
            endpoint,
            stream: Arc::new(Mutex::new(stream)),
        })
    }

    pub async fn connect(scheme: &'static str, endpoint: FtpEndpoint) -> Result<Self> {
        let mut stream = AsyncFtpStream::connect(format!("{}:{}", endpoint.host, endpoint.port))
            .await
            .map_err(|e| Error::Vfs(format!("ftp connect: {e}")))?;
        let user = if endpoint.user.is_empty() {
            "anonymous".to_string()
        } else {
            endpoint.user.clone()
        };
        let pass = if endpoint.user.is_empty() {
            "anonymous@example.com".to_string()
        } else {
            std::env::var("MC_RS_FTP_PASS").unwrap_or_default()
        };
        stream
            .login(user, pass)
            .await
            .map_err(|e| Error::Vfs(format!("ftp login: {e}")))?;
        Ok(Self {
            scheme,
            endpoint,
            stream: Arc::new(Mutex::new(stream)),
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &FtpEndpoint {
        &self.endpoint
    }

    fn key_for(&self, p: &VPath) -> Result<String> {
        let layer = p
            .layers()
            .iter()
            .rev()
            .find(|l| l.scheme == self.scheme)
            .ok_or_else(|| Error::InvalidPath(format!("vpath has no {} layer", self.scheme)))?;
        let s = layer.sub.to_string_lossy();
        let s = if s.is_empty() {
            "/".to_string()
        } else {
            s.into_owned()
        };
        Ok(s)
    }
}

fn ftp_to_entry(f: &FtpFile) -> Entry {
    let kind = if f.is_directory() {
        EntryKind::Dir
    } else if f.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::File
    };
    let mtime = Some(f.modified());
    let target = f.symlink().map(|p| p.to_string_lossy().into_owned());
    Entry {
        name: f.name().to_string(),
        kind,
        size: f.size() as u64,
        mtime,
        atime: None,
        ctime: None,
        mode: None,
        uid: f.uid(),
        gid: f.gid(),
        nlink: None,
        target,
    }
}

#[async_trait]
impl Vfs for FtpVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT
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
        // Stat via list of parent + filename match.
        let parent = Path::new(&key)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into());
        let leaf = Path::new(&key)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| key.clone());
        let lines = {
            let mut s = self.stream.lock().await;
            s.list(Some(parent.as_str()))
                .await
                .map_err(|e| Error::Vfs(format!("ftp list {parent}: {e}")))?
        };
        for line in &lines {
            if let Ok(f) = suppaftp::list::ListParser::parse_posix(line) {
                if f.name() == leaf {
                    return Ok(ftp_to_entry(&f));
                }
            }
        }
        Err(Error::Vfs(format!("not found: {key}")))
    }

    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>> {
        let key = self.key_for(p)?;
        let lines = {
            let mut s = self.stream.lock().await;
            s.list(Some(key.as_str()))
                .await
                .map_err(|e| Error::Vfs(format!("ftp list {key}: {e}")))?
        };
        let mut out = Vec::with_capacity(lines.len());
        for line in &lines {
            match suppaftp::list::ListParser::parse_posix(line) {
                Ok(f) => {
                    if f.name() == "." || f.name() == ".." {
                        continue;
                    }
                    out.push(ftp_to_entry(&f));
                }
                Err(e) => {
                    tracing::debug!("ftp parse list line {line:?}: {e}");
                }
            }
        }
        Ok(out)
    }

    async fn open_read(&self, p: &VPath) -> Result<AsyncReader> {
        let key = self.key_for(p)?;
        // Materialize fully: suppaftp's streaming retr keeps the lock held and
        // mixes data + control connections. For Phase 9 first cut we just
        // download into memory.
        let bytes = {
            let mut s = self.stream.lock().await;
            let mut stream = s
                .retr_as_stream(key.as_str())
                .await
                .map_err(|e| Error::Vfs(format!("ftp retr {key}: {e}")))?;
            let mut buf = Vec::new();
            stream
                .read_to_end(&mut buf)
                .await
                .map_err(|e| Error::Io(e))?;
            s.finalize_retr_stream(stream)
                .await
                .map_err(|e| Error::Vfs(format!("ftp finalize: {e}")))?;
            buf
        };
        let arc: Arc<[u8]> = Arc::from(bytes);
        Ok(Box::new(SliceReader { data: arc, pos: 0 }))
    }
}

struct SliceReader {
    data: Arc<[u8]>,
    pos: usize,
}

impl AsyncRead for SliceReader {
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

    #[test]
    fn parse_endpoint() {
        let e = FtpEndpoint::parse("anon@host:2121").unwrap();
        assert_eq!(e.user, "anon");
        assert_eq!(e.port, 2121);
        let e = FtpEndpoint::parse("host").unwrap();
        assert_eq!(e.user, "");
        assert_eq!(e.port, 21);
        assert!(FtpEndpoint::parse("").is_err());
    }
}
