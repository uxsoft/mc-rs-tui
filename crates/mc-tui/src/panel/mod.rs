//! Panel state and rendering.

pub mod sort;

use std::collections::HashSet;
use std::path::PathBuf;

use mc_config::{parse_color_name, AnsiColor, FileHighlight, SkinFile};
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
    Tree,
}

impl ListingMode {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Brief,
            Self::Brief => Self::Long,
            Self::Long => Self::Tree,
            Self::Tree => Self::Full,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub name: String,
    pub depth: u16,
    pub expanded: bool,
    /// Full VPath of this node.
    pub path: VPath,
    /// `true` if this entry is itself a directory (only dirs are listed).
    pub has_children: bool,
}

/// Tree-mode state: a flattened, indented view of nested directories.
/// Built by `App::rebuild_tree` on demand and after each navigation.
#[derive(Debug, Clone, Default)]
pub struct TreeState {
    pub nodes: Vec<TreeNode>,
    pub cursor: usize,
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
    pub tree: TreeState,
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
            tree: TreeState::default(),
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
    skin: &SkinFile,
) {
    let bg = ansi_to_ratatui(parse_color_name(&skin.panel.background));
    let border = ansi_to_ratatui(parse_color_name(&skin.panel.border));
    let active_border = ansi_to_ratatui(parse_color_name(&skin.panel.active_border));
    let cursor_bg = ansi_to_ratatui(parse_color_name(&skin.panel.cursor_bg));
    let cursor_fg = ansi_to_ratatui(parse_color_name(&skin.panel.cursor_fg));
    let marked_fg = ansi_to_ratatui(parse_color_name(&skin.panel.marked_fg));
    let title_style = if state.active {
        Style::default().fg(cursor_fg).bg(cursor_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(bg)
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
            Style::default().fg(active_border)
        } else {
            Style::default().fg(border)
        })
        .style(Style::default().bg(bg));

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
        let cursor = if matches!(state.mode, ListingMode::Tree) {
            state.tree.cursor
        } else {
            state.cursor
        };
        if cursor < state.view_offset {
            state.view_offset = cursor;
        } else if cursor >= state.view_offset + height {
            state.view_offset = cursor + 1 - height;
        }
    }

    let lines = if matches!(state.mode, ListingMode::Tree) {
        render_tree(state, height, bg, cursor_bg, cursor_fg)
    } else {
        render_entries(
            state,
            body_area.width as usize,
            height,
            highlight,
            skin,
            bg,
            cursor_bg,
            cursor_fg,
            marked_fg,
        )
    };
    let para = Paragraph::new(lines).style(Style::default().bg(bg));
    f.render_widget(para, body_area);

    // Status line: cursor position + size info.
    let status = if matches!(state.mode, ListingMode::Tree) {
        if let Some(n) = state.tree.nodes.get(state.tree.cursor) {
            format!("tree: {}", n.path)
        } else {
            String::from("(empty tree)")
        }
    } else if let Some(e) = state.entries.get(state.cursor) {
        format!("{:>10}  {}", human_size(e.size), e.name)
    } else {
        String::from("(empty)")
    };
    let status_line =
        Paragraph::new(status).style(Style::default().fg(Color::White).bg(bg));
    f.render_widget(status_line, status_area);
}

#[allow(clippy::too_many_arguments)]
fn render_entries(
    state: &PanelState,
    width: usize,
    height: usize,
    highlight: &FileHighlight,
    skin: &SkinFile,
    bg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
    marked_fg: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height);
    let end = (state.view_offset + height).min(state.entries.len());
    for i in state.view_offset..end {
        let e = &state.entries[i];
        let is_cursor = i == state.cursor && state.active;
        let is_marked = state.marks.contains(&e.name);
        lines.push(format_line(
            e,
            state.mode,
            width,
            is_cursor,
            is_marked,
            highlight,
            skin,
            bg,
            cursor_bg,
            cursor_fg,
            marked_fg,
        ));
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn format_line(
    e: &Entry,
    mode: ListingMode,
    width: usize,
    is_cursor: bool,
    is_marked: bool,
    highlight: &FileHighlight,
    skin: &SkinFile,
    bg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
    marked_fg: Color,
) -> Line<'static> {
    let mut style = entry_style(e, highlight, skin, bg);
    if is_marked {
        style = style.fg(marked_fg).add_modifier(Modifier::BOLD);
    }
    if is_cursor {
        style = style.bg(cursor_bg).fg(cursor_fg);
    }
    let text = match mode {
        ListingMode::Brief | ListingMode::Tree => e.name.clone(),
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

fn render_tree(
    state: &PanelState,
    height: usize,
    bg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height);
    let end = (state.view_offset + height).min(state.tree.nodes.len());
    for i in state.view_offset..end {
        let n = &state.tree.nodes[i];
        let mut style = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        if i == state.tree.cursor && state.active {
            style = style.bg(cursor_bg).fg(cursor_fg);
        }
        let marker = if n.has_children {
            if n.expanded { "▼" } else { "▶" }
        } else {
            " "
        };
        let indent: String = " ".repeat(usize::from(n.depth) * 2);
        let label = format!("{indent}{marker} {}", n.name);
        lines.push(Line::from(Span::styled(label, style)));
    }
    lines
}

fn entry_style(e: &Entry, highlight: &FileHighlight, skin: &SkinFile, bg: Color) -> Style {
    let base = Style::default().fg(Color::White).bg(bg);
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
            if let Some(group) = highlight.classify(&e.name) {
                if let Some(name) = skin.groups.get(group) {
                    return base.fg(ansi_to_ratatui(parse_color_name(name)));
                }
            }
            base
        }
        EntryKind::Other => base,
    }
}

fn ansi_to_ratatui(c: AnsiColor) -> Color {
    match c {
        AnsiColor::Black => Color::Black,
        AnsiColor::Red => Color::Red,
        AnsiColor::Green => Color::Green,
        AnsiColor::Yellow => Color::Yellow,
        AnsiColor::Blue => Color::Blue,
        AnsiColor::Magenta => Color::Magenta,
        AnsiColor::Cyan => Color::Cyan,
        AnsiColor::White => Color::White,
        AnsiColor::DarkGray => Color::DarkGray,
        AnsiColor::LightRed => Color::LightRed,
        AnsiColor::LightGreen => Color::LightGreen,
        AnsiColor::LightYellow => Color::LightYellow,
        AnsiColor::LightBlue => Color::LightBlue,
        AnsiColor::LightMagenta => Color::LightMagenta,
        AnsiColor::LightCyan => Color::LightCyan,
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
