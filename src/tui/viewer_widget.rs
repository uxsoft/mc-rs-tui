//! Phase 3 starter: minimal in-process text/hex viewer.
//!
//! Loads a file into memory (capped at 16 MiB for now), supports text and hex
//! modes, page navigation, and Esc/F10/q to close. Phase 3 will replace this
//! with rope/mmap + search + marks.

use std::path::Path;
use std::sync::OnceLock;

use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use crate::tui::theme::rtc;

/// Maximum source bytes for image preview decoding. Anything larger
/// renders as a placeholder line in the viewer.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Cap source size for syntect — keeps highlight latency bounded on huge files.
const MAX_HIGHLIGHT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    Text,
    Highlighted,
    Hex,
    Image,
}

fn syntax_set() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static T: OnceLock<ThemeSet> = OnceLock::new();
    T.get_or_init(ThemeSet::load_defaults)
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
    /// Cached syntect output: per-line vector of styled spans. Built lazily
    /// the first time `Highlighted` mode is entered. `None` means "not built
    /// yet"; an empty Vec means "no syntax matched, fall back to plain text".
    highlighted: Option<Vec<Vec<(SynStyle, String)>>>,
    /// Extension hint for syntect detection (file extension lowercased, no dot).
    ext_hint: String,
    /// Decoded image protocol for inline preview. `Some` only when the file
    /// looks like an image and decode + protocol negotiation succeeded.
    image: Option<StatefulProtocol>,
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
        let ext_hint = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();

        let image = try_load_image(path, total_size as u64);
        let mode = if image.is_some() {
            ViewerMode::Image
        } else {
            ViewerMode::Text
        };
        Ok(Self {
            title,
            bytes,
            text_lines,
            mode,
            offset: 0,
            truncated,
            search: None,
            encoding,
            highlighted: None,
            ext_hint,
            image,
        })
    }

    fn ensure_highlighted(&mut self) {
        if self.highlighted.is_some() {
            return;
        }
        // Build only once per viewer instance. If we exceed the cap or
        // syntect has no matching syntax, store an empty Vec so we don't
        // retry every keystroke.
        let ss = syntax_set();
        let ts = theme_set();
        let theme = ts
            .themes
            .get("base16-ocean.dark")
            .or_else(|| ts.themes.values().next());
        let Some(theme) = theme else {
            self.highlighted = Some(Vec::new());
            return;
        };
        let syntax = ss
            .find_syntax_by_extension(&self.ext_hint)
            .or_else(|| ss.find_syntax_by_first_line(self.text_lines.first().map_or("", |s| s)));
        let Some(syntax) = syntax else {
            self.highlighted = Some(Vec::new());
            return;
        };
        let limited = self.bytes.len() > MAX_HIGHLIGHT_BYTES;
        let source = if limited {
            // Decode only the first MAX_HIGHLIGHT_BYTES through the active
            // encoding so we never highlight beyond the cap.
            self.encoding
                .decode(&self.bytes[..MAX_HIGHLIGHT_BYTES])
                .0
                .into_owned()
        } else {
            self.encoding.decode(&self.bytes).0.into_owned()
        };
        let mut h = HighlightLines::new(syntax, theme);
        let mut out: Vec<Vec<(SynStyle, String)>> = Vec::with_capacity(self.text_lines.len());
        for line in LinesWithEndings::from(&source) {
            let ranges = h.highlight_line(line, ss).unwrap_or_default();
            let row = ranges
                .into_iter()
                .map(|(st, s)| (st, s.trim_end_matches('\n').to_string()))
                .collect();
            out.push(row);
        }
        self.highlighted = Some(out);
    }

    fn cycle_encoding(&mut self) {
        let cur = self.encoding.name().to_string();
        let idx = ENCODINGS.iter().position(|e| e.name() == cur).unwrap_or(0);
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
                self.mode = match self.mode {
                    ViewerMode::Text | ViewerMode::Highlighted | ViewerMode::Image => {
                        ViewerMode::Hex
                    }
                    ViewerMode::Hex => {
                        if self.image.is_some() {
                            ViewerMode::Image
                        } else {
                            ViewerMode::Text
                        }
                    }
                };
            }
            KeyCode::Char('h') if chord.mods == crate::core::key::KeyMods::ALT => {
                self.mode = match self.mode {
                    ViewerMode::Text => {
                        self.ensure_highlighted();
                        ViewerMode::Highlighted
                    }
                    ViewerMode::Highlighted => ViewerMode::Text,
                    ViewerMode::Hex | ViewerMode::Image => self.mode,
                };
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
            KeyCode::Char('e') if chord.mods == crate::core::key::KeyMods::ALT => {
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

    pub fn render(&mut self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        f.render_widget(Clear, area);
        let panel = Style::default()
            .fg(rtc(scheme.panel_fg))
            .bg(rtc(scheme.panel_bg));
        let bar_btn = Style::default()
            .fg(rtc(scheme.buttonbar_fg))
            .bg(rtc(scheme.buttonbar_bg));
        let bar_lbl = Style::default()
            .fg(rtc(scheme.buttonbar_label_fg))
            .bg(rtc(scheme.buttonbar_label_bg));
        let search = Style::default()
            .fg(rtc(scheme.search_fg))
            .bg(rtc(scheme.search_bg));

        let mode_label = match self.mode {
            ViewerMode::Text => "TEXT",
            ViewerMode::Highlighted => "SYN ",
            ViewerMode::Hex => "HEX ",
            ViewerMode::Image => "IMG ",
        };
        let trunc = if self.truncated { " (truncated)" } else { "" };
        let enc = self.encoding.name();
        let title = format!(" View [{mode_label} {enc}] {}{} ", self.title, trunc);
        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(rtc(scheme.panel_title_fg))
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(rtc(scheme.panel_border))
                    .bg(rtc(scheme.panel_bg)),
            )
            .style(panel);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        if matches!(self.mode, ViewerMode::Image) {
            if let Some(proto) = self.image.as_mut() {
                let widget = StatefulImage::default();
                f.render_stateful_widget(widget, chunks[0], proto);
            } else {
                let p = Paragraph::new(Line::from("(image decode failed)")).style(panel);
                f.render_widget(p, chunks[0]);
            }
        } else {
            let lines = match self.mode {
                ViewerMode::Text => self.render_text(chunks[0].height as usize),
                ViewerMode::Highlighted => self.render_highlighted(chunks[0].height as usize),
                ViewerMode::Hex => self.render_hex(chunks[0].height as usize),
                ViewerMode::Image => Vec::new(),
            };
            f.render_widget(Paragraph::new(lines).style(panel), chunks[0]);
        }

        let bar = if let Some(state) = &self.search {
            if state.typing {
                Line::from(vec![
                    Span::styled("/", search.add_modifier(Modifier::BOLD)),
                    Span::styled(state.pattern.clone(), search),
                    Span::raw("    Enter: search   Esc: cancel"),
                ])
            } else {
                Line::from(vec![
                    Span::styled("found ", bar_btn),
                    Span::styled(
                        state.pattern.clone(),
                        Style::default()
                            .fg(rtc(scheme.search_bg))
                            .bg(rtc(scheme.statusbar_bg))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    n: next   N: prev   /: new search   q: close"),
                ])
            }
        } else {
            Line::from(vec![
                Span::styled("F4", bar_btn),
                Span::styled("Hex", bar_lbl),
                Span::raw("  "),
                Span::styled("M-h", bar_btn),
                Span::styled("Syntax", bar_lbl),
                Span::raw("  "),
                Span::styled("/", bar_btn),
                Span::styled("Search", bar_lbl),
                Span::raw("  "),
                Span::styled("F10", bar_btn),
                Span::styled("Quit", bar_lbl),
            ])
        };
        f.render_widget(Paragraph::new(bar).style(bar_btn), chunks[1]);
    }

    fn render_text(&self, height: usize) -> Vec<Line<'static>> {
        let start = self.offset.min(self.text_lines.len().saturating_sub(1));
        let end = (start + height).min(self.text_lines.len());
        self.text_lines[start..end]
            .iter()
            .map(|s| Line::from(s.clone()))
            .collect()
    }

    fn render_highlighted(&self, height: usize) -> Vec<Line<'static>> {
        let Some(rows) = self.highlighted.as_ref() else {
            return self.render_text(height);
        };
        if rows.is_empty() {
            return self.render_text(height);
        }
        let start = self.offset.min(rows.len().saturating_sub(1));
        let end = (start + height).min(rows.len());
        rows[start..end]
            .iter()
            .map(|spans| {
                let line: Vec<Span<'static>> = spans
                    .iter()
                    .map(|(s, t)| Span::styled(t.clone(), syn_to_rt(*s)))
                    .collect();
                Line::from(line)
            })
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
                let c = if (0x20..=0x7e).contains(&b) {
                    b as char
                } else {
                    '.'
                };
                s.push(c);
            }
            out.push(Line::from(s));
        }
        out
    }
}

