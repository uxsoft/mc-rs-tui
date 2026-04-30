//! User-invokable commands. Single source of truth for keymap targets.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SortKey {
    Unsorted,
    Name,
    Extension,
    Size,
    Mtime,
    Atime,
    Ctime,
    Inode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CompareMode {
    Quick,
    Thorough,
    SizeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ViewerAction {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
    ToggleWrap,
    ToggleHex,
    ToggleRaw,
    ToggleParsed,
    ToggleRuler,
    Search,
    SearchNext,
    SearchPrev,
    Goto,
    SetMark(u8),
    GotoMark(u8),
    Charset,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DiffAction {
    NextHunk,
    PrevHunk,
    MergeHunk,
    EditLeft,
    EditRight,
    Swap,
    Refresh,
    ToggleIgnoreSpace,
    ToggleIgnoreCase,
    SetTabSize(u8),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DialogAction {
    Ok,
    Cancel,
    NextField,
    PrevField,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum Action {
    // Panel navigation
    PanelUp,
    PanelDown,
    PanelPgUp,
    PanelPgDn,
    PanelHome,
    PanelEnd,
    PanelToggleTag,
    PanelSelectGroup,
    PanelUnselectGroup,
    PanelInvertSelection,
    PanelCdParent,
    PanelCdHome,
    PanelSortBy(SortKey),
    PanelToggleReverse,
    PanelToggleHidden,
    PanelLayoutCycle,
    PanelSyncSibling,
    PanelSwap,
    PanelHistoryBack,
    PanelHistoryFwd,
    PanelHistoryList,

    // File ops
    Copy,
    Move,
    Delete,
    Mkdir,
    Chmod,
    Chown,
    Chattr,
    Hardlink,
    AbsSymlink,
    RelSymlink,
    EditSymlink,
    CompareDirs(CompareMode),

    // Tools
    ViewFile,
    ViewRaw,
    EditFile,
    Find,
    FindAndPanelize,
    Hotlist,
    AddToHotlist,
    UserMenu,
    ExtMenu,
    QuickCd,
    QuickSearch,

    // App
    SwitchPanel,
    RefreshPanel,
    ToggleQuickView,
    ToggleTreeMode,
    SuspendShell,
    Quit,
    MenuBar,
    Help,
    Learn,

    // Cmdline
    CmdlineSubmit,
    CmdlineHistoryUp,
    CmdlineHistoryDn,
    CmdlineInsertSelected,
    CmdlineInsertCwd,

    // Scoped
    Viewer(ViewerAction),
    Diff(DiffAction),
    Dialog(DialogAction),
}

impl Action {
    /// Stable string id used in keymap config files.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::PanelUp => "panel.up",
            Self::PanelDown => "panel.down",
            Self::PanelPgUp => "panel.pgup",
            Self::PanelPgDn => "panel.pgdn",
            Self::PanelHome => "panel.home",
            Self::PanelEnd => "panel.end",
            Self::PanelToggleTag => "panel.toggle_tag",
            Self::PanelSelectGroup => "panel.select_group",
            Self::PanelUnselectGroup => "panel.unselect_group",
            Self::PanelInvertSelection => "panel.invert_selection",
            Self::PanelCdParent => "panel.cd_parent",
            Self::PanelCdHome => "panel.cd_home",
            Self::PanelSortBy(_) => "panel.sort_by",
            Self::PanelToggleReverse => "panel.toggle_reverse",
            Self::PanelToggleHidden => "panel.toggle_hidden",
            Self::PanelLayoutCycle => "panel.layout_cycle",
            Self::PanelSyncSibling => "panel.sync_sibling",
            Self::PanelSwap => "panel.swap",
            Self::PanelHistoryBack => "panel.history_back",
            Self::PanelHistoryFwd => "panel.history_fwd",
            Self::PanelHistoryList => "panel.history_list",
            Self::Copy => "ops.copy",
            Self::Move => "ops.move",
            Self::Delete => "ops.delete",
            Self::Mkdir => "ops.mkdir",
            Self::Chmod => "ops.chmod",
            Self::Chown => "ops.chown",
            Self::Chattr => "ops.chattr",
            Self::Hardlink => "ops.hardlink",
            Self::AbsSymlink => "ops.abs_symlink",
            Self::RelSymlink => "ops.rel_symlink",
            Self::EditSymlink => "ops.edit_symlink",
            Self::CompareDirs(_) => "ops.compare_dirs",
            Self::ViewFile => "tools.view",
            Self::ViewRaw => "tools.view_raw",
            Self::EditFile => "tools.edit",
            Self::Find => "tools.find",
            Self::FindAndPanelize => "tools.find_and_panelize",
            Self::Hotlist => "tools.hotlist",
            Self::AddToHotlist => "tools.add_to_hotlist",
            Self::UserMenu => "tools.user_menu",
            Self::ExtMenu => "tools.ext_menu",
            Self::QuickCd => "tools.quick_cd",
            Self::QuickSearch => "tools.quick_search",
            Self::SwitchPanel => "app.switch_panel",
            Self::RefreshPanel => "app.refresh_panel",
            Self::ToggleQuickView => "app.toggle_quick_view",
            Self::ToggleTreeMode => "app.toggle_tree_mode",
            Self::SuspendShell => "app.suspend_shell",
            Self::Quit => "app.quit",
            Self::MenuBar => "app.menu_bar",
            Self::Help => "app.help",
            Self::Learn => "app.learn",
            Self::CmdlineSubmit => "cmdline.submit",
            Self::CmdlineHistoryUp => "cmdline.history_up",
            Self::CmdlineHistoryDn => "cmdline.history_dn",
            Self::CmdlineInsertSelected => "cmdline.insert_selected",
            Self::CmdlineInsertCwd => "cmdline.insert_cwd",
            Self::Viewer(_) => "viewer",
            Self::Diff(_) => "diff",
            Self::Dialog(_) => "dialog",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_for_distinct_variants() {
        // Sanity: distinct unit variants must have distinct ids.
        let ids = [
            Action::PanelUp.id(),
            Action::PanelDown.id(),
            Action::Copy.id(),
            Action::Move.id(),
            Action::Quit.id(),
        ];
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }
}
