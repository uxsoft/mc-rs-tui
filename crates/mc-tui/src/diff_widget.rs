//! Side-by-side diff modal — Phase 10 first cut.
//!
//! Loads two text files into memory (capped at 8 MiB each), computes a line
//! diff via mc-diff, and renders left/right panes with hunk navigation.

use std::path::{Path, PathBuf};

use mc_core::key::{KeyChord, KeyCode};
use mc_diff::{DiffModel, Row};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const MAX_BYTES: usize = 8 * 1024 * 1024;

pub struct DiffWidget {
    left_label: String,
    right_label: String,
    model: DiffModel,
    cursor: usize,
    view_offset: usize,
}

impl DiffWidget {
    pub fn open(left: &Path, right: &Path) -> std::io::Result<Self> {
        let l = read_capped(left)?;
        let r = read_capped(right)?;
        let model = DiffModel::build(&l, &r);
        let left_label = label_of(left);
        let right_label = label_of(right);
        Ok(Self {
            left_label,
            right_label,
            model,
            cursor: 0,
            view_offset: 0,
        })
    }

    /// Returns `true` to stay open, `false` to close.
    pub fn handle_key(&mut self, chord: KeyChord) -> bool {
        match chord.code {
            KeyCode::Escape | KeyCode::F(10) | KeyCode::Char('q') => return false,
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.model.rows.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.cursor =
                    (self.cursor + 20).min(self.model.rows.len().saturating_sub(1));
            }
            KeyCode::PageUp | KeyCode::Backspace => {
                self.cursor = self.cursor.saturating_sub(20);
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.model.rows.len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('n') => {
                if let Some(next) = self.model.next_hunk(self.cursor) {
                    self.cursor = next;
                }
            }
            KeyCode::Char('p') | KeyCode::Char('N') => {
                if let Some(prev) = self.model.prev_hunk(self.cursor) {
                    self.cursor = prev;
                }
            }
            _ => {}
        }
        true
    }

    pub fn render(&mut self, f: &mut Frame<'_>, area: Rect) {
        f.render_widget(Clear, area);
        let title = format!(
            " Diff [{} hunks]   {} | {} ",
            self.model.hunks.len(),
            self.left_label,
            self.right_label,
        );
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Blue))
            .style(Style::default().fg(Color::White).bg(Color::Blue));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let body = chunks[0];
        let hint = chunks[1];

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(body);

        let height = body.height as usize;
        if self.cursor < self.view_offset {
            self.view_offset = self.cursor;
        } else if height > 0 && self.cursor >= self.view_offset + height {
            self.view_offset = self.cursor + 1 - height;
        }

        let (left_lines, right_lines) = self.lines(self.view_offset, height);
        f.render_widget(
            Paragraph::new(left_lines).style(Style::default().fg(Color::White).bg(Color::Blue)),
            cols[0],
        );
        f.render_widget(
            Paragraph::new(right_lines).style(Style::default().fg(Color::White).bg(Color::Blue)),
            cols[1],
        );

        let hint_line = Line::from(vec![
            Span::raw(" "),
            Span::styled("Enter/n", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": next hunk  "),
            Span::styled("p", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": prev  "),
            Span::styled("Esc/F10/q", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": close"),
        ]);
        f.render_widget(
            Paragraph::new(hint_line).style(Style::default().bg(Color::Black).fg(Color::White)),
            hint,
        );
    }

    fn lines(
        &self,
        view_offset: usize,
        height: usize,
    ) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
        let mut left = Vec::with_capacity(height);
        let mut right = Vec::with_capacity(height);
        let end = (view_offset + height).min(self.model.rows.len());
        for i in view_offset..end {
            let row = &self.model.rows[i];
            let cursor = i == self.cursor;
            let (l_span, r_span) = match row {
                Row::Same(s) => {
                    let st = if cursor {
                        Style::default().fg(Color::Black).bg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::White).bg(Color::Blue)
                    };
                    (Span::styled(trim_nl(s), st), Span::styled(trim_nl(s), st))
                }
                Row::Removed(s) => {
                    let l = Style::default().fg(Color::White).bg(Color::Red);
                    let r = Style::default().fg(Color::DarkGray).bg(Color::Blue);
                    let cur = if cursor { l.add_modifier(Modifier::BOLD) } else { l };
                    (
                        Span::styled(trim_nl(s), cur),
                        Span::styled(String::new(), r),
                    )
                }
                Row::Added(s) => {
                    let l = Style::default().fg(Color::DarkGray).bg(Color::Blue);
                    let r = Style::default().fg(Color::White).bg(Color::Green);
                    let cur = if cursor { r.add_modifier(Modifier::BOLD) } else { r };
                    (
                        Span::styled(String::new(), l),
                        Span::styled(trim_nl(s), cur),
                    )
                }
                Row::Changed(l, r) => {
                    let lst = if cursor {
                        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White).bg(Color::Magenta)
                    };
                    let rst = if cursor {
                        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White).bg(Color::Magenta)
                    };
                    (Span::styled(trim_nl(l), lst), Span::styled(trim_nl(r), rst))
                }
            };
            left.push(Line::from(l_span));
            right.push(Line::from(r_span));
        }
        (left, right)
    }
}

fn trim_nl(s: &str) -> String {
    s.trim_end_matches(|c| c == '\n' || c == '\r').to_string()
}

fn read_capped(p: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let md = f.metadata()?;
    let total = md.len() as usize;
    let to_read = total.min(MAX_BYTES);
    let mut bytes = vec![0u8; to_read];
    f.read_exact(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn label_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Convenience: the App holds a couple of `PathBuf`s when it's about to open.
#[allow(dead_code)]
pub struct DiffOpenRequest {
    pub left: PathBuf,
    pub right: PathBuf,
}
