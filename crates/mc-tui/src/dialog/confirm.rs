use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{centered_rect, Dialog, DialogOutcome};

pub struct ConfirmDialog {
    title: String,
    message: String,
    yes: bool,
}

impl ConfirmDialog {
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            yes: false,
        }
    }
}

impl Dialog for ConfirmDialog {
    type Output = bool;

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        let rect = centered_rect(60, 6, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .title(self.title.clone())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Red))
            .style(Style::default().fg(Color::White).bg(Color::Red));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let yes_style = if self.yes {
            Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(Color::Red)
        };
        let no_style = if !self.yes {
            Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(Color::Red)
        };

        let buttons = Line::from(vec![
            ratatui::text::Span::raw("        "),
            ratatui::text::Span::styled(" [ Yes ] ", yes_style),
            ratatui::text::Span::raw("    "),
            ratatui::text::Span::styled(" [ No ] ", no_style),
        ]);

        let body = Paragraph::new(vec![
            Line::from(self.message.clone()),
            Line::from(""),
            buttons,
            Line::from(""),
            Line::from("y/n, Tab/←→: switch    Enter: OK    Esc: Cancel"),
        ]);
        f.render_widget(body, inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<bool> {
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Char('y') | KeyCode::Char('Y') => DialogOutcome::Submitted(true),
            KeyCode::Char('n') | KeyCode::Char('N') => DialogOutcome::Submitted(false),
            KeyCode::Enter => DialogOutcome::Submitted(self.yes),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.yes = !self.yes;
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
