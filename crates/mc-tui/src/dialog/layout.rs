//! Layout dialog: vertical/horizontal split toggle and left-panel size %.

use mc_config::{ColorScheme, LayoutConfig};
use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{centered_rect, Dialog, DialogOutcome};
use crate::theme::rtc;

pub struct LayoutDialog {
    cfg: LayoutConfig,
    cursor: usize,
}

impl LayoutDialog {
    #[must_use]
    pub fn new(cfg: LayoutConfig) -> Self {
        Self { cfg, cursor: 0 }
    }
}

impl Dialog for LayoutDialog {
    type Output = LayoutConfig;

    fn render(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let rect = centered_rect(50, 8, area);
        f.render_widget(Clear, rect);
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let block = Block::default()
            .title(Span::styled(
                " Layout ",
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

        let style_for = |i: usize| -> Style {
            if i == self.cursor {
                Style::default()
                    .fg(rtc(scheme.dialog_focus_fg))
                    .bg(rtc(scheme.dialog_focus_bg))
                    .add_modifier(Modifier::BOLD)
            } else {
                dlg
            }
        };

        let orient = if self.cfg.vertical {
            "( ) Horizontal split    (•) Vertical split"
        } else {
            "(•) Horizontal split    ( ) Vertical split"
        };

        let lines = vec![
            Line::from(Span::styled(format!(" {orient} "), style_for(0))),
            Line::from(Span::styled(
                format!(" Left/top size: {}% ", self.cfg.left_pct),
                style_for(1),
            )),
            Line::raw(""),
            Line::raw(" Space: toggle orientation    +/-: adjust %    Enter: ok    Esc: cancel"),
        ];
        f.render_widget(Paragraph::new(lines).style(dlg), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<LayoutConfig> {
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Enter => {
                let mut c = self.cfg;
                c.left_pct = c.left_pct.clamp(1, 99);
                DialogOutcome::Submitted(c)
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Down => {
                if self.cursor < 1 {
                    self.cursor += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Char(' ') if self.cursor == 0 => {
                self.cfg.vertical = !self.cfg.vertical;
                DialogOutcome::None
            }
            KeyCode::Char('+') | KeyCode::Right if self.cursor == 1 => {
                self.cfg.left_pct = self.cfg.left_pct.saturating_add(1).min(99);
                DialogOutcome::None
            }
            KeyCode::Char('-') | KeyCode::Left if self.cursor == 1 => {
                self.cfg.left_pct = self.cfg.left_pct.saturating_sub(1).max(1);
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}
