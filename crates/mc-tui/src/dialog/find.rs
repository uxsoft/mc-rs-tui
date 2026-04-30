use mc_core::key::{KeyChord, KeyCode, KeyMods};
use mc_core::VPath;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{centered_rect, Dialog, DialogOutcome};

#[derive(Debug, Clone)]
pub struct FindParams {
    pub name_pattern: String,
    pub content_pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub ignore_dirs: String, // colon-separated
}

impl Default for FindParams {
    fn default() -> Self {
        Self {
            name_pattern: "*".into(),
            content_pattern: String::new(),
            case_sensitive: false,
            whole_word: false,
            ignore_dirs: ".git:node_modules:target".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Content,
    IgnoreDirs,
    Case,
    WholeWord,
}

const FIELDS: [Field; 5] = [
    Field::Name,
    Field::Content,
    Field::IgnoreDirs,
    Field::Case,
    Field::WholeWord,
];

pub enum FindFormOutcome {
    /// User submitted; start searching with the given params.
    Start(FindParams),
    /// User cancelled / closed the form.
    Cancel,
}

pub struct FindForm {
    pub params: FindParams,
    field: usize,
}

impl FindForm {
    #[must_use]
    pub fn new(params: FindParams) -> Self {
        Self { params, field: 0 }
    }

    fn current_field(&self) -> Field {
        FIELDS[self.field]
    }

    fn current_field_text_mut(&mut self) -> Option<&mut String> {
        match self.current_field() {
            Field::Name => Some(&mut self.params.name_pattern),
            Field::Content => Some(&mut self.params.content_pattern),
            Field::IgnoreDirs => Some(&mut self.params.ignore_dirs),
            _ => None,
        }
    }
}

impl Dialog for FindForm {
    type Output = FindFormOutcome;

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        let rect = centered_rect(70, 13, area);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .title(" Find file ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Cyan))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let lines: Vec<Line> = vec![
            field_line("Filename glob", &self.params.name_pattern, self.current_field() == Field::Name),
            field_line("Content match", &self.params.content_pattern, self.current_field() == Field::Content),
            field_line("Ignore dirs",   &self.params.ignore_dirs,    self.current_field() == Field::IgnoreDirs),
            check_line("Case sensitive", self.params.case_sensitive, self.current_field() == Field::Case),
            check_line("Whole word",     self.params.whole_word,     self.current_field() == Field::WholeWord),
            Line::from(""),
            Line::from("Tab: next field   Space: toggle   Enter: start   Esc: cancel"),
        ];
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<FindFormOutcome> {
        match (chord.code, chord.mods) {
            (KeyCode::Escape, _) => {
                DialogOutcome::Submitted(FindFormOutcome::Cancel)
            }
            (KeyCode::Enter, _) => {
                DialogOutcome::Submitted(FindFormOutcome::Start(self.params.clone()))
            }
            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                self.field = (self.field + 1) % FIELDS.len();
                DialogOutcome::None
            }
            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                self.field = (self.field + FIELDS.len() - 1) % FIELDS.len();
                DialogOutcome::None
            }
            (KeyCode::Char(' '), m) if m.is_empty() => {
                match self.current_field() {
                    Field::Case => self.params.case_sensitive = !self.params.case_sensitive,
                    Field::WholeWord => self.params.whole_word = !self.params.whole_word,
                    _ => {
                        if let Some(s) = self.current_field_text_mut() {
                            s.push(' ');
                        }
                    }
                }
                DialogOutcome::None
            }
            (KeyCode::Backspace, _) => {
                if let Some(s) = self.current_field_text_mut() {
                    s.pop();
                }
                DialogOutcome::None
            }
            (KeyCode::Char(c), m) if m.is_empty() || m == KeyMods::SHIFT => {
                if let Some(s) = self.current_field_text_mut() {
                    s.push(c);
                }
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}

fn field_line(label: &'static str, value: &str, active: bool) -> Line<'static> {
    let style = if active {
        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    };
    Line::from(vec![
        Span::raw(format!("{:<14} ", label)),
        Span::styled(format!(" {:<48} ", value), style),
    ])
}

fn check_line(label: &'static str, value: bool, active: bool) -> Line<'static> {
    let style = if active {
        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    };
    let mark = if value { "[x]" } else { "[ ]" };
    Line::from(vec![
        Span::raw(format!("{:<14} ", label)),
        Span::styled(format!(" {mark} "), style),
    ])
}

// ---- Results dialog --------------------------------------------------------

pub struct FindResults {
    pub query_summary: String,
    pub items: Vec<VPath>,
    pub status: String,
    pub done: bool,
    cursor: usize,
    view_offset: usize,
}

pub enum FindResultsOutcome {
    Navigate(VPath),
}

impl FindResults {
    #[must_use]
    pub fn new(summary: String) -> Self {
        Self {
            query_summary: summary,
            items: Vec::new(),
            status: "scanning…".into(),
            done: false,
            cursor: 0,
            view_offset: 0,
        }
    }

    pub fn push(&mut self, p: VPath) {
        self.items.push(p);
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    pub fn finish(&mut self) {
        self.done = true;
        self.status = format!("{} matches", self.items.len());
    }
}

impl Dialog for FindResults {
    type Output = FindResultsOutcome;

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        let rect = centered_rect(80, 22, area);
        f.render_widget(Clear, rect);
        let title = format!(" Find results — {} ", self.query_summary);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Cyan))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        let height = layout[0].height as usize;

        let lines: Vec<Line> = self
            .items
            .iter()
            .enumerate()
            .skip(self.view_offset)
            .take(height)
            .map(|(i, p)| {
                let style = if i == self.cursor {
                    Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                };
                Line::from(Span::styled(format!(" {} ", p), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), layout[0]);

        let status = if self.done {
            self.status.clone()
        } else {
            format!("{} (so far {} matches)", self.status, self.items.len())
        };
        f.render_widget(
            Paragraph::new(Line::from(status))
                .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
            layout[1],
        );
        f.render_widget(
            Paragraph::new(Line::from("Enter: cd to dir   Esc: close")),
            layout[2],
        );
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<FindResultsOutcome> {
        match chord.code {
            KeyCode::Escape => DialogOutcome::Cancelled,
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.scroll_into_view();
                DialogOutcome::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.items.len() {
                    self.cursor += 1;
                }
                self.scroll_into_view();
                DialogOutcome::None
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(20);
                self.scroll_into_view();
                DialogOutcome::None
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 20).min(self.items.len().saturating_sub(1));
                self.scroll_into_view();
                DialogOutcome::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.view_offset = 0;
                DialogOutcome::None
            }
            KeyCode::End => {
                self.cursor = self.items.len().saturating_sub(1);
                self.scroll_into_view();
                DialogOutcome::None
            }
            KeyCode::Enter => {
                if let Some(p) = self.items.get(self.cursor).cloned() {
                    DialogOutcome::Submitted(FindResultsOutcome::Navigate(p))
                } else {
                    DialogOutcome::None
                }
            }
            _ => DialogOutcome::None,
        }
    }
}

impl FindResults {
    fn scroll_into_view(&mut self) {
        // best effort viewport tracking; UI re-renders set the actual height.
        if self.cursor < self.view_offset {
            self.view_offset = self.cursor;
        } else if self.cursor >= self.view_offset + 16 {
            self.view_offset = self.cursor + 1 - 16;
        }
    }
}
