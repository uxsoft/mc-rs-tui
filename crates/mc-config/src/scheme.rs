//! Resolved color scheme — the single source of truth for every UI color.
//!
//! Three built-in themes are bundled. The active scheme is produced by
//! [`crate::skin::SkinFile::resolve`], which picks a base theme by name and
//! applies any per-field overrides from the user's `skin.toml`.

use crate::skin::{ThemeColor, parse_color};

/// Concrete, fully-resolved palette referenced by every renderer.
///
/// Field naming: `<surface>_<role>` — `bg` for backgrounds, `fg` for text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorScheme {
    // ---- panel chrome ----
    pub panel_bg: ThemeColor,
    pub panel_fg: ThemeColor,
    pub panel_dim_fg: ThemeColor,
    pub panel_border: ThemeColor,
    pub panel_border_active: ThemeColor,
    pub panel_title_fg: ThemeColor,
    pub panel_title_active_bg: ThemeColor,
    pub panel_title_active_fg: ThemeColor,
    pub cursor_bg: ThemeColor,
    pub cursor_fg: ThemeColor,
    pub marked_fg: ThemeColor,

    // ---- file-type accents (rendered on panel_bg) ----
    pub file_dir: ThemeColor,
    pub file_symlink: ThemeColor,
    pub file_executable: ThemeColor,
    pub file_device: ThemeColor,
    pub file_special: ThemeColor,
    pub file_archive: ThemeColor,
    pub file_image: ThemeColor,
    pub file_media: ThemeColor,
    pub file_doc: ThemeColor,
    pub file_source: ThemeColor,
    pub file_build: ThemeColor,

    // ---- dialogs ----
    pub dialog_bg: ThemeColor,
    pub dialog_fg: ThemeColor,
    pub dialog_border: ThemeColor,
    pub dialog_title_fg: ThemeColor,
    pub dialog_focus_bg: ThemeColor,
    pub dialog_focus_fg: ThemeColor,
    pub input_bg: ThemeColor,
    pub input_fg: ThemeColor,

    // ---- destructive (confirm/delete) ----
    pub danger_bg: ThemeColor,
    pub danger_fg: ThemeColor,
    pub danger_focus_bg: ThemeColor,
    pub danger_focus_fg: ThemeColor,

    // ---- chrome ----
    pub statusbar_bg: ThemeColor,
    pub statusbar_fg: ThemeColor,
    pub buttonbar_bg: ThemeColor,
    pub buttonbar_fg: ThemeColor,
    pub buttonbar_label_bg: ThemeColor,
    pub buttonbar_label_fg: ThemeColor,
    pub search_bg: ThemeColor,
    pub search_fg: ThemeColor,
    pub op_status_bg: ThemeColor,
    pub op_status_fg: ThemeColor,

    // ---- diff / viewer ----
    pub diff_add_bg: ThemeColor,
    pub diff_add_fg: ThemeColor,
    pub diff_del_bg: ThemeColor,
    pub diff_del_fg: ThemeColor,

    pub muted_fg: ThemeColor,
}

