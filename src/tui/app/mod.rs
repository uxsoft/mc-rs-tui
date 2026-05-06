//! Top-level `App` state machine and event entry points.
//!
//! The original `app.rs` was split per concern:
//! - [`mod@events`]   panel-mode key handling (`handle_panel_key`, ctrl-x).
//! - [`mod@modal`]    modal dispatch (`handle_modal_key`, menu choice routing).
//! - [`mod@mouse`]    mouse routing (panels, menubar, buttonbar, dialogs).
//! - [`mod@render`]   per-frame drawing.
//! - [`mod@ops`]      file-operation helpers and the small free utilities
//!                    (`vpath_to_local`, `parent_path`, `parse_chown`, …).
//!
//! Sibling submodules reach private items in this file (`Modal`, `App` fields)
//! through Rust's standard "submodule sees parent's private items" rule;
//! cross-submodule helpers are marked `pub(super)`.

mod events;
mod modal;
mod mouse;
mod ops;
mod render;

pub use ops::{dst_input_is_dir, parent_path, parse_chown, parse_dst, vpath_to_local};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{
    AppConfig, ColorScheme, CompiledExtBindings, ConfigPaths, ExtBindings, FileHighlight, History,
    Hotlist, Keymap, SkinFile,
};
use crate::core::key::{KeyChord, KeyCode, KeyMods};
use crate::core::{Entry, EntryKind, VPath};
use crate::jobs::{CopyOptions, JobQueue, JobUpdateRx};
use crate::vfs::{Registry, Vfs};
use ratatui::layout::Rect;
use tracing::warn;

use crate::tui::dialog::{
    ConfirmDialog, CopyMoveSettingsDialog, FindForm, FindResults, HotlistDialog, InputDialog,
    MenuBar, ProgressDialog,
};
use crate::tui::panel::PanelState;

use ops::apply_panel_snapshot;

#[derive(Debug, Clone, Copy)]
pub(super) struct ButtonSegment {
    pub(super) fkey: u8,
    pub(super) x: u16,
    pub(super) w: u16,
}

/// Per-frame layout snapshot used by [`App::handle_mouse`] for hit-testing.
/// Refreshed at the top of [`App::render`]; never read from before the first
/// draw, but pre-initialized to zero rects so a mouse event delivered before
/// the first frame is harmless.
#[derive(Debug, Clone, Default)]
pub(super) struct LayoutSnapshot {
    /// Full terminal area as passed to dialogs' `render`/`handle_mouse`.
    pub(super) frame: Rect,
    pub(super) menubar: Rect,
    pub(super) left_body: Rect,
    pub(super) right_body: Rect,
    pub(super) buttonbar: Rect,
    pub(super) button_segments: Vec<ButtonSegment>,
}

#[derive(Debug, Clone)]
pub struct JobLogEntry {
    pub id: crate::jobs::JobId,
    pub description: String,
    pub status: String,
    pub finished: Option<crate::jobs::JobOutcome>,
}

