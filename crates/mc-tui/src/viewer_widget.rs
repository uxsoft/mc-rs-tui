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
    /// Active search state: pattern + last index in `text_lines`. `None` =
    /// no search; `Some` with empty pattern = prompt is open and user is typing.
    search: Option<SearchState>,
    encoding: &'static encoding_rs::Encoding,
}

const ENCODINGS: &[&encoding_rs::Encoding] = &[
    encoding_rs::UTF_8,
    encoding_rs::WINDOWS_1252,
    encoding_rs::ISO_8859_2,
    encoding_rs::WINDOWS_1250,
    encoding_rs::WINDOWS_1251,
    encoding_rs::SHIFT_JIS,
    encoding_rs::UTF_16LE,
    encoding_rs::UTF_16BE,
    encoding_rs::GBK,
];

#[derive(Debug, Clone, Default)]
struct SearchState {
    /// The pattern the user is typing (or has typed).
    pattern: String,
    /// `true` while the prompt is open. When `false`, the search has been
    /// committed and arrow keys do scroll, while `n`/`N` advance.
    typing: bool,
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

        let encoding = encoding_rs::UTF_8;
        let text_lines = decode_lines(&bytes, encoding);

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
            search: None,
            encoding,
        })
    }

    fn cycle_encoding(&mut self) {
        let cur = self
            .encoding
            .name()
            .to_string();
        let idx = ENCODINGS
            .iter()
            .position(|e| e.name() == cur)
            .unwrap_or(0);
        let next = (idx + 1) % ENCODINGS.len();
        self.encoding = ENCODINGS[next];
        self.text_lines = decode_lines(&self.bytes, self.encoding);
    }

    /// Returns `true` if still open, `false` to close.
    pub fn handle_key(&mut self, chord: KeyChord) -> bool {
        // Search prompt steals input while typing.
        if let Some(state) = &mut self.search {
            if state.typing {
                match chord.code {
                    KeyCode::Escape => {
                        self.search = None;
                    }
                    KeyCode::Enter => {
                        state.typing = false;
                        if !state.pattern.is_empty() {
                            self.find_from(self.offset, /*forward*/ true);
                        }
                    }
                    KeyCode::Backspace => {
                        state.pattern.pop();
                    }
                    KeyCode::Char(c) => {
                        state.pattern.push(c);
                    }
                    _ => {}
                }
                return true;
            }
        }

        match chord.code {
            KeyCode::Escape | KeyCode::F(10) | KeyCode::Char('q') => return false,
            KeyCode::F(4) => {
                self.mode = if self.mode == ViewerMode::Text { ViewerMode::Hex } else { ViewerMode::Text };
            }
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
            KeyCode::Char('/') => {
                self.search = Some(SearchState {
                    pattern: String::new(),
                    typing: true,
                });
            }
            KeyCode::Char('n') => {
                self.find_from(self.offset.saturating_add(1), true);
            }
            KeyCode::Char('N') => {
                self.find_from(self.offset.saturating_sub(1), false);
            }
            KeyCode::Char('e') if chord.mods == mc_core::key::KeyMods::ALT => {
                self.cycle_encoding();
            }
            _ => {}
        }
        true
    }

    fn find_from(&mut self, start: usize, forward: bool) {
        let pat = match &self.search {
            Some(s) if !s.pattern.is_empty() => s.pattern.to_lowercase(),
            _ => return,
        };
        let n = self.text_lines.len();
        if n == 0 {
            return;
        }
        let range: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(start..n)
        } else {
            Box::new((0..start.min(n)).rev())
        };
        for i in range {
            if self.text_lines[i].to_lowercase().contains(&pat) {
                self.offset = i;
                return;
            }
        }
    }

    pub fn render(&self, f: &mut Frame<'_>, area: Rect) {
        f.render_widget(Clear, area);
        let mode_label = match self.mode {
            ViewerMode::Text => "TEXT",
            ViewerMode::Hex => "HEX ",
        };
        let trunc = if self.truncated { " (truncated)" } else { "" };
        let enc = self.encoding.name();
        let title = format!(" View [{mode_label} {enc}] {}{} ", self.title, trunc);
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

        let bar = if let Some(state) = &self.search {
            if state.typing {
                Line::from(vec![
                    Span::styled("/", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(state.pattern.clone(), Style::default().fg(Color::Black).bg(Color::Yellow)),
                    Span::raw("    Enter: search   Esc: cancel"),
                ])
            } else {
                Line::from(vec![
                    Span::styled("found ", Style::default().fg(Color::White).bg(Color::Black)),
                    Span::styled(state.pattern.clone(), Style::default().fg(Color::Yellow).bg(Color::Black).add_modifier(Modifier::BOLD)),
                    Span::raw("    n: next   N: prev   /: new search   q: close"),
                ])
            }
        } else {
            Line::from(vec![
                Span::styled("F4", Style::default().fg(Color::White).bg(Color::Black)),
                Span::styled("Hex", Style::default().fg(Color::Black).bg(Color::Cyan)),
                Span::raw("  "),
                Span::styled("/", Style::default().fg(Color::White).bg(Color::Black)),
                Span::styled("Search", Style::default().fg(Color::Black).bg(Color::Cyan)),
                Span::raw("  "),
                Span::styled("F10", Style::default().fg(Color::White).bg(Color::Black)),
                Span::styled("Quit", Style::default().fg(Color::Black).bg(Color::Cyan)),
            ])
        };
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

fn decode_lines(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Vec<String> {
    let (cow, _enc, _had_errors) = encoding.decode(bytes);
    cow.split_inclusive('\n')
        .map(|l| l.trim_end_matches('\n').to_string())
        .collect()
}
