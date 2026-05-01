//! Password prompt with masked input.
//!
//! Behaves like `InputDialog` but renders `*` for each typed character. Used
//! by remote-VFS auth flows when agent + key auth fail.

use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode, KeyMods};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

pub struct PasswordDialog {
    title: String,
    prompt: String,
    value: String,
}

impl PasswordDialog {
    #[must_use]
    pub fn new(title: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            prompt: prompt.into(),
            value: String::new(),
        }
    }
}

impl Dialog for PasswordDialog {
    type Output = String;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(60, 6, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
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
            .style(dlg);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let masked: String = std::iter::repeat('*')
            .take(self.value.chars().count())
            .collect();
        let prompt_line = Line::from(self.prompt.clone());
        let value_line = Line::from(masked).style(
            Style::default()
                .fg(rtc(scheme.input_fg))
                .bg(rtc(scheme.input_bg))
                .add_modifier(Modifier::BOLD),
        );
        let hint = Line::from("Enter: OK    Esc: Cancel");
        let body = Paragraph::new(vec![prompt_line, value_line, Line::from(""), hint]).style(dlg);
        f.render_widget(body, inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<String> {
        match (chord.code, chord.mods) {
            (KeyCode::Escape, _) => DialogOutcome::Cancelled,
            (KeyCode::Enter, _) => DialogOutcome::Submitted(std::mem::take(&mut self.value)),
            (KeyCode::Backspace, _) => {
                self.value.pop();
                DialogOutcome::None
            }
            (KeyCode::Char(c), m) if m.is_empty() || m == KeyMods::SHIFT => {
                self.value.push(c);
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
