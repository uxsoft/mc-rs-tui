//! Panel-mode key handling: top-level `handle_panel_key`, the smaller
//! tree-mode variant, the Ctrl-X chord dispatcher, and the small selection
//! helpers (`apply_select_group`, `apply_quick_search`).

use std::path::PathBuf;

use mc_config::ExtAction;
use mc_core::EntryKind;
use mc_core::key::{KeyChord, KeyCode, KeyMods};

use crate::dialog::{ConfirmDialog, FindForm, FindParams, HotlistDialog, InputDialog};
use crate::glob::glob_match;
use crate::panel::ListingMode;

use super::ops::{next_sort, parent_path, vpath_to_local};
use super::{App, CopyMoveKind, Disposition, Modal, PendingOp};

/// Detect MIME for a local file via libmagic-style sniffing. Returns `None`
/// for non-local paths (caller falls back to glob-only matching).
fn sniff_mime(target: &mc_core::VPath) -> Option<&'static str> {
    let local = vpath_to_local(target)?;
    Some(tree_magic_mini::from_filepath(&local).unwrap_or("application/octet-stream"))
}

impl App {
    pub(super) fn apply_select_group(&mut self, pattern: &str, select: bool) {
        let p = self.active();
        let names: Vec<String> = p
            .entries
            .iter()
            .filter(|e| e.name != ".." && glob_match(pattern, &e.name))
            .map(|e| e.name.clone())
            .collect();
        for name in names {
            if select {
                p.marks.insert(name);
            } else {
                p.marks.remove(&name);
            }
        }
    }

    pub(super) fn apply_quick_search(&mut self, filter: &str) {
        if filter.is_empty() {
            return;
        }
        let lower = filter.to_lowercase();
        let p = self.active();
        if let Some(idx) = p
            .entries
            .iter()
            .position(|e| e.name.to_lowercase().starts_with(&lower))
        {
            p.cursor = idx;
        }
    }

