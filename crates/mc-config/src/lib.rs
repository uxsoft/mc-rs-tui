//! Configuration: paths, TOML schemas, keymap, skin (Phase 0 stubs).

pub mod app;
pub mod extbind;
pub mod filehighlight;
pub mod history;
pub mod hotlist;
pub mod icons;
pub mod io;
pub mod keymap;
pub mod paths;
pub mod scheme;
pub mod skin;

pub use app::{AppConfig, LayoutConfig, PanelStateSnapshot, VfsConfig};
pub use extbind::{CompiledExtBindings, ExtAction, ExtBindRule, ExtBindings};
pub use filehighlight::{FileHighlight, HighlightRule};
pub use history::History;
pub use hotlist::{Hotlist, HotlistEntry};
pub use icons::{IconMode, icon_for_kind, icon_for_name};
pub use io::{load_toml_or_default, write_user_file_atomic};
pub use keymap::{Keymap, KeymapFile, RemapEntry};
pub use paths::ConfigPaths;
pub use scheme::{ColorScheme, apply_override};
pub use skin::{SkinFile, ThemeColor, parse_color};
