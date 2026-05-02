//! F2 user menu — pick a labelled shell-template entry to execute.
//!
//! Phase 11 first cut: ships a built-in default menu. Future work loads
//! `~/.config/mc-rs/menu.toml` and merges with defaults.

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
pub struct UserMenuEntry {
    pub hotkey: char,
    pub label: String,
    /// Shell template, with `%f`/`%d`/`%t`/etc. tokens.
    pub template: String,
}

pub struct UserMenuDialog {
    entries: Vec<UserMenuEntry>,
    cursor: usize,
}

impl UserMenuDialog {
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::with_entries(default_entries())
    }

    #[must_use]
    pub fn with_entries(entries: Vec<UserMenuEntry>) -> Self {
        Self { entries, cursor: 0 }
    }
}

impl Dialog for UserMenuDialog {
    type Output = String; // emitted shell template (pre-macro-substitution)

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(70, 14, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " User menu ",
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
            .entries
            .iter()
            .enumerate()
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
                    Span::styled(format!("  {}  ", e.hotkey), style),
                    Span::raw(format!(" {:<30} ", e.label)),
                    Span::raw(format!(" {} ", e.template)),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(dlg), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<String> {
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.entries.len() {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Enter => {
                if let Some(e) = self.entries.get(self.cursor) {
                    DialogOutcome::Submitted(e.template.clone())
                } else {
                    DialogOutcome::None
                }
            }
            KeyCode::Char(c) => {
                // Hotkey shortcut.
                if let Some((_, e)) = self
                    .entries
                    .iter()
                    .enumerate()
                    .find(|(_, e)| e.hotkey.eq_ignore_ascii_case(&c))
                {
                    DialogOutcome::Submitted(e.template.clone())
                } else {
                    DialogOutcome::None
                }
            }
            _ => DialogOutcome::None,
        }
    }
}

fn default_entries() -> Vec<UserMenuEntry> {
    vec![
        UserMenuEntry {
            hotkey: 'x',
            label: "Make file executable".into(),
            template: "chmod +x %f".into(),
        },
        UserMenuEntry {
            hotkey: 'o',
            label: "Compress directory to .tar.gz".into(),
            template: "tar czf %f.tar.gz %f".into(),
        },
        UserMenuEntry {
            hotkey: 'u',
            label: "Extract archive (tar/zip/gz/bz2/xz/zst)".into(),
            template: "case %f in *.tar) tar xf %f ;; *.tar.gz|*.tgz) tar xzf %f ;; *.tar.bz2|*.tbz2) tar xjf %f ;; *.tar.xz|*.txz) tar xJf %f ;; *.tar.zst|*.tzst) tar --zstd -xf %f ;; *.zip) unzip %f ;; *.gz) gunzip %f ;; *.bz2) bunzip2 %f ;; *.xz) unxz %f ;; *.zst) unzstd %f ;; esac".into(),
        },
        UserMenuEntry {
            hotkey: 'g',
            label: "git status".into(),
            template: "git -C %d status".into(),
        },
        UserMenuEntry {
            hotkey: 'd',
            label: "Disk usage of selected".into(),
            template: "du -sh %s".into(),
        },
        UserMenuEntry {
            hotkey: 'c',
            label: "Show file with bat / cat".into(),
            template: "if command -v bat >/dev/null; then bat -p %f; else cat %f; fi".into(),
        },
    ]
}
