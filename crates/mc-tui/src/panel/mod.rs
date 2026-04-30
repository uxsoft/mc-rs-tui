//! Panel state and rendering.

pub mod sort;

use std::collections::HashSet;
use std::path::PathBuf;

use mc_config::FileHighlight;
use mc_core::action::SortKey;
use mc_core::{Entry, EntryKind, VPath};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use sort::sort_entries;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingMode {
    Full,
    Brief,
    Long,
}

impl ListingMode {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Brief,
            Self::Brief => Self::Long,
            Self::Long => Self::Full,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PanelState {
    pub cwd: VPath,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub view_offset: usize,
    pub active: bool,
    pub show_hidden: bool,
    pub mix_dirs: bool,
    pub sort_by: SortKey,
    pub reverse: bool,
    pub mode: ListingMode,
    /// Set of entry names currently tagged.
    pub marks: HashSet<String>,
    /// Back/forward history of cwds visited from this panel.
    pub history: Vec<VPath>,
    pub history_pos: usize,
}

impl PanelState {
    #[must_use]
    pub fn new(cwd: VPath) -> Self {
        Self {
            history: vec![cwd.clone()],
            cwd,
            entries: Vec::new(),
            cursor: 0,
            view_offset: 0,
            active: false,
            show_hidden: false,
            mix_dirs: false,
            sort_by: SortKey::Name,
            reverse: false,
            mode: ListingMode::Full,
            marks: HashSet::new(),
            history_pos: 0,
        }
    }

    pub fn apply_filter_sort(&mut self) {
        if !self.show_hidden {
            self.entries.retain(|e| !e.name.starts_with('.') || e.name == "..");
        }
        sort_entries(&mut self.entries, self.sort_by, self.reverse, self.mix_dirs);
        if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len().saturating_sub(1);
        }
    }

    /// Set new cwd and push to history.
    pub fn navigate(&mut self, new_cwd: VPath) {
        // Drop any forward history when navigating somewhere new.
        self.history.truncate(self.history_pos + 1);
        self.history.push(new_cwd.clone());
        self.history_pos = self.history.len() - 1;
        self.cwd = new_cwd;
        self.cursor = 0;
        self.view_offset = 0;
        self.marks.clear();
    }

    pub fn history_back(&mut self) -> bool {
        if self.history_pos == 0 {
            return false;
        }
        self.history_pos -= 1;
        self.cwd = self.history[self.history_pos].clone();
        self.cursor = 0;
        self.view_offset = 0;
        self.marks.clear();
        true
    }

    pub fn history_fwd(&mut self) -> bool {
        if self.history_pos + 1 >= self.history.len() {
            return false;
        }
        self.history_pos += 1;
        self.cwd = self.history[self.history_pos].clone();
        self.cursor = 0;
        self.view_offset = 0;
        self.marks.clear();
        true
    }

    pub fn toggle_mark(&mut self) {
        let Some(e) = self.entries.get(self.cursor) else { return };
        if e.name == ".." {
            return;
        }
        let name = e.name.clone();
        if !self.marks.remove(&name) {
            self.marks.insert(name);
        }
        if self.cursor + 1 < self.entries.len() {
            self.cursor += 1;
        }
    }

    /// Resolve the cursor entry to a child VPath, if possible.
    #[must_use]
    pub fn cursor_path(&self) -> Option<VPath> {
        let e = self.entries.get(self.cursor)?;
        let mut cwd = self.cwd.clone();
        let last = cwd.last()?.clone();
        if e.name == ".." {
            // Pop one path component.
            let mut sub = last.sub.clone();
            if !sub.pop() {
                return None;
            }
            let mut new_layer = last;
            new_layer.sub = sub;
            cwd.pop_layer();
            cwd.push_layer(new_layer);
            Some(cwd)
        } else {
            let mut sub: PathBuf = last.sub.clone();
            sub.push(&e.name);
            let mut new_layer = last;
            new_layer.sub = sub;
            cwd.pop_layer();
            cwd.push_layer(new_layer);
            Some(cwd)
        }
    }
}

