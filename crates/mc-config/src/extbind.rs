//! Filename → command mapping (mc.ext.ini equivalent), TOML-driven.
//!
//! Example config (`~/.config/mc-rs/extbind.toml`):
//!
//! ```toml
//! [[bind]]
//! glob = "*.{png,jpg,jpeg,gif,webp}"
//! open = "feh %f"
//!
//! [[bind]]
//! glob = "*.pdf"
//! open = "zathura %f"
//! ```
//!
//! Each rule has an optional `open` (Enter), `view` (F3), and `edit` (F4)
//! template. Templates use the same `%f %d %p %t` macros as the user menu.

use std::path::Path;

use globset::GlobMatcher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtBindings {
    #[serde(default)]
    pub bind: Vec<ExtBindRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtBindRule {
    /// Filename glob (case-insensitive).
    pub glob: String,
    pub open: Option<String>,
    pub view: Option<String>,
    pub edit: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ExtAction {
    Open,
    View,
    Edit,
}

#[derive(Debug)]
pub struct CompiledExtBindings {
    rules: Vec<(GlobMatcher, ExtBindRule)>,
}

impl CompiledExtBindings {
    pub fn from_config(cfg: &ExtBindings) -> Self {
        let mut rules = Vec::new();
        for r in &cfg.bind {
            match build_glob(&r.glob) {
                Ok(g) => rules.push((g, r.clone())),
                Err(e) => tracing::warn!("extbind: bad glob {:?}: {e}", r.glob),
            }
        }
        Self { rules }
    }

    /// Look up the template for `name` + `action`. Returns `None` if no rule matches.
    #[must_use]
    pub fn lookup(&self, name: &str, action: ExtAction) -> Option<&str> {
        for (g, r) in &self.rules {
            if g.is_match(name) {
                let t = match action {
                    ExtAction::Open => r.open.as_deref(),
                    ExtAction::View => r.view.as_deref(),
                    ExtAction::Edit => r.edit.as_deref(),
                };
                if t.is_some() {
                    return t;
                }
            }
        }
        None
    }
}

impl ExtBindings {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::defaults()),
            Err(e) => Err(e),
        }
    }

    /// Default ruleset shipped with the binary.
    #[must_use]
    pub fn defaults() -> Self {
        Self {
            bind: vec![
                ExtBindRule {
                    glob: "*.{png,jpg,jpeg,gif,webp,bmp,svg}".into(),
                    open: Some("xdg-open %f".into()),
                    view: None,
                    edit: None,
                },
                ExtBindRule {
                    glob: "*.pdf".into(),
                    open: Some("xdg-open %f".into()),
                    view: None,
                    edit: None,
                },
                ExtBindRule {
                    glob: "*.html".into(),
                    open: Some("xdg-open %f".into()),
                    view: None,
                    edit: None,
                },
            ],
        }
    }
}

fn build_glob(pat: &str) -> Result<GlobMatcher, globset::Error> {
    let mut b = globset::GlobBuilder::new(pat);
    b.case_insensitive(true);
    b.literal_separator(false);
    Ok(b.build()?.compile_matcher())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_image_open() {
        let cfg = ExtBindings::defaults();
        let c = CompiledExtBindings::from_config(&cfg);
        assert_eq!(c.lookup("foo.PNG", ExtAction::Open), Some("xdg-open %f"));
        assert_eq!(c.lookup("foo.txt", ExtAction::Open), None);
    }

    #[test]
    fn parse_toml() {
        let cfg = ExtBindings::from_toml(
            r#"
            [[bind]]
            glob = "*.rs"
            view = "bat %f"
            "#,
        )
        .unwrap();
        let c = CompiledExtBindings::from_config(&cfg);
        assert_eq!(c.lookup("lib.rs", ExtAction::View), Some("bat %f"));
        assert_eq!(c.lookup("lib.rs", ExtAction::Open), None);
    }

    #[test]
    fn falls_back_when_action_missing() {
        let cfg = ExtBindings::from_toml(
            r#"
            [[bind]]
            glob = "*"
            view = "less %f"
            "#,
        )
        .unwrap();
        let c = CompiledExtBindings::from_config(&cfg);
        assert_eq!(c.lookup("anything", ExtAction::View), Some("less %f"));
        assert_eq!(c.lookup("anything", ExtAction::Open), None);
    }
}
