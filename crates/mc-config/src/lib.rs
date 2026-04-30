//! Configuration: paths, TOML schemas, keymap, skin (Phase 0 stubs).

pub mod app;
pub mod extbind;
pub mod filehighlight;
pub mod history;
pub mod hotlist;
pub mod keymap;
pub mod paths;
pub mod skin;

pub use app::AppConfig;
pub use extbind::{CompiledExtBindings, ExtAction, ExtBindRule, ExtBindings};
pub use filehighlight::{FileHighlight, HighlightRule};
pub use history::History;
pub use hotlist::{Hotlist, HotlistEntry};
pub use keymap::{Keymap, KeymapFile, RemapEntry};
pub use paths::ConfigPaths;
pub use skin::{parse_color_name, AnsiColor, PanelSection, SkinFile};
