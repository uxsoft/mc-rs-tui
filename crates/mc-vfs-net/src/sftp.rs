//! Read-only SFTP backend (Phase 8 first cut).
//!
//! Authentication tries, in order:
//! 1. ssh-agent via `$SSH_AUTH_SOCK`
//! 2. private key files at `~/.ssh/id_ed25519` and `~/.ssh/id_rsa` (no passphrase)
//!
//! Host-key verification is currently TOFU-accept-everything; future work adds a
//! `~/.cache/mc-rs/known_hosts` store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Entry, EntryKind, Error, Result, VPath};
use mc_vfs::trait_::{AsyncReader, Capabilities, Vfs};
use russh::client;
use russh::keys::PublicKey;
use russh_sftp::client::SftpSession;
use tokio::io::AsyncRead;

#[derive(Debug, Clone)]
pub struct SftpEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SftpEndpoint {
    /// Parse a `[user@]host[:port]` location string (the `location` field of a
    /// [`mc_core::path::Layer`]).
    pub fn parse(loc: &str) -> Result<Self> {
        let (user, host_port) = match loc.rsplit_once('@') {
            Some((u, hp)) => (u.to_string(), hp),
            None => (
                std::env::var("USER").unwrap_or_else(|_| "root".into()),
                loc,
            ),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| Error::InvalidPath(format!("bad port in {loc:?}")))?,
            ),
            None => (host_port.to_string(), 22u16),
        };
        if host.is_empty() {
            return Err(Error::InvalidPath(format!("empty host in {loc:?}")));
        }
        Ok(Self { user, host, port })
    }

    #[must_use]
    pub fn display(&self) -> String {
        if self.port == 22 {
            format!("{}@{}", self.user, self.host)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

pub struct SftpVfs {
    scheme: &'static str,
    endpoint: SftpEndpoint,
    session: SftpSession,
    _handle: client::Handle<SshClient>,
}

struct SshClient;

impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        // TOFU accept. Future: persistent known_hosts store with prompt.
        Ok(true)
    }
}

impl SftpVfs {
    /// Connect to `endpoint` and open an SFTP subsystem channel.
    pub async fn connect(scheme: &'static str, endpoint: SftpEndpoint) -> Result<Self> {
        let cfg = Arc::new(client::Config::default());
        let mut handle =
            client::connect(cfg, (endpoint.host.as_str(), endpoint.port), SshClient)
                .await
                .map_err(|e| Error::Vfs(format!("ssh connect: {e}")))?;

        if !try_auth(&mut handle, &endpoint.user).await? {
            return Err(Error::Vfs(format!(
                "ssh authentication failed for {}@{} (no agent or matching key)",
                endpoint.user, endpoint.host
            )));
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| Error::Vfs(format!("ssh channel: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| Error::Vfs(format!("sftp subsystem: {e}")))?;
        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| Error::Vfs(format!("sftp init: {e}")))?;
        Ok(Self {
            scheme,
            endpoint,
            session,
            _handle: handle,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &SftpEndpoint {
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
        let s = if s.is_empty() { "/".to_string() } else { s.into_owned() };
        Ok(s)
    }
}

async fn try_auth(handle: &mut client::Handle<SshClient>, user: &str) -> Result<bool> {
    if let Ok(true) = try_agent(handle, user).await {
        return Ok(true);
    }
    if let Ok(true) = try_key_file(handle, user).await {
        return Ok(true);
    }
    Ok(false)
}

async fn try_agent(handle: &mut client::Handle<SshClient>, user: &str) -> Result<bool> {
    let sock = match std::env::var_os("SSH_AUTH_SOCK") {
        Some(s) => s,
        None => return Ok(false),
    };
    let mut agent = match russh::keys::agent::client::AgentClient::connect_uds(sock).await {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!("ssh-agent: {e}");
            return Ok(false);
        }
    };
    let identities = match agent.request_identities().await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("ssh-agent identities: {e}");
            return Ok(false);
        }
    };
    for id in identities {
        match handle
            .authenticate_publickey_with(
                user,
                id,
                Some(russh::keys::HashAlg::Sha512),
                &mut agent,
            )
            .await
        {
            Ok(res) if res.success() => return Ok(true),
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!("ssh-agent pubkey: {e}");
                continue;
            }
        }
    }
    Ok(false)
}

async fn try_key_file(handle: &mut client::Handle<SshClient>, user: &str) -> Result<bool> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Ok(false),
    };
    for fname in ["id_ed25519", "id_rsa", "id_ecdsa"] {
        let path = home.join(".ssh").join(fname);
        if !path.exists() {
            continue;
        }
        let key = match russh::keys::load_secret_key(&path, None) {
            Ok(k) => k,
            Err(e) => {
                tracing::debug!("load {} failed: {e}", path.display());
                continue;
            }
        };
        let private_key_with_hash = russh::keys::PrivateKeyWithHashAlg::new(
            Arc::new(key),
            Some(russh::keys::HashAlg::Sha512),
        );
        match handle.authenticate_publickey(user, private_key_with_hash).await {
            Ok(res) if res.success() => return Ok(true),
            Ok(_) => continue,
            Err(e) => {
                tracing::debug!("authenticate {}: {e}", path.display());
                continue;
            }
        }
    }
    Ok(false)
}

