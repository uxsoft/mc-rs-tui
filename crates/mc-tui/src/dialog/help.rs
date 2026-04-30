//! F1 help — scrollable keybindings reference.

use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{centered_rect, Dialog, DialogOutcome};

pub struct HelpDialog {
    lines: Vec<Line<'static>>,
    offset: usize,
}

impl HelpDialog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: build_help_lines(),
            offset: 0,
        }
    }
}

impl Default for HelpDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialog for HelpDialog {
    type Output = ();

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        let rect = centered_rect(76, 24, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .title(" Help — keybindings ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Cyan))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(1),
                ratatui::layout::Constraint::Length(1),
            ])
            .split(inner);
        let body = layout[0];
        let hint = layout[1];

        let height = body.height as usize;
        let end = (self.offset + height).min(self.lines.len());
        let visible: Vec<Line> = self.lines[self.offset..end].to_vec();
        f.render_widget(Paragraph::new(visible), body);
        f.render_widget(
            Paragraph::new(Line::from("PgUp/PgDn or j/k: scroll    Esc/F10/q: close")),
            hint,
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<()> {
        let max = self.lines.len();
        match chord.code {
            KeyCode::Escape | KeyCode::F(10) | KeyCode::Char('q') => DialogOutcome::Cancelled,
            KeyCode::Down | KeyCode::Char('j') => {
                self.offset = (self.offset + 1).min(max.saturating_sub(1));
                DialogOutcome::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.offset = self.offset.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.offset = (self.offset + 10).min(max.saturating_sub(1));
                DialogOutcome::None
            }
            KeyCode::PageUp | KeyCode::Backspace => {
                self.offset = self.offset.saturating_sub(10);
                DialogOutcome::None
            }
            KeyCode::Home => {
                self.offset = 0;
                DialogOutcome::None
            }
            KeyCode::End => {
                self.offset = max.saturating_sub(1);
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}

fn build_help_lines() -> Vec<Line<'static>> {
    let bold = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let mut out: Vec<Line<'static>> = Vec::new();
    let section = |out: &mut Vec<Line<'static>>, title: &str| {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled(title.to_string(), bold)));
    };
    let row = |out: &mut Vec<Line<'static>>, key: &str, desc: &str| {
        out.push(Line::from(vec![
            Span::styled(format!("  {:<14} ", key), bold),
            Span::raw(desc.to_string()),
        ]));
    };

    out.push(Line::from(Span::styled(
        "mc-rs-tui — Rust + Ratatui port of GNU Midnight Commander".to_string(),
        bold,
    )));
    out.push(Line::from(""));
    out.push(Line::from(
        "Customize bindings: ~/.config/mc-rs/keymap.toml (e.g. [[remap]] from=\"C-d\" to=\"F8\")"
            .to_string(),
    ));
    out.push(Line::from(
        "Customize colors:   ~/.config/mc-rs/skin.toml ([panel] background, [groups] archive=...)"
            .to_string(),
    ));

    section(&mut out, "Function keys");
    row(&mut out, "F1", "this help");
    row(&mut out, "F2", "user menu");
    row(&mut out, "F3", "view file (text + hex)");
    row(&mut out, "F4", "edit file (external $EDITOR / hx)");
    row(&mut out, "F5", "copy selected to other panel");
    row(&mut out, "F6", "rename (1 entry) or move to other panel");
    row(&mut out, "F7", "make directory");
    row(&mut out, "F8", "delete (recursive, confirmed)");
    row(&mut out, "F9", "menu bar");
    row(&mut out, "F10", "quit");

    section(&mut out, "Panel navigation");
    row(&mut out, "↑/↓/PgUp/PgDn", "move cursor");
    row(&mut out, "Home/End", "first / last entry");
    row(&mut out, "Enter", "descend dir or mount archive");
    row(&mut out, "Backspace", "parent dir / unmount archive");
    row(&mut out, "Tab", "swap active panel");
    row(&mut out, "Insert", "tag / untag entry");
    row(&mut out, "+", "select group (glob, e.g. *.txt)");
    row(&mut out, "\\", "unselect group");
    row(&mut out, "Alt-Y / Alt-U", "directory history back / forward");
    row(&mut out, "Alt-I", "mirror cwd into other panel");
    row(&mut out, "Alt-O", "load other panel with selected/parent dir");
    row(&mut out, "Ctrl-./Alt-.", "toggle hidden files");

    section(&mut out, "Sort & listing");
    row(&mut out, "Alt-T", "cycle Full / Brief / Long");
    row(&mut out, "Alt-S", "cycle sort key");
    row(&mut out, "Ctrl-R", "toggle reverse sort");
    row(&mut out, "Ctrl-S", "type-ahead quick search");

    section(&mut out, "Tools");
    row(&mut out, "Alt-?", "find file (filename + content)");
    row(&mut out, "Alt-C", "quick cd (typed path or sftp://… URL)");
    row(&mut out, "Ctrl-\\", "hotlist");
    row(&mut out, ":", "shell command line (Up/Dn: history)");
    row(&mut out, "Ctrl-X C", "chmod (octal)");
    row(&mut out, "Ctrl-X H", "add cwd to hotlist");
    row(&mut out, "Ctrl-X D", "diff cursor file vs other panel");
    row(&mut out, "Ctrl-X =", "compare directories (mark differing files)");
    row(&mut out, "Ctrl-X P", "copy active cwd to clipboard");
    row(&mut out, "Ctrl-X T", "copy cursor path to clipboard");
    row(&mut out, "Ctrl-K", "learn keys (terminal calibration)");
    row(&mut out, "Ctrl-J", "background jobs view");
    row(&mut out, "Ctrl-O", "drop to shell ($SHELL); exit/Ctrl-D returns");

    section(&mut out, "Inside viewer (F3)");
    row(&mut out, "F4", "toggle text / hex mode");
    row(&mut out, "/", "search forward");
    row(&mut out, "n / N", "next / prev match");
    row(&mut out, "Alt-E", "cycle text encoding");
    row(&mut out, "j/k or arrows", "scroll");
    row(&mut out, "Esc / F10 / q", "close");

    section(&mut out, "Inside diff (Ctrl-X D)");
    row(&mut out, "Enter / n", "next hunk");
    row(&mut out, "p", "previous hunk");
    row(&mut out, "Esc / F10 / q", "close");

    out
}
