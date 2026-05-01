//! Generic options dialog: a vertical list of named boolean (checkbox),
//! integer, or text fields. Used by Configuration, Confirmation, and
//! Virtual FS settings.

use mc_config::{AppConfig, ColorScheme};
use mc_core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::{Dialog, DialogOutcome, centered_rect};
use crate::theme::rtc;

#[derive(Debug, Clone)]
pub enum OptionField {
    Bool {
        label: &'static str,
        value: bool,
        key: OptionKey,
    },
    Int {
        label: &'static str,
        value: u32,
        key: OptionKey,
    },
    Text {
        label: &'static str,
        value: String,
        key: OptionKey,
    },
}

/// Identifies which `AppConfig` field a row writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKey {
    ShowHidden,
    MixDirs,
    CaseSensitiveSort,
    UseInternalView,
    ConfirmDelete,
    ConfirmOverwrite,
    ConfirmExit,
    ConfirmExecute,
    VfsTimeout,
    VfsAnonymousPassword,
}

pub struct OptionsDialog {
    title: &'static str,
    fields: Vec<OptionField>,
    cursor: usize,
}

impl OptionsDialog {
    /// Configuration submenu (panel + general options).
    #[must_use]
    pub fn configuration(c: &AppConfig) -> Self {
        Self {
            title: " Configuration ",
            cursor: 0,
            fields: vec![
                OptionField::Bool {
                    label: "Show hidden files",
                    value: c.panels.show_hidden,
                    key: OptionKey::ShowHidden,
                },
                OptionField::Bool {
                    label: "Mix dirs and files",
                    value: c.panels.mix_dirs,
                    key: OptionKey::MixDirs,
                },
                OptionField::Bool {
                    label: "Case-sensitive sort",
                    value: c.panels.case_sensitive_sort,
                    key: OptionKey::CaseSensitiveSort,
                },
                OptionField::Bool {
                    label: "Use internal viewer",
                    value: c.options.use_internal_view,
                    key: OptionKey::UseInternalView,
                },
            ],
        }
    }

    /// Confirmation submenu (the four confirm_* flags).
    #[must_use]
    pub fn confirmation(c: &AppConfig) -> Self {
        Self {
            title: " Confirmation ",
            cursor: 0,
            fields: vec![
                OptionField::Bool {
                    label: "Confirm delete",
                    value: c.options.confirm_delete,
                    key: OptionKey::ConfirmDelete,
                },
                OptionField::Bool {
                    label: "Confirm overwrite",
                    value: c.options.confirm_overwrite,
                    key: OptionKey::ConfirmOverwrite,
                },
                OptionField::Bool {
                    label: "Confirm exit",
                    value: c.options.confirm_exit,
                    key: OptionKey::ConfirmExit,
                },
                OptionField::Bool {
                    label: "Confirm execute",
                    value: c.options.confirm_execute,
                    key: OptionKey::ConfirmExecute,
                },
            ],
        }
    }

    /// Virtual FS settings submenu.
    #[must_use]
    pub fn vfs(c: &AppConfig) -> Self {
        Self {
            title: " Virtual FS ",
            cursor: 0,
            fields: vec![
                OptionField::Int {
                    label: "Timeout (seconds)",
                    value: c.vfs.timeout_secs,
                    key: OptionKey::VfsTimeout,
                },
                OptionField::Text {
                    label: "Anonymous FTP password",
                    value: c.vfs.ftp_anonymous_password.clone(),
                    key: OptionKey::VfsAnonymousPassword,
                },
            ],
        }
    }

