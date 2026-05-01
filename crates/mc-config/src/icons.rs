//! Per-entry icon glyphs (Nerd Font).
//!
//! Enabled by default. Disable via `[options] icons = false` in
//! `~/.config/mc-rs/config.toml` if your terminal lacks a Nerd Font.
//!
//! Lookup precedence: explicit filename → extension (case-insensitive) →
//! entry-kind fallback. All glyphs are single Unicode scalars in the Nerd
//! Font Private Use Area.

// The lookup tables intentionally keep groups of related extensions on
// separate match arms even when they share a glyph — readability beats
// merging unrelated groups into one OR-pattern.
#![allow(clippy::match_same_arms)]

use mc_core::EntryKind;

/// Glyph for a non-file entry kind.
#[must_use]
pub fn icon_for_kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Dir => "\u{f07b}",                     //  folder
        EntryKind::Symlink => "\u{f0c1}",                 //  link
        EntryKind::Fifo => "\u{f0e7}",                    //  pipe
        EntryKind::Socket => "\u{f6a7}",                  //  socket
        EntryKind::BlockDevice => "\u{f0a0}",             //  hdd
        EntryKind::CharDevice => "\u{f108}",              //  desktop
        EntryKind::File | EntryKind::Other => "\u{f15b}", //  file
    }
}

/// Icon for an entry name. `kind_fallback` is used when no name/extension
/// rule matches.
#[must_use]
pub fn icon_for_name(name: &str, kind_fallback: EntryKind) -> &'static str {
    if !matches!(kind_fallback, EntryKind::File) {
        return icon_for_kind(kind_fallback);
    }
    if let Some(g) = exact_name(name) {
        return g;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if let Some(ext) = ext {
        if let Some(g) = by_extension(&ext) {
            return g;
        }
    }
    icon_for_kind(kind_fallback)
}

fn exact_name(name: &str) -> Option<&'static str> {
    Some(match name {
        ".gitignore" | ".gitattributes" | ".gitmodules" => "\u{e702}", //  git
        "Cargo.toml" | "Cargo.lock" => "\u{e7a8}",                     //  rust
        "Dockerfile" | ".dockerignore" => "\u{f308}",                  //  docker
        "Makefile" | "makefile" | "GNUmakefile" => "\u{e779}",         //  gear
        "package.json" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" => "\u{e718}", //  npm
        "README.md" | "README" | "README.txt" | "README.rst" => "\u{f02d}", //  book
        "LICENSE" | "LICENSE.md" | "LICENSE.txt" | "COPYING" => "\u{f02d}",
        ".bashrc" | ".bash_profile" | ".zshrc" | ".profile" => "\u{f489}", //  terminal
        _ => return None,
    })
}

fn by_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        // Rust / systems
        "rs" => "\u{e7a8}",
        "go" => "\u{e626}",
        "c" | "h" => "\u{e61e}",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "\u{e61d}",
        "py" | "pyw" | "pyc" => "\u{e606}",
        "rb" => "\u{e791}",
        "java" => "\u{e738}",
        "kt" | "kts" => "\u{e634}",
        "swift" => "\u{e755}",
        "scala" | "sc" => "\u{e737}",
        "lua" => "\u{e620}",
        "zig" => "\u{e6a9}",
        // Web
        "js" | "mjs" | "cjs" => "\u{e74e}",
        "jsx" => "\u{e7ba}",
        "ts" => "\u{e628}",
        "tsx" => "\u{e7ba}",
        "html" | "htm" => "\u{e736}",
        "css" => "\u{e749}",
        "scss" | "sass" => "\u{e74b}",
        "json" => "\u{e60b}",
        "vue" => "\u{fd42}",
        "svelte" => "\u{e697}",
        // Shell / config
        "sh" | "bash" | "zsh" | "fish" => "\u{f489}",
        "ps1" => "\u{f489}",
        "toml" => "\u{e615}",
        "yaml" | "yml" => "\u{e6a8}",
        "ini" | "cfg" | "conf" => "\u{e615}",
        "lock" => "\u{f023}",
        // Docs
        "md" | "markdown" => "\u{e73e}",
        "rst" => "\u{f15c}",
        "txt" | "log" => "\u{f15c}",
        "pdf" => "\u{f1c1}",
        "doc" | "docx" | "odt" | "rtf" => "\u{f1c2}",
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => "\u{f1c3}",
        "ppt" | "pptx" | "odp" => "\u{f1c4}",
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" => "\u{f1c5}",
        "svg" => "\u{e60a}",
        // Audio / video
        "mp3" | "ogg" | "flac" | "wav" | "m4a" | "opus" => "\u{f1c7}",
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "mpg" | "mpeg" => "\u{f1c8}",
        // Archives
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "7z" | "rar"
        | "zst" | "tzst" | "lz" | "lzma" | "cpio" => "\u{f1c6}",
        // Binaries
        "exe" | "dll" | "so" | "dylib" | "a" | "o" => "\u{f471}",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_uses_kind_glyph() {
        let g = icon_for_name("anything", EntryKind::Dir);
        assert_eq!(g, icon_for_kind(EntryKind::Dir));
    }

    #[test]
    fn extension_lookup() {
        assert_eq!(icon_for_name("lib.rs", EntryKind::File), "\u{e7a8}");
        assert_eq!(icon_for_name("a.PY", EntryKind::File), "\u{e606}");
    }

    #[test]
    fn exact_name_wins_over_extension() {
        let by_name = icon_for_name("Cargo.toml", EntryKind::File);
        let by_ext = by_extension("toml").unwrap();
        assert_ne!(by_name, by_ext);
    }

    #[test]
    fn unknown_falls_back_to_kind() {
        let g = icon_for_name("nope.xyz", EntryKind::File);
        assert_eq!(g, icon_for_kind(EntryKind::File));
    }
}
