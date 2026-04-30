//! TOML skin loader.
//!
//! Lets users override panel/file colors. Phase-12 first cut: we ship a small
//! schema that the panel renderer reads at draw time.
//!
//! Example `~/.config/mc-rs/skin.toml`:
//!
//! ```toml
//! [panel]
//! background      = "blue"
//! border          = "white"
//! active_border   = "cyan"
//! cursor_bg       = "cyan"
//! cursor_fg       = "black"
//! marked_fg       = "yellow"
//!
//! [groups]
//! archive = "red"
//! image   = "magenta"
//! audio   = "lightmagenta"
//! video   = "lightmagenta"
//! doc     = "white"
//! source  = "lightcyan"
//! build   = "yellow"
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkinFile {
    pub panel: PanelSection,
    pub groups: HashMap<String, String>,
}

impl Default for SkinFile {
    fn default() -> Self {
        Self {
            panel: PanelSection::default(),
            groups: default_groups(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSection {
    pub background: String,
    pub border: String,
    pub active_border: String,
    pub cursor_bg: String,
    pub cursor_fg: String,
    pub marked_fg: String,
}

impl Default for PanelSection {
    fn default() -> Self {
        Self {
            background: "blue".into(),
            border: "white".into(),
            active_border: "cyan".into(),
            cursor_bg: "cyan".into(),
            cursor_fg: "black".into(),
            marked_fg: "yellow".into(),
        }
    }
}

fn default_groups() -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in [
        ("archive", "red"),
        ("image", "magenta"),
        ("audio", "lightmagenta"),
        ("video", "lightmagenta"),
        ("doc", "white"),
        ("source", "lightcyan"),
        ("build", "yellow"),
    ] {
        m.insert(k.into(), v.into());
    }
    m
}

impl SkinFile {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }
}

/// Parse a color name into a `(r, g, b)`-style indexed color. We use a simple
/// 16-color name table. Returns the closest ANSI color; unknown names fall back
/// to white.
#[must_use]
pub fn parse_color_name(name: &str) -> AnsiColor {
    match name.trim().to_ascii_lowercase().as_str() {
        "black" => AnsiColor::Black,
        "red" => AnsiColor::Red,
        "green" => AnsiColor::Green,
        "yellow" => AnsiColor::Yellow,
        "blue" => AnsiColor::Blue,
        "magenta" | "purple" => AnsiColor::Magenta,
        "cyan" => AnsiColor::Cyan,
        "white" | "gray" | "grey" => AnsiColor::White,
        "darkgray" | "darkgrey" => AnsiColor::DarkGray,
        "lightred" => AnsiColor::LightRed,
        "lightgreen" => AnsiColor::LightGreen,
        "lightyellow" => AnsiColor::LightYellow,
        "lightblue" => AnsiColor::LightBlue,
        "lightmagenta" | "lightpurple" => AnsiColor::LightMagenta,
        "lightcyan" => AnsiColor::LightCyan,
        _ => AnsiColor::White,
    }
}

/// Backend-neutral color enum so [`mc-config`] doesn't depend on ratatui.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = SkinFile::default();
        assert_eq!(s.panel.background, "blue");
        assert_eq!(s.groups.get("archive").map(String::as_str), Some("red"));
    }

    #[test]
    fn parse_partial_keeps_defaults() {
        let s = SkinFile::from_toml(
            r#"
            [panel]
            background = "black"
            "#,
        )
        .unwrap();
        assert_eq!(s.panel.background, "black");
        // Other panel fields keep defaults.
        assert_eq!(s.panel.cursor_bg, "cyan");
    }

    #[test]
    fn missing_file_yields_default() {
        let s = SkinFile::load(std::path::Path::new("/no/such/path.toml")).unwrap();
        assert_eq!(s.panel.background, "blue");
    }

    #[test]
    fn color_parser() {
        assert_eq!(parse_color_name("Cyan"), AnsiColor::Cyan);
        assert_eq!(parse_color_name("lightblue"), AnsiColor::LightBlue);
        assert_eq!(parse_color_name("nope"), AnsiColor::White);
    }
}
