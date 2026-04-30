//! VFS registry: scheme → backend.

use std::collections::HashMap;
use std::sync::Arc;

use mc_core::{Error, Result, VPath};

use crate::local::LocalVfs;
use crate::trait_::Vfs;

#[derive(Clone, Default)]
pub struct Registry {
    by_scheme: HashMap<String, Arc<dyn Vfs>>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a registry pre-populated with the local backend.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register("local", LocalVfs::shared());
        r
    }

    pub fn register(&mut self, scheme: impl Into<String>, vfs: Arc<dyn Vfs>) {
        self.by_scheme.insert(scheme.into(), vfs);
    }

    pub fn root_for(&self, p: &VPath) -> Result<Arc<dyn Vfs>> {
        let layer = p
            .layers()
            .first()
            .ok_or_else(|| Error::InvalidPath("empty vpath".into()))?;
        self.by_scheme
            .get(&layer.scheme)
            .cloned()
            .ok_or_else(|| Error::InvalidPath(format!("no backend for scheme {:?}", layer.scheme)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_backend_registered() {
        let r = Registry::with_defaults();
        let p = VPath::local("/tmp");
        assert!(r.root_for(&p).is_ok());
    }

    #[test]
    fn unknown_scheme_errors() {
        let r = Registry::with_defaults();
        let p: VPath = "ftp://host/x".parse().unwrap();
        assert!(r.root_for(&p).is_err());
    }
}