    /// Apply the dialog's edited values back into `cfg` in-place.
    pub fn apply(&self, cfg: &mut AppConfig) {
        for f in &self.fields {
            match f {
                OptionField::Bool { value, key, .. } => match key {
                    OptionKey::ShowHidden => cfg.panels.show_hidden = *value,
                    OptionKey::MixDirs => cfg.panels.mix_dirs = *value,
                    OptionKey::CaseSensitiveSort => cfg.panels.case_sensitive_sort = *value,
                    OptionKey::UseInternalView => cfg.options.use_internal_view = *value,
                    OptionKey::ConfirmDelete => cfg.options.confirm_delete = *value,
                    OptionKey::ConfirmOverwrite => cfg.options.confirm_overwrite = *value,
                    OptionKey::ConfirmExit => cfg.options.confirm_exit = *value,
                    OptionKey::ConfirmExecute => cfg.options.confirm_execute = *value,
                    _ => {}
                },
                OptionField::Int { value, key, .. } => {
                    if matches!(key, OptionKey::VfsTimeout) {
                        cfg.vfs.timeout_secs = *value;
                    }
                }
                OptionField::Text { value, key, .. } => {
                    if matches!(key, OptionKey::VfsAnonymousPassword) {
                        cfg.vfs.ftp_anonymous_password = value.clone();
                    }
                }
            }
        }
    }
}

impl Dialog for OptionsDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let h = (self.fields.len() as u16) + 4;
        let rect = centered_rect(60, h.max(8), area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                self.title,
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

        let lines: Vec<Line> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, fld)| {
                let style = if i == self.cursor {
                    Style::default()
                        .fg(rtc(scheme.dialog_focus_fg))
                        .bg(rtc(scheme.dialog_focus_bg))
                        .add_modifier(Modifier::BOLD)
                } else {
                    dlg
                };
                let s = match fld {
                    OptionField::Bool { label, value, .. } => {
                        format!(" [{}] {} ", if *value { 'x' } else { ' ' }, label)
                    }
                    OptionField::Int { label, value, .. } => format!(" {label}: {value} "),
                    OptionField::Text { label, value, .. } => format!(" {label}: {value} "),
                };
                Line::from(Span::styled(s, style))
            })
            .collect();

        let body_h = inner.height.saturating_sub(1);
        let body = Rect::new(inner.x, inner.y, inner.width, body_h);
        let hint = Rect::new(inner.x, inner.y + body_h, inner.width, 1);
        f.render_widget(Paragraph::new(lines).style(dlg), body);
        f.render_widget(
            Paragraph::new(Line::from(
                "↑↓: move    Space: toggle    +/-: adjust    type: edit text    Enter: ok    Esc: cancel",
            ))
            .style(
                Style::default()
                    .fg(rtc(scheme.panel_dim_fg))
                    .bg(rtc(scheme.dialog_bg)),
            ),
            hint,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        let max = self.fields.len();
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Enter => DialogOutcome::Submitted(()),
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < max {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Char(' ') => {
                if let Some(OptionField::Bool { value, .. }) = self.fields.get_mut(self.cursor) {
                    *value = !*value;
                }
                DialogOutcome::None
            }
            KeyCode::Char('+') => {
                if let Some(OptionField::Int { value, .. }) = self.fields.get_mut(self.cursor) {
                    *value = value.saturating_add(1);
                }
                DialogOutcome::None
            }
            KeyCode::Char('-') => {
                if let Some(OptionField::Int { value, .. }) = self.fields.get_mut(self.cursor) {
                    *value = value.saturating_sub(1);
                }
                DialogOutcome::None
            }
            KeyCode::Backspace => {
                if let Some(OptionField::Text { value, .. }) = self.fields.get_mut(self.cursor) {
                    value.pop();
                } else if let Some(OptionField::Int { value, .. }) =
                    self.fields.get_mut(self.cursor)
                {
                    *value = *value / 10;
                }
                DialogOutcome::None
            }
            KeyCode::Char(c) => {
                if let Some(fld) = self.fields.get_mut(self.cursor) {
                    match fld {
                        OptionField::Text { value, .. } => value.push(c),
                        OptionField::Int { value, .. } if c.is_ascii_digit() => {
                            *value = value
                                .saturating_mul(10)
                                .saturating_add(c as u32 - '0' as u32);
                        }
                        _ => {}
                    }
                }
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
