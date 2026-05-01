//! User-defined key remap.
//!
//! TOML schema (loaded from `$XDG_CONFIG_HOME/mc-rs/keymap.toml`):
//!
//! ```toml
//! [[remap]]
//! from = "C-d"
//! to   = "F8"
//!
//! [[remap]]
//! from = "M-x"
//! to   = "F9"
//! ```
//!
//! Each `from` chord (mc-style: `C-x`, `M-?`, `F1`, `S-Tab`, plain printable)
//! is translated to the `to` chord *before* the App dispatches. The dispatcher
//! continues to use match arms; users can rebind without recompiling.

use std::collections::HashMap;
use std::path::Path;

use mc_core::key::KeyChord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeymapFile {
    #[serde(default)]
    pub remap: Vec<RemapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemapEntry {
    pub from: String,
    pub to: String,
}

/// Compiled, fast-lookup remap table.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    table: HashMap<KeyChord, KeyChord>,
}

impl Keymap {
    pub fn from_file(file: &KeymapFile) -> Self {
        let mut table = HashMap::with_capacity(file.remap.len());
        for r in &file.remap {
            let from: KeyChord = match r.from.parse() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("keymap remap: bad `from` {:?}: {e}", r.from);
                    continue;
                }
            };
            let to: KeyChord = match r.to.parse() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("keymap remap: bad `to` {:?}: {e}", r.to);
                    continue;
                }
            };
            table.insert(from, to);
        }
        Self { table }
    }

    /// Apply the remap. Returns `chord` unchanged if no rule matches.
    #[must_use]
    pub fn translate(&self, chord: KeyChord) -> KeyChord {
        self.table.get(&chord).copied().unwrap_or(chord)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let cfg: KeymapFile = crate::io::load_toml_or_default(path)?;
        Ok(Self::from_file(&cfg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::key::{KeyCode, KeyMods};

    #[test]
    fn empty_table_is_identity() {
        let km = Keymap::default();
        let chord = KeyChord::new(KeyCode::Char('q'), KeyMods::CTRL);
        assert_eq!(km.translate(chord), chord);
    }

    #[test]
    fn remaps_apply() {
        let cfg = KeymapFile {
            remap: vec![
                RemapEntry {
                    from: "C-d".into(),
                    to: "F8".into(),
                },
                RemapEntry {
                    from: "M-x".into(),
                    to: "F9".into(),
                },
            ],
        };
        let km = Keymap::from_file(&cfg);
        let cd = KeyChord::new(KeyCode::Char('d'), KeyMods::CTRL);
        let mx = KeyChord::new(KeyCode::Char('x'), KeyMods::ALT);
        let f8 = KeyChord::plain(KeyCode::F(8));
        let f9 = KeyChord::plain(KeyCode::F(9));
        assert_eq!(km.translate(cd), f8);
        assert_eq!(km.translate(mx), f9);
        // Unmapped chord falls through.
        let other = KeyChord::plain(KeyCode::Tab);
        assert_eq!(km.translate(other), other);
    }

    #[test]
    fn bad_chord_is_skipped() {
        let cfg = KeymapFile {
            remap: vec![
                RemapEntry {
                    from: "not a chord".into(),
                    to: "F1".into(),
                },
                RemapEntry {
                    from: "F2".into(),
                    to: "F1".into(),
                },
            ],
        };
        let km = Keymap::from_file(&cfg);
        let f2 = KeyChord::plain(KeyCode::F(2));
        let f1 = KeyChord::plain(KeyCode::F(1));
        assert_eq!(km.translate(f2), f1);
    }

    #[test]
    fn missing_file_yields_default() {
        let km = Keymap::load(std::path::Path::new("/no/such/path.toml")).unwrap();
        let chord = KeyChord::plain(KeyCode::Tab);
        assert_eq!(km.translate(chord), chord);
    }
}
