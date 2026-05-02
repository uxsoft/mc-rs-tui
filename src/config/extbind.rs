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
    /// Filename glob (case-insensitive). At least one of `glob` or `mime`
    /// must match for the rule to apply. An empty glob is ignored.
    #[serde(default)]
    pub glob: String,
    /// MIME glob (e.g. `image/*`, `text/plain`). Matched against the file's
    /// detected MIME type when supplied. `None` (or empty) skips MIME match.
    #[serde(default)]
    pub mime: Option<String>,
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
struct CompiledRule {
    glob: Option<GlobMatcher>,
    mime: Option<GlobMatcher>,
    rule: ExtBindRule,
}

#[derive(Debug)]
pub struct CompiledExtBindings {
    rules: Vec<CompiledRule>,
}

impl CompiledExtBindings {
    pub fn from_config(cfg: &ExtBindings) -> Self {
        let mut rules = Vec::new();
        for r in &cfg.bind {
            let glob = if r.glob.is_empty() {
                None
            } else {
                match build_glob(&r.glob) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        tracing::warn!("extbind: bad glob {:?}: {e}", r.glob);
                        continue;
                    }
                }
            };
            let mime = match r.mime.as_deref().filter(|s| !s.is_empty()) {
                Some(m) => match build_glob(m) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        tracing::warn!("extbind: bad mime glob {:?}: {e}", m);
                        continue;
                    }
                },
                None => None,
            };
            if glob.is_none() && mime.is_none() {
                tracing::warn!("extbind: rule has neither glob nor mime; skipping");
                continue;
            }
            rules.push(CompiledRule {
                glob,
                mime,
                rule: r.clone(),
            });
        }
        Self { rules }
    }

    /// Look up the template for `name` + optional MIME + `action`. Returns
    /// `None` if no rule matches. A rule matches when *any* of its provided
    /// matchers (glob or mime) matches; rules with both must match both.
    #[must_use]
    pub fn lookup(&self, name: &str, mime: Option<&str>, action: ExtAction) -> Option<&str> {
        for cr in &self.rules {
            let glob_ok = match &cr.glob {
                Some(g) => g.is_match(name),
                None => true,
            };
            let mime_ok = match (&cr.mime, mime) {
                (Some(m), Some(actual)) => m.is_match(actual),
                (Some(_), None) => false,
                (None, _) => true,
            };
            // Require at least one of the two matchers actually matched on
            // its own merits (avoids "no glob, no mime → match anything").
            let glob_active = cr.glob.is_some();
            let mime_active = cr.mime.is_some();
            let any_match = (glob_active && glob_ok) || (mime_active && mime_ok);
            if !any_match {
                continue;
            }
            // If both are specified, both must match.
            if glob_active && mime_active && !(glob_ok && mime_ok) {
                continue;
            }
            let t = match action {
                ExtAction::Open => cr.rule.open.as_deref(),
                ExtAction::View => cr.rule.view.as_deref(),
                ExtAction::Edit => cr.rule.edit.as_deref(),
            };
            if t.is_some() {
                return t;
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
                    mime: None,
                    open: Some("xdg-open %f".into()),
                    view: None,
                    edit: None,
                },
                ExtBindRule {
                    glob: "*.pdf".into(),
                    mime: None,
                    open: Some("xdg-open %f".into()),
                    view: None,
                    edit: None,
                },
                ExtBindRule {
                    glob: "*.html".into(),
                    mime: None,
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
        assert_eq!(
            c.lookup("foo.PNG", None, ExtAction::Open),
            Some("xdg-open %f")
        );
        assert_eq!(c.lookup("foo.txt", None, ExtAction::Open), None);
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
        assert_eq!(c.lookup("lib.rs", None, ExtAction::View), Some("bat %f"));
        assert_eq!(c.lookup("lib.rs", None, ExtAction::Open), None);
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
        assert_eq!(c.lookup("anything", None, ExtAction::View), Some("less %f"));
        assert_eq!(c.lookup("anything", None, ExtAction::Open), None);
    }

    #[test]
    fn mime_only_rule_matches_by_mime() {
        let cfg = ExtBindings::from_toml(
            r#"
            [[bind]]
            mime = "image/*"
            open = "feh %f"
            "#,
        )
        .unwrap();
        let c = CompiledExtBindings::from_config(&cfg);
        assert_eq!(
            c.lookup("file-without-ext", Some("image/png"), ExtAction::Open),
            Some("feh %f")
        );
        assert_eq!(
            c.lookup("file-without-ext", Some("text/plain"), ExtAction::Open),
            None
        );
        assert_eq!(c.lookup("file-without-ext", None, ExtAction::Open), None);
    }

    #[test]
    fn glob_and_mime_must_both_match_when_both_present() {
        let cfg = ExtBindings::from_toml(
            r#"
            [[bind]]
            glob = "*.svg"
            mime = "image/*"
            open = "inkscape %f"
            "#,
        )
        .unwrap();
        let c = CompiledExtBindings::from_config(&cfg);
        assert_eq!(
            c.lookup("logo.svg", Some("image/svg+xml"), ExtAction::Open),
            Some("inkscape %f")
        );
        // glob matches but mime doesn't → no match
        assert_eq!(
            c.lookup("logo.svg", Some("text/plain"), ExtAction::Open),
            None
        );
    }
}
