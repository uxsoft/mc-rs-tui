use serde::{Deserialize, Serialize};

/// Logical UI elements that can be styled by a skin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Element {
    Base,
    PanelNormal,
    PanelSelected,
    PanelMarked,
    PanelMarkedSelected,
    PanelTitle,
    PanelTitleActive,
    PanelStatusbar,
    Dir,
    Executable,
    Symlink,
    SymlinkBroken,
    Device,
    Special,
    DialogNormal,
    DialogFocus,
    DialogTitle,
    DialogHotkey,
    DialogHotkeyFocus,
    Menu,
    MenuSelected,
    MenuHotkey,
    Buttonbar,
    ButtonbarHotkey,
    Statusbar,
    Help,
    HelpLink,
    HelpLinkSelected,
    ViewerNormal,
    ViewerSelected,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Color {
    /// 24-bit `0xRRGGBB`. Use `None` for "default" (terminal-default fg/bg).
    pub rgb: Option<u32>,
}

impl Color {
    pub const DEFAULT: Self = Self { rgb: None };

    #[must_use]
    pub fn rgb(rgb: u32) -> Self {
        Self { rgb: Some(rgb) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Theme {
    /// Sparse element → style overrides. Resolution falls back to `Element::Base`.
    pub styles: Vec<(Element, Style)>,
}

impl Theme {
    #[must_use]
    pub fn style_of(&self, e: Element) -> Style {
        self.styles
            .iter()
            .find(|(elem, _)| *elem == e)
            .map(|(_, s)| *s)
            .or_else(|| {
                self.styles
                    .iter()
                    .find(|(elem, _)| *elem == Element::Base)
                    .map(|(_, s)| *s)
            })
            .unwrap_or_default()
    }
}
