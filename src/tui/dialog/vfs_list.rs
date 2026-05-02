//! Active VFS list dialog. Shows all (scheme, location) mounts currently
//! registered in the VFS registry; lets the user navigate to one or
//! disconnect (free) it.

use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::tui::theme::rtc;

#[derive(Debug, Clone)]
pub enum VfsListAction {
    /// Navigate the active panel to `scheme://location/`.
    Goto { scheme: String, location: String },
    /// Unregister the mount.
    Free { scheme: String, location: String },
}

pub struct VfsListDialog {
    mounts: Vec<(String, String)>,
    cursor: usize,
    view_offset: usize,
}

impl VfsListDialog {
    #[must_use]
    pub fn new(mut mounts: Vec<(String, String)>) -> Self {
        mounts.sort();
        Self {
            mounts,
            cursor: 0,
            view_offset: 0,
        }
    }
}

impl Dialog for VfsListDialog {
    type Output = VfsListAction;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(78, 18, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Active VFS list ",
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
        let lines: Vec<Line> = if self.mounts.is_empty() {
            vec![Line::from("(no active mounts)")]
        } else {
            self.mounts
                .iter()
                .enumerate()
                .skip(self.view_offset)
                .take(height)
                .map(|(i, (scheme_, loc))| {
                    let style = if i == self.cursor {
                        Style::default()
                            .fg(rtc(scheme.dialog_focus_fg))
                            .bg(rtc(scheme.dialog_focus_bg))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        dlg
                    };
                    Line::from(Span::styled(format!(" {scheme_}://{loc} "), style))
                })
                .collect()
        };
        f.render_widget(Paragraph::new(lines).style(dlg), body);
        f.render_widget(
            Paragraph::new(Line::from("Enter: cd    Del/F8: free    Esc: close")).style(
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            ),
            hint_area,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<VfsListAction> {
        let max = self.mounts.len();
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
            KeyCode::Delete | KeyCode::F(8) => {
                if let Some((s, l)) = self.mounts.get(self.cursor).cloned() {
                    DialogOutcome::Submitted(VfsListAction::Free {
                        scheme: s,
                        location: l,
                    })
                } else {
                    DialogOutcome::None
                }
            }
            KeyCode::Enter => {
                if let Some((s, l)) = self.mounts.get(self.cursor).cloned() {
                    DialogOutcome::Submitted(VfsListAction::Goto {
                        scheme: s,
                        location: l,
                    })
                } else {
                    DialogOutcome::None
                }
            }
            _ => DialogOutcome::None,
        }
    }
}
