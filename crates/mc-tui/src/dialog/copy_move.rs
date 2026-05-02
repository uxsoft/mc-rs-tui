//! Settings + confirmation dialog for the F5 (Copy) and F6 (Move) flows.
//!
//! Presents a destination path on the first row (editable, with cursor) and
//! a small set of bool toggles below it. Tab / arrows move focus between
//! rows; Space toggles the focused bool; Enter submits; Esc cancels.

use crossterm::event::MouseEvent;
use mc_config::ColorScheme;
use mc_core::key::{KeyChord, KeyCode, KeyMods};
use mc_jobs::CopyOptions;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

/// Submitted form values.
#[derive(Debug, Clone)]
pub struct CopyMoveSettings {
    pub dst: String,
    pub opts: CopyOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Dst,
    Overwrite,
    PreserveAttrs,
    FollowSymlinks,
}

const FIELDS: [Field; 4] = [
    Field::Dst,
    Field::Overwrite,
    Field::PreserveAttrs,
    Field::FollowSymlinks,
];

pub struct CopyMoveSettingsDialog {
    title: String,
    prompt: String,
    dst: String,
    cursor: usize,
    focus: usize,
    opts: CopyOptions,
}

impl CopyMoveSettingsDialog {
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        dst: impl Into<String>,
        opts: CopyOptions,
    ) -> Self {
        let dst = dst.into();
        let cursor = dst.chars().count();
        Self {
            title: title.into(),
            prompt: prompt.into(),
            dst,
            cursor,
            focus: 0,
            opts,
        }
    }

    fn focused(&self) -> Field {
        FIELDS[self.focus]
    }

    fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % FIELDS.len();
    }

    fn focus_prev(&mut self) {
        self.focus = if self.focus == 0 {
            FIELDS.len() - 1
        } else {
            self.focus - 1
        };
    }

    fn toggle_focused(&mut self) {
        match self.focused() {
            Field::Dst => {}
            Field::Overwrite => self.opts.overwrite = !self.opts.overwrite,
            Field::PreserveAttrs => self.opts.preserve_attrs = !self.opts.preserve_attrs,
            Field::FollowSymlinks => self.opts.follow_symlinks = !self.opts.follow_symlinks,
        }
    }

    fn rect(area: Rect) -> Rect {
        centered_rect(64, 11, area)
    }

    fn submit(&self) -> DialogOutcome<CopyMoveSettings> {
        if self.dst.is_empty() {
            DialogOutcome::Cancelled
        } else {
            DialogOutcome::Submitted(CopyMoveSettings {
                dst: self.dst.clone(),
                opts: self.opts,
            })
        }
    }
}

impl Dialog for CopyMoveSettingsDialog {
    type Output = CopyMoveSettings;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = Self::rect(area);
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

        let dst_focused = self.focused() == Field::Dst;
        let dst_style = if dst_focused {
            Style::default()
                .fg(rtc(scheme.input_fg))
                .bg(rtc(scheme.input_bg))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(rtc(scheme.input_fg))
                .bg(rtc(scheme.input_bg))
        };

        let bool_line = |focused: bool, value: bool, label: &str| -> Line<'static> {
            let s = format!(" [{}] {} ", if value { 'x' } else { ' ' }, label);
            let style = if focused {
                Style::default()
                    .fg(rtc(scheme.dialog_focus_fg))
                    .bg(rtc(scheme.dialog_focus_bg))
                    .add_modifier(Modifier::BOLD)
            } else {
                dlg
            };
            Line::from(Span::styled(s, style))
        };

        let lines = vec![
            Line::from(self.prompt.clone()),
            Line::from(Span::styled(self.dst.clone(), dst_style)),
            Line::from(""),
            bool_line(
                self.focused() == Field::Overwrite,
                self.opts.overwrite,
                "Overwrite existing files",
            ),
            bool_line(
                self.focused() == Field::PreserveAttrs,
                self.opts.preserve_attrs,
                "Preserve attributes",
            ),
            bool_line(
                self.focused() == Field::FollowSymlinks,
                self.opts.follow_symlinks,
                "Follow symlinks",
            ),
            Line::from(""),
            Line::from(Span::styled(
                "Tab/↑↓: move    Space: toggle    Enter: OK    Esc: Cancel",
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            )),
        ];
        f.render_widget(Paragraph::new(lines).style(dlg), inner);

        if dst_focused {
            let cur_x = inner.x
                + u16::try_from(self.cursor)
                    .unwrap_or(0)
                    .min(inner.width.saturating_sub(1));
            let cur_y = inner.y + 1;
            f.set_cursor_position((cur_x, cur_y));
        }
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<CopyMoveSettings> {
        match (chord.code, chord.mods) {
            (KeyCode::Escape, _) => DialogOutcome::Cancelled,
            (KeyCode::Enter, _) => self.submit(),
            (KeyCode::Tab | KeyCode::Down, _) => {
                self.focus_next();
                DialogOutcome::None
            }
            (KeyCode::BackTab | KeyCode::Up, _) => {
                self.focus_prev();
                DialogOutcome::None
            }
            // Destination-row text editing.
            (KeyCode::Backspace, _) if self.focused() == Field::Dst => {
                if self.cursor > 0 {
                    let new_cursor = self.cursor - 1;
                    let mut chars: Vec<char> = self.dst.chars().collect();
                    chars.remove(new_cursor);
                    self.dst = chars.into_iter().collect();
                    self.cursor = new_cursor;
                }
                DialogOutcome::None
            }
            (KeyCode::Delete, _) if self.focused() == Field::Dst => {
                let mut chars: Vec<char> = self.dst.chars().collect();
                if self.cursor < chars.len() {
                    chars.remove(self.cursor);
                    self.dst = chars.into_iter().collect();
                }
                DialogOutcome::None
            }
            (KeyCode::Left, _) if self.focused() == Field::Dst => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            (KeyCode::Right, _) if self.focused() == Field::Dst => {
                let len = self.dst.chars().count();
                if self.cursor < len {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            (KeyCode::Home, _) if self.focused() == Field::Dst => {
                self.cursor = 0;
                DialogOutcome::None
            }
            (KeyCode::End, _) if self.focused() == Field::Dst => {
                self.cursor = self.dst.chars().count();
                DialogOutcome::None
            }
            (KeyCode::Char(' '), m) if m.is_empty() && self.focused() != Field::Dst => {
                self.toggle_focused();
                DialogOutcome::None
            }
            (KeyCode::Char(c), m)
                if (m.is_empty() || m == KeyMods::SHIFT) && self.focused() == Field::Dst =>
            {
                let mut chars: Vec<char> = self.dst.chars().collect();
                chars.insert(self.cursor, c);
                self.dst = chars.into_iter().collect();
                self.cursor += 1;
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }

    fn handle_mouse(&mut self, _ev: MouseEvent, _area: Rect) -> DialogOutcome<CopyMoveSettings> {
        DialogOutcome::None
    }
}
