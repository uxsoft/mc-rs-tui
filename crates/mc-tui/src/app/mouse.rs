//! Mouse-event routing: top-level `handle_mouse` (menubar / panels /
//! buttonbar / scroll / double-click) and the per-modal dispatcher
//! `handle_modal_mouse`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mc_core::VPath;
use mc_core::key::{KeyChord, KeyCode};
use ratatui::layout::Rect;

use crate::dialog::{
    Dialog, DialogOutcome, FindFormOutcome, FindResultsOutcome, HotlistAction, HotlistDialog,
    InputDialog,
};
use crate::panel::ListingMode;

use super::ops::{parent_path, parse_dst, parse_octal_mode};
use super::{App, CopyMoveKind, Disposition, Modal, PendingOp};

impl App {
    /// Handle a mouse event. Currently scoped to the top menubar:
    /// - left-click on row 0 opens the menu and selects the section under
    ///   the cursor (or closes the menu if it was already open and the
    ///   click landed off-section);
    /// - left-click on a dropdown row, when the menu is active, selects
    ///   that row's choice;
    /// - left-click anywhere else while the menu is active closes it.
    /// All other clicks are ignored (no panel-cell click yet).
    pub fn handle_mouse(&mut self, ev: crossterm::event::MouseEvent) -> Disposition {
        use crossterm::event::{MouseButton, MouseEventKind};
        let col = ev.column;
        let row = ev.row;

        // ---- 1) Menu open: title row + dropdown ---------------------------
        if matches!(self.modal, Modal::Menu) {
            if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
                return Disposition::None;
            }
            if row == self.layout.menubar.y {
                if let Some(idx) = self.menubar.section_at_column(col) {
                    if self.menubar.active_section == idx {
                        self.modal = Modal::None;
                    } else {
                        self.menubar.open_at(idx);
                    }
                    return Disposition::Redraw;
                }
                self.modal = Modal::None;
                return Disposition::Redraw;
            }
            let drop_area = ratatui::layout::Rect::new(
                self.layout.frame.x,
                self.layout.menubar.y + 1,
                self.layout.frame.width,
                self.layout.frame.height.saturating_sub(1),
            );
            let drop = self.menubar.dropdown_rect(drop_area);
            if row > drop.y
                && row < drop.y + drop.height.saturating_sub(1)
                && col > drop.x
                && col < drop.x + drop.width.saturating_sub(1)
            {
                let row_in_body = row - (drop.y + 1);
                if let Some(choice) = self.menubar.item_choice_at(row_in_body) {
                    return self.handle_menu_choice(choice);
                }
                return Disposition::Redraw;
            }
            self.modal = Modal::None;
            return Disposition::Redraw;
        }

        // ---- 2) Other modal: route to dialog ------------------------------
        if !matches!(self.modal, Modal::None | Modal::PrefixCtrlX) {
            return self.handle_modal_mouse(ev);
        }

        // ---- 3) Click on the menubar title row → open the menu -----------
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
            && row == self.layout.menubar.y
        {
            if let Some(idx) = self.menubar.section_at_column(col) {
                self.menubar.open_at(idx);
                self.modal = Modal::Menu;
                return Disposition::Redraw;
            }
        }

        // ---- 4) Buttonbar (F1..F10) clicks -------------------------------
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
            && row == self.layout.buttonbar.y
        {
            let segments = self.layout.button_segments.clone();
            for seg in segments {
                if col >= seg.x && col < seg.x.saturating_add(seg.w) {
                    return self.handle_panel_key(KeyChord::plain(KeyCode::F(seg.fkey)));
                }
            }
        }

        // ---- 5) Panel area: scroll / focus / select / right-click /
        //         double-click → enter -----------------------------------
        let on_left = rect_contains(self.layout.left_body, col, row);
        let on_right = rect_contains(self.layout.right_body, col, row);

