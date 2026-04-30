use mc_core::key::{KeyChord, KeyCode, KeyMods};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{centered_rect, Dialog, DialogOutcome};

pub struct InputDialog {
    title: String,
    prompt: String,
    value: String,
    cursor: usize,
}

impl InputDialog {
    #[must_use]
    pub fn new(title: impl Into<String>, prompt: impl Into<String>, initial: impl Into<String>) -> Self {
        let value = initial.into();
        let cursor = value.chars().count();
        Self {
            title: title.into(),
            prompt: prompt.into(),
            value,
            cursor,
        }
    }
}

impl Dialog for InputDialog {
    type Output = String;

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        let rect = centered_rect(60, 6, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .title(self.title.clone())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Cyan))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let prompt_line = Line::from(self.prompt.clone());
        let value_line = Line::from(self.value.clone()).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        let hint = Line::from("Enter: OK    Esc: Cancel");
        let body = Paragraph::new(vec![prompt_line, value_line, Line::from(""), hint]);
        f.render_widget(body, inner);

        // Draw cursor.
        let cur_x = inner.x + u16::try_from(self.cursor).unwrap_or(0).min(inner.width.saturating_sub(1));
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
}
