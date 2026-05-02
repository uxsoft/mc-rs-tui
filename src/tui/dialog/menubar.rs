//! Top menu bar (mc parity): Left, File, Command, Options, Right.
//!
//! The title row is rendered persistently at the top of the screen. F9
//! activates dropdown navigation; selecting an item emits a [`MenuChoice`]
//! that the App routes to existing handlers or new dialog modals.
//!
//! The menubar is no longer a `Dialog`; it owns its state on `App` directly.
//! See [`MenuBar::handle_key`] and [`MenuBar::render_titles`] /
//! [`MenuBar::render_dropdown`].

use crate::config::ColorScheme;
use crate::core::key::{KeyChord, KeyCode, KeyMods};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::DialogOutcome;
use crate::tui::theme::rtc;

/// Pop-up dialogs the menu can ask `App` to open. The variants here have
/// **no built-in keybinding** today; `App` constructs the matching `Modal::*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuDialog {
    Chown,
    Chattr,
    Hardlink,
    Symlink { relative: bool },
    EditSymlink,
    Filter,
    FtpLink,
    SftpLink,
    ShellLink,
    ActiveVfsList,
    ExternalPanelize,
    ShowDirSizes,
    EditMenuFile,
    EditExtensionFile,
    EditHighlightingFile,
    Configuration,
    Layout,
    Confirmation,
    VirtualFs,
    SaveSetup,
    FindAndPanelize,
    Encoding,
    DisplayBits,
    Theme,
}

