//! TUI front-end: app state, event loop, panels, dialogs.

pub mod app;
pub mod clipboard;
pub mod dialog;
pub mod diff_widget;
pub mod editor_spawn;
pub mod event;
pub mod glob;
pub mod loop_;
pub mod panel;
pub mod subshell;
pub mod theme;
pub mod viewer_widget;
pub mod watcher;

pub use app::App;
pub use loop_::run;