impl ColorScheme {
    /// Look up a built-in theme by its lowercase name.
    /// Recognised: `modern-dark`, `tokyo-night`, `solarized-light`.
    #[must_use]
    pub fn from_named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "modern-dark" | "modern_dark" | "default" => Some(Self::modern_dark()),
            "tokyo-night" | "tokyo_night" => Some(Self::tokyo_night()),
            "solarized-light" | "solarized_light" => Some(Self::solarized_light()),
            _ => None,
        }
    }

    /// Built-in themes as `(name, display_label)` pairs. The names round-trip
    /// through [`Self::from_named`].
    #[must_use]
    pub fn available_themes() -> &'static [(&'static str, &'static str)] {
        &[
            ("modern-dark", "Modern Dark"),
            ("tokyo-night", "Tokyo Night"),
            ("solarized-light", "Solarized Light"),
        ]
    }

    /// Default theme — Catppuccin-Mocha inspired dark palette tuned for
    /// WCAG-AA contrast on every text-bearing pair.
    #[must_use]
    pub fn modern_dark() -> Self {
        // base palette
        let base = ThemeColor::rgb(0x1e, 0x1e, 0x2e); // panel bg
        let mantle = ThemeColor::rgb(0x18, 0x18, 0x25); // statusbar bg
        let surface0 = ThemeColor::rgb(0x31, 0x32, 0x44); // dialog bg
        let surface1 = ThemeColor::rgb(0x45, 0x47, 0x5a); // muted/border
        let surface2 = ThemeColor::rgb(0x58, 0x5b, 0x70);
        let text = ThemeColor::rgb(0xcd, 0xd6, 0xf4); // primary fg
        let subtext = ThemeColor::rgb(0xba, 0xc2, 0xde);
        let blue = ThemeColor::rgb(0x89, 0xb4, 0xfa);
        let teal = ThemeColor::rgb(0x94, 0xe2, 0xd5);
        let green = ThemeColor::rgb(0xa6, 0xe3, 0xa1);
        let yellow = ThemeColor::rgb(0xf9, 0xe2, 0xaf);
        let peach = ThemeColor::rgb(0xfa, 0xb3, 0x87);
        let mauve = ThemeColor::rgb(0xcb, 0xa6, 0xf7);
        let pink = ThemeColor::rgb(0xf5, 0xc2, 0xe7);
        let red = ThemeColor::rgb(0xf3, 0x8b, 0xa8);

        Self {
            panel_bg: base,
            panel_fg: text,
            panel_dim_fg: subtext,
            panel_border: surface1,
            panel_border_active: blue,
            panel_title_fg: subtext,
            panel_title_active_bg: blue,
            panel_title_active_fg: base,
            cursor_bg: blue,
            cursor_fg: base,
            marked_fg: yellow,

            file_dir: blue,
            file_symlink: teal,
            file_executable: green,
            file_device: yellow,
            file_special: mauve,
            file_archive: red,
            file_image: mauve,
            file_media: pink,
            file_doc: text,
            file_source: teal,
            file_build: peach,

            dialog_bg: surface0,
            dialog_fg: text,
            dialog_border: blue,
            dialog_title_fg: blue,
            dialog_focus_bg: yellow,
            dialog_focus_fg: base,
            input_bg: surface2,
            input_fg: text,

            danger_bg: red,
            danger_fg: base,
            danger_focus_bg: yellow,
            danger_focus_fg: base,

            statusbar_bg: mantle,
            statusbar_fg: subtext,
            buttonbar_bg: mantle,
            buttonbar_fg: subtext,
            buttonbar_label_bg: blue,
            buttonbar_label_fg: base,
            search_bg: yellow,
            search_fg: base,
            op_status_bg: yellow,
            op_status_fg: base,

            diff_add_bg: ThemeColor::rgb(0x26, 0x3a, 0x29),
            diff_add_fg: green,
            diff_del_bg: ThemeColor::rgb(0x3f, 0x22, 0x2a),
            diff_del_fg: red,

            muted_fg: surface2,
        }
    }

    /// Tokyo Night — cooler, deeper bluish dark theme.
    #[must_use]
    pub fn tokyo_night() -> Self {
        let bg = ThemeColor::rgb(0x1a, 0x1b, 0x26);
        let bg_dark = ThemeColor::rgb(0x16, 0x16, 0x1e);
        let bg_hl = ThemeColor::rgb(0x29, 0x2e, 0x42);
        let surface = ThemeColor::rgb(0x24, 0x28, 0x3b);
        let muted = ThemeColor::rgb(0x54, 0x5c, 0x7e);
        let fg = ThemeColor::rgb(0xc0, 0xca, 0xf5);
        let fg_dim = ThemeColor::rgb(0xa9, 0xb1, 0xd6);
        let blue = ThemeColor::rgb(0x7a, 0xa2, 0xf7);
        let cyan = ThemeColor::rgb(0x7d, 0xcf, 0xff);
        let teal = ThemeColor::rgb(0x73, 0xda, 0xca);
        let green = ThemeColor::rgb(0x9e, 0xce, 0x6a);
        let yellow = ThemeColor::rgb(0xe0, 0xaf, 0x68);
        let orange = ThemeColor::rgb(0xff, 0x9e, 0x64);
        let magenta = ThemeColor::rgb(0xbb, 0x9a, 0xf7);
        let red = ThemeColor::rgb(0xf7, 0x76, 0x8e);

        Self {
            panel_bg: bg,
            panel_fg: fg,
            panel_dim_fg: fg_dim,
            panel_border: bg_hl,
            panel_border_active: blue,
            panel_title_fg: fg_dim,
            panel_title_active_bg: blue,
            panel_title_active_fg: bg_dark,
            cursor_bg: blue,
            cursor_fg: bg_dark,
            marked_fg: yellow,

            file_dir: blue,
            file_symlink: cyan,
            file_executable: green,
            file_device: yellow,
            file_special: magenta,
            file_archive: red,
            file_image: magenta,
            file_media: magenta,
            file_doc: fg,
            file_source: teal,
            file_build: orange,

            dialog_bg: surface,
            dialog_fg: fg,
            dialog_border: blue,
            dialog_title_fg: blue,
            dialog_focus_bg: yellow,
            dialog_focus_fg: bg_dark,
            input_bg: bg_hl,
            input_fg: fg,

            danger_bg: red,
            danger_fg: bg_dark,
            danger_focus_bg: yellow,
            danger_focus_fg: bg_dark,

            statusbar_bg: bg_dark,
            statusbar_fg: fg_dim,
            buttonbar_bg: bg_dark,
            buttonbar_fg: fg_dim,
            buttonbar_label_bg: blue,
            buttonbar_label_fg: bg_dark,
            search_bg: yellow,
            search_fg: bg_dark,
            op_status_bg: yellow,
            op_status_fg: bg_dark,

            diff_add_bg: ThemeColor::rgb(0x20, 0x32, 0x2a),
            diff_add_fg: green,
            diff_del_bg: ThemeColor::rgb(0x37, 0x1f, 0x28),
            diff_del_fg: red,

            muted_fg: muted,
        }
    }

    /// Solarized Light — high-luminance background for daytime use.
    #[must_use]
    pub fn solarized_light() -> Self {
        let base3 = ThemeColor::rgb(0xfd, 0xf6, 0xe3); // panel bg
        let base2 = ThemeColor::rgb(0xee, 0xe8, 0xd5); // dialog bg / selection
        let base1 = ThemeColor::rgb(0x93, 0xa1, 0xa1); // muted
        let base00 = ThemeColor::rgb(0x65, 0x7b, 0x83); // dim text
        let base02 = ThemeColor::rgb(0x07, 0x36, 0x42); // primary text
        let base03 = ThemeColor::rgb(0x00, 0x2b, 0x36); // strong text
        let yellow = ThemeColor::rgb(0xb5, 0x89, 0x00);
        let orange = ThemeColor::rgb(0xcb, 0x4b, 0x16);
        let red = ThemeColor::rgb(0xdc, 0x32, 0x2f);
        let magenta = ThemeColor::rgb(0xd3, 0x36, 0x82);
        let violet = ThemeColor::rgb(0x6c, 0x71, 0xc4);
        let blue = ThemeColor::rgb(0x26, 0x8b, 0xd2);
        let cyan = ThemeColor::rgb(0x2a, 0xa1, 0x98);
        let green = ThemeColor::rgb(0x85, 0x99, 0x00);

        Self {
            panel_bg: base3,
            panel_fg: base02,
            panel_dim_fg: base00,
            panel_border: base1,
            panel_border_active: blue,
            panel_title_fg: base00,
            panel_title_active_bg: blue,
            panel_title_active_fg: base3,
            cursor_bg: blue,
            cursor_fg: base3,
            marked_fg: orange,

            file_dir: blue,
            file_symlink: cyan,
            file_executable: green,
            file_device: yellow,
            file_special: violet,
            file_archive: red,
            file_image: magenta,
            file_media: magenta,
            file_doc: base02,
            file_source: cyan,
            file_build: orange,

            dialog_bg: base2,
            dialog_fg: base03,
            dialog_border: blue,
            dialog_title_fg: blue,
            dialog_focus_bg: yellow,
            dialog_focus_fg: base3,
            input_bg: base3,
            input_fg: base03,

            danger_bg: red,
            danger_fg: base3,
            danger_focus_bg: yellow,
            danger_focus_fg: base3,

            statusbar_bg: base2,
            statusbar_fg: base02,
            buttonbar_bg: base2,
            buttonbar_fg: base02,
            buttonbar_label_bg: blue,
            buttonbar_label_fg: base3,
            search_bg: yellow,
            search_fg: base3,
            op_status_bg: yellow,
            op_status_fg: base3,

            diff_add_bg: ThemeColor::rgb(0xe6, 0xf0, 0xc8),
            diff_add_fg: green,
            diff_del_bg: ThemeColor::rgb(0xf6, 0xd6, 0xd2),
            diff_del_fg: red,

            muted_fg: base1,
        }
    }
}