#[derive(Debug, Clone)]
pub enum PendingOp {
    Mkdir {
        in_dir: VPath,
        name: String,
    },
    Chmod {
        targets: Vec<VPath>,
        mode: u32,
    },
    RunEditor {
        file: PathBuf,
        line: Option<u32>,
    },
    /// Submit a recursive copy job. The raw destination string is resolved
    /// asynchronously in the event loop: it may resolve to a directory
    /// (use source basenames) or — for a single source — to a target
    /// file path (combined copy + rename).
    SubmitCopy {
        sources: Vec<VPath>,
        dst_input: String,
        src_cwd: VPath,
        opts: CopyOptions,
    },
    /// Submit a recursive move job. Destination resolution mirrors
    /// `SubmitCopy`: a directory keeps source basenames; for a single
    /// source a file path renames the source as part of the move.
    SubmitMove {
        sources: Vec<VPath>,
        dst_input: String,
        src_cwd: VPath,
        opts: CopyOptions,
    },
    /// Submit a recursive delete job.
    SubmitDelete {
        targets: Vec<VPath>,
    },
    /// Start a Find over the active panel's cwd with the given form params.
    StartFind {
        start: VPath,
        params: crate::tui::dialog::FindParams,
    },
    /// Run a shell command line with terminal suspend/restore. `cwd` is the
    /// directory the command runs in; `cmd` already has `%`-macros expanded.
    RunShell {
        cwd: PathBuf,
        cmd: String,
    },
    /// Suspend the TUI, drop the user into `$SHELL` interactive in `cwd`,
    /// restore on exit. Phase 13 first cut (no cwd-sync via PROMPT_COMMAND).
    DropToShell {
        cwd: PathBuf,
    },
    /// User submitted a password for a remote-VFS retry.
    RetryRemoteWithPassword {
        scheme: String,
        location: String,
        password: String,
    },
    /// User confirmed an unknown SSH host fingerprint; record it in
    /// known_hosts and retry the SFTP connection.
    AcceptHostKeyAndRetry {
        scheme: String,
        location: String,
        algorithm: String,
        fingerprint: String,
    },
    /// Change owner / group on each target. `uid` / `gid` may be `None`
    /// to leave that side unchanged.
    Chown {
        targets: Vec<VPath>,
        uid: Option<u32>,
        gid: Option<u32>,
    },
    /// Create a hard link.
    Hardlink {
        src: PathBuf,
        link: PathBuf,
    },
    /// Create a symbolic link. If `relative`, `target` is rewritten
    /// relative to `link`'s parent before calling `symlink`.
    Symlink {
        target: PathBuf,
        link: PathBuf,
        relative: bool,
    },
    /// Replace `link`'s target with `new_target`.
    EditSymlink {
        link: PathBuf,
        new_target: PathBuf,
    },
    /// Recursively compute size of each subdirectory of `cwd`. Local only.
    ComputeSizes {
        cwd: VPath,
    },
    /// Recursively compute size of a single subdirectory `name` of `cwd`.
    /// Local only. Triggered by Space on a directory.
    ComputeDirSize {
        cwd: VPath,
        name: String,
    },
    /// Run `cmd` via `sh -c`, parse stdout as one path per line, and
    /// populate the active panel with those entries.
    ExternalPanelize {
        cwd: PathBuf,
        cmd: String,
    },
    /// Open the given source paths' filenames in `$EDITOR`, then rename
    /// each entry whose line was edited. All sources must share `parent`
    /// (the panel cwd at the time of submission). Two-phase rename handles
    /// cycles like `a→b, b→a`.
    BulkRename {
        parent: VPath,
        sources: Vec<VPath>,
    },
}

#[derive(Debug)]
pub enum Disposition {
    None,
    Redraw,
    Quit,
    /// Reload the active panel from its VFS.
    ReloadActive,
    /// Reload both panels (e.g. after Alt-I sync).
    ReloadBoth,
    /// Rebuild the active panel's tree-mode nodes (when entering Tree mode).
    RebuildTree,
    /// Toggle expand/collapse of the cursor node in tree mode.
    TreeToggle,
    /// Execute a side-effecting op via the loop, then reload.
    RunOp(PendingOp),
}

/// Distinguishes Copy (F5) from Move (F6) for the unified destination prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyMoveKind {
    Copy,
    Move,
}

impl CopyMoveKind {
    fn title(self) -> &'static str {
        match self {
            Self::Copy => " Copy ",
            Self::Move => " Move ",
        }
    }
    fn verb(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
        }
    }
}

