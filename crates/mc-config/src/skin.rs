//! TOML skin loader.
//!
//! Picks one of the built-in themes and optionally overrides individual
//! semantic colors. All colors are 24-bit RGB hex (`#rrggbb` or `#rgb`),
//! or `"reset"` / `"default"` for the terminal's default.
//!
//! Example `~/.config/mc-rs/skin.toml`:
//!
//! ```toml
//! theme = "modern-dark"   # "modern-dark" | "tokyo-night" | "solarized-light"
//!
//! [colors]                # optional per-field overrides
//! panel_bg  = "#101018"
//! cursor_bg = "#ffaf00"
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::scheme::{ColorScheme, apply_override};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkinFile {
    pub theme: String,
    pub colors: HashMap<String, String>,
}

impl Default for SkinFile {
    fn default() -> Self {
        Self {
            theme: "modern-dark".into(),
            colors: HashMap::new(),
        }
    }
}

impl SkinFile {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        crate::io::load_toml_or_default(path)
    }

    /// Serialize to TOML and write to `path` atomically (tempfile + rename;
    /// `0600` perms on Unix). Creates parent dirs as needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let s = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        crate::io::write_user_file_atomic(path, s.as_bytes())
    }

    /// Resolve to a concrete [`ColorScheme`]: pick the named base theme,
    /// then apply any per-field overrides from `[colors]`. Bad theme names
    /// fall back to the default; unknown override keys / unparseable values
    /// are skipped (callers may inspect the returned warnings).
    #[must_use]
    pub fn resolve(&self) -> (ColorScheme, Vec<String>) {
        let mut warnings = Vec::new();
        let mut scheme = match ColorScheme::from_named(&self.theme) {
            Some(s) => s,
            None => {
                warnings.push(format!(
                    "skin: unknown theme '{}', falling back to 'modern-dark'",
                    self.theme
                ));
                ColorScheme::modern_dark()
            }
        };
        for (key, value) in &self.colors {
            if let Err(msg) = apply_override(&mut scheme, key, value) {
                warnings.push(format!("skin: {msg}"));
            }
        }
        (scheme, warnings)
    }
}

/// 24-bit RGB color, or "use the terminal's default".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeColor {
    Rgb(u8, u8, u8),
    Reset,
}

impl ThemeColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb(r, g, b)
    }
}

/// Parse a color string. Accepts `#rrggbb`, `#rgb`, or `reset`/`default`.
///
/// # Errors
/// Returns a human-readable message if the string is not a recognised form.
pub fn parse_color(s: &str) -> Result<ThemeColor, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower == "reset" || lower == "default" {
        return Ok(ThemeColor::Reset);
    }
    let hex = t
        .strip_prefix('#')
        .ok_or_else(|| format!("color '{s}' must be #rrggbb, #rgb, or 'reset'"))?;
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
            Ok(ThemeColor::Rgb(r, g, b))
        }
        3 => {
            let parse_nib = |c: char| -> Result<u8, String> {
                c.to_digit(16)
                    .map(|n| (n as u8) * 0x11)
                    .ok_or_else(|| format!("invalid hex digit '{c}'"))
            };
            let mut chars = hex.chars();
            let r = parse_nib(chars.next().unwrap())?;
            let g = parse_nib(chars.next().unwrap())?;
            let b = parse_nib(chars.next().unwrap())?;
            Ok(ThemeColor::Rgb(r, g, b))
        }
        _ => Err(format!("color '{s}' must be #rrggbb or #rgb")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = SkinFile::default();
        assert_eq!(s.theme, "modern-dark");
        assert!(s.colors.is_empty());
    }

    #[test]
    fn parse_partial_keeps_defaults() {
        let s = SkinFile::from_toml(
            r#"
            theme = "tokyo-night"
            "#,
        )
        .unwrap();
        assert_eq!(s.theme, "tokyo-night");
        assert!(s.colors.is_empty());
    }

    #[test]
    fn parse_with_overrides() {
        let s = SkinFile::from_toml(
            r##"
            theme = "modern-dark"
            [colors]
            panel_bg  = "#101018"
            cursor_bg = "#ffaf00"
            "##,
        )
        .unwrap();
        assert_eq!(
            s.colors.get("panel_bg").map(String::as_str),
            Some("#101018")
        );
    }

    #[test]
    fn missing_file_yields_default() {
        let s = SkinFile::load(Path::new("/no/such/path.toml")).unwrap();
        assert_eq!(s.theme, "modern-dark");
    }

    #[test]
    fn parse_color_hex() {
        assert_eq!(
            parse_color("#1e1e2e"),
            Ok(ThemeColor::Rgb(0x1e, 0x1e, 0x2e))
        );
        assert_eq!(parse_color("#FFF"), Ok(ThemeColor::Rgb(0xff, 0xff, 0xff)));
        assert_eq!(
            parse_color("  #abc  "),
            Ok(ThemeColor::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert_eq!(parse_color("reset"), Ok(ThemeColor::Reset));
        assert_eq!(parse_color("Default"), Ok(ThemeColor::Reset));
        assert!(parse_color("blue").is_err());
        assert!(parse_color("#zz0000").is_err());
        assert!(parse_color("#1234").is_err());
    }

    #[test]
    fn resolve_default_theme() {
        let s = SkinFile::default();
        let (scheme, warnings) = s.resolve();
        assert!(warnings.is_empty());
        assert_eq!(scheme.panel_bg, ColorScheme::modern_dark().panel_bg);
    }

    #[test]
    fn resolve_unknown_theme_warns() {
        let mut s = SkinFile::default();
        s.theme = "no-such-theme".into();
        let (_, warnings) = s.resolve();
        assert!(warnings.iter().any(|w| w.contains("unknown theme")));
    }

    #[test]
    fn resolve_applies_override() {
        let mut s = SkinFile::default();
        s.colors.insert("panel_bg".into(), "#010203".into());
        let (scheme, warnings) = s.resolve();
        assert!(warnings.is_empty());
        assert_eq!(scheme.panel_bg, ThemeColor::Rgb(1, 2, 3));
    }

    #[test]
    fn resolve_unknown_field_warns() {
        let mut s = SkinFile::default();
        s.colors.insert("not_a_field".into(), "#000000".into());
        let (_, warnings) = s.resolve();
        assert!(warnings.iter().any(|w| w.contains("not_a_field")));
    }

    #[test]
    fn resolve_bad_color_warns() {
        let mut s = SkinFile::default();
        s.colors.insert("panel_bg".into(), "puce".into());
        let (_, warnings) = s.resolve();
        assert!(warnings.iter().any(|w| w.contains("panel_bg")));
    }

    #[test]
    fn save_round_trip_preserves_theme_and_overrides() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("skin.toml");
        let s = SkinFile {
            theme: "tokyo-night".into(),
            colors: [("panel_bg".into(), "#101018".into())].into(),
        };
        s.save(&p).unwrap();
        let loaded = SkinFile::load(&p).unwrap();
        assert_eq!(loaded.theme, "tokyo-night");
        assert_eq!(
            loaded.colors.get("panel_bg").map(String::as_str),
            Some("#101018")
        );
    }

    #[test]
    fn available_themes_all_resolve() {
        for (name, _) in ColorScheme::available_themes() {
            assert!(
                ColorScheme::from_named(name).is_some(),
                "theme '{name}' missing from from_named()"
            );
        }
    }
}
