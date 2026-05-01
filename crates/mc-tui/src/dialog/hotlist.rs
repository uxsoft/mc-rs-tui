use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use mc_config::{ColorScheme, Hotlist};
use mc_core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

pub enum HotlistAction {
    /// Navigate to the path display string.
    Navigate(String),
    /// Add the active panel cwd to the hotlist.
    AddCurrent,
    /// Remove the entry at this index.
    Remove(usize),
}

pub struct HotlistDialog {
    pub hotlist: Hotlist,
    cursor: usize,
    view_offset: usize,
}

impl HotlistDialog {
    #[must_use]
    pub fn new(hotlist: Hotlist) -> Self {
        Self {
            hotlist,
            cursor: 0,
            view_offset: 0,
        }
    }
}

impl Dialog for HotlistDialog {
    type Output = HotlistAction;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(70, 18, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Hotlist ",
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
        let lines: Vec<Line> = if self.hotlist.entries.is_empty() {
            vec![Line::from(
                "(no entries — press 'a' to add current directory)",
            )]
        } else {
            self.hotlist
                .entries
                .iter()
                .enumerate()
                .skip(self.view_offset)
                .take(height)
                .map(|(i, e)| {
                    let style = if i == self.cursor {
                        Style::default()
                            .fg(rtc(scheme.dialog_focus_fg))
                            .bg(rtc(scheme.dialog_focus_bg))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        dlg
                    };
                    Line::from(vec![
                        Span::styled(format!(" {:<22} ", e.label), style),
                        Span::raw(" "),
                        Span::raw(e.path.clone()),
                    ])
                })
                .collect()
        };
        f.render_widget(Paragraph::new(lines).style(dlg), body);
        f.render_widget(
            Paragraph::new(Line::from(
                "Enter: cd    a: add current    d: delete    Esc: close",
            ))
            .style(
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            ),
            hint_area,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<HotlistAction> {
        let max = self.hotlist.entries.len();
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Char('a') | KeyCode::Char('A') => {
                DialogOutcome::Submitted(HotlistAction::AddCurrent)
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if max == 0 {
                    DialogOutcome::None
                } else {
                    DialogOutcome::Submitted(HotlistAction::Remove(self.cursor))
                }
            }
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
            KeyCode::Enter => {
                if let Some(e) = self.hotlist.entries.get(self.cursor) {
                    DialogOutcome::Submitted(HotlistAction::Navigate(e.path.clone()))
                } else {
                    DialogOutcome::None
                }
            }
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent, area: Rect) -> DialogOutcome<HotlistAction> {
        let rect = centered_rect(70, 18, area);
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
                if self.cursor + 1 < self.hotlist.entries.len() {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    return DialogOutcome::Cancelled;
                }
                // Inner body sits inside the 1-col border, status hint is the
                // last row inside (height 1).
                let body_y = rect.y + 1;
                let body_h = rect.height.saturating_sub(2).saturating_sub(1);
                if ev.row < body_y || ev.row >= body_y + body_h {
                    return DialogOutcome::None;
                }
                let row_in_body = (ev.row - body_y) as usize;
                let target = self.view_offset + row_in_body;
                if target >= self.hotlist.entries.len() {
                    return DialogOutcome::None;
                }
                if target == self.cursor {
                    let path = self.hotlist.entries[target].path.clone();
                    return DialogOutcome::Submitted(HotlistAction::Navigate(path));
                }
                self.cursor = target;
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
