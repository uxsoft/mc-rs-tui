//! Configuration: paths, TOML schemas, keymap, skin (Phase 0 stubs).

pub mod app;
pub mod filehighlight;
pub mod hotlist;
pub mod paths;

pub use app::AppConfig;
pub use filehighlight::{FileHighlight, HighlightRule};
pub use hotlist::{Hotlist, HotlistEntry};
pub use paths::ConfigPaths;
