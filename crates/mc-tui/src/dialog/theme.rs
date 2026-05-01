//! Theme picker: select one of the built-in color schemes.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

pub struct ThemeDialog {
    themes: &'static [(&'static str, &'static str)],
    cursor: usize,
}

impl ThemeDialog {
    #[must_use]
    pub fn new(current: &str) -> Self {
        let themes = ColorScheme::available_themes();
        let cursor = themes
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(current))
            .unwrap_or(0);
        Self { themes, cursor }
    }

    fn dialog_rect(&self, area: Rect) -> Rect {
        let height = u16::try_from(self.themes.len()).unwrap_or(0) + 4;
        centered_rect(40, height, area)
    }
}

impl Dialog for ThemeDialog {
    type Output = String;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = self.dialog_rect(area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Theme ",
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

        let mut lines: Vec<Line> = self
            .themes
            .iter()
            .enumerate()
            .map(|(i, (_, label))| {
                let style = if i == self.cursor {
                    Style::default()
                        .fg(rtc(scheme.dialog_focus_fg))
                        .bg(rtc(scheme.dialog_focus_bg))
                        .add_modifier(Modifier::BOLD)
                } else {
                    dlg
                };
                Line::from(Span::styled(format!(" {label:<36} "), style))
            })
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            " Enter: select   Esc: cancel",
            Style::default()
                .fg(rtc(scheme.panel_dim_fg))
                .bg(rtc(scheme.dialog_bg)),
        )));
        f.render_widget(Paragraph::new(lines).style(dlg), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<String> {
        let max = self.themes.len();
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < max {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogOutcome::None
            }
            KeyCode::End => {
                self.cursor = max.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Enter => match self.themes.get(self.cursor) {
                Some((name, _)) => DialogOutcome::Submitted((*name).to_string()),
                None => DialogOutcome::None,
            },
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<String> {
        let rect = self.dialog_rect(area);
        let inside = ev.column >= rect.x
            && ev.column < rect.x + rect.width
            && ev.row >= rect.y
            && ev.row < rect.y + rect.height;
        match ev.kind {
            MouseEventKind::ScrollUp if inside => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            MouseEventKind::ScrollDown if inside => {
                if self.cursor + 1 < self.themes.len() {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    return DialogOutcome::Cancelled;
                }
                let body_y = rect.y + 1;
                let row_in_body = ev.row.saturating_sub(body_y) as usize;
                if row_in_body >= self.themes.len() {
                    return DialogOutcome::None;
                }
                if row_in_body == self.cursor {
                    let (name, _) = self.themes[row_in_body];
                    return DialogOutcome::Submitted(name.to_string());
                }
                self.cursor = row_in_body;
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