/// Action emitted when the user selects a menu item.
#[derive(Debug, Clone)]
pub enum MenuChoice {
    /// Dispatch a synthetic key chord through `App::handle_panel_key`.
    /// Used for items whose behavior already has a keybinding.
    KeyChord(KeyChord),
    /// Run a Ctrl-X two-key chord: `c` = chmod, `=` = compare dirs,
    /// `d` = compare files, `h` = add to hotlist, etc. Routed through
    /// `App::handle_ctrl_x`.
    CtrlX(char),
    /// Quit the application.
    Quit,
    /// Refresh / reread the active panel.
    Reread,
    /// Swap the contents of the left and right panels.
    SwapPanels,
    /// Focus a panel before performing the follow-up action.
    /// Used by the Left/Right menus so each item operates on the
    /// corresponding panel regardless of current focus.
    FocusThen { left: bool, then: Box<MenuChoice> },
    /// Open a dialog for items that don't map to an existing key chord.
    OpenDialog(MenuDialog),
    /// Show a transient status message (used for stub items).
    Status(&'static str),
    /// Open the User menu directly (so the "User menu" item doesn't depend
    /// on F2's keybinding, which has been reassigned to opening this
    /// menubar).
    OpenUserMenu,
}

#[derive(Debug, Clone)]
struct MenuItem {
    label: &'static str,
    hint: &'static str,
    choice: MenuChoice,
}

#[derive(Debug, Clone)]
enum MenuEntry {
    Item(MenuItem),
    Separator,
}

#[derive(Debug, Clone)]
struct Section {
    title: &'static str,
    /// First letter — uppercase mnemonic shown highlighted in the title row
    /// and used as a keyboard shortcut to jump to this section while the
    /// menubar is active.
    mnemonic: char,
    entries: Vec<MenuEntry>,
}

pub struct MenuBar {
    sections: Vec<Section>,
    pub active_section: usize,
    pub active_item: usize,
}

struct DropdownLayout {
    rect: Rect,
    max_label: usize,
    max_hint: usize,
}

impl MenuBar {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: build_sections(),
            active_section: 0,
            active_item: 0,
        }
    }

    /// Reset to the first item of the first section.
    pub fn reset(&mut self) {
        self.active_section = 0;
        self.active_item = self.first_item_index(0);
    }

    fn first_item_index(&self, section: usize) -> usize {
        self.sections[section]
            .entries
            .iter()
            .position(|e| matches!(e, MenuEntry::Item(_)))
            .unwrap_or(0)
    }

    fn last_item_index(&self, section: usize) -> usize {
        self.sections[section]
            .entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, e)| matches!(e, MenuEntry::Item(_)).then_some(i))
            .unwrap_or(0)
    }

    /// Move the active item up, skipping separators (wraps).
    fn move_up(&mut self) {
        let entries = &self.sections[self.active_section].entries;
        let n = entries.len();
        if n == 0 {
            return;
        }
        let mut i = self.active_item;
        for _ in 0..n {
            i = if i == 0 { n - 1 } else { i - 1 };
            if matches!(entries[i], MenuEntry::Item(_)) {
                self.active_item = i;
                return;
            }
        }
    }

    /// Move the active item down, skipping separators (wraps).
    fn move_down(&mut self) {
        let entries = &self.sections[self.active_section].entries;
        let n = entries.len();
        if n == 0 {
            return;
        }
        let mut i = self.active_item;
        for _ in 0..n {
            i = (i + 1) % n;
            if matches!(entries[i], MenuEntry::Item(_)) {
                self.active_item = i;
                return;
            }
        }
    }

    fn jump_to_section(&mut self, idx: usize) {
        if idx < self.sections.len() {
            self.active_section = idx;
            self.active_item = self.first_item_index(idx);
        }
    }

    /// Hit-test the title row: returns the section index whose title spans
    /// `column` (0-based), or `None` if the click landed on whitespace
    /// between titles or past the last one. The layout matches
    /// [`MenuBar::render_titles`]: leading 1-space margin, then for each
    /// section `" {title} "` followed by 1 separator space.
    #[must_use]
    pub fn section_at_column(&self, column: u16) -> Option<usize> {
        let mut x: u16 = 1; // leading margin
        for (i, s) in self.sections.iter().enumerate() {
            let title_w = (s.title.len() as u16).saturating_add(2); // " title "
            if column >= x && column < x + title_w {
                return Some(i);
            }
            x = x.saturating_add(title_w + 1); // + separator
        }
        None
    }

    /// Hit-test the active section's dropdown body. Caller passes the
    /// dropdown's inner body rect (the area inside the borders) and the
    /// click's row offset within that area. Returns `Some(MenuChoice)` if
    /// the row is an item (and updates the cursor); `None` for separator
    /// or out-of-range.
    pub fn item_choice_at(&mut self, row_in_body: u16) -> Option<MenuChoice> {
        let entries = &self.sections[self.active_section].entries;
        let i = row_in_body as usize;
        match entries.get(i)? {
            MenuEntry::Item(it) => {
                self.active_item = i;
                Some(it.choice.clone())
            }
            MenuEntry::Separator => None,
        }
    }

    /// Open the menu and select the given section. Helper for mouse code.
    pub fn open_at(&mut self, section: usize) {
        if section < self.sections.len() {
            self.active_section = section;
            self.active_item = self.first_item_index(section);
        }
    }

    /// Geometry for the active section's dropdown: maximum label / hint
    /// widths (used for both layout and per-row formatting), plus the
    /// already-clipped on-screen rect.
    fn dropdown_layout(&self, area: Rect) -> DropdownLayout {
        let section = &self.sections[self.active_section];
        let max_label = section
            .entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(it) => Some(it.label.len()),
                MenuEntry::Separator => None,
            })
            .max()
            .unwrap_or(0);
        let max_hint = section
            .entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(it) => Some(it.hint.len()),
                MenuEntry::Separator => None,
            })
            .max()
            .unwrap_or(0);
        let inner_w = (max_label + max_hint + 5) as u16;
        let w = inner_w + 2;
        let h = section.entries.len() as u16 + 2;

        // Mirror the title-row layout: leading 1-space margin, then each
        // preceding section is `" {title} "` (len+2) plus a 1-space separator.
        let mut x = area.x + 1;
        for s in self.sections.iter().take(self.active_section) {
            x += s.title.len() as u16 + 3;
        }
        let rect = Rect::new(
            x,
            area.y,
            w.min(area.width.saturating_sub(x.saturating_sub(area.x))),
            h.min(area.height),
        );
        DropdownLayout {
            rect,
            max_label,
            max_hint,
        }
    }

    /// Return the on-screen rect for the active section's dropdown.
    #[must_use]
    pub fn dropdown_rect(&self, area: Rect) -> Rect {
        self.dropdown_layout(area).rect
    }

    /// Render the always-visible 1-row title strip into `area`. When
    /// `focused` is true the active section is highlighted (used while the
    /// menu has keyboard focus); otherwise titles are drawn in the chrome
    /// style.
    pub fn render_titles(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        scheme: &ColorScheme,
        focused: bool,
    ) {
        let chrome = Style::default()
            .fg(rtc(scheme.buttonbar_label_fg))
            .bg(rtc(scheme.buttonbar_label_bg));
        let chrome_active = Style::default()
            .fg(rtc(scheme.dialog_focus_fg))
            .bg(rtc(scheme.dialog_focus_bg))
            .add_modifier(Modifier::BOLD);

        f.render_widget(Clear, Rect::new(area.x, area.y, area.width, 1));

        let mut spans: Vec<Span> = vec![Span::styled(" ", chrome)];
        for (i, s) in self.sections.iter().enumerate() {
            let style = if focused && i == self.active_section {
                chrome_active
            } else {
                chrome
            };
            spans.push(Span::styled(format!(" {} ", s.title), style));
            spans.push(Span::styled(" ", chrome));
        }
        let bar = Paragraph::new(Line::from(spans)).style(chrome);
        f.render_widget(bar, Rect::new(area.x, area.y, area.width, 1));
    }

    /// Render the dropdown for the active section. Caller positions `area`
    /// such that its top-left is just below the title row.
    pub fn render_dropdown(&self, f: &mut Frame<'_>, area: Rect, scheme: &ColorScheme) {
        let dlg = Style::default()
            .fg(rtc(scheme.dialog_fg))
            .bg(rtc(scheme.dialog_bg));
        let dlg_focus = Style::default()
            .fg(rtc(scheme.dialog_focus_fg))
            .bg(rtc(scheme.dialog_focus_bg))
            .add_modifier(Modifier::BOLD);
        let border = Style::default()
            .fg(rtc(scheme.dialog_border))
            .bg(rtc(scheme.dialog_bg));

        let section = &self.sections[self.active_section];
        let layout = self.dropdown_layout(area);
        let DropdownLayout {
            rect: dropdown,
            max_label,
            max_hint,
        } = layout;

        f.render_widget(Clear, dropdown);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .style(dlg);
        let inner = block.inner(dropdown);
        f.render_widget(block, dropdown);

        let lines: Vec<Line> = section
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| match e {
                MenuEntry::Separator => {
                    let bar = "─".repeat(max_label + max_hint + 4);
                    Line::from(Span::styled(bar, border))
                }
                MenuEntry::Item(it) => {
                    let style = if i == self.active_item {
                        dlg_focus
                    } else {
                        dlg
                    };
                    let label = format!(
                        " {:<lw$}  {:>hw$} ",
                        it.label,
                        it.hint,
                        lw = max_label,
                        hw = max_hint
                    );
                    Line::from(Span::styled(label, style))
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(dlg), inner);
    }

    /// Handle a key while the menu has keyboard focus.
    pub fn handle_key(&mut self, chord: KeyChord) -> DialogOutcome<MenuChoice> {
        match (chord.code, chord.mods) {
            (KeyCode::Escape, _) | (KeyCode::F(9), _) => DialogOutcome::Cancelled,
            (KeyCode::Left, _) => {
                if self.active_section == 0 {
                    self.active_section = self.sections.len() - 1;
                } else {
                    self.active_section -= 1;
                }
                self.active_item = self.first_item_index(self.active_section);
                DialogOutcome::None
            }
            (KeyCode::Right, _) | (KeyCode::Tab, _) => {
                self.active_section = (self.active_section + 1) % self.sections.len();
                self.active_item = self.first_item_index(self.active_section);
                DialogOutcome::None
            }
            (KeyCode::Up, _) => {
                self.move_up();
                DialogOutcome::None
            }
            (KeyCode::Down, _) => {
                self.move_down();
                DialogOutcome::None
            }
            (KeyCode::Home, _) => {
                self.active_item = self.first_item_index(self.active_section);
                DialogOutcome::None
            }
            (KeyCode::End, _) => {
                self.active_item = self.last_item_index(self.active_section);
                DialogOutcome::None
            }
            (KeyCode::Enter, _) => {
                match &self.sections[self.active_section].entries[self.active_item] {
                    MenuEntry::Item(it) => DialogOutcome::Submitted(it.choice.clone()),
                    MenuEntry::Separator => DialogOutcome::None,
                }
            }
            (KeyCode::Char(c), m) if m.is_empty() || m == KeyMods::SHIFT => {
                let want = c.to_ascii_lowercase();
                if let Some(idx) = self
                    .sections
                    .iter()
                    .position(|s| s.mnemonic.to_ascii_lowercase() == want)
                {
                    self.jump_to_section(idx);
                }
                DialogOutcome::None
            }
            _ => DialogOutcome::None,
        }
    }
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Section builders. Items are defined declaratively to keep the surface area
// readable. `KeyChord::plain` and `KeyChord::new` give us the equivalent of
// the user pressing the corresponding hotkey, which `handle_panel_key`
// already understands.
// ---------------------------------------------------------------------------

fn build_sections() -> Vec<Section> {
    vec![
        Section {
            title: "Left",
            mnemonic: 'L',
            entries: panel_section_entries(true),
        },
        Section {
            title: "File",
            mnemonic: 'F',
            entries: file_section_entries(),
        },
        Section {
            title: "Command",
            mnemonic: 'C',
            entries: command_section_entries(),
        },
        Section {
            title: "Options",
            mnemonic: 'O',
            entries: options_section_entries(),
        },
        Section {
            title: "Appearance",
            mnemonic: 'A',
            entries: appearance_section_entries(),
        },
        Section {
            title: "Right",
            mnemonic: 'R',
            entries: panel_section_entries(false),
        },
    ]
}

fn focus_then(left: bool, then: MenuChoice) -> MenuChoice {
    MenuChoice::FocusThen {
        left,
        then: Box::new(then),
    }
}

fn key(code: KeyCode) -> MenuChoice {
    MenuChoice::KeyChord(KeyChord::plain(code))
}

fn key_mod(code: KeyCode, mods: KeyMods) -> MenuChoice {
    MenuChoice::KeyChord(KeyChord::new(code, mods))
}

fn dlg(d: MenuDialog) -> MenuChoice {
    MenuChoice::OpenDialog(d)
}

fn item(label: &'static str, hint: &'static str, choice: MenuChoice) -> MenuEntry {
    MenuEntry::Item(MenuItem {
        label,
        hint,
        choice,
    })
}

fn sep() -> MenuEntry {
    MenuEntry::Separator
}

fn panel_section_entries(left: bool) -> Vec<MenuEntry> {
    vec![
        item(
            "Listing format...",
            "M-t",
            focus_then(left, key_mod(KeyCode::Char('t'), KeyMods::ALT)),
        ),
        item(
            "Sort order...",
            "M-s",
            focus_then(left, key_mod(KeyCode::Char('s'), KeyMods::ALT)),
        ),
        item(
            "Reverse sort",
            "C-r",
            focus_then(left, key_mod(KeyCode::Char('r'), KeyMods::CTRL)),
        ),
        sep(),
        item("Filter...", "", focus_then(left, dlg(MenuDialog::Filter))),
        item(
            "Encoding...",
            "",
            focus_then(left, dlg(MenuDialog::Encoding)),
        ),
        item(
            "Tree",
            "",
            focus_then(left, key_mod(KeyCode::Char('t'), KeyMods::ALT)),
        ),
        sep(),
        item("Reread", "", focus_then(left, MenuChoice::Reread)),
        item(
            "FTP link...",
            "",
            focus_then(left, dlg(MenuDialog::FtpLink)),
        ),
        item(
            "SFTP link...",
            "",
            focus_then(left, dlg(MenuDialog::SftpLink)),
        ),
        item(
            "Shell link...",
            "",
            focus_then(left, dlg(MenuDialog::ShellLink)),
        ),
        sep(),
        item(
            "Active VFS list...",
            "",
            focus_then(left, dlg(MenuDialog::ActiveVfsList)),
        ),
    ]
}

fn file_section_entries() -> Vec<MenuEntry> {
    vec![
        item("View", "F3", key(KeyCode::F(3))),
        item("Edit", "F4", key(KeyCode::F(4))),
        sep(),
        item("Copy", "F5", key(KeyCode::F(5))),
        item("Rename or Move", "F6", key(KeyCode::F(6))),
        item("Make directory", "F7", key(KeyCode::F(7))),
        item("Delete", "F8", key(KeyCode::F(8))),
        sep(),
        item("Chmod", "C-x c", MenuChoice::CtrlX('c')),
        item("Chown", "", dlg(MenuDialog::Chown)),
        item("Chattr", "", dlg(MenuDialog::Chattr)),
        sep(),
        item("Hardlink", "", dlg(MenuDialog::Hardlink)),
        item(
            "Symbolic link",
            "",
            dlg(MenuDialog::Symlink { relative: false }),
        ),
        item(
            "Relative symlink",
            "",
            dlg(MenuDialog::Symlink { relative: true }),
        ),
        item("Edit symlink", "", dlg(MenuDialog::EditSymlink)),
        sep(),
        item("Quick cd", "M-c", key_mod(KeyCode::Char('c'), KeyMods::ALT)),
        item(
            "Select group",
            "+",
            MenuChoice::KeyChord(KeyChord::plain(KeyCode::Char('+'))),
        ),
        item(
            "Unselect group",
            "\\",
            MenuChoice::KeyChord(KeyChord::plain(KeyCode::Char('\\'))),
        ),
        item(
            "Invert selection",
            "*",
            MenuChoice::KeyChord(KeyChord::plain(KeyCode::Char('*'))),
        ),
        sep(),
        item("Exit", "F10", MenuChoice::Quit),
    ]
}

fn command_section_entries() -> Vec<MenuEntry> {
    vec![
        item("User menu", "", MenuChoice::OpenUserMenu),
        sep(),
        item(
            "Find file",
            "M-?",
            key_mod(KeyCode::Char('?'), KeyMods::ALT),
        ),
        item("Find and panelize", "", dlg(MenuDialog::FindAndPanelize)),
        item(
            "External panelize...",
            "",
            dlg(MenuDialog::ExternalPanelize),
        ),
        item("Show directory sizes", "", dlg(MenuDialog::ShowDirSizes)),
        sep(),
        item(
            "Directory hotlist",
            "C-\\",
            key_mod(KeyCode::Char('\\'), KeyMods::CTRL),
        ),
        sep(),
        item("Compare directories", "C-x =", MenuChoice::CtrlX('=')),
        item("Compare files", "C-x d", MenuChoice::CtrlX('d')),
        item("Swap panels", "", MenuChoice::SwapPanels),
        sep(),
        item(
            "Background jobs",
            "C-j",
            key_mod(KeyCode::Char('j'), KeyMods::CTRL),
        ),
        sep(),
        item("Edit menu file", "", dlg(MenuDialog::EditMenuFile)),
        item(
            "Edit extension file",
            "",
            dlg(MenuDialog::EditExtensionFile),
        ),
        item(
            "Edit highlighting file",
            "",
            dlg(MenuDialog::EditHighlightingFile),
        ),
        sep(),
        item("Help", "F1", key(KeyCode::F(1))),
        item(
            "Learn keys",
            "C-k",
            key_mod(KeyCode::Char('k'), KeyMods::CTRL),
        ),
    ]
}

fn options_section_entries() -> Vec<MenuEntry> {
    vec![
        item("Configuration...", "", dlg(MenuDialog::Configuration)),
        item("Layout...", "", dlg(MenuDialog::Layout)),
        item("Confirmation...", "", dlg(MenuDialog::Confirmation)),
        item("Display bits...", "", dlg(MenuDialog::DisplayBits)),
        item("Virtual FS...", "", dlg(MenuDialog::VirtualFs)),
        sep(),
        item("Save setup", "", dlg(MenuDialog::SaveSetup)),
    ]
}

fn appearance_section_entries() -> Vec<MenuEntry> {
    vec![item("Theme...", "", dlg(MenuDialog::Theme))]
}
