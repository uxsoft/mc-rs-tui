use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use mc_config::{AppConfig, ConfigPaths, FileHighlight, Hotlist};
use mc_core::action::SortKey;
use mc_core::key::{KeyChord, KeyCode, KeyMods};
use mc_core::{Entry, EntryKind, VPath};
use mc_jobs::{JobQueue, JobUpdateRx};
use mc_vfs::{Registry, Vfs};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tracing::warn;

use crate::dialog::{
    ConfirmDialog, Dialog, DialogOutcome, HotlistAction, HotlistDialog, InputDialog,
    MenuBar, MenuChoice, ProgressDialog,
};
use crate::panel::{render_panel, PanelState};

#[derive(Debug, Clone)]
pub enum PendingOp {
    Mkdir { in_dir: VPath, name: String },
    Rename { src: VPath, new_name: String },
    Chmod { targets: Vec<VPath>, mode: u32 },
    RunEditor { file: PathBuf, line: Option<u32> },
    /// Submit a recursive copy job.
    SubmitCopy { sources: Vec<VPath>, dst_dir: VPath },
    /// Submit a recursive move job.
    SubmitMove { sources: Vec<VPath>, dst_dir: VPath },
    /// Submit a recursive delete job.
    SubmitDelete { targets: Vec<VPath> },
}

#[derive(Debug)]
pub enum Disposition {
    None,
    Redraw,
    Quit,
    /// Reload the active panel from its VFS.
    ReloadActive,
    /// Execute a side-effecting op via the loop, then reload.
    RunOp(PendingOp),
}

/// What the UI is currently focused on. Modal state lives here.
enum Modal {
    None,
    Mkdir(InputDialog),
    DeleteConfirm(ConfirmDialog, Vec<VPath>),
    Rename(InputDialog, VPath),
    SelectGroup { dlg: InputDialog, select: bool },
    Chmod { dlg: InputDialog, targets: Vec<VPath> },
    /// Two-step Ctrl-X chord: waiting for the second key.
    PrefixCtrlX,
    /// A long-running job is in flight (or just finished).
    Progress(ProgressDialog),
    Hotlist(HotlistDialog),
    Menu(MenuBar),
    Viewer(crate::viewer_widget::ViewerWidget),
    /// Type-ahead filter; `String` is the current filter.
    QuickSearch(String),
}

pub struct App {
    pub config: AppConfig,
    pub registry: Registry,
    pub left: PanelState,
    pub right: PanelState,
    pub active_left: bool,
    pub jobs: JobQueue,
    pub highlight: FileHighlight,
    pub hotlist: Hotlist,
    pub paths: ConfigPaths,
    modal: Modal,
}

impl App {
    pub fn new(config: AppConfig, start: PathBuf) -> (Self, JobUpdateRx) {
        let registry = Registry::with_defaults();
        let cwd = VPath::local(start);
        let mut left = PanelState::new(cwd.clone());
        let right = PanelState::new(cwd);
        left.active = true;
        let (jobs, rx) = JobQueue::new(256);
        let paths = ConfigPaths::discover();
        let hotlist = Hotlist::load(&paths.config_dir.join("hotlist.toml")).unwrap_or_default();
        let app = Self {
            config,
            registry,
            left,
            right,
            active_left: true,
            jobs,
            highlight: FileHighlight::defaults(),
            hotlist,
            paths,
            modal: Modal::None,
        };
        (app, rx)
    }

    fn save_hotlist(&self) {
        let path = self.paths.config_dir.join("hotlist.toml");
        if let Err(e) = self.hotlist.save(&path) {
            tracing::warn!("save hotlist {}: {e}", path.display());
        }
    }

