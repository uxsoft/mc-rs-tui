//! Read-only SFTP backend (Phase 8 first cut).
//!
//! Authentication tries, in order:
//! 1. ssh-agent via `$SSH_AUTH_SOCK`
//! 2. private key files at `~/.ssh/id_ed25519` and `~/.ssh/id_rsa` (no passphrase)
//!
//! Host-key verification:
//!   - On a *new* host the connection is **refused** and the unknown
//!     fingerprint is surfaced as [`crate::core::Error::HostKeyUnknown`]. The UI
//!     layer is expected to prompt the user, then call
//!     [`SftpVfs::connect_trusting`] to record the fingerprint and retry.
//!     Silently auto-accepting would defeat MITM protection on the very
//!     first connection.
//!   - On a *recorded* host whose fingerprint differs the connection is
//!     refused outright (no UI prompt for tampering).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::core::{Entry, EntryKind, Error, Result, VPath};
use crate::vfs::trait_::{AsyncReader, Capabilities, Vfs};
use async_trait::async_trait;
use russh::client;
use russh::keys::PublicKey;
use russh_sftp::client::SftpSession;
use tokio::io::AsyncRead;

use crate::vfs_net::known_hosts::{CheckResult, KnownHosts};

#[derive(Debug, Clone)]
pub struct SftpEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SftpEndpoint {
    /// Parse a `[user@]host[:port]` location string (the `location` field of a
    /// [`crate::core::path::Layer`]).
    pub fn parse(loc: &str) -> Result<Self> {
        let (user, host_port) = match loc.rsplit_once('@') {
            Some((u, hp)) => (u.to_string(), hp),
            None => (std::env::var("USER").unwrap_or_else(|_| "root".into()), loc),
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

struct SshClient {
    host_port: String,
    known_hosts: Arc<Mutex<KnownHosts>>,
    /// If set, the user has already confirmed this fingerprint for this
    /// session and we should treat a `NewHost` outcome as accepted (and
    /// record it) instead of refusing.
    trust_on_match: Option<(String, String)>,
    /// Side-channel used by [`SshClient::check_server_key`] to communicate
    /// which fingerprint a `NewHost` rejection saw. The connector reads
    /// this slot after a failed `client::connect` so the UI can prompt the
    /// user.
    pending_unknown: Arc<Mutex<Option<(String, String, String)>>>,
}

impl client::Handler for SshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        let algo = key.algorithm().as_str().to_string();
        let fp = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let mut kh = self.known_hosts.lock().expect("known_hosts mutex poisoned");
        match kh.check(&self.host_port, &algo, &fp) {
            CheckResult::Match => Ok(true),
            CheckResult::NewHost => {
                if let Some((trusted_algo, trusted_fp)) = &self.trust_on_match {
                    if trusted_algo == &algo && trusted_fp == &fp {
                        if let Err(e) = kh.record(&self.host_port, &algo, &fp) {
                            tracing::warn!("known_hosts write {}: {e}", self.host_port);
                        }
                        tracing::info!(
                            "known_hosts: recorded user-confirmed host {} ({fp})",
                            self.host_port,
                        );
                        return Ok(true);
                    }
                    tracing::error!(
                        "host-key changed between confirmation and connect for {}: \
                         confirmed {trusted_fp}, server presented {fp}",
                        self.host_port,
                    );
                }
                // Unknown host and no user confirmation yet: refuse the
                // connection and stash the fingerprint so the connector can
                // surface it as Error::HostKeyUnknown.
                if let Ok(mut slot) = self.pending_unknown.lock() {
                    *slot = Some((self.host_port.clone(), algo, fp));
                }
                Ok(false)
            }
            CheckResult::Mismatch { recorded } => {
                tracing::error!(
                    "known_hosts MISMATCH for {}: server presented {fp}, recorded {recorded}",
                    self.host_port,
                );
                Ok(false)
            }
        }
    }
}

impl SftpVfs {
    /// Connect to `endpoint` and open an SFTP subsystem channel.
    /// Uses the default known_hosts file at `$XDG_CACHE_HOME/mc-rs/known_hosts`.
    pub async fn connect(scheme: &'static str, endpoint: SftpEndpoint) -> Result<Self> {
        Self::connect_inner(scheme, endpoint, None, None).await
    }