pub fn render_panel(
    f: &mut Frame<'_>,
    area: Rect,
    state: &mut PanelState,
    highlight: &FileHighlight,
) {
    let title_style = if state.active {
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::Blue)
    };
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(state.cwd.to_string(), title_style),
        Span::raw(" "),
    ]);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if state.active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        })
        .style(Style::default().bg(Color::Blue));

    // Reserve 1 row at bottom for status, body uses the rest.
    let inner = block.inner(area);
    f.render_widget(block.clone(), area);

    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let body_area = chunks[0];
    let status_area = chunks[1];

    // Adjust view_offset so cursor is visible.
    let height = body_area.height as usize;
    if height > 0 {
        if state.cursor < state.view_offset {
            state.view_offset = state.cursor;
        } else if state.cursor >= state.view_offset + height {
            state.view_offset = state.cursor + 1 - height;
        }
    }

    let lines = render_entries(state, body_area.width as usize, height, highlight);
    let para = Paragraph::new(lines).style(Style::default().bg(Color::Blue));
    f.render_widget(para, body_area);

    // Status line: cursor position + size info.
    let status = if let Some(e) = state.entries.get(state.cursor) {
        format!("{:>10}  {}", human_size(e.size), e.name)
    } else {
        String::from("(empty)")
    };
    let status_line =
        Paragraph::new(status).style(Style::default().fg(Color::White).bg(Color::Blue));
    f.render_widget(status_line, status_area);
}

fn render_entries(
    state: &PanelState,
    width: usize,
    height: usize,
    highlight: &FileHighlight,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height);
    let end = (state.view_offset + height).min(state.entries.len());
    for i in state.view_offset..end {
        let e = &state.entries[i];
        let is_cursor = i == state.cursor && state.active;
        let is_marked = state.marks.contains(&e.name);
        lines.push(format_line(e, state.mode, width, is_cursor, is_marked, highlight));
    }
    lines
}

fn format_line(
    e: &Entry,
    mode: ListingMode,
    width: usize,
    is_cursor: bool,
    is_marked: bool,
    highlight: &FileHighlight,
) -> Line<'static> {
    let mut style = entry_style(e, highlight);
    if is_marked {
        style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    }
    if is_cursor {
        style = style.bg(Color::Cyan).fg(Color::Black);
    }
    let text = match mode {
        ListingMode::Brief => e.name.clone(),
        ListingMode::Full => format!("{:<name$} {:>10}", e.name, human_size(e.size), name = width.saturating_sub(13)),
        ListingMode::Long => {
            let mode_str = unix_mode_str(e);
            format!(
                "{} {:>4} {:>10} {:<name$}",
                mode_str,
                e.nlink.unwrap_or(0),
                human_size(e.size),
                e.name,
                name = width.saturating_sub(28),
            )
        }
    };
    Line::from(Span::styled(text, style))
}

fn entry_style(e: &Entry, highlight: &FileHighlight) -> Style {
    let base = Style::default().fg(Color::White).bg(Color::Blue);
    match e.kind {
        EntryKind::Dir => base.add_modifier(Modifier::BOLD),
        EntryKind::Symlink => base.fg(Color::Cyan),
        EntryKind::Fifo | EntryKind::Socket => base.fg(Color::Magenta),
        EntryKind::BlockDevice | EntryKind::CharDevice => base.fg(Color::Yellow),
        EntryKind::File => {
            if let Some(mode) = e.mode {
                if mode & 0o111 != 0 {
                    return base.fg(Color::Green);
                }
            }
            // File-highlight by extension group.
            match highlight.classify(&e.name) {
                Some("archive") => base.fg(Color::Red),
                Some("image") => base.fg(Color::Magenta),
                Some("audio") | Some("video") => base.fg(Color::LightMagenta),
                Some("doc") => base.fg(Color::White).add_modifier(Modifier::BOLD),
                Some("source") => base.fg(Color::LightCyan),
                Some("build") => base.fg(Color::Yellow),
                _ => base,
            }
        }
        EntryKind::Other => base,
    }
}

fn unix_mode_str(e: &Entry) -> String {
    let kind_ch = match e.kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink => 'l',
        EntryKind::Fifo => 'p',
        EntryKind::Socket => 's',
        EntryKind::BlockDevice => 'b',
        EntryKind::CharDevice => 'c',
        EntryKind::File | EntryKind::Other => '-',
    };
    let m = e.mode.unwrap_or(0);
    let bit = |mask: u32, c: char| if m & mask != 0 { c } else { '-' };
    format!(
        "{}{}{}{}{}{}{}{}{}{}",
        kind_ch,
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        bit(0o100, 'x'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        bit(0o010, 'x'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        bit(0o001, 'x'),
    )
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut size = bytes as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes}")
    } else {
        format!("{size:.1}{}", UNITS[idx])
    }
}
