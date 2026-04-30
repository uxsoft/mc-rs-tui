//! Phase 3 starter: minimal in-process text/hex viewer.
//!
//! Loads a file into memory (capped at 16 MiB for now), supports text and hex
//! modes, page navigation, and Esc/F10/q to close. Phase 3 will replace this
//! with rope/mmap + search + marks.

use std::path::Path;

use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Text,
    Hex,
}

pub struct ViewerWidget {
    title: String,
    bytes: Vec<u8>,
    text_lines: Vec<String>,
    mode: ViewerMode,
    offset: usize,
    truncated: bool,
}

impl ViewerWidget {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        use std::io::Read;
        let mut f = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        let metadata = f.metadata()?;
        let total_size = metadata.len() as usize;
        let to_read = total_size.min(MAX_BYTES);
        bytes.resize(to_read, 0);
        f.read_exact(&mut bytes)?;
        let truncated = total_size > MAX_BYTES;

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let text_lines = text.split_inclusive('\n').map(|l| l.trim_end_matches('\n').to_string()).collect();

        let title = path.file_name().map_or_else(
            || path.display().to_string(),
            |s| s.to_string_lossy().into_owned(),
        );

        Ok(Self {
            title,
            bytes,
            text_lines,
            mode: ViewerMode::Text,
            offset: 0,
            truncated,
        })
    }

    /// Returns `true` if still open, `false` to close.
    pub fn handle_key(&mut self, chord: KeyChord) -> bool {
        match chord.code {
            KeyCode::Escape | KeyCode::F(10) | KeyCode::Char('q') => return false,
            KeyCode::F(4) => self.mode = if self.mode == ViewerMode::Text { ViewerMode::Hex } else { ViewerMode::Text },
            KeyCode::Down | KeyCode::Char('j') => self.offset += 1,
            KeyCode::Up | KeyCode::Char('k') => {
                self.offset = self.offset.saturating_sub(1);
            }
            KeyCode::PageDown | KeyCode::Char(' ') => self.offset += 20,
            KeyCode::PageUp | KeyCode::Backspace => {
                self.offset = self.offset.saturating_sub(20);
            }
            KeyCode::Home => self.offset = 0,
            KeyCode::End => self.offset = usize::MAX,
            _ => {}
        }
        true
    }

    pub fn render(&self, f: &mut Frame<'_>, area: Rect) {
        f.render_widget(Clear, area);
        let mode_label = match self.mode {
            ViewerMode::Text => "TEXT",
            ViewerMode::Hex => "HEX ",
        };
        let trunc = if self.truncated { " (truncated)" } else { "" };
        let title = format!(" View [{mode_label}] {}{} ", self.title, trunc);
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

        let lines = match self.mode {
            ViewerMode::Text => self.render_text(chunks[0].height as usize),
            ViewerMode::Hex => self.render_hex(chunks[0].height as usize),
        };
        f.render_widget(
            Paragraph::new(lines).style(Style::default().fg(Color::White).bg(Color::Blue)),
            chunks[0],
        );

        let bar = Line::from(vec![
            Span::styled("F4", Style::default().fg(Color::White).bg(Color::Black)),
            Span::styled("Hex", Style::default().fg(Color::Black).bg(Color::Cyan)),
            Span::raw("  "),
            Span::styled("F10", Style::default().fg(Color::White).bg(Color::Black)),
            Span::styled("Quit", Style::default().fg(Color::Black).bg(Color::Cyan)),
        ]);
        f.render_widget(
            Paragraph::new(bar).style(Style::default().bg(Color::Black).add_modifier(Modifier::DIM)),
            chunks[1],
        );
    }

    fn render_text(&self, height: usize) -> Vec<Line<'static>> {
        let start = self.offset.min(self.text_lines.len().saturating_sub(1));
        let end = (start + height).min(self.text_lines.len());
        self.text_lines[start..end]
            .iter()
            .map(|s| Line::from(s.clone()))
            .collect()
    }

    fn render_hex(&self, height: usize) -> Vec<Line<'static>> {
        const ROW: usize = 16;
        let total_rows = self.bytes.len().div_ceil(ROW);
        let start_row = self.offset.min(total_rows.saturating_sub(1));
        let end_row = (start_row + height).min(total_rows);
        let mut out = Vec::with_capacity(end_row - start_row);
        for row in start_row..end_row {
            let off = row * ROW;
            let chunk = &self.bytes[off..(off + ROW).min(self.bytes.len())];
            let mut s = format!("{off:08x}  ");
            for b in chunk {
                s.push_str(&format!("{b:02x} "));
            }
            for _ in chunk.len()..ROW {
                s.push_str("   ");
            }
            s.push(' ');
            for &b in chunk {
                let c = if (0x20..=0x7e).contains(&b) { b as char } else { '.' };
                s.push(c);
            }
            out.push(Line::from(s));
        }
        out
    }
}
