//! TUI front-end: app state, event loop, panels, dialogs.

pub mod app;
pub mod dialog;
pub mod editor_spawn;
pub mod event;
pub mod loop_;
pub mod panel;
pub mod viewer_widget;

pub use app::App;
pub use loop_::run;