    /// Receive a job update; returns true if the modal should stay visible.
    /// Set the active modal to a progress dialog tracking the given handle.
    pub fn show_progress(&mut self, handle: mc_jobs::JobHandle, description: String) {
        self.modal = Modal::Progress(ProgressDialog::new(handle, description));
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    pub fn handle_job_update(&mut self, update: mc_jobs::JobUpdate) {
        if let Modal::Progress(dlg) = &mut self.modal {
            if dlg.handle.id != update.id {
                return;
            }
            match update.kind {
                mc_jobs::JobUpdateKind::Started { description } => dlg.description = description,
                mc_jobs::JobUpdateKind::Progress(p) => dlg.progress = p,
                mc_jobs::JobUpdateKind::Status(s) => dlg.status = s,
                mc_jobs::JobUpdateKind::Log(_) => {}
                mc_jobs::JobUpdateKind::Finished(o) => dlg.finished = Some(o),
            }
        }
    }

    #[must_use]
    pub fn modal_is_progress(&self) -> bool {
        matches!(self.modal, Modal::Progress(_))
    }

    #[must_use]
    pub fn progress_finished(&self) -> bool {
        matches!(&self.modal, Modal::Progress(d) if d.finished.is_some())
    }

    fn active(&mut self) -> &mut PanelState {
        if self.active_left {
            &mut self.left
        } else {
            &mut self.right
        }
    }

    fn active_ref(&self) -> &PanelState {
        if self.active_left {
            &self.left
        } else {
            &self.right
        }
    }

    pub async fn refresh_panel(&mut self, left: bool) {
        let panel = if left { &mut self.left } else { &mut self.right };
        match self.registry.root_for(&panel.cwd) {
            Ok(vfs) => match read_dir_with_parent(vfs.as_ref(), &panel.cwd).await {
                Ok(entries) => {
                    panel.entries = entries;
                    panel.apply_filter_sort();
                }
                Err(e) => {
                    warn!("read_dir failed: {e}");
                    panel.entries.clear();
                }
            },
            Err(e) => warn!("no backend for {}: {e}", panel.cwd),
        }
    }

    pub async fn refresh_both(&mut self) {
        self.refresh_panel(true).await;
        self.refresh_panel(false).await;
    }

    pub async fn refresh_active(&mut self) {
        self.refresh_panel(self.active_left).await;
    }

    pub fn handle_key(&mut self, chord: KeyChord) -> Disposition {
        // If a modal is active, route the key there first.
        if !matches!(self.modal, Modal::None) {
            return self.handle_modal_key(chord);
        }
        self.handle_panel_key(chord)
    }

    fn handle_modal_key(&mut self, chord: KeyChord) -> Disposition {
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None => Disposition::None,
            Modal::Mkdir(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Mkdir(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(name) => Disposition::RunOp(PendingOp::Mkdir {
                    in_dir: self.active_ref().cwd.clone(),
                    name,
                }),
            },
            Modal::DeleteConfirm(mut dlg, targets) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::DeleteConfirm(dlg, targets);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(false) => Disposition::Redraw,
                DialogOutcome::Submitted(true) => {
                    Disposition::RunOp(PendingOp::SubmitDelete { targets })
                }
            },
            Modal::Chmod { mut dlg, targets } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Chmod { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(s) => match parse_octal_mode(&s) {
                    Some(mode) => Disposition::RunOp(PendingOp::Chmod { targets, mode }),
                    None => {
                        // Re-open with same dialog. Keep value entered.
                        self.modal = Modal::Chmod {
                            dlg: InputDialog::new(" Chmod ", "Octal mode (e.g. 755):", s),
                            targets,
                        };
                        Disposition::Redraw
                    }
                },
            },
            Modal::PrefixCtrlX => self.handle_ctrl_x(chord),
            Modal::Rename(mut dlg, src) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Rename(dlg, src);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(new_name) => Disposition::RunOp(PendingOp::Rename {
                    src,
                    new_name,
                }),
            },
            Modal::SelectGroup { mut dlg, select } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::SelectGroup { dlg, select };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(pattern) => {
                    self.apply_select_group(&pattern, select);
                    Disposition::Redraw
                }
            },
            Modal::Menu(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Menu(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(choice) => self.handle_menu_choice(choice),
            },
            Modal::Hotlist(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Hotlist(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(action) => {
                    match action {
                        HotlistAction::AddCurrent => {
                            let p = self.active_ref().cwd.to_string();
                            let label = p
                                .rsplit_once(['/', '\\'])
                                .map_or(p.clone(), |(_, n)| n.to_string());
                            self.hotlist.add(label, p);
                            self.save_hotlist();
                            self.modal = Modal::Hotlist(HotlistDialog::new(self.hotlist.clone()));
                            Disposition::Redraw
                        }
                        HotlistAction::Remove(idx) => {
                            self.hotlist.remove_at(idx);
                            self.save_hotlist();
                            self.modal = Modal::Hotlist(HotlistDialog::new(self.hotlist.clone()));
                            Disposition::Redraw
                        }
                        HotlistAction::Navigate(s) => {
                            if let Ok(p) = s.parse::<VPath>() {
                                self.active().navigate(p);
                                Disposition::ReloadActive
                            } else {
                                tracing::warn!("hotlist: bad vpath {s:?}");
                                Disposition::Redraw
                            }
                        }
                    }
                }
            },
            Modal::Progress(dlg) => match (chord.code, dlg.finished.is_some()) {
                (KeyCode::Escape, false) => {
                    dlg.handle.cancel();
                    self.modal = Modal::Progress(dlg);
                    Disposition::Redraw
                }
                (KeyCode::Escape, true) | (KeyCode::Enter, _) => Disposition::Redraw,
                _ => {
                    self.modal = Modal::Progress(dlg);
                    Disposition::None
                }
            },
            Modal::Viewer(mut v) => {
                if v.handle_key(chord) {
                    self.modal = Modal::Viewer(v);
                    Disposition::Redraw
                } else {
                    Disposition::Redraw
                }
            }
            Modal::QuickSearch(mut filter) => match (chord.code, chord.mods) {
                (KeyCode::Escape, _) | (KeyCode::Enter, _) => Disposition::Redraw,
                (KeyCode::Backspace, _) => {
                    filter.pop();
                    self.apply_quick_search(&filter);
                    self.modal = Modal::QuickSearch(filter);
                    Disposition::Redraw
                }
                (KeyCode::Char(c), m) if m.is_empty() || m == KeyMods::SHIFT => {
                    filter.push(c);
                    self.apply_quick_search(&filter);
                    self.modal = Modal::QuickSearch(filter);
                    Disposition::Redraw
                }
                _ => {
                    self.modal = Modal::QuickSearch(filter);
                    Disposition::None
                }
            },
        }
    }