    /// Connect using an explicit password (UI-prompt path).
    pub async fn connect_with_password(
        scheme: &'static str,
        endpoint: SftpEndpoint,
        password: &str,
    ) -> Result<Self> {
        Self::connect_inner(scheme, endpoint, Some(password.to_string()), None).await
    }

    /// Connect after the user has confirmed an unknown host fingerprint.
    /// On success the fingerprint is appended to known_hosts. If the server
    /// presents a *different* fingerprint than the one the user confirmed,
    /// the connection is refused (defends against a swap-while-prompting
    /// race).
    pub async fn connect_trusting(
        scheme: &'static str,
        endpoint: SftpEndpoint,
        algorithm: String,
        fingerprint: String,
        password: Option<String>,
    ) -> Result<Self> {
        Self::connect_inner(scheme, endpoint, password, Some((algorithm, fingerprint))).await
    }

    /// Connect with a caller-provided known_hosts store (useful for tests).
    pub async fn connect_with_known_hosts(
        scheme: &'static str,
        endpoint: SftpEndpoint,
        known_hosts: Arc<Mutex<KnownHosts>>,
    ) -> Result<Self> {
        Self::connect_with_known_hosts_inner(scheme, endpoint, known_hosts, None, None).await
    }

    async fn connect_inner(
        scheme: &'static str,
        endpoint: SftpEndpoint,
        password: Option<String>,
        trust_on_match: Option<(String, String)>,
    ) -> Result<Self> {
        let known_hosts = Arc::new(Mutex::new(KnownHosts::load(KnownHosts::default_path())));
        Self::connect_with_known_hosts_inner(
            scheme,
            endpoint,
            known_hosts,
            password,
            trust_on_match,
        )
        .await
    }

    async fn connect_with_known_hosts_inner(
        scheme: &'static str,
        endpoint: SftpEndpoint,
        known_hosts: Arc<Mutex<KnownHosts>>,
        password: Option<String>,
        trust_on_match: Option<(String, String)>,
    ) -> Result<Self> {
        let host_port = format!("{}:{}", endpoint.host, endpoint.port);
        let pending_unknown: Arc<Mutex<Option<(String, String, String)>>> =
            Arc::new(Mutex::new(None));
        let cfg = Arc::new(client::Config::default());
        let handler = SshClient {
            host_port: host_port.clone(),
            known_hosts,
            trust_on_match,
            pending_unknown: pending_unknown.clone(),
        };
        let mut handle =
            match client::connect(cfg, (endpoint.host.as_str(), endpoint.port), handler).await {
                Ok(h) => h,
                Err(e) => {
                    if let Some((host, algo, fp)) =
                        pending_unknown.lock().ok().and_then(|mut s| s.take())
                    {
                        return Err(Error::HostKeyUnknown {
                            host_port: host,
                            algorithm: algo,
                            fingerprint: fp,
                        });
                    }
                    return Err(Error::Vfs(format!("ssh connect: {e}")));
                }
            };
        if let Some(pw) = &password {
            match handle.authenticate_password(&endpoint.user, pw).await {
                Ok(res) if res.success() => {}
                Ok(_) => return Err(Error::Vfs(format!("password auth failed for {host_port}"))),
                Err(e) => return Err(Error::Vfs(format!("password auth error: {e}"))),
            }
        } else if !try_auth(&mut handle, &endpoint.user).await? {
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
        let s = if s.is_empty() {
            "/".to_string()
        } else {
            s.into_owned()
        };
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
    if let Ok(true) = try_password_env(handle, user).await {
        return Ok(true);
    }
    Ok(false)
}

async fn try_password_env(handle: &mut client::Handle<SshClient>, user: &str) -> Result<bool> {
    let pass = match std::env::var("MC_RS_SFTP_PASS") {
        Ok(s) if !s.is_empty() => s,
        _ => return Ok(false),
    };
    match handle.authenticate_password(user, pass).await {
        Ok(res) if res.success() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) => {
            tracing::debug!("ssh password auth: {e}");
            Ok(false)
        }
    }
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
            .authenticate_publickey_with(user, id, Some(russh::keys::HashAlg::Sha512), &mut agent)
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
        match handle
            .authenticate_publickey(user, private_key_with_hash)
            .await
        {
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
