use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::tui::theme::rtc;

/// OK-only modal that shows an error message and stays on screen until the
/// user acknowledges it (Enter / Esc / click).
pub struct ErrorDialog {
    title: String,
    message: String,
}

impl ErrorDialog {
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }

    /// Compute the on-screen rect. Mirrors the layout used by `render` so the
    /// mouse handler can hit-test against the same geometry.
    fn rect(&self, area: Rect) -> Rect {
        let max_w = area.width.saturating_mul(8) / 10;
        let width = max_w.max(40).min(area.width);
        let inner_width = width.saturating_sub(2).max(1);
        let msg_lines: u16 = self
            .message
            .split('\n')
            .map(|line| {
                let len = u16::try_from(line.chars().count()).unwrap_or(u16::MAX);
                len.div_ceil(inner_width).max(1)
            })
            .sum();
        // body = msg_lines + blank + button + blank + hint
        let height = msg_lines
            .saturating_add(4)
            .saturating_add(2)
            .min(area.height);
        centered_rect(width, height, area)
    }
}

impl Dialog for ErrorDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = self.rect(area);
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

        let mut lines: Vec<Line> = self
            .message
            .split('\n')
            .map(|l| Line::from(l.to_string()))
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            ratatui::text::Span::raw("        "),
            ratatui::text::Span::styled(" [ OK ] ", focus_style),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from("Enter/Esc: dismiss"));

        let body = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(body, inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        match chord.code {
            KeyCode::Enter | KeyCode::Escape | KeyCode::Char(' ') => DialogOutcome::Submitted(()),
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<()> {
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return DialogOutcome::None;
        }
        let rect = self.rect(area);
        // Click outside the dialog dismisses it.
        if ev.column < rect.x
            || ev.column >= rect.x + rect.width
            || ev.row < rect.y
            || ev.row >= rect.y + rect.height
        {
            return DialogOutcome::Submitted(());
        }
        // Left-click anywhere inside the dialog also dismisses — there's only
        // one action available.
        DialogOutcome::Submitted(())
    }
}
