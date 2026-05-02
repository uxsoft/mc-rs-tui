//! Lightweight modal dialogs (confirm, input).

pub mod confirm;
pub mod copy_move;
pub mod find;
pub mod help;
pub mod hotlist;
pub mod input;
pub mod jobs_view;
pub mod layout;
pub mod learn_keys;
pub mod menubar;
pub mod options;
pub mod password;
pub mod progress;
pub mod theme;
pub mod user_menu;
pub mod vfs_list;

use crossterm::event::MouseEvent;
use mc_config::ColorScheme;
use mc_core::key::KeyChord;
use ratatui::Frame;
use ratatui::layout::Rect;

pub use confirm::ConfirmDialog;
pub use copy_move::{CopyMoveSettings, CopyMoveSettingsDialog};
pub use find::{FindForm, FindFormOutcome, FindParams, FindResults, FindResultsOutcome};
pub use help::HelpDialog;
pub use hotlist::{HotlistAction, HotlistDialog};
pub use input::InputDialog;
pub use jobs_view::{JobRow, JobsViewDialog};
pub use layout::LayoutDialog;
pub use learn_keys::LearnKeysDialog;
pub use menubar::{MenuBar, MenuChoice, MenuDialog};
pub use options::{OptionField, OptionKey, OptionsDialog};
pub use password::PasswordDialog;
pub use progress::ProgressDialog;
pub use theme::ThemeDialog;
pub use user_menu::{UserMenuDialog, UserMenuEntry};
pub use vfs_list::{VfsListAction, VfsListDialog};

#[derive(Debug)]
pub enum DialogOutcome<T> {
    None,
    Cancelled,
    Submitted(T),
}

/// Center a width×height rectangle inside `area`.
#[must_use]
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

pub trait Dialog {
    type Output;
    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme);
    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<Self::Output>;
    /// Default no-op mouse handler. Override on dialogs that benefit from
    /// clicking. `area` is the same outer rect passed to `render`; the dialog
    /// re-derives its on-screen rect inside (matching the renderer's geometry)
    /// and hit-tests there.
    fn handle_mouse(&mut self, _ev: MouseEvent, _area: Rect) -> DialogOutcome<Self::Output> {
        DialogOutcome::None
    }
}