/// Decode `path` as an image and build a stateful protocol for inline
/// rendering. Returns `None` for non-images, oversize files, or terminals
/// without graphics support (we still try unicode-halfblocks fallback).
fn try_load_image(path: &Path, size: u64) -> Option<StatefulProtocol> {
    if size == 0 || size > MAX_IMAGE_BYTES {
        return None;
    }
    // Cheap pre-filter: only attempt decode for files tree_magic_mini
    // identifies as image/*.
    let mime = tree_magic_mini::from_filepath(path).unwrap_or("application/octet-stream");
    if !mime.starts_with("image/") {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.decode().ok()?;
    let picker = picker_or_fallback();
    Some(picker.new_resize_protocol(img))
}

fn picker_or_fallback() -> Picker {
    Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
}

fn syn_to_rt(s: SynStyle) -> Style {
    let fg = Color::Rgb(s.foreground.r, s.foreground.g, s.foreground.b);
    let mut style = Style::default().fg(fg);
    let f = s.font_style;
    if f.contains(syntect::highlighting::FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if f.contains(syntect::highlighting::FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if f.contains(syntect::highlighting::FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn decode_lines(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Vec<String> {
    let (cow, _enc, _had_errors) = encoding.decode(bytes);
    cow.split_inclusive('\n')
        .map(|l| l.trim_end_matches('\n').to_string())
        .collect()
}
