//! Modal dispatch: per-modal key handling, menu-choice routing, and the
//! `open_menu_dialog` factory that constructs each `Modal` variant.

use std::path::PathBuf;

use crate::core::VPath;
use crate::core::key::{KeyChord, KeyCode, KeyMods};

use crate::tui::dialog::{
    Dialog, DialogOutcome, FindForm, FindFormOutcome, FindParams, FindResultsOutcome,
    HotlistAction, HotlistDialog, InputDialog, MenuChoice, MenuDialog,
};

use super::ops::{default_link_name, parse_chown, parse_octal_mode, vpath_to_local};
use super::{App, CopyMoveKind, Disposition, Modal, PendingOp};

impl App {
    pub(super) fn handle_modal_key(&mut self, chord: KeyChord) -> Disposition {
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
            Modal::CopyMove {
                mut dlg,
                sources,
                src_cwd,
                kind,
            } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::CopyMove {
                        dlg,
                        sources,
                        src_cwd,
                        kind,
                    };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(settings) => Disposition::RunOp(match kind {
                    CopyMoveKind::Copy => PendingOp::SubmitCopy {
                        sources,
                        dst_input: settings.dst,
                        src_cwd,
                        opts: settings.opts,
                    },
                    CopyMoveKind::Move => PendingOp::SubmitMove {
                        sources,
                        dst_input: settings.dst,
                        src_cwd,
                        opts: settings.opts,
                    },
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
                    let cmd = crate::core::substitute(&raw, &ctx);
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
                    let cmd = crate::core::substitute(&template, &ctx);
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
                    let target = super::ops::parent_path(&p).unwrap_or(p);
                    self.active().navigate(target);
                    Disposition::ReloadActive
                }
                DialogOutcome::Submitted(FindResultsOutcome::Panelize(items)) => {
                    self.panelize_active(items);
                    Disposition::Redraw
                }
            },
            Modal::Menu => match self.menubar.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Menu;
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
                DialogOutcome::Submitted(action) => match action {
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
                },
            },
            Modal::Progress(dlg) => match (chord.code, chord.mods) {
                (KeyCode::Char('c'), m) if m == KeyMods::CTRL => {
                    if dlg.finished.is_none() {
                        dlg.handle.cancel();
                    }
                    Disposition::Redraw
                }
                (KeyCode::Escape, _) | (KeyCode::Enter, _) => Disposition::Redraw,
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
            Modal::Password {
                mut dlg,
                scheme,
                location,
            } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Password {
                        dlg,
                        scheme,
                        location,
                    };
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
            Modal::HostKeyConfirm {
                mut dlg,
                scheme,
                location,
                algorithm,
                fingerprint,
            } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::HostKeyConfirm {
                        dlg,
                        scheme,
                        location,
                        algorithm,
                        fingerprint,
                    };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(false) => {
                    self.set_status(format!("rejected host key for {location}"));
                    Disposition::Redraw
                }
                DialogOutcome::Submitted(true) => {
                    Disposition::RunOp(PendingOp::AcceptHostKeyAndRetry {
                        scheme,
                        location,
                        algorithm,
                        fingerprint,
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
                        let expanded = if let Some(rest) = s.strip_prefix("~") {
                            let home = std::env::var_os("HOME")
                                .map(PathBuf::from)
                                .unwrap_or_else(|| PathBuf::from("/"));
                            home.join(rest.trim_start_matches('/'))
                        } else {
                            PathBuf::from(s)
                        };
                        Some(VPath::local(expanded))
                    } else if let Some(layer) = self.active_ref().cwd.last() {
                        if layer.scheme == "local" {
                            Some(VPath::local(layer.sub.join(s)))
                        } else {
                            None
                        }
                    } else {
                        None
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
            Modal::Chown { mut dlg, targets } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Chown { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(s) => match parse_chown(&s) {
                    Some((uid, gid)) => Disposition::RunOp(PendingOp::Chown { targets, uid, gid }),
                    None => {
                        self.set_status(format!("chown: cannot resolve {s:?}"));
                        Disposition::Redraw
                    }
                },
            },
            Modal::Chattr { mut dlg, targets } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Chattr { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(attrs) => {
                    let mut quoted = String::new();
                    for t in &targets {
                        if let Some(local) = vpath_to_local(t) {
                            if !quoted.is_empty() {
                                quoted.push(' ');
                            }
                            quoted.push('\'');
                            quoted.push_str(&local.to_string_lossy().replace('\'', "'\\''"));
                            quoted.push('\'');
                        }
                    }
                    if quoted.is_empty() {
                        self.set_status("chattr: local files only");
                        return Disposition::Redraw;
                    }
                    let cwd = self
                        .active_ref()
                        .cwd
                        .last()
                        .map(|l| l.sub.clone())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let cmd = format!("chattr {} -- {}", attrs, quoted);
                    Disposition::RunOp(PendingOp::RunShell { cwd, cmd })
                }
            },
            Modal::Hardlink { mut dlg, src } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Hardlink { dlg, src };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(link_str) => {
                    let Some(src_local) = vpath_to_local(&src) else {
                        self.set_status("hardlink: local files only");
                        return Disposition::Redraw;
                    };
                    Disposition::RunOp(PendingOp::Hardlink {
                        src: src_local,
                        link: PathBuf::from(link_str),
                    })
                }
            },
            Modal::Symlink {
                mut dlg,
                src,
                relative,
            } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Symlink { dlg, src, relative };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(link_str) => {
                    let Some(src_local) = vpath_to_local(&src) else {
                        self.set_status("symlink: local files only");
                        return Disposition::Redraw;
                    };
                    Disposition::RunOp(PendingOp::Symlink {
                        target: src_local,
                        link: PathBuf::from(link_str),
                        relative,
                    })
                }
            },
            Modal::EditSymlink { mut dlg, link } => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::EditSymlink { dlg, link };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(new_target) => {
                    Disposition::RunOp(PendingOp::EditSymlink {
                        link,
                        new_target: PathBuf::from(new_target),
                    })
                }
            },
            Modal::Filter(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Filter(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(s) => {
                    let s = s.trim().to_string();
                    self.active().filter = if s.is_empty() { None } else { Some(s) };
                    Disposition::ReloadActive
                }
            },
            Modal::VfsList(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::VfsList(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(crate::tui::dialog::VfsListAction::Free {
                    scheme,
                    location,
                }) => {
                    self.registry.unregister_mount(&scheme, &location);
                    let mounts = self.registry.mounts();
                    self.modal = Modal::VfsList(crate::tui::dialog::VfsListDialog::new(mounts));
                    Disposition::Redraw
                }
                DialogOutcome::Submitted(crate::tui::dialog::VfsListAction::Goto {
                    scheme,
                    location,
                }) => {
                    let url = format!("{scheme}://{location}");
                    if let Ok(p) = url.parse::<VPath>() {
                        self.active().navigate(p);
                        Disposition::ReloadActive
                    } else {
                        self.set_status(format!("cannot parse {url:?}"));
                        Disposition::Redraw
                    }
                }
            },
            Modal::ExternalPanelize(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::ExternalPanelize(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(cmd) => {
                    let cwd = self
                        .active_ref()
                        .cwd
                        .last()
                        .map(|l| l.sub.clone())
                        .unwrap_or_else(|| PathBuf::from("."));
                    Disposition::RunOp(PendingOp::ExternalPanelize { cwd, cmd })
                }
            },
            Modal::Configuration(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Configuration(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(()) => {
                    dlg.apply(&mut self.config);
                    self.left.show_hidden = self.config.panels.show_hidden;
                    self.right.show_hidden = self.config.panels.show_hidden;
                    self.left.mix_dirs = self.config.panels.mix_dirs;
                    self.right.mix_dirs = self.config.panels.mix_dirs;
                    Disposition::ReloadBoth
                }
            },
            Modal::Layout(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Layout(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(layout) => {
                    self.config.layout = layout;
                    Disposition::Redraw
                }
            },
            Modal::Theme(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Theme(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(name) => {
                    self.skin.theme = name;
                    let (scheme, warnings) = self.skin.resolve();
                    self.scheme = scheme;
                    for w in warnings {
                        self.set_status(w);
                    }
                    if let Err(e) = self.skin.save(&self.paths.skin()) {
                        self.set_status(format!("save skin: {e}"));
                    }
                    Disposition::Redraw
                }
            },
            Modal::Confirmation(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Confirmation(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(()) => {
                    dlg.apply(&mut self.config);
                    Disposition::Redraw
                }
            },
            Modal::VirtualFs(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::VirtualFs(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(()) => {
                    dlg.apply(&mut self.config);
                    Disposition::Redraw
                }
            },
            Modal::QuitConfirm(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::QuitConfirm(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(false) => Disposition::Redraw,
                DialogOutcome::Submitted(true) => Disposition::Quit,
            },
            Modal::Error(mut dlg) => match dlg.handle_key(chord) {
                DialogOutcome::None => {
                    self.modal = Modal::Error(dlg);
                    Disposition::None
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::QuickSearch(mut filter) => match (chord.code, chord.mods) {
                (KeyCode::Escape, _) | (KeyCode::Enter, _) => Disposition::Redraw,
                (KeyCode::Backspace, _) => {
                    filter.pop();
                    self.apply_quick_search(&filter);
                    self.modal = Modal::QuickSearch(filter);
                    Disposition::Redraw
                }
                (KeyCode::Down, _) | (KeyCode::Char('n'), KeyMods::CTRL) => {
                    self.apply_quick_search_next(&filter, 1);
                    self.modal = Modal::QuickSearch(filter);
                    Disposition::Redraw
                }
                (KeyCode::Up, _) | (KeyCode::Char('p'), KeyMods::CTRL) => {
                    self.apply_quick_search_next(&filter, -1);
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

    pub(super) fn handle_menu_choice(&mut self, c: MenuChoice) -> Disposition {
        match c {
            MenuChoice::Quit => self.maybe_confirm_quit(),
            MenuChoice::KeyChord(chord) => self.handle_panel_key(chord),
            MenuChoice::CtrlX(ch) => self.handle_ctrl_x(KeyChord::plain(KeyCode::Char(ch))),
            MenuChoice::Reread => Disposition::ReloadActive,
            MenuChoice::SwapPanels => {
                std::mem::swap(&mut self.left, &mut self.right);
                self.left.active = self.active_left;
                self.right.active = !self.active_left;
                Disposition::Redraw
            }
            MenuChoice::FocusThen { left, then } => {
                self.active_left = left;
                self.left.active = left;
                self.right.active = !left;
                self.handle_menu_choice(*then)
            }
            MenuChoice::Status(msg) => {
                self.set_status(msg);
                Disposition::Redraw
            }
            MenuChoice::OpenUserMenu => {
                self.modal = Modal::UserMenu(crate::tui::dialog::UserMenuDialog::with_defaults());
                Disposition::Redraw
            }
            MenuChoice::OpenDialog(d) => self.open_menu_dialog(d),
        }
    }

    /// Open a custom dialog for a menu item that doesn't map to a key chord.
    /// Each arm constructs the appropriate `Modal::*` variant; the dispatcher
    /// in `handle_modal_key` then drives the dialog and runs the resulting
    /// `PendingOp`.
    fn open_menu_dialog(&mut self, d: MenuDialog) -> Disposition {
        match d {
            MenuDialog::Chown => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Disposition::Redraw;
                }
                self.modal = Modal::Chown {
                    dlg: InputDialog::new(
                        " Chown ",
                        "user:group (e.g. me:me, or 1000:1000):",
                        String::new(),
                    ),
                    targets,
                };
                Disposition::Redraw
            }
            MenuDialog::Chattr => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Disposition::Redraw;
                }
                self.modal = Modal::Chattr {
                    dlg: InputDialog::new(" Chattr ", "Attrs (e.g. +i, -i, +a):", "+i"),
                    targets,
                };
                Disposition::Redraw
            }
            MenuDialog::Hardlink => {
                let Some(src) = self.active_ref().cursor_path().filter(|_| {
                    self.active_ref()
                        .entries
                        .get(self.active_ref().cursor)
                        .map(|e| e.name != "..")
                        .unwrap_or(false)
                }) else {
                    return Disposition::Redraw;
                };
                let default_link = default_link_name(&src);
                self.modal = Modal::Hardlink {
                    dlg: InputDialog::new(" Hardlink ", "Link path:", default_link),
                    src,
                };
                Disposition::Redraw
            }
            MenuDialog::Symlink { relative } => {
                let Some(src) = self.active_ref().cursor_path().filter(|_| {
                    self.active_ref()
                        .entries
                        .get(self.active_ref().cursor)
                        .map(|e| e.name != "..")
                        .unwrap_or(false)
                }) else {
                    return Disposition::Redraw;
                };
                let default_link = default_link_name(&src);
                let title = if relative {
                    " Relative symlink "
                } else {
                    " Symbolic link "
                };
                self.modal = Modal::Symlink {
                    dlg: InputDialog::new(title, "Link path:", default_link),
                    src,
                    relative,
                };
                Disposition::Redraw
            }
            MenuDialog::EditSymlink => {
                let Some(target) = self.active_ref().cursor_path() else {
                    return Disposition::Redraw;
                };
                let local = match vpath_to_local(&target) {
                    Some(p) => p,
                    None => {
                        self.set_status("edit symlink: local files only");
                        return Disposition::Redraw;
                    }
                };
                let current = match std::fs::read_link(&local) {
                    Ok(t) => t.to_string_lossy().into_owned(),
                    Err(_) => {
                        self.set_status("edit symlink: not a symlink");
                        return Disposition::Redraw;
                    }
                };
                self.modal = Modal::EditSymlink {
                    dlg: InputDialog::new(" Edit symlink ", "Symlink target:", current),
                    link: local,
                };
                Disposition::Redraw
            }
            MenuDialog::Filter => {
                let current = self.active_ref().filter.clone().unwrap_or_default();
                self.modal = Modal::Filter(InputDialog::new(
                    " Filter ",
                    "Glob (empty to clear):",
                    current,
                ));
                Disposition::Redraw
            }
            MenuDialog::FtpLink => {
                self.modal = Modal::QuickCd(InputDialog::new(
                    " FTP link ",
                    "URL (ftp://user@host/path):",
                    "ftp://".to_string(),
                ));
                Disposition::Redraw
            }
            MenuDialog::SftpLink => {
                self.modal = Modal::QuickCd(InputDialog::new(
                    " SFTP link ",
                    "URL (sftp://user@host/path):",
                    "sftp://".to_string(),
                ));
                Disposition::Redraw
            }
            MenuDialog::ShellLink => {
                self.set_status("shell link (sh://) not supported by this build");
                Disposition::Redraw
            }
            MenuDialog::ActiveVfsList => {
                let mounts = self.registry.mounts();
                if mounts.is_empty() {
                    self.set_status("no active VFS mounts");
                    return Disposition::Redraw;
                }
                self.modal = Modal::VfsList(crate::tui::dialog::VfsListDialog::new(mounts));
                Disposition::Redraw
            }
            MenuDialog::ExternalPanelize => {
                self.modal = Modal::ExternalPanelize(InputDialog::new(
                    " External panelize ",
                    "Shell command (one path per line on stdout):",
                    String::new(),
                ));
                Disposition::Redraw
            }
            MenuDialog::ShowDirSizes => {
                let cwd = self.active_ref().cwd.clone();
                Disposition::RunOp(PendingOp::ComputeSizes { cwd })
            }
            MenuDialog::EditMenuFile => self.edit_config_file(self.paths.user_menu()),
            MenuDialog::EditExtensionFile => self.edit_config_file(self.paths.extbind()),
            MenuDialog::EditHighlightingFile => self.edit_config_file(self.paths.filehighlight()),
            MenuDialog::Configuration => {
                self.modal = Modal::Configuration(
                    crate::tui::dialog::OptionsDialog::configuration(&self.config),
                );
                Disposition::Redraw
            }
            MenuDialog::Layout => {
                self.modal =
                    Modal::Layout(crate::tui::dialog::LayoutDialog::new(self.config.layout));
                Disposition::Redraw
            }
            MenuDialog::Confirmation => {
                self.modal = Modal::Confirmation(crate::tui::dialog::OptionsDialog::confirmation(
                    &self.config,
                ));
                Disposition::Redraw
            }
            MenuDialog::VirtualFs => {
                self.modal = Modal::VirtualFs(crate::tui::dialog::OptionsDialog::vfs(&self.config));
                Disposition::Redraw
            }
            MenuDialog::SaveSetup => {
                self.snapshot_panels_into_config();
                let path = self.paths.main_config();
                match self.config.save(&path) {
                    Ok(()) => self.set_status(format!("setup saved to {}", path.display())),
                    Err(e) => self.set_status(format!("save setup: {e}")),
                }
                Disposition::Redraw
            }
            MenuDialog::FindAndPanelize => {
                let mut params = FindParams::default();
                params.panelize = true;
                self.modal = Modal::FindForm(FindForm::new(params));
                Disposition::Redraw
            }
            MenuDialog::Encoding => {
                self.set_status("encoding: UTF-8 throughout (no conversion needed)");
                Disposition::Redraw
            }
            MenuDialog::DisplayBits => {
                self.set_status("display bits: UTF-8 only");
                Disposition::Redraw
            }
            MenuDialog::Theme => {
                self.modal = Modal::Theme(crate::tui::dialog::ThemeDialog::new(&self.skin.theme));
                Disposition::Redraw
            }
        }
    }

    /// Open the top menubar, resetting cursor to the first item of the first
    /// section. Shared by F2, F9, and (indirectly) the menu-choice "User
    /// menu"-style menubar entry points.
    pub(super) fn open_menubar(&mut self) -> Disposition {
        self.menubar.reset();
        self.modal = Modal::Menu;
        Disposition::Redraw
    }
}
