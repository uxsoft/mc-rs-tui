//! Network-backed VFS backends.

pub mod dav;
pub mod ftp;
pub mod known_hosts;
pub mod sftp;

pub use dav::DavVfs;
pub use ftp::FtpVfs;
pub use known_hosts::{CheckResult, KnownHosts};
pub use sftp::SftpVfs;
