use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode, KeyMods};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

pub struct InputDialog {
    title: String,
    prompt: String,
    value: String,
    cursor: usize,
    /// Snapshot of history at modal-open time (oldest → newest); `history_pos`
    /// indexes into this. `None` means history is disabled for this modal.
    history: Option<Vec<String>>,
    /// `Some(i)` means the value currently shown is `history[i]`. `None` means
    /// the user is editing fresh text.
    history_pos: Option<usize>,
    /// Saved value before the user started Up-arrowing through history; restored
    /// when they reach the bottom again.
    saved: Option<String>,
}

impl InputDialog {
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        initial: impl Into<String>,
    ) -> Self {
        let value = initial.into();
        let cursor = value.chars().count();
        Self {
            title: title.into(),
            prompt: prompt.into(),
            value,
            cursor,
            history: None,
            history_pos: None,
            saved: None,
        }
    }

    /// Attach a history snapshot so Up/Down recall past entries.
    #[must_use]
    pub fn with_history(mut self, entries: Vec<String>) -> Self {
        self.history = Some(entries);
        self
    }

    fn history_up(&mut self) {
        let h = match &self.history {
            Some(h) if !h.is_empty() => h,
            _ => return,
        };
        let new_pos = match self.history_pos {
            Some(0) => return,
            Some(p) => p - 1,
            None => {
                self.saved = Some(self.value.clone());
                h.len() - 1
            }
        };
        self.history_pos = Some(new_pos);
        self.value = h[new_pos].clone();
        self.cursor = self.value.chars().count();
    }

    fn history_down(&mut self) {
        let h = match &self.history {
            Some(h) if !h.is_empty() => h,
            _ => return,
        };
        match self.history_pos {
            None => return,
            Some(p) if p + 1 >= h.len() => {
                self.history_pos = None;
                self.value = self.saved.take().unwrap_or_default();
                self.cursor = self.value.chars().count();
            }
            Some(p) => {
                self.history_pos = Some(p + 1);
                self.value = h[p + 1].clone();
                self.cursor = self.value.chars().count();
            }
        }
    }
}

impl Dialog for InputDialog {
    type Output = String;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(60, 6, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .title(ratatui::text::Span::styled(
                self.title.clone(),
                Style::default()
                    .fg(rtc(scheme.dialog_title_fg))
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(rtc(scheme.dialog_border))
                    .bg(rtc(scheme.dialog_bg)),
            )
            .style(
                Style::default()
                    .fg(rtc(scheme.dialog_fg))
                    .bg(rtc(scheme.dialog_bg)),
            );
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let prompt_line = Line::from(self.prompt.clone());
        let value_line = Line::from(self.value.clone()).style(
            Style::default()
                .fg(rtc(scheme.input_fg))
                .bg(rtc(scheme.input_bg))
                .add_modifier(Modifier::BOLD),
        );
        let hint = Line::from("Enter: OK    Esc: Cancel");
        let body = Paragraph::new(vec![prompt_line, value_line, Line::from(""), hint]);
        f.render_widget(body, inner);

        // Draw cursor.
        let cur_x = inner.x
            + u16::try_from(self.cursor)
                .unwrap_or(0)
                .min(inner.width.saturating_sub(1));
        let cur_y = inner.y + 1;
        f.set_cursor_position((cur_x, cur_y));
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<String> {
        match (chord.code, chord.mods) {
            (KeyCode::Escape, _) => DialogOutcome::Cancelled,
            (KeyCode::Enter, _) => {
                if self.value.is_empty() {
                    DialogOutcome::Cancelled
                } else {
                    DialogOutcome::Submitted(self.value.clone())
                }
            }
            (KeyCode::Backspace, _) => {
                if self.cursor > 0 {
                    let new_cursor = self.cursor - 1;
                    let mut chars: Vec<char> = self.value.chars().collect();
                    chars.remove(new_cursor);
                    self.value = chars.into_iter().collect();
                    self.cursor = new_cursor;
                }
                DialogOutcome::None
            }
            (KeyCode::Delete, _) => {
                let mut chars: Vec<char> = self.value.chars().collect();
                if self.cursor < chars.len() {
                    chars.remove(self.cursor);
                    self.value = chars.into_iter().collect();
                }
                DialogOutcome::None
            }
            (KeyCode::Left, _) => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            (KeyCode::Right, _) => {
                let len = self.value.chars().count();
                if self.cursor < len {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            (KeyCode::Home, _) => {
                self.cursor = 0;
                DialogOutcome::None
            }
            (KeyCode::End, _) => {
                self.cursor = self.value.chars().count();
                DialogOutcome::None
            }
            (KeyCode::Up, _) => {
                self.history_up();
                DialogOutcome::None
            }
            (KeyCode::Down, _) => {
                self.history_down();
                DialogOutcome::None
            }
            (KeyCode::Char(c), m) if m.is_empty() || m == KeyMods::SHIFT => {
                let mut chars: Vec<char> = self.value.chars().collect();
                chars.insert(self.cursor, c);
                self.value = chars.into_iter().collect();
                self.cursor += 1;
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<String> {
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return DialogOutcome::None;
        }
        let rect = super::centered_rect(60, 6, area);
        let inside = ev.column >= rect.x
            && ev.column < rect.x + rect.width
            && ev.row >= rect.y
            && ev.row < rect.y + rect.height;
        if !inside {
            return DialogOutcome::Cancelled;
        }
        let inner_x = rect.x + 1;
        let value_y = rect.y + 2; // inner.y + 1
        if ev.row == value_y && ev.column >= inner_x {
            let col = (ev.column - inner_x) as usize;
            self.cursor = col.min(self.value.chars().count());
        }
        DialogOutcome::None
    }
}
