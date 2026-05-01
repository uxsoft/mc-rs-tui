use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mc_config::{
    AppConfig, ColorScheme, CompiledExtBindings, ConfigPaths, ExtAction, ExtBindings,
    FileHighlight, History, Hotlist, Keymap, SkinFile,
};
use mc_core::action::SortKey;
use mc_core::key::{KeyChord, KeyCode, KeyMods};
use mc_core::{Entry, EntryKind, VPath};
use mc_jobs::{JobQueue, JobUpdateRx};
use mc_vfs::{Registry, Vfs};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tracing::warn;

use crate::theme::rtc;

use crate::dialog::{
    ConfirmDialog, Dialog, DialogOutcome, FindForm, FindFormOutcome, FindParams, FindResults,
    FindResultsOutcome, HotlistAction, HotlistDialog, InputDialog, MenuBar, MenuChoice,
    ProgressDialog,
};
use crate::panel::{render_panel, ListingMode, PanelState};

#[derive(Debug, Clone)]
pub struct JobLogEntry {
    pub id: mc_jobs::JobId,
    pub description: String,
    pub status: String,
    pub finished: Option<mc_jobs::JobOutcome>,
}

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
    /// Start a Find over the active panel's cwd with the given form params.
    StartFind { start: VPath, params: FindParams },
    /// Run a shell command line with terminal suspend/restore. `cwd` is the
    /// directory the command runs in; `cmd` already has `%`-macros expanded.
    RunShell { cwd: PathBuf, cmd: String },
    /// Suspend the TUI, drop the user into `$SHELL` interactive in `cwd`,
    /// restore on exit. Phase 13 first cut (no cwd-sync via PROMPT_COMMAND).
    DropToShell { cwd: PathBuf },
    /// User submitted a password for a remote-VFS retry.
    RetryRemoteWithPassword {
        scheme: String,
        location: String,
        password: String,
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
    FindForm(FindForm),
    FindResults(FindResults),
    CmdLine(InputDialog),
    UserMenu(crate::dialog::UserMenuDialog),
    Diff(crate::diff_widget::DiffWidget),
    Help(crate::dialog::HelpDialog),
    QuickCd(InputDialog),
    LearnKeys(crate::dialog::LearnKeysDialog),
    JobsView(crate::dialog::JobsViewDialog),
    /// Awaiting a password to retry auth for `(scheme, location, user)`.
    Password {
        dlg: crate::dialog::PasswordDialog,
        scheme: String,
        location: String,
    },
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
    status_msg: Option<(String, Instant)>,
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
        let ext_cfg = ExtBindings::load(&paths.config_dir.join("extbind.toml"))
            .unwrap_or_else(|e| {
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
        let app = Self {
            config,
            registry,
            left,
            right,
            active_left: true,
            jobs,
            job_log: std::collections::VecDeque::with_capacity(64),
            highlight: FileHighlight::defaults(),
            hotlist,
            paths,
            extbinds,
            keymap,
            skin,
            scheme,
            cmd_history,
            status_msg: None,
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

    #[must_use]
    pub fn modal_is_find_results(&self) -> bool {
        matches!(self.modal, Modal::FindResults(_))
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    pub fn handle_job_update(&mut self, update: mc_jobs::JobUpdate) {
        // Update / append the job-log row.
        let row = self
            .job_log
            .iter_mut()
            .find(|r| r.id == update.id);
        match (&update.kind, row) {
            (mc_jobs::JobUpdateKind::Started { description }, None) => {
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
            (mc_jobs::JobUpdateKind::Started { description }, Some(r)) => {
                r.description = description.clone();
            }
            (mc_jobs::JobUpdateKind::Progress(_), Some(_)) => {}
            (mc_jobs::JobUpdateKind::Status(s), Some(r)) => {
                r.status = s.clone();
            }
            (mc_jobs::JobUpdateKind::Finished(o), Some(r)) => {
                r.finished = Some(o.clone());
                r.status = match o {
                    mc_jobs::JobOutcome::Success => "done".into(),
                    mc_jobs::JobOutcome::Cancelled => "cancelled".into(),
                    mc_jobs::JobOutcome::Failed(e) => format!("failed: {e}"),
                };
            }
            _ => {}
        }

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

    /// If the active panel's cwd has a remote scheme (`sftp`) whose mount is
    /// not yet registered, connect and register it. Any auth / connection
    /// failure logs a warning and is treated as if no backend exists (panel
    /// will simply show empty).
    /// Open a password modal for `(scheme, location)` if no other modal is
    /// currently displayed; the user then submits and the loop fires
    /// `RetryRemoteWithPassword` to retry the connection.
    pub fn prompt_password(&mut self, scheme: String, location: String) {
        if !matches!(self.modal, Modal::None) {
            return;
        }
        let prompt = format!("password for {scheme}://{location}:");
        self.modal = Modal::Password {
            dlg: crate::dialog::PasswordDialog::new(" Authenticate ", prompt),
            scheme,
            location,
        };
    }

    pub async fn ensure_remote_mount(&mut self) {
        // Snapshot every (scheme, location, sample-vpath) triple that needs a
        // backend; mutate registry/modal afterwards (no borrows during await).
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
                    let endpoint = match mc_vfs_net::sftp::SftpEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("sftp endpoint parse: {e}");
                            continue;
                        }
                    };
                    match mc_vfs_net::SftpVfs::connect("sftp", endpoint).await {
                        Ok(vfs) => {
                            self.registry.register_mount(
                                "sftp",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                        }
                        Err(e) => {
                            tracing::warn!("sftp connect {location}: {e}");
                            self.prompt_password("sftp".into(), location);
                            return;
                        }
                    }
                }
                "ftp" => {
                    let endpoint = match mc_vfs_net::ftp::FtpEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("ftp endpoint parse: {e}");
                            continue;
                        }
                    };
                    match mc_vfs_net::FtpVfs::connect("ftp", endpoint).await {
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
                    let endpoint = match mc_vfs_net::dav::DavEndpoint::parse(&location) {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!("dav endpoint parse: {e}");
                            continue;
                        }
                    };
                    match mc_vfs_net::DavVfs::open("dav", endpoint) {
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
        // The cleanest way to get characters into our modals' InputDialog is
        // to synthesize Char chords. This is uniform across CmdLine/Mkdir/
        // Rename/SelectGroup/Chmod/QuickCd/FindForm/QuickSearch.
        for c in text.chars() {
            if c == '\t' {
                continue;
            }
            let chord = mc_core::key::KeyChord {
                code: mc_core::key::KeyCode::Char(c),
                mods: mc_core::key::KeyMods::empty(),
            };
            // Only route if a modal is active; otherwise pastes are dropped on
            // the floor (panel mode doesn't have a text-input target).
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

    fn current_status(&self) -> Option<&str> {
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
        nodes.push(crate::panel::TreeNode {
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
            let mut dirs: Vec<&mc_core::Entry> = entries.iter().filter(|e| e.is_dir() && e.name != "..").collect();
            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            for d in dirs {
                if let Some(child) = child_path(&cwd, &d.name) {
                    nodes.push(crate::panel::TreeNode {
                        name: d.name.clone(),
                        depth: 1,
                        expanded: false,
                        path: child,
                        has_children: true,
                    });
                }
            }
        }
        let panel = if self.active_left { &mut self.left } else { &mut self.right };
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
            let panel = if self.active_left { &mut self.left } else { &mut self.right };
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
            let mut children: Vec<crate::panel::TreeNode> = Vec::new();
            if let Ok(entries) = vfs.read_dir(&path).await {
                let mut dirs: Vec<&mc_core::Entry> = entries.iter().filter(|e| e.is_dir() && e.name != "..").collect();
                dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                for d in dirs {
                    if let Some(c) = child_path(&path, &d.name) {
                        children.push(crate::panel::TreeNode {
                            name: d.name.clone(),
                            depth: depth + 1,
                            expanded: false,
                            path: c,
                            has_children: true,
                        });
                    }
                }
            }
            let panel = if self.active_left { &mut self.left } else { &mut self.right };
            for (i, c) in children.into_iter().enumerate() {
                panel.tree.nodes.insert(cursor + 1 + i, c);
            }
            panel.tree.nodes[cursor].expanded = true;
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
        // Apply user remap before dispatch. Modal text-input dialogs receive
        // the remapped chord too so users can rebind e.g. C-d → F8 globally.
        let chord = self.keymap.translate(chord);
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
            Modal::CmdLine(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::CmdLine(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(raw) => {
                    self.cmd_history.push(raw.clone());
                    let ctx = self.macro_ctx();
                    let cmd = mc_core::substitute(&raw, &ctx);
                    let cwd = self
                        .active_ref()
                        .cwd
                        .last()
                        .map(|l| l.sub.clone())
                        .unwrap_or_else(|| PathBuf::from("."));
                    Disposition::RunOp(PendingOp::RunShell { cwd, cmd })
                }
            },
            Modal::UserMenu(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::UserMenu(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(template) => {
                    let ctx = self.macro_ctx();
                    let cmd = mc_core::substitute(&template, &ctx);
                    let cwd = self
                        .active_ref()
                        .cwd
                        .last()
                        .map(|l| l.sub.clone())
                        .unwrap_or_else(|| PathBuf::from("."));
                    Disposition::RunOp(PendingOp::RunShell { cwd, cmd })
                }
            },
            Modal::FindForm(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::FindForm(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(FindFormOutcome::Cancel) => Disposition::Redraw,
                DialogOutcome::Submitted(FindFormOutcome::Start(params)) => {
                    let start = self.active_ref().cwd.clone();
                    Disposition::RunOp(PendingOp::StartFind { start, params })
                }
            },
            Modal::FindResults(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::FindResults(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(FindResultsOutcome::Navigate(p)) => {
                    // If `p` is a file, navigate to its parent dir and place
                    // cursor on it on the next refresh (placement is best-effort
                    // — we just cd to parent for now).
                    let target = parent_path(&p).unwrap_or(p);
                    self.active().navigate(target);
                    Disposition::ReloadActive
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
            Modal::Diff(mut d) => {
                if d.handle_key(chord) {
                    self.modal = Modal::Diff(d);
                    Disposition::Redraw
                } else {
                    Disposition::Redraw
                }
            }
            Modal::Help(mut d) => match d.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Help(d);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::LearnKeys(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::LearnKeys(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::JobsView(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::JobsView(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::Password { mut dlg, scheme, location } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Password { dlg, scheme, location };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(pw) => {
                    Disposition::RunOp(PendingOp::RetryRemoteWithPassword {
                        scheme,
                        location,
                        password: pw,
                    })
                }
            },
            Modal::QuickCd(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::QuickCd(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(s) => {
                    let s = s.trim();
                    let vp = if s.contains("://") || s.starts_with("local:") {
                        s.parse::<VPath>().ok()
                    } else if s.starts_with('/') || s.starts_with('~') {
                        // Treat as a local path; expand a leading ~ for convenience.
                        let expanded = if let Some(rest) = s.strip_prefix("~") {
                            let home = std::env::var_os("HOME")
                                .map(PathBuf::from)
                                .unwrap_or_else(|| PathBuf::from("/"));
                            home.join(rest.trim_start_matches('/'))
                        } else {
                            PathBuf::from(s)
                        };
                        Some(VPath::local(expanded))
                    } else {
                        // Relative: resolve against current cwd if local.
                        if let Some(layer) = self.active_ref().cwd.last() {
                            if layer.scheme == "local" {
                                Some(VPath::local(layer.sub.join(s)))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(target) = vp {
                        self.active().navigate(target);
                        Disposition::ReloadActive
                    } else {
                        tracing::warn!("quick cd: could not parse {s:?}");
                        Disposition::Redraw
                    }
                }
            },
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
            mc_vfs_archive::ArchiveKind::Cpio => "cpio",
            mc_vfs_archive::ArchiveKind::SevenZ => "7z",
            #[cfg(feature = "rar")]
            mc_vfs_archive::ArchiveKind::Rar => "rar",
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

    /// Open diff between the active panel's cursor file and the other panel's
    /// cursor file. Both must be local regular files for this initial cut.
    fn open_diff(&mut self) -> Disposition {
        let active = self.active_ref();
        let other = if self.active_left { &self.right } else { &self.left };
        let lp = match active.cursor_path() {
            Some(p) => p,
            None => return Disposition::Redraw,
        };
        let rp = {
            // Use other panel's cursor.
            let layer = match other.cwd.last() {
                Some(l) => l.clone(),
                None => return Disposition::Redraw,
            };
            let name = match other.entries.get(other.cursor) {
                Some(e) if e.name != ".." => e.name.clone(),
                _ => return Disposition::Redraw,
            };
            let mut sub = layer.sub.clone();
            sub.push(&name);
            let mut new_layer = layer;
            new_layer.sub = sub;
            let mut p = other.cwd.clone();
            p.pop_layer();
            p.push_layer(new_layer);
            p
        };
        let l_local = match vpath_to_local(&lp) {
            Some(p) => p,
            None => {
                tracing::warn!("diff: left path is not local");
                return Disposition::Redraw;
            }
        };
        let r_local = match vpath_to_local(&rp) {
            Some(p) => p,
            None => {
                tracing::warn!("diff: right path is not local");
                return Disposition::Redraw;
            }
        };
        match crate::diff_widget::DiffWidget::open(&l_local, &r_local) {
            Ok(d) => {
                self.modal = Modal::Diff(d);
                Disposition::Redraw
            }
            Err(e) => {
                tracing::warn!("diff open: {e}");
                Disposition::Redraw
            }
        }
    }

    /// Expand `template` against the active panel's macro context and dispatch
    /// a [`PendingOp::RunShell`] to execute it.
    fn run_template(&self, template: &str) -> Disposition {
        let ctx = self.macro_ctx();
        let cmd = mc_core::substitute(template, &ctx);
        let cwd = self
            .active_ref()
            .cwd
            .last()
            .map(|l| l.sub.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        Disposition::RunOp(PendingOp::RunShell { cwd, cmd })
    }

    fn macro_ctx(&self) -> mc_core::MacroCtx {
        let active = self.active_ref();
        let other = if self.active_left { &self.right } else { &self.left };
        let current = active
            .entries
            .get(active.cursor)
            .filter(|e| e.name != "..")
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let other_current = other
            .entries
            .get(other.cursor)
            .filter(|e| e.name != "..")
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let cwd = active
            .cwd
            .last()
            .map(|l| l.sub.display().to_string())
            .unwrap_or_default();
        let other_cwd = other
            .cwd
            .last()
            .map(|l| l.sub.display().to_string())
            .unwrap_or_default();
        mc_core::MacroCtx {
            cwd,
            current,
            marked: active.marks.iter().cloned().collect(),
            other_cwd,
            other_current,
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
                self.modal = Modal::FindForm(FindForm::new(FindParams::default()));
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

    fn compare_dirs(&mut self) {
        // Build maps from name → size for both panels (skipping ".." and dirs).
        let other = if self.active_left { &self.right } else { &self.left };
        let other_by_name: std::collections::HashMap<String, u64> = other
            .entries
            .iter()
            .filter(|e| e.name != ".." && !e.is_dir())
            .map(|e| (e.name.clone(), e.size))
            .collect();
        let active = if self.active_left { &mut self.left } else { &mut self.right };
        active.marks.clear();
        for e in &active.entries {
            if e.name == ".." || e.is_dir() {
                continue;
            }
            match other_by_name.get(&e.name) {
                Some(sz) if *sz == e.size => {} // identical
                _ => {
                    active.marks.insert(e.name.clone());
                }
            }
        }
    }

    fn handle_ctrl_x(&mut self, chord: KeyChord) -> Disposition {
        // mc Ctrl-X chords; we only implement a few here (more in Phase 11).
        match (chord.code, chord.mods) {
            (KeyCode::Char('='), m) if m.is_empty() => {
                self.compare_dirs();
                let n = if self.active_left { self.left.marks.len() } else { self.right.marks.len() };
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

    fn handle_tree_panel_key(&mut self, chord: KeyChord) -> Disposition {
        match (chord.code, chord.mods) {
            (KeyCode::F(10), m) if m.is_empty() => Disposition::Quit,
            (KeyCode::Char('q'), m) if m == KeyMods::CTRL => Disposition::Quit,
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
            // Switch back to Full listing.
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

    fn handle_panel_key(&mut self, chord: KeyChord) -> Disposition {
        // Tree-mode panel handles a smaller set of chords distinctly.
        if matches!(self.active_ref().mode, ListingMode::Tree) {
            return self.handle_tree_panel_key(chord);
        }
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
                        // 1) Archive auto-mount.
                        if let Some(t) = &target {
                            if let Some(mounted) = self.try_mount_archive(t) {
                                self.active().navigate(mounted);
                                return Disposition::ReloadActive;
                            }
                        }
                        // 2) Configured Open binding.
                        if let Some(template) = self.extbinds.lookup(&e.name, ExtAction::Open) {
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

            // Listing-mode cycle (mc Alt-T). When transitioning into Tree we
            // need an async rebuild — emit RebuildTree so the loop runs it.
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
            (KeyCode::F(9), m) if m.is_empty() => {
                self.modal = Modal::Menu(MenuBar::new());
                Disposition::Redraw
            }

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

            // Alt-C — Quick cd (typed path; supports local + sftp:// URLs)
            (KeyCode::Char('c'), m) if m == KeyMods::ALT => {
                self.modal = Modal::QuickCd(InputDialog::new(
                    " Quick cd ",
                    "Path or URL (e.g. /tmp, sftp://user@host/srv, ftp://anon@host/pub):",
                    String::new(),
                ));
                Disposition::Redraw
            }

            // F2 — User menu
            (KeyCode::F(2), m) if m.is_empty() => {
                self.modal = Modal::UserMenu(crate::dialog::UserMenuDialog::with_defaults());
                Disposition::Redraw
            }

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
                let entry = self.active_ref().entries.get(self.active_ref().cursor).cloned();
                let target = self.active_ref().cursor_path();
                if let (Some(e), Some(target)) = (entry, target) {
                    // Configured View binding takes precedence over the built-in viewer.
                    if let Some(template) = self.extbinds.lookup(&e.name, ExtAction::View) {
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

        render_panel(f, panels[0], &mut self.left, &self.highlight, &self.scheme);
        render_panel(f, panels[1], &mut self.right, &self.highlight, &self.scheme);
        if let Some(status) = self.current_status() {
            let p = Paragraph::new(Line::from(format!(" {status} "))).style(
                Style::default()
                    .fg(rtc(self.scheme.op_status_fg))
                    .bg(rtc(self.scheme.op_status_bg))
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );
            f.render_widget(p, chunks[1]);
        } else {
            render_buttonbar(f, chunks[1], &self.scheme);
        }

        let scheme = &self.scheme;
        match &mut self.modal {
            Modal::None | Modal::PrefixCtrlX => {}
            Modal::Mkdir(d) | Modal::Rename(d, _) => d.render(f, area, scheme),
            Modal::SelectGroup { dlg, .. } | Modal::Chmod { dlg, .. } => dlg.render(f, area, scheme),
            Modal::DeleteConfirm(d, _) => d.render(f, area, scheme),
            Modal::Viewer(v) => v.render(f, area, scheme),
            Modal::Progress(d) => d.render(f, area, scheme),
            Modal::Hotlist(d) => d.render(f, area, scheme),
            Modal::Menu(d) => d.render(f, area, scheme),
            Modal::FindForm(d) => d.render(f, area, scheme),
            Modal::FindResults(d) => d.render(f, area, scheme),
            Modal::CmdLine(d) => d.render(f, area, scheme),
            Modal::UserMenu(d) => d.render(f, area, scheme),
            Modal::Diff(d) => d.render(f, area, scheme),
            Modal::Help(d) => d.render(f, area, scheme),
            Modal::QuickCd(d) => d.render(f, area, scheme),
            Modal::LearnKeys(d) => d.render(f, area, scheme),
            Modal::JobsView(d) => d.render(f, area, scheme),
            Modal::Password { dlg, .. } => dlg.render(f, area, scheme),
            Modal::QuickSearch(filter) => render_quick_search(f, area, filter, scheme),
        }
        if matches!(self.modal, Modal::PrefixCtrlX) {
            let hint = Line::from(" C-x: c=chmod   (Esc to cancel) ");
            let p = Paragraph::new(hint)
                .style(Style::default().fg(rtc(self.scheme.op_status_fg)).bg(rtc(self.scheme.op_status_bg)));
            let rect = ratatui::layout::Rect::new(area.x, area.y + area.height.saturating_sub(2), area.width, 1);
            f.render_widget(p, rect);
        }
    }
}

fn render_quick_search(f: &mut Frame<'_>, area: ratatui::layout::Rect, filter: &str, scheme: &ColorScheme) {
    let line = Line::from(vec![
        Span::raw(" Search: "),
        Span::styled(
            filter.to_string(),
            Style::default()
                .fg(rtc(scheme.search_fg))
                .bg(rtc(scheme.search_bg))
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
    ]);
    let p = Paragraph::new(line).style(Style::default().fg(rtc(scheme.statusbar_fg)).bg(rtc(scheme.statusbar_bg)));
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

pub fn parent_path(p: &VPath) -> Option<VPath> {
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

fn render_buttonbar(f: &mut Frame<'_>, area: ratatui::layout::Rect, scheme: &ColorScheme) {
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
            Style::default().fg(rtc(scheme.buttonbar_fg)).bg(rtc(scheme.buttonbar_bg)),
        ));
        spans.push(Span::styled(
            *name,
            Style::default().fg(rtc(scheme.buttonbar_label_fg)).bg(rtc(scheme.buttonbar_label_bg)),
        ));
    }
    let line = Line::from(spans);
    let p = Paragraph::new(line).style(Style::default().bg(rtc(scheme.buttonbar_bg)));
    f.render_widget(p, area);
}