        match ev.kind {
            MouseEventKind::ScrollUp => {
                let target_left = if on_left {
                    true
                } else if on_right {
                    false
                } else {
                    self.active_left
                };
                let panel = if target_left {
                    &mut self.left
                } else {
                    &mut self.right
                };
                if matches!(panel.mode, ListingMode::Tree) {
                    panel.tree.cursor = panel.tree.cursor.saturating_sub(3);
                } else {
                    panel.cursor = panel.cursor.saturating_sub(3);
                }
                return Disposition::Redraw;
            }
            MouseEventKind::ScrollDown => {
                let target_left = if on_left {
                    true
                } else if on_right {
                    false
                } else {
                    self.active_left
                };
                let panel = if target_left {
                    &mut self.left
                } else {
                    &mut self.right
                };
                if matches!(panel.mode, ListingMode::Tree) {
                    let last = panel.tree.nodes.len().saturating_sub(1);
                    panel.tree.cursor = (panel.tree.cursor + 3).min(last);
                } else if !panel.entries.is_empty() {
                    let last = panel.entries.len() - 1;
                    panel.cursor = (panel.cursor + 3).min(last);
                }
                return Disposition::Redraw;
            }
            MouseEventKind::Down(MouseButton::Left | MouseButton::Right) => {}
            _ => return Disposition::None,
        }

        let body = if on_left {
            Some((true, self.layout.left_body))
        } else if on_right {
            Some((false, self.layout.right_body))
        } else {
            None
        };
        let Some((target_left, body_rect)) = body else {
            return Disposition::None;
        };

        if self.active_left != target_left {
            self.active_left = target_left;
            self.left.active = target_left;
            self.right.active = !target_left;
        }
        let row_in_body = (row - body_rect.y) as usize;

        if matches!(self.active_ref().mode, ListingMode::Tree) {
            let panel = self.active();
            let target = panel.view_offset + row_in_body;
            if target < panel.tree.nodes.len() {
                panel.tree.cursor = target;
            }
            return Disposition::Redraw;
        }

