//! Lightweight modal dialogs (confirm, input).

pub mod input;
pub mod confirm;

use mc_core::key::KeyChord;
use ratatui::layout::Rect;
use ratatui::Frame;

pub use confirm::ConfirmDialog;
pub use input::InputDialog;

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
    fn render(&self, f: &mut Frame<'_>, area: Rect);
    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<Self::Output>;
}
