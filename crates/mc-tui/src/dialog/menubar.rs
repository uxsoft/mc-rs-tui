//! F9 pull-down menu.
//!
//! Sections (mc parity, simplified): File, Command, Options. Selecting an item
//! emits a [`MenuChoice`] that the App maps to an action.

use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::{Dialog, DialogOutcome};

#[derive(Debug, Clone, Copy)]
pub enum MenuChoice {
    View,
    Edit,
    Copy,
    Move,
    Mkdir,
    Delete,
    Chmod,
    Quit,
    Find,
    Hotlist,
    AddToHotlist,
    ToggleHidden,
    SortCycle,
    ToggleListingMode,
}

#[derive(Debug, Clone)]
struct MenuItem {
    label: &'static str,
    hint: &'static str,
    choice: MenuChoice,
}

#[derive(Debug, Clone)]
struct Section {
    title: &'static str,
    items: Vec<MenuItem>,
}

pub struct MenuBar {
    sections: Vec<Section>,
    active_section: usize,
    active_item: usize,
}

impl MenuBar {
    #[must_use]
    pub fn new() -> Self {
        let sections = vec![
            Section {
                title: "File",
                items: vec![
                    MenuItem { label: "View",   hint: "F3",       choice: MenuChoice::View },
                    MenuItem { label: "Edit",   hint: "F4",       choice: MenuChoice::Edit },
                    MenuItem { label: "Copy",   hint: "F5",       choice: MenuChoice::Copy },
                    MenuItem { label: "Move",   hint: "F6",       choice: MenuChoice::Move },
                    MenuItem { label: "Mkdir",  hint: "F7",       choice: MenuChoice::Mkdir },
                    MenuItem { label: "Delete", hint: "F8",       choice: MenuChoice::Delete },
                    MenuItem { label: "Chmod",  hint: "C-x C",    choice: MenuChoice::Chmod },
                    MenuItem { label: "Quit",   hint: "F10",      choice: MenuChoice::Quit },
                ],
            },
            Section {
                title: "Command",
                items: vec![
                    MenuItem { label: "Find file",        hint: "M-?", choice: MenuChoice::Find },
                    MenuItem { label: "Hotlist",          hint: "C-\\",choice: MenuChoice::Hotlist },
                    MenuItem { label: "Add to hotlist",   hint: "C-x H", choice: MenuChoice::AddToHotlist },
                ],
            },
            Section {
                title: "Options",
                items: vec![
                    MenuItem { label: "Toggle hidden",    hint: "C-.", choice: MenuChoice::ToggleHidden },
                    MenuItem { label: "Cycle sort",       hint: "M-S", choice: MenuChoice::SortCycle },
                    MenuItem { label: "Cycle listing",    hint: "M-T", choice: MenuChoice::ToggleListingMode },
                ],
            },
        ];
        Self {
            sections,
            active_section: 0,
            active_item: 0,
        }
    }
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialog for MenuBar {
    type Output = MenuChoice;

    fn render(&self, f: &mut Frame<'_>, area: Rect) {
        f.render_widget(Clear, Rect::new(area.x, area.y, area.width, 1));

        // Top bar: section titles.
        let mut spans: Vec<Span> = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            let style = if i == self.active_section {
                Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            };
            spans.push(Span::styled(format!(" {} ", s.title), style));
            spans.push(Span::raw(" "));
        }
        let bar = Paragraph::new(Line::from(spans))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        f.render_widget(bar, Rect::new(area.x, area.y, area.width, 1));

        // Drop-down for active section.
        let section = &self.sections[self.active_section];
        let max_label = section.items.iter().map(|i| i.label.len()).max().unwrap_or(0);
        let max_hint = section.items.iter().map(|i| i.hint.len()).max().unwrap_or(0);
        let inner_w = (max_label + max_hint + 5) as u16;
        let w = inner_w + 2;
        let h = section.items.len() as u16 + 2;
        // Place the dropdown under the section's title approximately.
        let mut x = area.x;
        for s in self.sections.iter().take(self.active_section) {
            x += s.title.len() as u16 + 3;
        }
        let dropdown = Rect::new(x, area.y + 1, w.min(area.width.saturating_sub(x)), h);
        f.render_widget(Clear, dropdown);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(Color::Cyan))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan));
        let inner = block.inner(dropdown);
        f.render_widget(block, dropdown);

        let lines: Vec<Line> = section
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let style = if i == self.active_item {
                    Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                };
                let label = format!(" {:<lw$}  {:>hw$} ", it.label, it.hint, lw = max_label, hw = max_hint);
                Line::from(Span::styled(label, style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<MenuChoice> {
        match chord.code {
            KeyCode::Escape | KeyCode::F(9) => DialogOutcome::Cancelled,
            KeyCode::Left => {
                if self.active_section > 0 {
                    self.active_section -= 1;
                    self.active_item = 0;
                }
                DialogOutcome::None
            }
            KeyCode::Right => {
                if self.active_section + 1 < self.sections.len() {
                    self.active_section += 1;
                    self.active_item = 0;
                }
                DialogOutcome::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.active_item = self.active_item.saturating_sub(1);
                DialogOutcome::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.sections[self.active_section].items.len();
                if self.active_item + 1 < max {
                    self.active_item += 1;
                }
                DialogOutcome::None
            }
            KeyCode::Enter => {
                let choice = self.sections[self.active_section].items[self.active_item].choice;
                DialogOutcome::Submitted(choice)
            }
            _ => DialogOutcome::None,
        }
    }
}