        let entries_len = self.active_ref().entries.len();
        let target = self.active_ref().view_offset + row_in_body;
        if target >= entries_len {
            return Disposition::Redraw;
        }

        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Right)) {
            let panel = self.active();
            panel.cursor = target;
            panel.toggle_mark();
            return Disposition::Redraw;
        }

        let now = Instant::now();
        let is_double = self.last_click.is_some_and(|(c, r, t)| {
            c == col && r == row && now.duration_since(t) < Duration::from_millis(400)
        });
        self.last_click = Some((col, row, now));

        if is_double && target == self.active_ref().cursor {
            self.last_click = None;
            return self.handle_panel_key(KeyChord::plain(KeyCode::Enter));
        }

        self.active().cursor = target;
        Disposition::Redraw
    }

    /// Route a mouse event to whichever modal dialog is currently active.
    /// Mirrors [`App::handle_modal_key`] but calls `handle_mouse` on the dialog.
    fn handle_modal_mouse(&mut self, ev: crossterm::event::MouseEvent) -> Disposition {
        let area = self.layout.frame;
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None | Modal::Menu | Modal::PrefixCtrlX => Disposition::None,
            Modal::Mkdir(mut dlg) => match dlg.handle_mouse(ev, area) {
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
            Modal::DeleteConfirm(mut dlg, targets) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::DeleteConfirm(dlg, targets);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(false) => Disposition::Redraw,
                DialogOutcome::Submitted(true) => {
                    Disposition::RunOp(PendingOp::SubmitDelete { targets })
                }
            },
            Modal::Chmod { mut dlg, targets } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Chmod { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(s) => match parse_octal_mode(&s) {
                    Some(mode) => Disposition::RunOp(PendingOp::Chmod { targets, mode }),
                    None => {
                        self.modal = Modal::Chmod {
                            dlg: InputDialog::new(" Chmod ", "Octal mode (e.g. 755):", s),
                            targets,
                        };
                        Disposition::Redraw
                    }
                },
            },
            Modal::CopyMove {
                mut dlg,
                sources,
                src_cwd,
                kind,
            } => match dlg.handle_mouse(ev, area) {
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
                DialogOutcome::Submitted(input) => match parse_dst(&input, &src_cwd) {
                    Some(dst_dir) => Disposition::RunOp(match kind {
                        CopyMoveKind::Copy => PendingOp::SubmitCopy { sources, dst_dir },
                        CopyMoveKind::Move => PendingOp::SubmitMove { sources, dst_dir },
                    }),
                    None => {
                        self.set_status(format!("{}: invalid destination: {input}", kind.verb()));
                        let prompt = format!("{} to:", kind.verb());
                        self.modal = Modal::CopyMove {
                            dlg: InputDialog::new(kind.title(), &prompt, input),
                            sources,
                            src_cwd,
                            kind,
                        };
                        Disposition::Redraw
                    }
                },
            },
            Modal::Rename(mut dlg, src) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Rename(dlg, src);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(new_name) => {
                    Disposition::RunOp(PendingOp::Rename { src, new_name })
                }
            },
            Modal::SelectGroup { mut dlg, select } => match dlg.handle_mouse(ev, area) {
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
            Modal::CmdLine(mut dlg) => match dlg.handle_mouse(ev, area) {
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
            Modal::FindForm(mut dlg) => match dlg.handle_mouse(ev, area) {
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
            Modal::FindResults(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::FindResults(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(FindResultsOutcome::Navigate(p)) => {
                    let target = parent_path(&p).unwrap_or(p);
                    self.active().navigate(target);
                    Disposition::ReloadActive
                }
                DialogOutcome::Submitted(FindResultsOutcome::Panelize(items)) => {
                    self.panelize_active(items);
                    Disposition::Redraw
                }
            },
            Modal::Hotlist(mut dlg) => match dlg.handle_mouse(ev, area) {
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
            Modal::Progress(dlg) => {
                self.modal = Modal::Progress(dlg);
                Disposition::None
            }
            Modal::Viewer(v) => {
                self.modal = Modal::Viewer(v);
                Disposition::None
            }
            Modal::Diff(d) => {
                self.modal = Modal::Diff(d);
                Disposition::None
            }
            Modal::Help(mut d) => match d.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Help(d);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::LearnKeys(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::LearnKeys(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::JobsView(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::JobsView(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(()) => Disposition::Redraw,
            },
            Modal::UserMenu(mut dlg) => match dlg.handle_mouse(ev, area) {
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
            Modal::Password {
                mut dlg,
                scheme,
                location,
            } => match dlg.handle_mouse(ev, area) {
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
            } => match dlg.handle_mouse(ev, area) {
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
            Modal::QuickCd(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::QuickCd(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::QuickSearch(filter) => {
                self.modal = Modal::QuickSearch(filter);
                Disposition::None
            }
            Modal::Chown { mut dlg, targets } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Chown { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled => Disposition::Redraw,
                DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Chattr { mut dlg, targets } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Chattr { dlg, targets };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Hardlink { mut dlg, src } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Hardlink { dlg, src };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Symlink {
                mut dlg,
                src,
                relative,
            } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Symlink { dlg, src, relative };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::EditSymlink { mut dlg, link } => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::EditSymlink { dlg, link };
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Filter(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Filter(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::ExternalPanelize(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::ExternalPanelize(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::VfsList(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::VfsList(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Configuration(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Configuration(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Confirmation(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Confirmation(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::VirtualFs(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::VirtualFs(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Layout(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Layout(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::Theme(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::Theme(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(_) => Disposition::Redraw,
            },
            Modal::QuitConfirm(mut dlg) => match dlg.handle_mouse(ev, area) {
                DialogOutcome::None => {
                    self.modal = Modal::QuitConfirm(dlg);
                    Disposition::Redraw
                }
                DialogOutcome::Cancelled | DialogOutcome::Submitted(false) => Disposition::Redraw,
                DialogOutcome::Submitted(true) => Disposition::Quit,
            },
        }
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}
