//! Panel state and rendering.

pub mod sort;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::config::{ColorScheme, FileHighlight, icon_for_name};
use crate::core::action::SortKey;
use crate::core::{Entry, EntryKind, VPath};

use crate::tui::git::GitGlyph;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme::rtc;
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
    /// Optional name-glob filter applied at `apply_filter_sort` time.
    pub filter: Option<String>,
    /// `true` once "Show directory sizes" has populated subdir sizes for the
    /// current cwd. Cleared on `navigate`.
    pub sizes_computed: bool,
    /// Names of directory entries whose recursive size has been computed
    /// (via Space on a single dir, or the panel-wide "Show directory sizes").
    /// Directories not in this set render as `<DIR>` instead of a byte count.
    /// Cleared on navigate / history back / history forward.
    pub computed_dir_sizes: HashSet<String>,
    /// `true` when the panel was populated by Find-and-panelize / External
    /// panelize. Suppresses the next reload from VFS.
    pub is_virtual_panelized: bool,
    /// Top-level entry-name → git status glyph, populated on refresh when
    /// `[options] git_status = true` and the cwd is inside a local repo.
    /// `None` means "not in a repo / lookup disabled / failed".
    pub git_status: Option<HashMap<String, GitGlyph>>,
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
            filter: None,
            sizes_computed: false,
            computed_dir_sizes: HashSet::new(),
            is_virtual_panelized: false,
            git_status: None,
        }
    }

    pub fn apply_filter_sort(&mut self) {
        if !self.show_hidden {
            self.entries
                .retain(|e| !e.name.starts_with('.') || e.name == "..");
        }
        if let Some(g) = self.filter.as_deref() {
            if !g.is_empty() {
                self.entries
                    .retain(|e| e.name == ".." || crate::tui::glob::glob_match(g, &e.name));
            }
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
        self.sizes_computed = false;
        self.computed_dir_sizes.clear();
        self.is_virtual_panelized = false;
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
        self.sizes_computed = false;
        self.computed_dir_sizes.clear();
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
        self.sizes_computed = false;
        self.computed_dir_sizes.clear();
        true
    }

    pub fn toggle_mark(&mut self) {
        let Some(e) = self.entries.get(self.cursor) else {
            return;
        };
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

/// The on-screen rect where panel rows are drawn — i.e. the area inside
/// the surrounding `Block` border with the bottom status row sliced off.
/// Mirrors the geometry [`render_panel`] uses (it must, otherwise mouse
/// hit-tests will pick the wrong row).
#[must_use]
pub fn panel_body_rect(area: Rect) -> Rect {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner)[0]
}

/// Decorations toggled by user options that affect per-row rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct PanelDecor {
    pub icons: bool,
    pub git_status: bool,
}

