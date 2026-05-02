//! Trust-on-first-use host-key store.
//!
//! Stored as a tiny text file keyed by `host:port`, one record per line:
//!
//! ```text
//! host:port  algo  base64fingerprint
//! ```
//!
//! On first connection to a host we record its key fingerprint. On subsequent
//! connections we require an exact match, otherwise the connection is refused.
//! The store lives at `$XDG_CACHE_HOME/mc-rs/known_hosts` by default.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub host_port: String,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Debug)]
pub enum CheckResult {
    /// Key matches the recorded fingerprint.
    Match,
    /// First time we've seen this host; caller should record before accepting.
    NewHost,
    /// We have a record for this host but the fingerprint differs; reject.
    Mismatch { recorded: String },
}

impl KnownHosts {
    pub fn load(path: PathBuf) -> Self {
        let entries = read_file(&path).unwrap_or_default();
        Self { path, entries }
    }

    /// Default location at `$XDG_CACHE_HOME/mc-rs/known_hosts`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);
        cache.join("mc-rs").join("known_hosts")
    }

    pub fn check(&self, host_port: &str, algorithm: &str, fingerprint: &str) -> CheckResult {
        for e in &self.entries {
            if e.host_port == host_port && e.algorithm == algorithm {
                return if e.fingerprint == fingerprint {
                    CheckResult::Match
                } else {
                    CheckResult::Mismatch {
                        recorded: e.fingerprint.clone(),
                    }
                };
            }
        }
        CheckResult::NewHost
    }

    pub fn record(
        &mut self,
        host_port: &str,
        algorithm: &str,
        fingerprint: &str,
    ) -> std::io::Result<()> {
        self.entries
            .retain(|e| !(e.host_port == host_port && e.algorithm == algorithm));
        self.entries.push(Entry {
            host_port: host_port.to_string(),
            algorithm: algorithm.to_string(),
            fingerprint: fingerprint.to_string(),
        });
        write_file(&self.path, &self.entries)
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

fn read_file(path: &Path) -> std::io::Result<Vec<Entry>> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let host_port = match parts.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let algorithm = match parts.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let fingerprint = match parts.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        out.push(Entry {
            host_port,
            algorithm,
            fingerprint,
        });
    }
    Ok(out)
}

fn write_file(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf: Vec<u8> = Vec::new();
    writeln!(buf, "# mc-rs known_hosts (TOFU)")?;
    for e in entries {
        writeln!(buf, "{} {} {}", e.host_port, e.algorithm, e.fingerprint)?;
    }
    // Atomic write + owner-only perms so a hostile co-tenant on a shared
    // machine cannot tamper with our recorded fingerprints (which would
    // enable a silent MITM on the next connection).
    let tmp = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        PathBuf::from(s)
    };
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&buf)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn missing_file_is_empty() {
        let td = fixture_dir();
        let kh = KnownHosts::load(td.path().join("nope/known_hosts"));
        assert!(matches!(
            kh.check("example.com:22", "ssh-ed25519", "AAAA"),
            CheckResult::NewHost
        ));
    }

    #[test]
    fn record_then_check_match() {
        let td = fixture_dir();
        let mut kh = KnownHosts::load(td.path().join("known_hosts"));
        kh.record("h:22", "ssh-ed25519", "abc123").unwrap();
        let kh = KnownHosts::load(td.path().join("known_hosts"));
        assert!(matches!(
            kh.check("h:22", "ssh-ed25519", "abc123"),
            CheckResult::Match
        ));
    }

    #[test]
    fn mismatch_detected() {
        let td = fixture_dir();
        let mut kh = KnownHosts::load(td.path().join("known_hosts"));
        kh.record("h:22", "ssh-ed25519", "GOOD").unwrap();
        match kh.check("h:22", "ssh-ed25519", "BAD") {
            CheckResult::Mismatch { recorded } => assert_eq!(recorded, "GOOD"),
            other => panic!("expected mismatch, got {other:?}"),
        }
    }

    #[test]
    fn rerecord_replaces() {
        let td = fixture_dir();
        let mut kh = KnownHosts::load(td.path().join("known_hosts"));
        kh.record("h:22", "ssh-ed25519", "v1").unwrap();
        kh.record("h:22", "ssh-ed25519", "v2").unwrap();
        let kh2 = KnownHosts::load(td.path().join("known_hosts"));
        assert_eq!(kh2.entries().len(), 1);
        assert_eq!(kh2.entries()[0].fingerprint, "v2");
    }
}
