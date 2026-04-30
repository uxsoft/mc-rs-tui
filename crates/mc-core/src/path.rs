//! Layered URI-style virtual paths.
//!
//! Each layer is `scheme:location` plus an optional sub-path. Layers are joined
//! by `!` to express archive nesting, e.g.
//! `local:/tmp/a.tar.gz!targz:/dir/file.txt`
//! `sftp://me@host:22/srv/a.tar!tar:/dir/x`

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Layer {
    pub scheme: String,
    /// e.g. host or empty for non-network schemes
    pub location: String,
    pub sub: PathBuf,
}

impl Layer {
    pub fn local(sub: impl Into<PathBuf>) -> Self {
        Self {
            scheme: "local".into(),
            location: String::new(),
            sub: sub.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VPath {
    layers: SmallVec<[Layer; 2]>,
}

pub type VPathBuf = VPath;

impl VPath {
    pub fn new(layers: impl IntoIterator<Item = Layer>) -> Self {
        Self {
            layers: layers.into_iter().collect(),
        }
    }

    pub fn local(p: impl Into<PathBuf>) -> Self {
        Self::new([Layer::local(p)])
    }

    #[must_use]
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    #[must_use]
    pub fn last(&self) -> Option<&Layer> {
        self.layers.last()
    }

    pub fn push_layer(&mut self, layer: Layer) {
        self.layers.push(layer);
    }

    pub fn pop_layer(&mut self) -> Option<Layer> {
        self.layers.pop()
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, layer) in self.layers.iter().enumerate() {
            if i > 0 {
                f.write_str("!")?;
            }
            if layer.location.is_empty() {
                write!(f, "{}:{}", layer.scheme, layer.sub.display())?;
            } else {
                write!(f, "{}://{}{}", layer.scheme, layer.location, layer.sub.display())?;
            }
        }
        Ok(())
    }
}

impl FromStr for VPath {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Err(Error::InvalidPath("empty".into()));
        }
        let mut layers = SmallVec::new();
        for raw in s.split('!') {
            layers.push(parse_layer(raw)?);
        }
        Ok(Self { layers })
    }
}

fn parse_layer(s: &str) -> Result<Layer> {
    // scheme://location/sub  or  scheme:/sub
    let (scheme, rest) = s
        .split_once(':')
        .ok_or_else(|| Error::InvalidPath(format!("missing scheme in {s:?}")))?;
    if scheme.is_empty() {
        return Err(Error::InvalidPath(format!("empty scheme in {s:?}")));
    }
    if let Some(after_authority) = rest.strip_prefix("//") {
        let (location, sub) = match after_authority.find('/') {
            Some(i) => (&after_authority[..i], &after_authority[i..]),
            None => (after_authority, "/"),
        };
        Ok(Layer {
            scheme: scheme.to_owned(),
            location: location.to_owned(),
            sub: PathBuf::from(sub),
        })
    } else {
        Ok(Layer {
            scheme: scheme.to_owned(),
            location: String::new(),
            sub: PathBuf::from(rest),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local() {
        let p: VPath = "local:/home/me/x.txt".parse().unwrap();
        assert_eq!(p.layers().len(), 1);
        assert_eq!(p.layers()[0].scheme, "local");
        assert_eq!(p.layers()[0].location, "");
        assert_eq!(p.layers()[0].sub.to_str().unwrap(), "/home/me/x.txt");
    }

    #[test]
    fn parse_sftp() {
        let p: VPath = "sftp://me@host:22/srv/a.tar".parse().unwrap();
        assert_eq!(p.layers()[0].scheme, "sftp");
        assert_eq!(p.layers()[0].location, "me@host:22");
        assert_eq!(p.layers()[0].sub.to_str().unwrap(), "/srv/a.tar");
    }

    #[test]
    fn parse_layered() {
        let p: VPath = "local:/tmp/a.tar.gz!targz:/dir/file".parse().unwrap();
        assert_eq!(p.layers().len(), 2);
        assert_eq!(p.layers()[1].scheme, "targz");
        assert_eq!(p.layers()[1].sub.to_str().unwrap(), "/dir/file");
    }

    #[test]
    fn round_trip() {
        let cases = [
            "local:/home/me/x",
            "sftp://me@host:22/srv/a.tar",
            "local:/a.tar!tar:/inner.zip!zip:/x.txt",
        ];
        for s in cases {
            let p: VPath = s.parse().unwrap();
            assert_eq!(p.to_string(), s);
        }
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<VPath>().is_err());
        assert!(":x".parse::<VPath>().is_err());
    }
}
