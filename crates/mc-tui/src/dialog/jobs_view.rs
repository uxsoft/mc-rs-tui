//! Background-jobs view (Ctrl-J).

use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

#[derive(Debug, Clone)]
pub struct JobRow {
    pub id_str: String,
    pub description: String,
    pub status: String,
    pub finished: bool,
}

pub struct JobsViewDialog {
    rows: Vec<JobRow>,
    cursor: usize,
    offset: usize,
}

impl JobsViewDialog {
    #[must_use]
    pub fn new(rows: Vec<JobRow>) -> Self {
        Self {
            rows,
            cursor: 0,
            offset: 0,
        }
    }
}

impl Dialog for JobsViewDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(80, 18, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Background jobs ",
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

        let layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(1),
                ratatui::layout::Constraint::Length(1),
            ])
            .split(inner);
        let body = layout[0];
        let hint_area = layout[1];

        let height = body.height as usize;
        let lines: Vec<Line> = if self.rows.is_empty() {
            vec![Line::from("(no jobs yet)")]
        } else {
            self.rows
                .iter()
                .enumerate()
                .skip(self.offset)
                .take(height)
                .map(|(i, r)| {
                    let style = if i == self.cursor {
                        Style::default()
                            .fg(rtc(scheme.dialog_focus_fg))
                            .bg(rtc(scheme.dialog_focus_bg))
                            .add_modifier(Modifier::BOLD)
                    } else if r.finished {
                        Style::default()
                            .fg(rtc(scheme.muted_fg))
                            .bg(rtc(scheme.dialog_bg))
                    } else {
                        dlg
                    };
                    Line::from(vec![
                        Span::styled(format!(" {:>4} ", r.id_str), style),
                        Span::raw(" "),
                        Span::raw(format!("{:<32}", truncate(&r.description, 32))),
                        Span::raw("  "),
                        Span::raw(r.status.clone()),
                    ])
                })
                .collect()
        };
        f.render_widget(Paragraph::new(lines).style(dlg), body);
        f.render_widget(
            Paragraph::new(Line::from("j/k or arrows: scroll    Esc: close")).style(
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            ),
            hint_area,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        match chord.code {
            KeyCode::Escape | KeyCode::F(10) | KeyCode::Char('q') => DialogOutcome::Cancelled,
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.rows.len() {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}

fn truncate(s: &str, n: usize) -> &str {
    let mut end = 0;
    for (i, _) in s.char_indices().take(n) {
        end = i;
    }
    if s.len() > end + 1 {
        // include the boundary char
        let bound = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
        &s[..bound]
    } else {
        s
    }
}
