//! Persistent directory bookmarks.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hotlist {
    #[serde(default)]
    pub entries: Vec<HotlistEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotlistEntry {
    pub label: String,
    /// Stored as a [`mc_core::VPath`] display string.
    pub path: String,
}

impl Hotlist {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        crate::io::load_toml_or_default(path)
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        crate::io::write_user_file_atomic(path, s.as_bytes())
    }

    pub fn add(&mut self, label: String, path: String) {
        // Replace if same path already there.
        self.entries.retain(|e| e.path != path);
        self.entries.push(HotlistEntry { label, path });
    }

    pub fn remove_at(&mut self, idx: usize) {
        if idx < self.entries.len() {
            self.entries.remove(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("hot.toml");
        let mut h = Hotlist::default();
        h.add("home".into(), "local:/home/me".into());
        h.add("work".into(), "local:/work".into());
        h.save(&p).unwrap();

        let loaded = Hotlist::load(&p).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].label, "home");
    }

    #[test]
    fn add_replaces_duplicate() {
        let mut h = Hotlist::default();
        h.add("a".into(), "local:/x".into());
        h.add("a-2".into(), "local:/x".into());
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0].label, "a-2");
    }

    #[test]
    fn missing_file_yields_empty() {
        let h = Hotlist::load(std::path::Path::new("/no/such/path.toml")).unwrap();
        assert!(h.entries.is_empty());
    }
}
