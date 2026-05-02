//! Top-level application configuration (TOML root).

use std::path::Path;

use crate::core::action::SortKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub panels: PanelsConfig,
    pub options: OptionsConfig,
    pub editor: EditorConfig,
    pub layout: LayoutConfig,
    pub vfs: VfsConfig,
    /// Last-saved snapshot of the left/right panel UI state. Restored on
    /// startup. Populated by "Save setup".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_left: Option<PanelStateSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_right: Option<PanelStateSnapshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelsConfig {
    pub show_hidden: bool,
    pub mix_dirs: bool,
    pub case_sensitive_sort: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionsConfig {
    pub use_internal_view: bool,
    pub confirm_delete: bool,
    pub confirm_overwrite: bool,
    pub confirm_exit: bool,
    pub confirm_execute: bool,
    /// Per-row Nerd Font glyphs in panel listings. On by default — set to
    /// `false` if your terminal lacks a Nerd Font.
    pub icons: bool,
    /// Show git status indicators next to filenames inside repos. On by
    /// default — set to `false` to skip the per-refresh `git status` call.
    pub git_status: bool,
}

impl Default for OptionsConfig {
    fn default() -> Self {
        Self {
            use_internal_view: true,
            confirm_delete: true,
            confirm_overwrite: true,
            confirm_exit: true,
            confirm_execute: false,
            icons: true,
            git_status: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// External editor command. Defaults to `hx`, falls back to `$EDITOR`.
    pub command: Option<String>,
    /// Argument template for `+lineno` style invocation, e.g. `"+%lineno"`.
    pub line_template: String,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            command: None,
            line_template: "+%lineno".into(),
        }
    }
}

/// Two-panel split orientation and ratio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// `true` stacks the panels vertically (left on top, right on bottom).
    /// `false` (default) is the classic side-by-side layout.
    pub vertical: bool,
    /// Percentage of width (or height in vertical mode) given to the left
    /// (or top) panel. Clamped to 1..=99 at apply time.
    pub left_pct: u8,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            vertical: false,
            left_pct: 50,
        }
    }
}

/// Tunables for the remote/virtual filesystems.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VfsConfig {
    /// Default password used for anonymous FTP logins.
    pub ftp_anonymous_password: String,
    /// Connection timeout in seconds. `0` disables the timeout.
    pub timeout_secs: u32,
}

impl Default for VfsConfig {
    fn default() -> Self {
        Self {
            ftp_anonymous_password: "anonymous@".into(),
            timeout_secs: 30,
        }
    }
}

/// Persisted per-panel UI state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelStateSnapshot {
    pub sort_by: SortKey,
    pub reverse: bool,
    pub listing: String, // "Full" | "Brief" | "Long" | "Tree"
    pub show_hidden: bool,
    pub mix_dirs: bool,
    pub filter: Option<String>,
}

impl Default for PanelStateSnapshot {
    fn default() -> Self {
        Self {
            sort_by: SortKey::Name,
            reverse: false,
            listing: "Full".into(),
            show_hidden: false,
            mix_dirs: false,
            filter: None,
        }
    }
}

impl AppConfig {
    /// Parse from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Load from `path`, falling back to `Self::default()` when the file is
    /// absent. A malformed file returns `InvalidData`.
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        crate::config::io::load_toml_or_default(path)
    }

    /// Serialize to TOML and write to `path` atomically (tempfile + rename;
    /// `0600` perms on Unix). Creates parent dirs as needed.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        crate::config::io::write_user_file_atomic(path, s.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = AppConfig::default();
        assert!(c.options.confirm_delete);
        assert_eq!(c.editor.line_template, "+%lineno");
        assert_eq!(c.layout.left_pct, 50);
        assert_eq!(c.vfs.timeout_secs, 30);
    }

    #[test]
    fn parse_partial() {
        let c = AppConfig::from_toml(
            r#"
            [panels]
            show_hidden = true
        "#,
        )
        .unwrap();
        assert!(c.panels.show_hidden);
        assert!(c.options.confirm_delete);
    }

    #[test]
    fn round_trip_save() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("conf.toml");
        let mut c = AppConfig::default();
        c.layout.vertical = true;
        c.layout.left_pct = 30;
        c.save(&p).unwrap();
        let l = AppConfig::load(&p).unwrap();
        assert!(l.layout.vertical);
        assert_eq!(l.layout.left_pct, 30);
    }
}