/// What the UI is currently focused on. Modal state lives here.
pub(super) enum Modal {
    None,
    Mkdir(InputDialog),
    DeleteConfirm(ConfirmDialog, Vec<VPath>),
    /// MC-style "Copy to:" / "Move to:" settings dialog. Sources collected at
    /// F5/F6 time; on Enter the destination is parsed and the corresponding
    /// job is submitted with the chosen options. `kind` selects Copy vs Move.
    CopyMove {
        dlg: CopyMoveSettingsDialog,
        sources: Vec<VPath>,
        src_cwd: VPath,
        kind: CopyMoveKind,
    },
    SelectGroup {
        dlg: InputDialog,
        select: bool,
    },
    Chmod {
        dlg: InputDialog,
        targets: Vec<VPath>,
    },
    /// Two-step Ctrl-X chord: waiting for the second key.
    PrefixCtrlX,
    /// A long-running job is in flight (or just finished).
    Progress(ProgressDialog),
    Hotlist(HotlistDialog),
    /// Menubar has keyboard focus; the dropdown is open. Menubar state lives
    /// on `App::menubar` so the title row can be drawn even when this is
    /// inactive.
    Menu,
    FindForm(FindForm),
    FindResults(FindResults),
    CmdLine(InputDialog),
    UserMenu(crate::tui::dialog::UserMenuDialog),
    Diff(crate::tui::diff_widget::DiffWidget),
    Help(crate::tui::dialog::HelpDialog),
    QuickCd(InputDialog),
    LearnKeys(crate::tui::dialog::LearnKeysDialog),
    JobsView(crate::tui::dialog::JobsViewDialog),
    /// Awaiting a password to retry auth for `(scheme, location, user)`.
    Password {
        dlg: crate::tui::dialog::PasswordDialog,
        scheme: String,
        location: String,
    },
    /// Awaiting confirmation that the user trusts this SSH host's fingerprint
    /// for the first time. On Yes, the fingerprint is recorded in
    /// known_hosts and the connection is retried.
    HostKeyConfirm {
        dlg: ConfirmDialog,
        scheme: String,
        location: String,
        algorithm: String,
        fingerprint: String,
    },
    Viewer(crate::tui::viewer_widget::ViewerWidget),
    /// Type-ahead filter; `String` is the current filter.
    QuickSearch(String),
    Chown {
        dlg: InputDialog,
        targets: Vec<VPath>,
    },
    Chattr {
        dlg: InputDialog,
        targets: Vec<VPath>,
    },
    Hardlink {
        dlg: InputDialog,
        src: VPath,
    },
    Symlink {
        dlg: InputDialog,
        src: VPath,
        relative: bool,
    },
    EditSymlink {
        dlg: InputDialog,
        link: PathBuf,
    },
    Filter(InputDialog),
    VfsList(crate::tui::dialog::VfsListDialog),
    ExternalPanelize(InputDialog),
    Configuration(crate::tui::dialog::OptionsDialog),
    Layout(crate::tui::dialog::LayoutDialog),
    Confirmation(crate::tui::dialog::OptionsDialog),
    VirtualFs(crate::tui::dialog::OptionsDialog),
    Theme(crate::tui::dialog::ThemeDialog),
    QuitConfirm(ConfirmDialog),
    /// OK-only acknowledgement dialog shown when a job (copy/move/delete/...)
    /// fails. Carries the description of the failed operation and the
    /// underlying error string.
    Error(crate::tui::dialog::ErrorDialog),
}

pub struct App {
    pub config: AppConfig,
    pub registry: Registry,
    pub left: PanelState,
    pub right: PanelState,
    pub active_left: bool,
    pub jobs: JobQueue,
    /// Lifecycle log for the background-jobs view (Ctrl-J).
    pub job_log: std::collections::VecDeque<JobLogEntry>,
    pub highlight: FileHighlight,
    pub hotlist: Hotlist,
    pub paths: ConfigPaths,
    pub extbinds: CompiledExtBindings,
    pub keymap: Keymap,
    pub skin: SkinFile,
    pub scheme: ColorScheme,
    pub cmd_history: History,
    /// Transient status: (text, deadline). Shown in place of buttonbar until
    /// `deadline` is reached.
    pub(super) status_msg: Option<(String, Instant)>,
    pub menubar: MenuBar,
    pub(super) modal: Modal,
    /// Geometry from the last `render()` call — used by `handle_mouse` to
    /// hit-test panels, the buttonbar, and the menu title row.
    pub(super) layout: LayoutSnapshot,
    /// Last left-click `(col, row, when)` for double-click detection in panels.
    /// Cleared after a double-click fires so triple-clicks don't repeat.
    pub(super) last_click: Option<(u16, u16, Instant)>,
}

