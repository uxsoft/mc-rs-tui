//! Learn-keys: shows the canonical name of the last few key chords received.
//! Useful for verifying that terminal escape sequences map to the chords we
//! expect.

use std::collections::VecDeque;

use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

pub struct LearnKeysDialog {
    history: VecDeque<KeyChord>,
}

impl LearnKeysDialog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(16),
        }
    }
}

impl Default for LearnKeysDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialog for LearnKeysDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(60, 14, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Learn keys ",
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

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Press any key to see its canonical name.",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for c in self.history.iter().rev().take(10) {
            lines.push(Line::from(format!("  {c}")));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Press Esc twice to close (Esc once is recorded).",
        ));
        f.render_widget(Paragraph::new(lines).style(dlg), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        // Special-case: a second Escape closes; first Escape gets recorded.
        if matches!(chord.code, KeyCode::Escape)
            && self
                .history
                .front()
                .is_some_and(|c| c.code == KeyCode::Escape)
        {
            return DialogOutcome::Cancelled;
        }
        self.history.push_front(chord);
        if self.history.len() > 32 {
            self.history.pop_back();
        }
        DialogOutcome::None
    }
}
