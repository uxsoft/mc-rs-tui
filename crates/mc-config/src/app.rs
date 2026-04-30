//! Top-level application configuration (TOML root).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub panels: PanelsConfig,
    pub options: OptionsConfig,
    pub editor: EditorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelsConfig {
    pub show_hidden: bool,
    pub mix_dirs: bool,
    pub case_sensitive_sort: bool,
}

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            show_hidden: false,
            mix_dirs: false,
            case_sensitive_sort: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionsConfig {
    pub use_internal_view: bool,
    pub confirm_delete: bool,
    pub confirm_overwrite: bool,
}

impl Default for OptionsConfig {
    fn default() -> Self {
        Self {
            use_internal_view: true,
            confirm_delete: true,
            confirm_overwrite: true,
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

impl AppConfig {
    /// Parse from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
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
}
