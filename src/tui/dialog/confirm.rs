use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::tui::theme::rtc;

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

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(60, 6, area);
        f.render_widget(Clear, rect);
        let bg_style = Style::default()
            .fg(rtc(scheme.danger_fg))
            .bg(rtc(scheme.danger_bg));
        let focus_style = Style::default()
            .fg(rtc(scheme.danger_focus_fg))
            .bg(rtc(scheme.danger_focus_bg))
            .add_modifier(Modifier::BOLD);
        let block = Block::default()
            .title(self.title.clone())
            .borders(Borders::ALL)
            .border_style(bg_style)
            .style(bg_style);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let yes_style = if self.yes { focus_style } else { bg_style };
        let no_style = if !self.yes { focus_style } else { bg_style };

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

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<bool> {
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return DialogOutcome::None;
        }
        let rect = centered_rect(60, 6, area);
        // Click outside dialog → cancel.
        if ev.column < rect.x
            || ev.column >= rect.x + rect.width
            || ev.row < rect.y
            || ev.row >= rect.y + rect.height
        {
            return DialogOutcome::Cancelled;
        }
        // The buttons row matches `render`: inner top-left is (rect.x+1, rect.y+1).
        // body lines: 0=message, 1=blank, 2=buttons, 3=blank, 4=hint.
        let inner_x = rect.x + 1;
        let inner_y = rect.y + 1;
        let buttons_y = inner_y + 2;
        if ev.row != buttons_y {
            return DialogOutcome::None;
        }
        // Spans: "        " (8), " [ Yes ] " (9), "    " (4), " [ No ] " (8).
        let yes_start = inner_x + 8;
        let yes_end = yes_start + 9;
        let no_start = yes_end + 4;
        let no_end = no_start + 8;
        if ev.column >= yes_start && ev.column < yes_end {
            self.yes = true;
            return DialogOutcome::Submitted(true);
        }
        if ev.column >= no_start && ev.column < no_end {
            self.yes = false;
            return DialogOutcome::Submitted(false);
        }
        DialogOutcome::None
    }
}