fn metadata_to_entry(name: String, md: russh_sftp::protocol::FileAttributes) -> Entry {
    let kind = if md.is_dir() {
        EntryKind::Dir
    } else if md.is_symlink() {
        EntryKind::Symlink
    } else if md.is_regular() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let mtime = md
        .mtime
        .map(|t| std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::from(t)));
    let atime = md
        .atime
        .map(|t| std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::from(t)));
    Entry {
        name,
        kind,
        size: md.size.unwrap_or(0),
        mtime,
        atime,
        ctime: None,
        mode: md.permissions,
        uid: md.uid,
        gid: md.gid,
        nlink: None,
        target: None,
    }
}

#[async_trait]
impl Vfs for SftpVfs {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::READ | Capabilities::STAT | Capabilities::SYMLINK
    }

    async fn stat(&self, p: &VPath) -> Result<Entry> {
        let key = self.key_for(p)?;
        let md = self
            .session
            .symlink_metadata(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp stat {key}: {e}")))?;
        let name = Path::new(&key)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| key.clone());
        Ok(metadata_to_entry(name, md))
    }

    async fn read_dir(&self, p: &VPath) -> Result<Vec<Entry>> {
        let key = self.key_for(p)?;
        let entries = self
            .session
            .read_dir(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp read_dir {key}: {e}")))?;
        let mut out = Vec::new();
        for e in entries {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            out.push(metadata_to_entry(name, e.metadata()));
        }
        Ok(out)
    }

    async fn open_read(&self, p: &VPath) -> Result<AsyncReader> {
        let key = self.key_for(p)?;
        let f = self
            .session
            .open(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp open {key}: {e}")))?;
        Ok(Box::new(SftpAsyncReader { inner: f }))
    }

    async fn mkdir(&self, p: &VPath) -> Result<()> {
        let key = self.key_for(p)?;
        self.session
            .create_dir(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp mkdir {key}: {e}")))
    }

    async fn rmdir(&self, p: &VPath) -> Result<()> {
        let key = self.key_for(p)?;
        self.session
            .remove_dir(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp rmdir {key}: {e}")))
    }

    async fn unlink(&self, p: &VPath) -> Result<()> {
        let key = self.key_for(p)?;
        self.session
            .remove_file(&key)
            .await
            .map_err(|e| Error::Vfs(format!("sftp unlink {key}: {e}")))
    }

    async fn rename(&self, from: &VPath, to: &VPath) -> Result<()> {
        let f = self.key_for(from)?;
        let t = self.key_for(to)?;
        self.session
            .rename(&f, &t)
            .await
            .map_err(|e| Error::Vfs(format!("sftp rename {f}->{t}: {e}")))
    }
}

struct SftpAsyncReader {
    inner: russh_sftp::client::fs::File,
}

impl AsyncRead for SftpAsyncReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let me = self.get_mut();
        std::pin::Pin::new(&mut me.inner).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint() {
        let e = SftpEndpoint::parse("me@host:2222").unwrap();
        assert_eq!(e.user, "me");
        assert_eq!(e.host, "host");
        assert_eq!(e.port, 2222);

        let e = SftpEndpoint::parse("me@host").unwrap();
        assert_eq!(e.port, 22);

        let e = SftpEndpoint::parse("host").unwrap();
        assert_eq!(e.host, "host");

        assert!(SftpEndpoint::parse("").is_err());
        assert!(SftpEndpoint::parse("me@host:abc").is_err());
    }
}