pub fn render_panel(
    f: &mut Frame<'_>,
    area: Rect,
    state: &mut PanelState,
    highlight: &FileHighlight,
    scheme: &ColorScheme,
    decor: PanelDecor,
) {
    let bg = rtc(scheme.panel_bg);
    let fg = rtc(scheme.panel_fg);
    let border = rtc(scheme.panel_border);
    let active_border = rtc(scheme.panel_border_active);
    let cursor_bg = rtc(scheme.cursor_bg);
    let cursor_fg = rtc(scheme.cursor_fg);
    let marked_fg = rtc(scheme.marked_fg);
    let title_style = if state.active {
        Style::default()
            .fg(rtc(scheme.panel_title_active_fg))
            .bg(rtc(scheme.panel_title_active_bg))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(rtc(scheme.panel_title_fg)).bg(bg)
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

    let git_status = state.git_status.as_ref();
    let lines = if matches!(state.mode, ListingMode::Tree) {
        render_tree(state, height, bg, fg, cursor_bg, cursor_fg)
    } else {
        render_entries(
            state,
            body_area.width as usize,
            height,
            highlight,
            scheme,
            bg,
            cursor_bg,
            cursor_fg,
            marked_fg,
            decor,
            git_status,
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
        format!(
            "{:>10}  {}",
            size_cell(e, &state.computed_dir_sizes),
            e.name
        )
    } else {
        String::from("(empty)")
    };
    let status_line =
        Paragraph::new(status).style(Style::default().fg(rtc(scheme.panel_dim_fg)).bg(bg));
    f.render_widget(status_line, status_area);
}

#[allow(clippy::too_many_arguments)]
fn render_entries(
    state: &PanelState,
    width: usize,
    height: usize,
    highlight: &FileHighlight,
    scheme: &ColorScheme,
    bg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
    marked_fg: Color,
    decor: PanelDecor,
    git_status: Option<&HashMap<String, GitGlyph>>,
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
            scheme,
            bg,
            cursor_bg,
            cursor_fg,
            marked_fg,
            &state.computed_dir_sizes,
            decor,
            git_status,
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
    scheme: &ColorScheme,
    bg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
    marked_fg: Color,
    computed_sizes: &HashSet<String>,
    decor: PanelDecor,
    git_status: Option<&HashMap<String, GitGlyph>>,
) -> Line<'static> {
    let mut style = entry_style(e, highlight, scheme, bg);
    if is_marked {
        style = style.fg(marked_fg).add_modifier(Modifier::BOLD);
    }
    if is_cursor {
        style = style.bg(cursor_bg).fg(cursor_fg);
    }
    // Git status glyph (2 cols: glyph + space). Empty when no map or no entry.
    let git_prefix: String = if decor.git_status && e.name != ".." {
        match git_status.and_then(|m| m.get(&e.name)) {
            Some(g) => format!("{} ", g.as_str()),
            None => "  ".into(),
        }
    } else {
        String::new()
    };
    let git_cols = if git_prefix.is_empty() { 0 } else { 2 };
    // Icon prefix (2 cols: glyph + space) only when configured. Skipped for
    // `..` so the parent-up entry stays visually distinct.
    let icon_prefix: String = if decor.icons && e.name != ".." {
        format!("{} ", icon_for_name(&e.name, e.kind))
    } else {
        String::new()
    };
    let icon_cols = if icon_prefix.is_empty() { 0 } else { 2 };
    let prefix_cols = git_cols + icon_cols;
    let text = match mode {
        ListingMode::Brief | ListingMode::Tree => {
            format!("{git_prefix}{icon_prefix}{}", e.name)
        }
        ListingMode::Full => {
            let name_w = width.saturating_sub(13).saturating_sub(prefix_cols);
            format!(
                "{git_prefix}{icon_prefix}{:<name$} {:>10}",
                e.name,
                size_cell(e, computed_sizes),
                name = name_w
            )
        }
        ListingMode::Long => {
            let mode_str = unix_mode_str(e);
            let name_w = width.saturating_sub(28).saturating_sub(prefix_cols);
            format!(
                "{} {:>4} {:>10} {git_prefix}{icon_prefix}{:<name$}",
                mode_str,
                e.nlink.unwrap_or(0),
                size_cell(e, computed_sizes),
                e.name,
                name = name_w,
            )
        }
    };
    Line::from(Span::styled(text, style))
}

fn render_tree(
    state: &PanelState,
    height: usize,
    bg: Color,
    fg: Color,
    cursor_bg: Color,
    cursor_fg: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height);
    let end = (state.view_offset + height).min(state.tree.nodes.len());
    for i in state.view_offset..end {
        let n = &state.tree.nodes[i];
        let mut style = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
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

fn entry_style(e: &Entry, highlight: &FileHighlight, scheme: &ColorScheme, bg: Color) -> Style {
    let base = Style::default().fg(rtc(scheme.panel_fg)).bg(bg);
    match e.kind {
        EntryKind::Dir => base.fg(rtc(scheme.file_dir)).add_modifier(Modifier::BOLD),
        EntryKind::Symlink => base.fg(rtc(scheme.file_symlink)),
        EntryKind::Fifo | EntryKind::Socket => base.fg(rtc(scheme.file_special)),
        EntryKind::BlockDevice | EntryKind::CharDevice => base.fg(rtc(scheme.file_device)),
        EntryKind::File => {
            if let Some(mode) = e.mode {
                if mode & 0o111 != 0 {
                    return base.fg(rtc(scheme.file_executable));
                }
            }
            if let Some(group) = highlight.classify(&e.name) {
                let c = match group {
                    "archive" => scheme.file_archive,
                    "image" => scheme.file_image,
                    "audio" | "video" => scheme.file_media,
                    "doc" => scheme.file_doc,
                    "source" => scheme.file_source,
                    "build" => scheme.file_build,
                    _ => scheme.panel_fg,
                };
                return base.fg(rtc(c));
            }
            base
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

fn size_cell(e: &Entry, computed: &HashSet<String>) -> String {
    if e.name == ".." {
        return "UP".into();
    }
    if matches!(e.kind, EntryKind::Dir) && !computed.contains(&e.name) {
        return "DIR".into();
    }
    human_size(e.size)
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
