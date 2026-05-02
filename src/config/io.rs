//! Shared TOML load + atomic-write helpers used by every config file.
//!
//! Centralised here so that:
//! - missing files vs. malformed files are handled the same way everywhere
//!   (NotFound → defaults; parse error → `InvalidData` surfaced to caller),
//! - persisted dotfiles are written atomically (tempfile + rename) so two
//!   instances racing never truncate each other, and on Unix end up with
//!   `0600` permissions so credentials, hotlist paths, and command history
//!   aren't world-readable on shared hosts.

use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;

/// Read a TOML file from `path` and deserialize it into `T`.
///
/// - Missing file → `T::default()`.
/// - Present but malformed → `Err(InvalidData)` so the caller can log it.
/// - Other I/O errors are surfaced unchanged.
pub fn load_toml_or_default<T>(path: &Path) -> std::io::Result<T>
where
    T: Default + DeserializeOwned,
{
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

/// Atomic, owner-only write of `contents` to `path`.
///
/// Creates parent directories as needed, writes to `<path>.tmp`, fsyncs, and
/// renames into place. On Unix the destination is then chmod'd to `0o600` so
/// any sensitive content (history, hotlist with SFTP targets, fingerprints in
/// known_hosts) is not world-readable.
pub fn write_user_file_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
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

fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
    struct Sample {
        n: u32,
        s: String,
    }

    #[test]
    fn missing_file_returns_default() {
        let s: Sample = load_toml_or_default(Path::new("/no/such/file.toml")).unwrap();
        assert_eq!(s, Sample::default());
    }

    #[test]
    fn malformed_returns_invalid_data() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("bad.toml");
        std::fs::write(&p, b"this is = not valid = toml = at all").unwrap();
        let err = load_toml_or_default::<Sample>(&p).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn atomic_write_round_trip() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("nested").join("file.toml");
        write_user_file_atomic(&p, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        // tmp file should be gone after success
        let mut tmp = p.clone().into_os_string();
        tmp.push(".tmp");
        assert!(!std::path::Path::new(&tmp).exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("perms.toml");
        write_user_file_atomic(&p, b"x").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