impl App {
    pub fn new(config: AppConfig, start: PathBuf) -> (Self, JobUpdateRx) {
        let registry = Registry::with_defaults();
        let cwd = VPath::local(start);
        let mut left = PanelState::new(cwd.clone());
        let mut right = PanelState::new(cwd);
        left.active = true;
        for p in [&mut left, &mut right] {
            p.show_hidden = config.panels.show_hidden;
            p.mix_dirs = config.panels.mix_dirs;
        }
        if let Some(s) = &config.panel_left {
            apply_panel_snapshot(&mut left, s);
        }
        if let Some(s) = &config.panel_right {
            apply_panel_snapshot(&mut right, s);
        }
        let (jobs, rx) = JobQueue::new(256);
        let paths = ConfigPaths::discover();
        let hotlist = Hotlist::load(&paths.config_dir.join("hotlist.toml")).unwrap_or_default();
        let ext_cfg =
            ExtBindings::load(&paths.config_dir.join("extbind.toml")).unwrap_or_else(|e| {
                tracing::warn!("extbind: load failed: {e}");
                ExtBindings::defaults()
            });
        let extbinds = CompiledExtBindings::from_config(&ext_cfg);
        let keymap = Keymap::load(&paths.keymap()).unwrap_or_else(|e| {
            tracing::warn!("keymap: load failed: {e}");
            Keymap::default()
        });
        let skin = SkinFile::load(&paths.skin()).unwrap_or_else(|e| {
            tracing::warn!("skin: load failed: {e}");
            SkinFile::default()
        });
        let (scheme, scheme_warnings) = skin.resolve();
        for w in scheme_warnings {
            tracing::warn!("{w}");
        }
        let cmd_history = History::load(paths.config_dir.join("cmd_history"), 100);
        let highlight = FileHighlight::load(&paths.filehighlight()).unwrap_or_else(|e| {
            tracing::warn!("filehighlight: load failed: {e}");
            FileHighlight::defaults()
        });
        let app = Self {
            config,
            registry,
            left,
            right,
            active_left: true,
            jobs,
            job_log: std::collections::VecDeque::with_capacity(64),
            highlight,
            hotlist,
            paths,
            extbinds,
            keymap,
            skin,
            scheme,
            cmd_history,
            status_msg: None,
            menubar: MenuBar::new(),
            modal: Modal::None,
            layout: LayoutSnapshot::default(),
            last_click: None,
        };
        (app, rx)
    }

    pub(super) fn save_hotlist(&self) {
        let path = self.paths.config_dir.join("hotlist.toml");
        if let Err(e) = self.hotlist.save(&path) {
            tracing::warn!("save hotlist {}: {e}", path.display());
        }
    }

    /// Set the active modal to a progress dialog tracking the given handle.
    pub fn show_progress(&mut self, handle: crate::jobs::JobHandle, description: String) {
        self.modal = Modal::Progress(ProgressDialog::new(handle, description));
    }

    /// Replace the modal with a streaming Find results dialog.
    pub fn show_find_results(&mut self, summary: String) {
        self.modal = Modal::FindResults(FindResults::new(summary));
    }

    /// Append a result to the active find dialog. Silently no-ops otherwise.
    pub fn find_push(&mut self, p: VPath) {
        if let Modal::FindResults(r) = &mut self.modal {
            r.push(p);
        }
    }

    pub fn find_set_status(&mut self, s: impl Into<String>) {
        if let Modal::FindResults(r) = &mut self.modal {
            r.set_status(s);
        }
    }

    pub fn find_finish(&mut self) {
        if let Modal::FindResults(r) = &mut self.modal {
            r.finish();
        }
    }

    /// Snapshot of the items currently held by the active FindResults modal.
    /// Empty when no Find is active or the modal is something else.
    #[must_use]
    pub fn find_results_items(&self) -> Vec<VPath> {
        if let Modal::FindResults(r) = &self.modal {
            r.items.clone()
        } else {
            Vec::new()
        }
    }