/// Apply a single override key=value to an existing scheme.
///
/// # Errors
/// Returns a human-readable message if the key is unknown or the value
/// cannot be parsed.
pub fn apply_override(scheme: &mut ColorScheme, key: &str, value: &str) -> Result<(), String> {
    let color = parse_color(value).map_err(|e| format!("{key}: {e}"))?;
    macro_rules! fields {
        ($($name:ident),* $(,)?) => {
            match key {
                $(stringify!($name) => { scheme.$name = color; Ok(()) })*
                other => Err(format!("unknown color field '{other}'")),
            }
        };
    }
    fields!(
        panel_bg,
        panel_fg,
        panel_dim_fg,
        panel_border,
        panel_border_active,
        panel_title_fg,
        panel_title_active_bg,
        panel_title_active_fg,
        cursor_bg,
        cursor_fg,
        marked_fg,
        file_dir,
        file_symlink,
        file_executable,
        file_device,
        file_special,
        file_archive,
        file_image,
        file_media,
        file_doc,
        file_source,
        file_build,
        dialog_bg,
        dialog_fg,
        dialog_border,
        dialog_title_fg,
        dialog_focus_bg,
        dialog_focus_fg,
        input_bg,
        input_fg,
        danger_bg,
        danger_fg,
        danger_focus_bg,
        danger_focus_fg,
        statusbar_bg,
        statusbar_fg,
        buttonbar_bg,
        buttonbar_fg,
        buttonbar_label_bg,
        buttonbar_label_fg,
        search_bg,
        search_fg,
        op_status_bg,
        op_status_fg,
        diff_add_bg,
        diff_add_fg,
        diff_del_bg,
        diff_del_fg,
        muted_fg,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_named_themes() {
        assert!(ColorScheme::from_named("modern-dark").is_some());
        assert!(ColorScheme::from_named("Modern_Dark").is_some());
        assert!(ColorScheme::from_named("tokyo-night").is_some());
        assert!(ColorScheme::from_named("solarized-light").is_some());
        assert!(ColorScheme::from_named("nope").is_none());
    }

    #[test]
    fn override_changes_field() {
        let mut s = ColorScheme::modern_dark();
        apply_override(&mut s, "panel_bg", "#010203").unwrap();
        assert_eq!(s.panel_bg, ThemeColor::Rgb(1, 2, 3));
    }

    #[test]
    fn override_unknown_field_errs() {
        let mut s = ColorScheme::modern_dark();
        let err = apply_override(&mut s, "nope", "#000000").unwrap_err();
        assert!(err.contains("nope"));
    }

    #[test]
    fn override_bad_color_errs() {
        let mut s = ColorScheme::modern_dark();
        let err = apply_override(&mut s, "panel_bg", "puce").unwrap_err();
        assert!(err.contains("panel_bg"));
    }
}
