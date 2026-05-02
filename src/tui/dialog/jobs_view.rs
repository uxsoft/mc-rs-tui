//! Background-jobs view (Ctrl-J).
//!
//! Scrollable, searchable list of recent jobs. Typing characters filters the
//! list by `description`, `status`, or job id (case-insensitive substring).

use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode, KeyMods};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::tui::theme::rtc;

const DIALOG_W: u16 = 80;
const DIALOG_H: u16 = 18;
// Inner area = DIALOG_H - 2 (borders) = 16.
// Layout splits inner into list / search / hint, so list = 14 rows.
const LIST_HEIGHT: usize = 14;
const PAGE: usize = 12;

#[derive(Debug, Clone)]
pub struct JobRow {
    pub id_str: String,
    pub description: String,
    pub status: String,
    pub finished: bool,
}

pub struct JobsViewDialog {
    rows: Vec<JobRow>,
    /// Indices into `rows` that match the current `query`.
    filtered: Vec<usize>,
    query: String,
    /// Index into `filtered`.
    cursor: usize,
    /// Top of viewport, index into `filtered`.
    offset: usize,
}

impl JobsViewDialog {
    #[must_use]
    pub fn new(rows: Vec<JobRow>) -> Self {
        let filtered = (0..rows.len()).collect();
        Self {
            rows,
            filtered,
            query: String::new(),
            cursor: 0,
            offset: 0,
        }
    }

    fn recompute_filter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches_query(r, &q))
            .map(|(i, _)| i)
            .collect();
        self.cursor = 0;
        self.offset = 0;
    }

    fn scroll_into_view(&mut self) {
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + LIST_HEIGHT {
            self.offset = self.cursor + 1 - LIST_HEIGHT;
        }
    }

    fn move_down(&mut self, n: usize) {
        if self.filtered.is_empty() {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        let last = self.filtered.len() - 1;
        self.cursor = self.cursor.saturating_add(n).min(last);
        self.scroll_into_view();
    }

    fn move_up(&mut self, n: usize) {
        if self.filtered.is_empty() {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        self.cursor = self.cursor.saturating_sub(n);
        self.scroll_into_view();
    }
}

fn matches_query(row: &JobRow, q_lower: &str) -> bool {
    if q_lower.is_empty() {
        return true;
    }
    row.description.to_lowercase().contains(q_lower)
        || row.status.to_lowercase().contains(q_lower)
        || row.id_str.contains(q_lower)
}

impl Dialog for JobsViewDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(DIALOG_W, DIALOG_H, area);
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
                ratatui::layout::Constraint::Length(1),
            ])
            .split(inner);
        let body = layout[0];
        let search_area = layout[1];
        let hint_area = layout[2];

        let height = body.height as usize;
        let lines: Vec<Line> = if self.rows.is_empty() {
            vec![Line::from("(no jobs yet)")]
        } else if self.filtered.is_empty() {
            vec![Line::from("(no matches)")]
        } else {
            self.filtered
                .iter()
                .enumerate()
                .skip(self.offset)
                .take(height)
                .map(|(i, &row_idx)| {
                    let r = &self.rows[row_idx];
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

        // Search row: " Search: <query>_                              N/M "
        let focus = Style::default()
            .fg(rtc(scheme.dialog_focus_fg))
            .bg(rtc(scheme.dialog_focus_bg));
        let counter = format!(" {}/{} ", self.filtered.len(), self.rows.len());
        let search_text = format!(" Search: {}_", self.query);
        let avail = search_area.width as usize;
        let counter_len = counter.chars().count();
        let pad = avail
            .saturating_sub(search_text.chars().count())
            .saturating_sub(counter_len);
        let search_line = Line::from(vec![
            Span::styled(search_text, focus),
            Span::styled(" ".repeat(pad), focus),
            Span::styled(
                counter,
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_focus_bg)),
            ),
        ]);
        f.render_widget(Paragraph::new(search_line), search_area);

        f.render_widget(
            Paragraph::new(Line::from(
                "type to filter   \u{2191}/\u{2193} PgUp/PgDn Home/End: scroll   Esc: clear/close",
            ))
            .style(
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            ),
            hint_area,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        // Hard close: F10 always closes regardless of state.
        if chord.code == KeyCode::F(10) {
            return DialogOutcome::Cancelled;
        }
        // Esc: clear query first, then close on a second press.
        if chord.code == KeyCode::Escape {
            if self.query.is_empty() {
                return DialogOutcome::Cancelled;
            }
            self.query.clear();
            self.recompute_filter();
            return DialogOutcome::None;
        }

        match chord.code {
            KeyCode::Down => {
                self.move_down(1);
                DialogOutcome::None
            }
            KeyCode::Up => {
                self.move_up(1);
                DialogOutcome::None
            }
            KeyCode::PageDown => {
                self.move_down(PAGE);
                DialogOutcome::None
            }
            KeyCode::PageUp => {
                self.move_up(PAGE);
                DialogOutcome::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.offset = 0;
                DialogOutcome::None
            }
            KeyCode::End => {
                if !self.filtered.is_empty() {
                    self.cursor = self.filtered.len() - 1;
                    self.scroll_into_view();
                }
                DialogOutcome::None
            }
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.recompute_filter();
                }
                DialogOutcome::None
            }
            KeyCode::Char(c) if chord.mods.is_empty() || chord.mods == KeyMods::SHIFT => {
                self.query.push(c);
                self.recompute_filter();
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<()> {
        let rect = centered_rect(DIALOG_W, DIALOG_H, area);
        let inside = ev.column >= rect.x
            && ev.column < rect.x + rect.width
            && ev.row >= rect.y
            && ev.row < rect.y + rect.height;
        match ev.kind {
            MouseEventKind::ScrollUp if inside => {
                self.move_up(1);
                DialogOutcome::None
            }
            MouseEventKind::ScrollDown if inside => {
                self.move_down(1);
                DialogOutcome::None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    return DialogOutcome::Cancelled;
                }
                let body_y = rect.y + 1;
                // Inner height minus the search row (1) and hint row (1).
                let body_h = rect.height.saturating_sub(2).saturating_sub(2);
                if ev.row < body_y || ev.row >= body_y + body_h {
                    return DialogOutcome::None;
                }
                let row_in_body = (ev.row - body_y) as usize;
                let target = self.offset + row_in_body;
                if target < self.filtered.len() {
                    self.cursor = target;
                    self.scroll_into_view();
                }
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
