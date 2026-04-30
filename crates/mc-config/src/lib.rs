//! Configuration: paths, TOML schemas, keymap, skin (Phase 0 stubs).

pub mod app;
pub mod extbind;
pub mod filehighlight;
pub mod hotlist;
pub mod paths;

pub use app::AppConfig;
pub use extbind::{CompiledExtBindings, ExtAction, ExtBindRule, ExtBindings};
pub use filehighlight::{FileHighlight, HighlightRule};
pub use hotlist::{Hotlist, HotlistEntry};
pub use paths::ConfigPaths;