    pub(super) fn handle_ctrl_x(&mut self, chord: KeyChord) -> Disposition {
        // mc Ctrl-X chords; we only implement a few here (more in Phase 11).
        match (chord.code, chord.mods) {
            (KeyCode::Char('='), m) if m.is_empty() => {
                self.compare_dirs();
                let n = if self.active_left {
                    self.left.marks.len()
                } else {
                    self.right.marks.len()
                };
                self.set_status(format!("Compare dirs: {n} files differ"));
                Disposition::Redraw
            }
            (KeyCode::Char('c'), m) | (KeyCode::Char('C'), m)
                if m.is_empty() || m == KeyMods::SHIFT =>
            {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Disposition::Redraw;
                }
                self.modal = Modal::Chmod {
                    dlg: InputDialog::new(" Chmod ", "Octal mode (e.g. 755):", "644"),
                    targets,
                };
                Disposition::Redraw
            }
            (KeyCode::Char('h'), m) | (KeyCode::Char('H'), m)
                if m.is_empty() || m == KeyMods::SHIFT =>
            {
                let p = self.active_ref().cwd.to_string();
                let label = p
                    .rsplit_once(['/', '\\'])
                    .map_or(p.clone(), |(_, n)| n.to_string());
                self.hotlist.add(label, p);
                self.save_hotlist();
                Disposition::Redraw
            }
            (KeyCode::Char('d'), m) | (KeyCode::Char('D'), m)
                if m.is_empty() || m == KeyMods::SHIFT =>
            {
                self.open_diff()
            }
            (KeyCode::Char('p'), m) | (KeyCode::Char('P'), m)
                if m.is_empty() || m == KeyMods::SHIFT =>
            {
                let cwd = self
                    .active_ref()
                    .cwd
                    .last()
                    .map(|l| l.sub.display().to_string())
                    .unwrap_or_default();
                match crate::clipboard::copy(&cwd) {
                    Some(via) => self.set_status(format!("Copied cwd to {via}")),
                    None => self.set_status("Clipboard unavailable"),
                }
                Disposition::Redraw
            }
            (KeyCode::Char('t'), m) | (KeyCode::Char('T'), m)
                if m.is_empty() || m == KeyMods::SHIFT =>
            {
                let active = self.active_ref();
                let mut full = active
                    .cwd
                    .last()
                    .map(|l| l.sub.display().to_string())
                    .unwrap_or_default();
                if let Some(e) = active.entries.get(active.cursor) {
                    if e.name != ".." {
                        if !full.ends_with('/') && !full.is_empty() {
                            full.push('/');
                        }
                        full.push_str(&e.name);
                    }
                }
                match crate::clipboard::copy(&full) {
                    Some(via) => self.set_status(format!("Copied path to {via}")),
                    None => self.set_status("Clipboard unavailable"),
                }
                Disposition::Redraw
            }
            _ => Disposition::Redraw,
        }
    }

    fn handle_tree_panel_key(&mut self, chord: KeyChord) -> Disposition {
        match (chord.code, chord.mods) {
            (KeyCode::F(10), m) if m.is_empty() => Disposition::Quit,
            (KeyCode::Char('q'), m) if m == KeyMods::CTRL => self.maybe_confirm_quit(),
            (KeyCode::Tab, _) => {
                self.active_left = !self.active_left;
                self.left.active = self.active_left;
                self.right.active = !self.active_left;
                Disposition::Redraw
            }
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                let p = self.active();
                p.tree.cursor = p.tree.cursor.saturating_sub(1);
                Disposition::Redraw
            }
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                let p = self.active();
                if p.tree.cursor + 1 < p.tree.nodes.len() {
                    p.tree.cursor += 1;
                }
                Disposition::Redraw
            }
            (KeyCode::Home, _) => {
                self.active().tree.cursor = 0;
                Disposition::Redraw
            }
            (KeyCode::End, _) => {
                let p = self.active();
                p.tree.cursor = p.tree.nodes.len().saturating_sub(1);
                Disposition::Redraw
            }
            (KeyCode::Enter, _) | (KeyCode::Char(' '), _) => Disposition::TreeToggle,
            (KeyCode::Char('t'), m) if m == KeyMods::ALT => {
                self.active().mode = ListingMode::Full;
                Disposition::Redraw
            }
            (KeyCode::F(1), m) if m.is_empty() => {
                self.modal = Modal::Help(crate::dialog::HelpDialog::new());
                Disposition::Redraw
            }
            _ => Disposition::None,
        }
    }

    pub(super) fn handle_panel_key(&mut self, chord: KeyChord) -> Disposition {
        if matches!(self.active_ref().mode, ListingMode::Tree) {
            return self.handle_tree_panel_key(chord);
        }
        match (chord.code, chord.mods) {
            (KeyCode::F(10), m) if m.is_empty() => Disposition::Quit,
            (KeyCode::Char('q'), m) if m == KeyMods::CTRL => self.maybe_confirm_quit(),
            (KeyCode::Tab, _) => {
                self.active_left = !self.active_left;
                self.left.active = self.active_left;
                self.right.active = !self.active_left;
                Disposition::Redraw
            }

            // Cursor
            (KeyCode::Up, _) => {
                let p = self.active();
                p.cursor = p.cursor.saturating_sub(1);
                Disposition::Redraw
            }
            (KeyCode::Down, _) => {
                let p = self.active();
                if p.cursor + 1 < p.entries.len() {
                    p.cursor += 1;
                }
                Disposition::Redraw
            }
            (KeyCode::PageUp, _) => {
                let p = self.active();
                p.cursor = p.cursor.saturating_sub(20);
                Disposition::Redraw
            }
            (KeyCode::PageDown, _) => {
                let p = self.active();
                p.cursor = (p.cursor + 20).min(p.entries.len().saturating_sub(1));
                Disposition::Redraw
            }
            (KeyCode::Home, _) => {
                self.active().cursor = 0;
                Disposition::Redraw
            }
            (KeyCode::End, _) => {
                let p = self.active();
                p.cursor = p.entries.len().saturating_sub(1);
                Disposition::Redraw
            }

            // Navigation
            (KeyCode::Enter, _) => {
                let cursor_entry = self
                    .active_ref()
                    .entries
                    .get(self.active_ref().cursor)
                    .cloned();
                if let Some(e) = cursor_entry {
                    let target = self.active_ref().cursor_path();
                    if matches!(e.kind, EntryKind::Dir) {
                        if let Some(t) = target {
                            self.active().navigate(t);
                            return Disposition::ReloadActive;
                        }
                    } else if matches!(e.kind, EntryKind::File) {
                        if let Some(t) = &target {
                            if let Some(mounted) = self.try_mount_archive(t) {
                                self.active().navigate(mounted);
                                return Disposition::ReloadActive;
                            }
                        }
                        let mime = target.as_ref().and_then(sniff_mime);
                        if let Some(template) = self.extbinds.lookup(&e.name, mime, ExtAction::Open)
                        {
                            let template = template.to_string();
                            return self.run_template(&template);
                        }
                    }
                }
                Disposition::None
            }
            (KeyCode::Backspace, _) => {
                if let Some(target) = parent_path(&self.active_ref().cwd) {
                    self.active().navigate(target);
                    return Disposition::ReloadActive;
                }
                if self.active_ref().cwd.layers().len() > 1 {
                    let mut new_cwd = self.active_ref().cwd.clone();
                    if let Some(layer) = new_cwd.pop_layer() {
                        let _ = layer;
                    }
                    self.active().navigate(new_cwd);
                    return Disposition::ReloadActive;
                }
                Disposition::None
            }

            // Tagging
            (KeyCode::Insert, _) => {
                self.active().toggle_mark();
                Disposition::Redraw
            }
            (KeyCode::Char(' '), m) if m.is_empty() || m == KeyMods::SHIFT => {
                let mut compute: Option<PendingOp> = None;
                {
                    let p = self.active();
                    if let Some(e) = p.entries.get(p.cursor) {
                        if matches!(e.kind, EntryKind::Dir)
                            && e.name != ".."
                            && !p.computed_dir_sizes.contains(&e.name)
                        {
                            compute = Some(PendingOp::ComputeDirSize {
                                cwd: p.cwd.clone(),
                                name: e.name.clone(),
                            });
                        }
                    }
                }
                self.active().toggle_mark();
                if let Some(op) = compute {
                    Disposition::RunOp(op)
                } else {
                    Disposition::Redraw
                }
            }

            // Hidden toggle
            (KeyCode::Char('.'), m) if m == KeyMods::CTRL || m == KeyMods::ALT => {
                self.active().show_hidden = !self.active().show_hidden;
                Disposition::ReloadActive
            }

            // Listing-mode cycle (mc Alt-T).
            (KeyCode::Char('t'), m) if m == KeyMods::ALT => {
                self.active().mode = self.active().mode.next();
                if matches!(self.active().mode, ListingMode::Tree) {
                    Disposition::RebuildTree
                } else {
                    Disposition::Redraw
                }
            }

            // Sort: Alt-S cycles key, Ctrl-R reverses.
            (KeyCode::Char('s'), m) if m == KeyMods::ALT => {
                let p = self.active();
                p.sort_by = next_sort(p.sort_by);
                p.apply_filter_sort();
                Disposition::Redraw
            }
            (KeyCode::Char('r'), m) if m == KeyMods::CTRL => {
                let p = self.active();
                p.reverse = !p.reverse;
                p.apply_filter_sort();
                Disposition::Redraw
            }

            // Quick search (mc Ctrl-S).
            (KeyCode::Char('s'), m) if m == KeyMods::CTRL => {
                self.modal = Modal::QuickSearch(String::new());
                Disposition::Redraw
            }

            // Ctrl-X chord prefix.
            (KeyCode::Char('x'), m) if m == KeyMods::CTRL => {
                self.modal = Modal::PrefixCtrlX;
                Disposition::Redraw
            }

            // Ctrl-\ — hotlist
            (KeyCode::Char('\\'), m) if m == KeyMods::CTRL => {
                self.modal = Modal::Hotlist(HotlistDialog::new(self.hotlist.clone()));
                Disposition::Redraw
            }

            // F9 — menu bar
            (KeyCode::F(9), m) if m.is_empty() => self.open_menubar(),

            // Alt-? — Find file
            (KeyCode::Char('?'), m) if m == KeyMods::ALT || m == KeyMods::ALT | KeyMods::SHIFT => {
                self.modal = Modal::FindForm(FindForm::new(FindParams::default()));
                Disposition::Redraw
            }

            // F1 — Help
            (KeyCode::F(1), m) if m.is_empty() => {
                self.modal = Modal::Help(crate::dialog::HelpDialog::new());
                Disposition::Redraw
            }

            // Ctrl-K — Learn keys
            (KeyCode::Char('k'), m) if m == KeyMods::CTRL => {
                self.modal = Modal::LearnKeys(crate::dialog::LearnKeysDialog::new());
                Disposition::Redraw
            }

            // Ctrl-J — Background jobs view
            (KeyCode::Char('j'), m) if m == KeyMods::CTRL => {
                let rows: Vec<crate::dialog::JobRow> = self
                    .job_log
                    .iter()
                    .map(|r| crate::dialog::JobRow {
                        id_str: format!("{}", r.id.0),
                        description: r.description.clone(),
                        status: r.status.clone(),
                        finished: r.finished.is_some(),
                    })
                    .collect();
                self.modal = Modal::JobsView(crate::dialog::JobsViewDialog::new(rows));
                Disposition::Redraw
            }

            // Ctrl-O — drop to a shell in the active panel's cwd
            (KeyCode::Char('o'), m) if m == KeyMods::CTRL => {
                let cwd = self
                    .active_ref()
                    .cwd
                    .last()
                    .map(|l| l.sub.clone())
                    .unwrap_or_else(|| PathBuf::from("."));
                Disposition::RunOp(PendingOp::DropToShell { cwd })
            }

            // Alt-C — Quick cd
            (KeyCode::Char('c'), m) if m == KeyMods::ALT => {
                self.modal = Modal::QuickCd(InputDialog::new(
                    " Quick cd ",
                    "Path or URL (e.g. /tmp, sftp://user@host/srv, ftp://anon@host/pub):",
                    String::new(),
                ));
                Disposition::Redraw
            }

            // F2 — open the top menu bar.
            (KeyCode::F(2), m) if m.is_empty() => self.open_menubar(),

            // ":" — open the command line
            (KeyCode::Char(':'), m) if m.is_empty() || m == KeyMods::SHIFT => {
                let entries: Vec<String> = self.cmd_history.entries().iter().cloned().collect();
                self.modal = Modal::CmdLine(
                    InputDialog::new(" Command ", "$", String::new()).with_history(entries),
                );
                Disposition::Redraw
            }

            // Alt-I — mirror active panel cwd to the other panel.
            (KeyCode::Char('i'), m) if m == KeyMods::ALT => {
                let cwd = self.active_ref().cwd.clone();
                if self.active_left {
                    self.right.navigate(cwd);
                } else {
                    self.left.navigate(cwd);
                }
                Disposition::ReloadBoth
            }
            // Alt-O — load active panel's parent (or selected dir) to the other panel.
            (KeyCode::Char('o'), m) if m == KeyMods::ALT => {
                let target = {
                    let active = self.active_ref();
                    let entry = active.entries.get(active.cursor).cloned();
                    match entry {
                        Some(e) if matches!(e.kind, EntryKind::Dir) && e.name != ".." => {
                            active.cursor_path()
                        }
                        _ => parent_path(&active.cwd).or_else(|| Some(active.cwd.clone())),
                    }
                };
                if let Some(t) = target {
                    if self.active_left {
                        self.right.navigate(t);
                    } else {
                        self.left.navigate(t);
                    }
                }
                Disposition::ReloadActive
            }

            // History
            (KeyCode::Char('y'), m) if m == KeyMods::ALT => {
                if self.active().history_back() {
                    Disposition::ReloadActive
                } else {
                    Disposition::None
                }
            }
            (KeyCode::Char('u'), m) if m == KeyMods::ALT => {
                if self.active().history_fwd() {
                    Disposition::ReloadActive
                } else {
                    Disposition::None
                }
            }

            // F3 — View
            (KeyCode::F(3), m) if m.is_empty() => {
                let entry = self
                    .active_ref()
                    .entries
                    .get(self.active_ref().cursor)
                    .cloned();
                let target = self.active_ref().cursor_path();
                if let (Some(e), Some(target)) = (entry, target) {
                    let mime = sniff_mime(&target);
                    if let Some(template) = self.extbinds.lookup(&e.name, mime, ExtAction::View) {
                        let template = template.to_string();
                        return self.run_template(&template);
                    }
                    if let Some(local) = vpath_to_local(&target) {
                        match crate::viewer_widget::ViewerWidget::open(&local) {
                            Ok(v) => {
                                self.modal = Modal::Viewer(v);
                                return Disposition::Redraw;
                            }
                            Err(e) => tracing::warn!("view {}: {e}", local.display()),
                        }
                    }
                }
                Disposition::None
            }

            // F4 — Edit
            (KeyCode::F(4), m) if m.is_empty() => {
                if let Some(target) = self.active_ref().cursor_path() {
                    if let Some(local) = vpath_to_local(&target) {
                        return Disposition::RunOp(PendingOp::RunEditor {
                            file: local,
                            line: None,
                        });
                    }
                }
                Disposition::None
            }

            // F5 — copy: prompt for destination, then submit job.
            (KeyCode::F(5), m) if m.is_empty() => self.open_copy_move(CopyMoveKind::Copy),

            // Shift-F6 — bulk rename via $EDITOR.
            (KeyCode::F(6), m) if m == KeyMods::SHIFT => {
                let sources = self.selected_targets();
                if sources.is_empty() {
                    return Disposition::None;
                }
                let parent = self.active_ref().cwd.clone();
                return Disposition::RunOp(PendingOp::BulkRename { parent, sources });
            }

            // F6 — single-cursored entry → rename, otherwise → move.
            (KeyCode::F(6), m) if m.is_empty() => {
                let sources = self.selected_targets();
                if sources.is_empty() {
                    return Disposition::None;
                }
                if sources.len() == 1 && self.active_ref().marks.is_empty() {
                    let entry = self
                        .active_ref()
                        .entries
                        .get(self.active_ref().cursor)
                        .cloned();
                    let src = sources.into_iter().next().unwrap();
                    if let Some(e) = entry {
                        if e.name != ".." {
                            self.modal = Modal::Rename(
                                InputDialog::new(" Rename ", "New name:", e.name),
                                src,
                            );
                            return Disposition::Redraw;
                        }
                    }
                    return Disposition::None;
                }
                self.open_copy_move_with(sources, CopyMoveKind::Move)
            }

            // + select group, \\ unselect group
            (KeyCode::Char('+'), m) if m.is_empty() || m == KeyMods::SHIFT => {
                self.modal = Modal::SelectGroup {
                    dlg: InputDialog::new(" Select group ", "Pattern (glob, e.g. *.txt):", "*"),
                    select: true,
                };
                Disposition::Redraw
            }
            (KeyCode::Char('\\'), m) if m.is_empty() => {
                self.modal = Modal::SelectGroup {
                    dlg: InputDialog::new(" Unselect group ", "Pattern (glob):", "*"),
                    select: false,
                };
                Disposition::Redraw
            }

            // F7 — mkdir
            (KeyCode::F(7), m) if m.is_empty() => {
                self.modal = Modal::Mkdir(InputDialog::new(
                    " Create directory ",
                    "Enter directory name:",
                    String::new(),
                ));
                Disposition::Redraw
            }

            // F8 — delete
            (KeyCode::F(8), m) if m.is_empty() | (m == KeyMods::SHIFT) => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Disposition::None;
                }
                let msg = if targets.len() == 1 {
                    format!(
                        "Delete \"{}\"?",
                        targets[0]
                            .last()
                            .map(|l| l.sub.display().to_string())
                            .unwrap_or_default()
                    )
                } else {
                    format!("Delete {} items?", targets.len())
                };
                self.modal = Modal::DeleteConfirm(ConfirmDialog::new(" Delete ", msg), targets);
                Disposition::Redraw
            }
            _ => Disposition::None,
        }
    }
}