    /// If `target` points at a known archive on a local FS, mount it and
    /// return a [`VPath`] pointing at the archive's root inside the mount.
    fn try_mount_archive(&mut self, target: &VPath) -> Option<VPath> {
        // Only local files are mountable in Phase 7.
        let local_path = vpath_to_local(target)?;
        let kind = mc_vfs_archive::ArchiveKind::detect(&local_path)?;
        let scheme: &'static str = match kind {
            mc_vfs_archive::ArchiveKind::Tar
            | mc_vfs_archive::ArchiveKind::TarGz
            | mc_vfs_archive::ArchiveKind::TarBz2
            | mc_vfs_archive::ArchiveKind::TarXz
            | mc_vfs_archive::ArchiveKind::TarZst => "tar",
            mc_vfs_archive::ArchiveKind::Zip => "zip",
        };
        let mount_id = next_mount_id();
        let location = format!("mount-{mount_id}");
        match mc_vfs_archive::mount_local(&local_path, kind, scheme) {
            Ok(vfs) => {
                self.registry
                    .register_mount(scheme, location.clone(), vfs);
                let mut p = target.clone();
                p.push_layer(mc_core::path::Layer {
                    scheme: scheme.to_string(),
                    location,
                    sub: "/".into(),
                });
                Some(p)
            }
            Err(e) => {
                tracing::warn!("mount {} ({:?}): {e}", local_path.display(), kind);
                None
            }
        }
    }

    fn handle_menu_choice(&mut self, c: MenuChoice) -> Disposition {
        // Translate the menu pick into the equivalent panel-mode key chord.
        let chord = match c {
            MenuChoice::View => KeyChord::plain(KeyCode::F(3)),
            MenuChoice::Edit => KeyChord::plain(KeyCode::F(4)),
            MenuChoice::Copy => KeyChord::plain(KeyCode::F(5)),
            MenuChoice::Move => KeyChord::plain(KeyCode::F(6)),
            MenuChoice::Mkdir => KeyChord::plain(KeyCode::F(7)),
            MenuChoice::Delete => KeyChord::plain(KeyCode::F(8)),
            MenuChoice::Quit => return Disposition::Quit,
            MenuChoice::Hotlist => KeyChord::new(KeyCode::Char('\\'), KeyMods::CTRL),
            MenuChoice::ToggleHidden => KeyChord::new(KeyCode::Char('.'), KeyMods::CTRL),
            MenuChoice::SortCycle => KeyChord::new(KeyCode::Char('s'), KeyMods::ALT),
            MenuChoice::ToggleListingMode => KeyChord::new(KeyCode::Char('t'), KeyMods::ALT),
            MenuChoice::Find => {
                tracing::info!("Find: not implemented yet");
                return Disposition::Redraw;
            }
            MenuChoice::Chmod => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Disposition::Redraw;
                }
                self.modal = Modal::Chmod {
                    dlg: InputDialog::new(" Chmod ", "Octal mode (e.g. 755):", "644"),
                    targets,
                };
                return Disposition::Redraw;
            }
            MenuChoice::AddToHotlist => {
                let p = self.active_ref().cwd.to_string();
                let label = p
                    .rsplit_once(['/', '\\'])
                    .map_or(p.clone(), |(_, n)| n.to_string());
                self.hotlist.add(label, p);
                self.save_hotlist();
                return Disposition::Redraw;
            }
        };
        self.handle_panel_key(chord)
    }

    fn handle_ctrl_x(&mut self, chord: KeyChord) -> Disposition {
        // mc Ctrl-X chords; we only implement a few here (more in Phase 11).
        match (chord.code, chord.mods) {
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
            _ => Disposition::Redraw,
        }
    }

    fn apply_select_group(&mut self, pattern: &str, select: bool) {
        // Simple glob: '*' matches any sequence, '?' matches one char, others literal.
        // We do a tiny custom matcher to avoid pulling globset for Phase 1.
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

    fn apply_quick_search(&mut self, filter: &str) {
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

    fn selected_targets(&self) -> Vec<VPath> {
        let p = self.active_ref();
        let mut out: Vec<VPath> = Vec::new();
        if !p.marks.is_empty() {
            for e in &p.entries {
                if p.marks.contains(&e.name) {
                    if let Some(child) = child_path(&p.cwd, &e.name) {
                        out.push(child);
                    }
                }
            }
        } else if let Some(child) = p.cursor_path() {
            // Skip ".." entry.
            if let Some(e) = p.entries.get(p.cursor) {
                if e.name != ".." {
                    out.push(child);
                }
            }
        }
        out
    }

    fn handle_panel_key(&mut self, chord: KeyChord) -> Disposition {
        match (chord.code, chord.mods) {
            (KeyCode::F(10), m) if m.is_empty() => Disposition::Quit,
            (KeyCode::Char('q'), m) if m == KeyMods::CTRL => Disposition::Quit,
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
                let cursor_entry = self.active_ref().entries.get(self.active_ref().cursor).cloned();
                if let Some(e) = cursor_entry {
                    let target = self.active_ref().cursor_path();
                    if matches!(e.kind, EntryKind::Dir) {
                        if let Some(t) = target {
                            self.active().navigate(t);
                            return Disposition::ReloadActive;
                        }
                    } else if matches!(e.kind, EntryKind::File) {
                        // Try mounting archives.
                        if let Some(t) = target {
                            if let Some(mounted) = self.try_mount_archive(&t) {
                                self.active().navigate(mounted);
                                return Disposition::ReloadActive;
                            }
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
                // At the root of the current layer; if we're inside an archive,
                // pop the archive layer to return to the parent FS.
                if self.active_ref().cwd.layers().len() > 1 {
                    let mut new_cwd = self.active_ref().cwd.clone();
                    if let Some(layer) = new_cwd.pop_layer() {
                        // Best-effort cleanup — keep the mount registered so
                        // re-entering is fast; mounts are tiny.
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

            // Hidden toggle
            (KeyCode::Char('.'), m) if m == KeyMods::CTRL || m == KeyMods::ALT => {
                self.active().show_hidden = !self.active().show_hidden;
                Disposition::ReloadActive
            }

            // Listing-mode cycle (mc Alt-T).
            (KeyCode::Char('t'), m) if m == KeyMods::ALT => {
                self.active().mode = self.active().mode.next();
                Disposition::Redraw
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
            (KeyCode::F(9), m) if m.is_empty() => {
                self.modal = Modal::Menu(MenuBar::new());
                Disposition::Redraw
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
                if let Some(target) = self.active_ref().cursor_path() {
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

            // F5 — copy (recursive, via job queue)
            (KeyCode::F(5), m) if m.is_empty() => {
                let sources = self.selected_targets();
                if sources.is_empty() {
                    return Disposition::None;
                }
                let dst_dir = if self.active_left {
                    self.right.cwd.clone()
                } else {
                    self.left.cwd.clone()
                };
                Disposition::RunOp(PendingOp::SubmitCopy { sources, dst_dir })
            }

            // F6 — when a single non-".." entry is targeted in same dir → rename dialog.
            // Otherwise: move to other panel (recursive job).
            (KeyCode::F(6), m) if m.is_empty() => {
                let sources = self.selected_targets();
                if sources.is_empty() {
                    return Disposition::None;
                }
                if sources.len() == 1 && self.active_ref().marks.is_empty() {
                    // Single cursored item → rename dialog (mc behaviour).
                    let entry = self.active_ref().entries.get(self.active_ref().cursor).cloned();
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
                let dst_dir = if self.active_left {
                    self.right.cwd.clone()
                } else {
                    self.left.cwd.clone()
                };
                Disposition::RunOp(PendingOp::SubmitMove { sources, dst_dir })
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
                self.modal = Modal::DeleteConfirm(
                    ConfirmDialog::new(" Delete ", msg),
                    targets,
                );
                Disposition::Redraw
            }
            // Esc closes a finished progress dialog.
            (KeyCode::Escape, _) if self.progress_finished() => {
                self.modal = Modal::None;
                Disposition::Redraw
            }

            _ => Disposition::None,
        }
    }

    pub fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[0]);

        render_panel(f, panels[0], &mut self.left, &self.highlight);
        render_panel(f, panels[1], &mut self.right, &self.highlight);
        render_buttonbar(f, chunks[1]);

        match &self.modal {
            Modal::None | Modal::PrefixCtrlX => {}
            Modal::Mkdir(d) | Modal::Rename(d, _) => d.render(f, area),
            Modal::SelectGroup { dlg, .. } | Modal::Chmod { dlg, .. } => dlg.render(f, area),
            Modal::DeleteConfirm(d, _) => d.render(f, area),
            Modal::Viewer(v) => v.render(f, area),
            Modal::Progress(d) => d.render(f, area),
            Modal::Hotlist(d) => d.render(f, area),
            Modal::Menu(d) => d.render(f, area),
            Modal::QuickSearch(filter) => render_quick_search(f, area, filter),
        }
        if matches!(self.modal, Modal::PrefixCtrlX) {
            let hint = Line::from(" C-x: c=chmod   (Esc to cancel) ");
            let p = Paragraph::new(hint)
                .style(Style::default().fg(Color::Black).bg(Color::Yellow));
            let rect = ratatui::layout::Rect::new(area.x, area.y + area.height.saturating_sub(2), area.width, 1);
            f.render_widget(p, rect);
        }
    }
}

fn render_quick_search(f: &mut Frame<'_>, area: ratatui::layout::Rect, filter: &str) {
    let line = Line::from(vec![
        Span::raw(" Search: "),
        Span::styled(
            filter.to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
    ]);
    let p = Paragraph::new(line).style(Style::default().fg(Color::White).bg(Color::Black));
    let rect = ratatui::layout::Rect::new(area.x, area.y + area.height.saturating_sub(2), area.width, 1);
    f.render_widget(p, rect);
}

fn next_sort(s: SortKey) -> SortKey {
    match s {
        SortKey::Name => SortKey::Extension,
        SortKey::Extension => SortKey::Size,
        SortKey::Size => SortKey::Mtime,
        SortKey::Mtime => SortKey::Atime,
        SortKey::Atime => SortKey::Ctime,
        SortKey::Ctime => SortKey::Unsorted,
        SortKey::Unsorted | SortKey::Inode => SortKey::Name,
    }
}

fn parent_path(p: &VPath) -> Option<VPath> {
    let last = p.last()?.clone();
    let mut sub = last.sub.clone();
    if !sub.pop() {
        return None;
    }
    let mut new_layer = last;
    new_layer.sub = sub;
    let mut new_path = p.clone();
    new_path.pop_layer();
    new_path.push_layer(new_layer);
    Some(new_path)
}

fn child_path(parent: &VPath, name: &str) -> Option<VPath> {
    let last = parent.last()?.clone();
    let mut sub = last.sub.clone();
    sub.push(name);
    let mut new_layer = last;
    new_layer.sub = sub;
    let mut new = parent.clone();
    new.pop_layer();
    new.push_layer(new_layer);
    Some(new)
}

fn next_mount_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

fn parse_octal_mode(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim(), 8).ok().filter(|&m| m <= 0o7777)
}

/// Tiny glob: `*` matches any chars, `?` matches one char, case-insensitive.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    glob_inner(&p, &t)
}

fn glob_inner(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[must_use]
pub fn vpath_to_local(p: &VPath) -> Option<PathBuf> {
    let layer = p.last()?;
    if layer.scheme != "local" {
        return None;
    }
    Some(layer.sub.clone())
}

async fn read_dir_with_parent(vfs: &dyn Vfs, p: &VPath) -> mc_core::Result<Vec<Entry>> {
    let mut entries = vfs.read_dir(p).await?;
    if parent_path(p).is_some() {
        entries.insert(
            0,
            Entry {
                name: "..".into(),
                kind: EntryKind::Dir,
                size: 0,
                mtime: None,
                atime: None,
                ctime: None,
                mode: None,
                uid: None,
                gid: None,
                nlink: None,
                target: None,
            },
        );
    }
    Ok(entries)
}

#[allow(dead_code)]
const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<Arc<dyn Vfs>>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_basic() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.txt", "foo.txt"));
        assert!(glob_match("*.txt", "FOO.TXT"));
        assert!(glob_match("?oo.txt", "foo.txt"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(!glob_match("*.txt", "foo.md"));
        assert!(!glob_match("foo", "bar"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
    }

    #[test]
    fn parent_path_pops_segment() {
        let p = VPath::local("/a/b/c");
        let parent = parent_path(&p).unwrap();
        assert_eq!(parent.last().unwrap().sub.to_str().unwrap(), "/a/b");
    }

    #[test]
    fn child_path_appends() {
        let p = VPath::local("/a/b");
        let c = child_path(&p, "c").unwrap();
        assert_eq!(c.last().unwrap().sub.to_str().unwrap(), "/a/b/c");
    }
}

fn render_buttonbar(f: &mut Frame<'_>, area: ratatui::layout::Rect) {
    let labels = [
        (1, "Help"),
        (2, "Menu"),
        (3, "View"),
        (4, "Edit"),
        (5, "Copy"),
        (6, "RenMov"),
        (7, "Mkdir"),
        (8, "Delete"),
        (9, "PullDn"),
        (10, "Quit"),
    ];
    let mut spans: Vec<Span> = Vec::with_capacity(labels.len() * 3);
    for (i, (n, name)) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("{n}"),
            Style::default().fg(Color::White).bg(Color::Black),
        ));
        spans.push(Span::styled(
            *name,
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
    }
    let line = Line::from(spans);
    let p = Paragraph::new(line).style(Style::default().bg(Color::Black));
    f.render_widget(p, area);
}
