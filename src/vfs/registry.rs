//! VFS registry: scheme → backend.

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::{Error, Result, VPath};

use crate::vfs::local::LocalVfs;
use crate::vfs::trait_::Vfs;

#[derive(Clone, Default)]
pub struct Registry {
    /// Generic scheme handlers (one per scheme, e.g. `local`).
    by_scheme: HashMap<String, Arc<dyn Vfs>>,
    /// Per-(scheme, location) handlers — used for mounted archives where each
    /// instance owns a distinct backing file.
    by_pair: HashMap<(String, String), Arc<dyn Vfs>>,
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

    /// Register a per-mount backend keyed by `(scheme, location)`.
    pub fn register_mount(
        &mut self,
        scheme: impl Into<String>,
        location: impl Into<String>,
        vfs: Arc<dyn Vfs>,
    ) {
        self.by_pair.insert((scheme.into(), location.into()), vfs);
    }

    /// Forget a per-mount backend.
    pub fn unregister_mount(&mut self, scheme: &str, location: &str) {
        self.by_pair
            .remove(&(scheme.to_string(), location.to_string()));
    }

    /// Snapshot of all per-(scheme, location) mounts. The order is
    /// implementation-defined; callers should sort if a stable display order
    /// is required.
    #[must_use]
    pub fn mounts(&self) -> Vec<(String, String)> {
        self.by_pair.keys().cloned().collect()
    }

    /// Resolve the backend for the *last* layer of `p` (the deepest archive).
    pub fn root_for(&self, p: &VPath) -> Result<Arc<dyn Vfs>> {
        let layer = p
            .layers()
            .last()
            .ok_or_else(|| Error::InvalidPath("empty vpath".into()))?;
        if let Some(v) = self
            .by_pair
            .get(&(layer.scheme.clone(), layer.location.clone()))
        {
            return Ok(v.clone());
        }
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
