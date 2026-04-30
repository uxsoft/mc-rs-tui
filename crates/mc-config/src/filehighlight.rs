//! File highlighting rules: classify files into named "color groups" by
//! filename pattern or by mode bits.
//!
//! Loaded from TOML — example:
//!
//! ```toml
//! [[rule]]
//! group = "archive"
//! extensions = ["tar", "gz", "tgz", "zip", "bz2", "xz", "7z", "rar", "zst"]
//!
//! [[rule]]
//! group = "image"
//! extensions = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"]
//!
//! [[rule]]
//! group = "source"
//! extensions = ["rs", "c", "h", "cpp", "py", "go", "ts", "js"]
//! ```

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHighlight {
    #[serde(default)]
    pub rule: Vec<HighlightRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightRule {
    pub group: String,
    /// Lowercase extensions without leading dot.
    #[serde(default)]
    pub extensions: Vec<String>,
}

impl FileHighlight {
    /// Default ruleset shipped with the binary.
    #[must_use]
    pub fn defaults() -> Self {
        let rules = [
            ("archive", &[
                "tar", "gz", "tgz", "zip", "bz2", "tbz", "tbz2", "xz", "txz", "7z", "rar", "zst", "tzst", "lz", "lzma",
            ][..]),
            ("image", &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff",
            ][..]),
            ("audio", &["mp3", "ogg", "flac", "wav", "m4a", "opus"][..]),
            ("video", &["mp4", "mkv", "webm", "mov", "avi", "mpg", "mpeg"][..]),
            ("doc", &["pdf", "epub", "djvu", "doc", "docx", "odt", "rtf", "md", "txt"][..]),
            ("source", &[
                "rs", "c", "h", "cpp", "cc", "cxx", "hpp", "py", "go", "ts", "tsx", "js", "jsx",
                "java", "kt", "scala", "rb", "swift", "lua", "sh", "fish", "zsh", "ps1",
            ][..]),
            ("build", &["toml", "yaml", "yml", "json", "lock", "ini", "conf", "cfg"][..]),
        ];
        Self {
            rule: rules
                .into_iter()
                .map(|(g, exts)| HighlightRule {
                    group: g.into(),
                    extensions: exts.iter().map(|s| (*s).to_string()).collect(),
                })
                .collect(),
        }
    }

    /// Classify a filename into a group, or `None` if no rule matches.
    #[must_use]
    pub fn classify(&self, name: &str) -> Option<&str> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let ext = ext?;
        for rule in &self.rule {
            if rule.extensions.iter().any(|x| x.eq_ignore_ascii_case(&ext)) {
                return Some(&rule.group);
            }
        }
        None
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_known_groups() {
        let h = FileHighlight::defaults();
        assert_eq!(h.classify("foo.rs"), Some("source"));
        assert_eq!(h.classify("image.PNG"), Some("image"));
        assert_eq!(h.classify("data.tar.gz"), Some("archive"));
        assert_eq!(h.classify("Cargo.toml"), Some("build"));
        assert_eq!(h.classify("plain"), None);
        assert_eq!(h.classify("unknown.xyz"), None);
    }

    #[test]
    fn parse_toml() {
        let h: FileHighlight = FileHighlight::from_toml(
            r#"
            [[rule]]
            group = "x"
            extensions = ["foo", "bar"]
            "#,
        )
        .unwrap();
        assert_eq!(h.classify("a.foo"), Some("x"));
    }
}