    #[must_use]
    pub fn modal_is_find_results(&self) -> bool {
        matches!(self.modal, Modal::FindResults(_))
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    pub fn handle_job_update(&mut self, update: crate::jobs::JobUpdate) {
        let row = self.job_log.iter_mut().find(|r| r.id == update.id);
        match (&update.kind, row) {
            (crate::jobs::JobUpdateKind::Started { description }, None) => {
                if self.job_log.len() >= 64 {
                    self.job_log.pop_front();
                }
                self.job_log.push_back(JobLogEntry {
                    id: update.id,
                    description: description.clone(),
                    status: "started".into(),
                    finished: None,
                });
            }
            (crate::jobs::JobUpdateKind::Started { description }, Some(r)) => {
                r.description = description.clone();
            }
            (crate::jobs::JobUpdateKind::Progress(_), Some(_)) => {}
            (crate::jobs::JobUpdateKind::Status(s), Some(r)) => {
                r.status = s.clone();
            }
            (crate::jobs::JobUpdateKind::Finished(o), Some(r)) => {
                r.finished = Some(o.clone());
                r.status = match o {
                    crate::jobs::JobOutcome::Success => "done".into(),
                    crate::jobs::JobOutcome::Cancelled => "cancelled".into(),
                    crate::jobs::JobOutcome::Failed(e) => format!("failed: {e}"),
                };
            }
            _ => {}
        }

        // Mirror live updates onto the progress dialog if it's the one in
        // front and refers to the same job.
        let progress_active_for_job = matches!(
            &self.modal,
            Modal::Progress(dlg) if dlg.handle.id == update.id
        );
        if progress_active_for_job {
            if let Modal::Progress(dlg) = &mut self.modal {
                match &update.kind {
                    crate::jobs::JobUpdateKind::Started { description } => {
                        dlg.description = description.clone();
                    }
                    crate::jobs::JobUpdateKind::Progress(p) => dlg.progress = *p,
                    crate::jobs::JobUpdateKind::Status(s) => dlg.status = s.clone(),
                    crate::jobs::JobUpdateKind::Log(_)
                    | crate::jobs::JobUpdateKind::Finished(_) => {}
                }
            }
        }

        // Resolve the description for terminal outcomes from the job log
        // (it was stored on `Started` earlier in this same call).
        if let crate::jobs::JobUpdateKind::Finished(o) = &update.kind {
            let description = self
                .job_log
                .iter()
                .find(|r| r.id == update.id)
                .map_or_else(|| "operation".to_string(), |r| r.description.clone());
            match o {
                crate::jobs::JobOutcome::Success => {
                    if progress_active_for_job {
                        self.modal = Modal::None;
                    }
                }
                crate::jobs::JobOutcome::Cancelled => {
                    if progress_active_for_job {
                        self.modal = Modal::None;
                    }
                    self.set_status(format!("{description}: cancelled"));
                }
                crate::jobs::JobOutcome::Failed(e) => {
                    // Always surface the failure in the status bar — that
                    // line stays visible even if the error modal is dismissed
                    // immediately by a queued keypress, and acts as a safety
                    // net for races we may not have spotted.
                    self.set_status(format!("{description} failed: {e}"));
                    // Replace the progress modal (if any) with the error
                    // dialog so the user must acknowledge the failure. If
                    // the user is in the middle of an unrelated modal, the
                    // status-bar message above is still visible and the
                    // failure remains in the job log (Ctrl-J).
                    let can_show_modal = matches!(self.modal, Modal::None | Modal::Progress(_));
                    if can_show_modal {
                        let message = format!("{description}\n\n{e}");
                        self.modal =
                            Modal::Error(crate::tui::dialog::ErrorDialog::new(" Error ", message));
                    }
                }
            }
        }
    }

    pub(super) fn active(&mut self) -> &mut PanelState {
        if self.active_left {
            &mut self.left
        } else {
            &mut self.right
        }
    }

    pub(super) fn active_ref(&self) -> &PanelState {
        if self.active_left {
            &self.left
        } else {
            &self.right
        }
    }

    /// Open a password modal for `(scheme, location)` if no other modal is
    /// currently displayed; the user then submits and the loop fires
    /// `RetryRemoteWithPassword` to retry the connection.
    pub fn prompt_password(&mut self, scheme: String, location: String) {
        if !matches!(self.modal, Modal::None) {
            return;
        }
        let prompt = format!("password for {scheme}://{location}:");
        self.modal = Modal::Password {
            dlg: crate::tui::dialog::PasswordDialog::new(" Authenticate ", prompt),
            scheme,
            location,
        };
    }

    /// Open a confirm modal for an unknown SSH host fingerprint. On Yes the
    /// fingerprint is recorded and the connection retried.
    pub fn prompt_host_key_confirm(
        &mut self,
        scheme: String,
        location: String,
        algorithm: String,
        fingerprint: String,
    ) {
        if !matches!(self.modal, Modal::None) {
            return;
        }
        let message = format!(
            "Unknown host {location}.\n{algorithm} fingerprint:\n{fingerprint}\n\nTrust and continue?"
        );
        self.modal = Modal::HostKeyConfirm {
            dlg: ConfirmDialog::new(" Unknown host key ", message),
            scheme,
            location,
            algorithm,
            fingerprint,
        };
    }

    /// If the active panel's cwd has a remote scheme (`sftp`/`ftp`/`dav`)
    /// whose mount is not yet registered, connect and register it. Any auth
    /// or connection failure logs a warning and is treated as if no backend
    /// exists (panel will simply show empty).
    pub async fn ensure_remote_mount(&mut self) {
        let mut needed: Vec<(String, String, VPath)> = Vec::new();
        for panel in [&self.left, &self.right] {
            for layer in panel.cwd.layers() {
                if matches!(layer.scheme.as_str(), "sftp" | "ftp" | "dav") {
                    needed.push((
                        layer.scheme.clone(),
                        layer.location.clone(),
                        panel.cwd.clone(),
                    ));
                }
            }
        }
        for (scheme, location, sample) in needed {
            if self.registry.root_for(&sample).is_ok() {
                continue;
            }
            match scheme.as_str() {
                "sftp" => {
                    let endpoint = match crate::vfs_net::sftp::SftpEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("sftp endpoint parse: {e}");
                            continue;
                        }
                    };
                    match crate::vfs_net::SftpVfs::connect("sftp", endpoint).await {
                        Ok(vfs) => {
                            self.registry.register_mount(
                                "sftp",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                        }
                        Err(crate::core::Error::HostKeyUnknown {
                            host_port: _,
                            algorithm,
                            fingerprint,
                        }) => {
                            self.prompt_host_key_confirm(
                                "sftp".into(),
                                location,
                                algorithm,
                                fingerprint,
                            );
                            return;
                        }
                        Err(e) => {
                            tracing::warn!("sftp connect {location}: {e}");
                            self.prompt_password("sftp".into(), location);
                            return;
                        }
                    }
                }
                "ftp" => {
                    let endpoint = match crate::vfs_net::ftp::FtpEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("ftp endpoint parse: {e}");
                            continue;
                        }
                    };
                    match crate::vfs_net::FtpVfs::connect("ftp", endpoint).await {
                        Ok(vfs) => {
                            self.registry.register_mount(
                                "ftp",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                        }
                        Err(e) => {
                            tracing::warn!("ftp connect {location}: {e}");
                            self.prompt_password("ftp".into(), location);
                            return;
                        }
                    }
                }
                "dav" => {
                    let endpoint = match crate::vfs_net::dav::DavEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("dav endpoint parse: {e}");
                            continue;
                        }
                    };
                    match crate::vfs_net::DavVfs::open("dav", endpoint) {
                        Ok(vfs) => {
                            self.registry.register_mount(
                                "dav",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                        }
                        Err(e) => {
                            tracing::warn!("dav open {location}: {e}");
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Forward a bracketed-paste payload to the focused text-input modal, if
    /// any. Multiline pastes are flattened to single lines (newlines → spaces).
    pub fn handle_paste(&mut self, text: String) {
        let text = text.replace(['\r', '\n'], " ");
        if text.is_empty() {
            return;
        }
        for c in text.chars() {
            if c == '\t' {
                continue;
            }
            let chord = KeyChord {
                code: KeyCode::Char(c),
                mods: KeyMods::empty(),
            };
            if matches!(self.modal, Modal::None) {
                return;
            }
            let _ = self.handle_modal_key(chord);
        }
    }

    /// Show a transient status message for ~3 seconds (overlays the buttonbar).
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status_msg = Some((text.into(), Instant::now() + Duration::from_secs(3)));
    }

    pub(super) fn current_status(&self) -> Option<&str> {
        match &self.status_msg {
            Some((s, deadline)) if Instant::now() < *deadline => Some(s.as_str()),
            _ => None,
        }
    }

    /// Build (or rebuild) tree-mode nodes for the active panel: cwd plus its
    /// immediate subdirs at depth 1. Subsequent expansions go through
    /// [`tree_toggle`].
    pub async fn rebuild_tree(&mut self) {
        let cwd = self.active_ref().cwd.clone();
        let vfs = match self.registry.root_for(&cwd) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut nodes = Vec::new();
        nodes.push(crate::tui::panel::TreeNode {
            name: cwd
                .last()
                .map(|l| l.sub.display().to_string())
                .unwrap_or_else(|| ".".into()),
            depth: 0,
            expanded: true,
            path: cwd.clone(),
            has_children: true,
        });
        if let Ok(entries) = vfs.read_dir(&cwd).await {
            let mut dirs: Vec<&crate::core::Entry> = entries
                .iter()
                .filter(|e| e.is_dir() && e.name != "..")
                .collect();
            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            for d in dirs {
                if let Some(child) = VPath::child(&cwd, &d.name) {
                    nodes.push(crate::tui::panel::TreeNode {
                        name: d.name.clone(),
                        depth: 1,
                        expanded: false,
                        path: child,
                        has_children: true,
                    });
                }
            }
        }
        let panel = if self.active_left {
            &mut self.left
        } else {
            &mut self.right
        };
        panel.tree.nodes = nodes;
        panel.tree.cursor = 0;
    }

    /// Toggle expansion of the tree-cursor node.
    pub async fn tree_toggle(&mut self) {
        let (cursor, depth, path, expanded) = {
            let p = self.active_ref();
            match p.tree.nodes.get(p.tree.cursor) {
                Some(n) if n.has_children => (p.tree.cursor, n.depth, n.path.clone(), n.expanded),
                _ => return,
            }
        };
        if expanded {
            let panel = if self.active_left {
                &mut self.left
            } else {
                &mut self.right
            };
            let mut end = cursor + 1;
            while end < panel.tree.nodes.len() && panel.tree.nodes[end].depth > depth {
                end += 1;
            }
            panel.tree.nodes.drain((cursor + 1)..end);
            panel.tree.nodes[cursor].expanded = false;
        } else {
            let vfs = match self.registry.root_for(&path) {
                Ok(v) => v,
                Err(_) => return,
            };
            let mut children: Vec<crate::tui::panel::TreeNode> = Vec::new();
            if let Ok(entries) = vfs.read_dir(&path).await {
                let mut dirs: Vec<&crate::core::Entry> = entries
                    .iter()
                    .filter(|e| e.is_dir() && e.name != "..")
                    .collect();
                dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                for d in dirs {
                    if let Some(c) = VPath::child(&path, &d.name) {
                        children.push(crate::tui::panel::TreeNode {
                            name: d.name.clone(),
                            depth: depth + 1,
                            expanded: false,
                            path: c,
                            has_children: true,
                        });
                    }
                }
            }
            let panel = if self.active_left {
                &mut self.left
            } else {
                &mut self.right
            };
            for (i, c) in children.into_iter().enumerate() {
                panel.tree.nodes.insert(cursor + 1 + i, c);
            }
            panel.tree.nodes[cursor].expanded = true;
        }
    }

    pub async fn refresh_panel(&mut self, left: bool) {
        let panel = if left {
            &mut self.left
        } else {
            &mut self.right
        };
        if panel.is_virtual_panelized {
            return;
        }
        let cwd = panel.cwd.clone();
        match self.registry.root_for(&cwd) {
            Ok(vfs) => match read_dir_with_parent(vfs.as_ref(), &cwd).await {
                Ok(entries) => {
                    let panel = if left {
                        &mut self.left
                    } else {
                        &mut self.right
                    };
                    panel.entries = entries;
                    panel.apply_filter_sort();
                }
                Err(e) => {
                    warn!("read_dir failed: {e}");
                    let panel = if left {
                        &mut self.left
                    } else {
                        &mut self.right
                    };
                    panel.entries.clear();
                }
            },
            Err(e) => warn!("no backend for {}: {e}", cwd),
        }
        // Optional: refresh git status overlay for this panel.
        let want_git = self.config.options.git_status;
        let local = ops::vpath_to_local(&cwd);
        let map = if want_git {
            match local {
                Some(p) => crate::tui::git::status_for_cwd(&p).await,
                None => None,
            }
        } else {
            None
        };
        let panel = if left {
            &mut self.left
        } else {
            &mut self.right
        };
        panel.git_status = map;
    }

    pub async fn refresh_both(&mut self) {
        self.refresh_panel(true).await;
        self.refresh_panel(false).await;
    }

    pub async fn refresh_active(&mut self) {
        self.refresh_panel(self.active_left).await;
    }

    pub fn handle_key(&mut self, chord: KeyChord) -> Disposition {
        // Apply user remap before dispatch. Modal text-input dialogs receive
        // the remapped chord too so users can rebind e.g. C-d → F8 globally.
        let chord = self.keymap.translate(chord);
        if !matches!(self.modal, Modal::None) {
            return self.handle_modal_key(chord);
        }
        self.handle_panel_key(chord)
    }

    pub(super) fn selected_targets(&self) -> Vec<VPath> {
        let p = self.active_ref();
        let mut out: Vec<VPath> = Vec::new();
        if !p.marks.is_empty() {
            for e in &p.entries {
                if p.marks.contains(&e.name) {
                    if let Some(child) = VPath::child(&p.cwd, &e.name) {
                        out.push(child);
                    }
                }
            }
        } else if let Some(child) = p.cursor_path() {
            if let Some(e) = p.entries.get(p.cursor) {
                if e.name != ".." {
                    out.push(child);
                }
            }
        }
        out
    }
}

async fn read_dir_with_parent(vfs: &dyn Vfs, p: &VPath) -> crate::core::Result<Vec<Entry>> {
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
    use super::ops::{
        dst_input_is_dir, ensure_trailing_slash, join_with_slash, parent_path, parse_chown,
        parse_dst,
    };
    use crate::core::VPath;

    #[test]
    fn parse_chown_numeric_pair() {
        assert_eq!(parse_chown("1000:2000"), Some((Some(1000), Some(2000))));
        assert_eq!(parse_chown("1000"), Some((Some(1000), None)));
        assert_eq!(parse_chown(":2000"), Some((None, Some(2000))));
        assert_eq!(parse_chown(""), None);
        assert_eq!(parse_chown("   "), None);
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
        let c = VPath::child(&p, "c").unwrap();
        let expected = format!("/a/b{}c", std::path::MAIN_SEPARATOR);
        assert_eq!(c.last().unwrap().sub.to_str().unwrap(), expected);
    }

    #[test]
    fn dst_input_trailing_slash_detected() {
        assert!(dst_input_is_dir("/foo/"));
        assert!(dst_input_is_dir("/foo/  "));
        assert!(dst_input_is_dir("foo/"));
        assert!(!dst_input_is_dir("/foo/bar"));
        assert!(!dst_input_is_dir("foo"));
        assert!(!dst_input_is_dir(""));
    }

    #[test]
    fn join_with_slash_handles_trailing() {
        assert_eq!(join_with_slash("/a/b", "c"), "/a/b/c");
        assert_eq!(join_with_slash("/a/b/", "c"), "/a/b/c");
        assert_eq!(join_with_slash("", "c"), "c");
    }

    #[test]
    fn ensure_trailing_slash_idempotent() {
        assert_eq!(ensure_trailing_slash("/x".into()), "/x/");
        assert_eq!(ensure_trailing_slash("/x/".into()), "/x/");
    }

    #[cfg(unix)]
    #[test]
    fn parse_dst_relative_under_local_cwd() {
        let cwd = VPath::local("/home/u");
        let p = parse_dst("foo.txt", &cwd).unwrap();
        assert_eq!(p.last().unwrap().sub.to_str().unwrap(), "/home/u/foo.txt");
    }

    #[cfg(unix)]
    #[test]
    fn parse_dst_absolute_local() {
        let cwd = VPath::local("/home/u");
        let p = parse_dst("/etc/hosts", &cwd).unwrap();
        assert_eq!(p.last().unwrap().sub.to_str().unwrap(), "/etc/hosts");
    }

    #[test]
    fn parse_dst_empty_is_none() {
        let cwd = VPath::local("/home/u");
        assert!(parse_dst("", &cwd).is_none());
        assert!(parse_dst("   ", &cwd).is_none());
    }

    #[test]
    fn failed_job_outcome_opens_error_modal() {
        use crate::config::AppConfig;
        use crate::jobs::{JobId, JobOutcome, JobUpdate, JobUpdateKind};
        use std::path::PathBuf;
        let (mut app, _rx) = super::App::new(AppConfig::default(), PathBuf::from("/tmp"));
        let id = JobId::next();
        app.handle_job_update(JobUpdate {
            id,
            kind: JobUpdateKind::Started {
                description: "Copy foo".into(),
            },
        });
        app.handle_job_update(JobUpdate {
            id,
            kind: JobUpdateKind::Finished(JobOutcome::Failed("permission denied".into())),
        });
        assert!(
            matches!(app.modal, super::Modal::Error(_)),
            "expected Modal::Error after JobOutcome::Failed",
        );
    }

    #[test]
    fn failed_job_outcome_replaces_progress_modal_with_error() {
        use crate::config::AppConfig;
        use crate::jobs::{JobHandle, JobId, JobOutcome, JobUpdate, JobUpdateKind};
        use std::path::PathBuf;
        use tokio_util::sync::CancellationToken;
        let (mut app, _rx) = super::App::new(AppConfig::default(), PathBuf::from("/tmp"));
        let id = JobId::next();
        app.handle_job_update(JobUpdate {
            id,
            kind: JobUpdateKind::Started {
                description: "Copy foo".into(),
            },
        });
        // Simulate the 250 ms-delayed Progress dialog being promoted to active.
        let handle = JobHandle {
            id,
            cancel: CancellationToken::new(),
        };
        app.show_progress(handle, "Copy foo".into());
        assert!(matches!(app.modal, super::Modal::Progress(_)));
        app.handle_job_update(JobUpdate {
            id,
            kind: JobUpdateKind::Finished(JobOutcome::Failed("permission denied".into())),
        });
        assert!(
            matches!(app.modal, super::Modal::Error(_)),
            "expected Modal::Error to replace Modal::Progress on Failed",
        );
    }
}
