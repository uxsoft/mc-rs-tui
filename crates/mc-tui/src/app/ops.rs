//! File-operation helpers and the small free functions used across the
//! `app` submodules. Everything here is reachable from `App` either as a
//! method on `impl App` or via `super::ops::*`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use mc_core::action::SortKey;
use mc_core::{Entry, EntryKind, VPath};

use crate::dialog::{ConfirmDialog, CopyMoveSettingsDialog};
use crate::panel::{ListingMode, PanelState};

use super::{App, CopyMoveKind, Disposition, Modal, PendingOp};

impl App {
    /// If `target` points at a known archive on a local FS, mount it and
    /// return a [`VPath`] pointing at the archive's root inside the mount.
    pub(super) fn try_mount_archive(&mut self, target: &VPath) -> Option<VPath> {
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
                self.registry.register_mount(scheme, location.clone(), vfs);
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
    pub(super) fn open_diff(&mut self) -> Disposition {
        let active = self.active_ref();
        let other = if self.active_left {
            &self.right
        } else {
            &self.left
        };
        let lp = match active.cursor_path() {
            Some(p) => p,
            None => return Disposition::Redraw,
        };
        let rp = {
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
    pub(super) fn run_template(&self, template: &str) -> Disposition {
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

    pub(super) fn macro_ctx(&self) -> mc_core::MacroCtx {
        let active = self.active_ref();
        let other = if self.active_left {
            &self.right
        } else {
            &self.left
        };
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

    pub(super) fn edit_config_file(&mut self, path: PathBuf) -> Disposition {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, "# created by mc-rs\n");
        }
        Disposition::RunOp(PendingOp::RunEditor {
            file: path,
            line: None,
        })
    }

    /// Replace the active panel's entries with synthetic rows for `items`,
    /// flagging the panel as virtually panelized (next reload skipped). Used
    /// by Find-and-panelize and External panelize.
    pub fn panelize_active(&mut self, items: Vec<VPath>) {
        let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
        for vp in &items {
            let local = vpath_to_local(vp);
            let (size, kind) = match local
                .as_ref()
                .and_then(|p| std::fs::symlink_metadata(p).ok())
            {
                Some(md) => {
                    let k = if md.is_dir() {
                        EntryKind::Dir
                    } else if md.file_type().is_symlink() {
                        EntryKind::Symlink
                    } else {
                        EntryKind::File
                    };
                    (md.len(), k)
                }
                None => (0, EntryKind::File),
            };
            entries.push(Entry {
                name: vp.to_string(),
                kind,
                size,
                mtime: None,
                atime: None,
                ctime: None,
                mode: None,
                uid: None,
                gid: None,
                nlink: None,
                target: None,
            });
        }
        let panel = if self.active_left {
            &mut self.left
        } else {
            &mut self.right
        };
        panel.entries = entries;
        panel.cursor = 0;
        panel.view_offset = 0;
        panel.is_virtual_panelized = true;
        self.modal = Modal::None;
        self.set_status(format!("panelized {} items", items.len()));
    }

    /// Collect the F5/F6 sources from the active panel, then defer to
    /// [`open_copy_move_with`].
    pub(super) fn open_copy_move(&mut self, kind: CopyMoveKind) -> Disposition {
        let sources = self.selected_targets();
        if sources.is_empty() {
            return Disposition::None;
        }
        self.open_copy_move_with(sources, kind)
    }

    /// Open the unified Copy/Move destination prompt for the given sources.
    /// The default destination mirrors mc: the *other* panel's cwd.
    pub(super) fn open_copy_move_with(
        &mut self,
        sources: Vec<VPath>,
        kind: CopyMoveKind,
    ) -> Disposition {
        let dst_dir = if self.active_left {
            self.right.cwd.clone()
        } else {
            self.left.cwd.clone()
        };
        let prompt = if sources.len() == 1 {
            format!("{} {} to:", kind.verb(), display_basename(&sources[0]))
        } else {
            format!("{} {} items to:", kind.verb(), sources.len())
        };
        let prefilled = display_dst(&dst_dir);
        let src_cwd = self.active_ref().cwd.clone();
        self.modal = Modal::CopyMove {
            dlg: CopyMoveSettingsDialog::new(
                kind.title(),
                &prompt,
                prefilled,
                mc_jobs::CopyOptions::default(),
            ),
            sources,
            src_cwd,
            kind,
        };
        Disposition::Redraw
    }

    /// Either return `Disposition::Quit` directly or open a confirmation
    /// modal, depending on `config.options.confirm_exit`.
    pub(super) fn maybe_confirm_quit(&mut self) -> Disposition {
        if self.config.options.confirm_exit {
            self.modal =
                Modal::QuitConfirm(ConfirmDialog::new(" Quit ", "Quit Midnight Commander?"));
            Disposition::Redraw
        } else {
            Disposition::Quit
        }
    }

    pub(super) fn snapshot_panels_into_config(&mut self) {
        self.config.panel_left = Some(panel_snapshot(&self.left));
        self.config.panel_right = Some(panel_snapshot(&self.right));
    }

    pub(super) fn compare_dirs(&mut self) {
        let other = if self.active_left {
            &self.right
        } else {
            &self.left
        };
        let other_by_name: std::collections::HashMap<String, u64> = other
            .entries
            .iter()
            .filter(|e| e.name != ".." && !e.is_dir())
            .map(|e| (e.name.clone(), e.size))
            .collect();
        let active = if self.active_left {
            &mut self.left
        } else {
            &mut self.right
        };
        active.marks.clear();
        for e in &active.entries {
            if e.name == ".." || e.is_dir() {
                continue;
            }
            match other_by_name.get(&e.name) {
                Some(sz) if *sz == e.size => {}
                _ => {
                    active.marks.insert(e.name.clone());
                }
            }
        }
    }
}

pub(super) fn next_sort(s: SortKey) -> SortKey {
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

/// Display a destination directory for the Copy/Move prompt: bare local path
/// for single-layer local cwds, full `Display` form (e.g. `sftp://...`)
/// otherwise.
pub(super) fn display_dst(p: &VPath) -> String {
    match p.last() {
        Some(l) if l.scheme == "local" && p.layers().len() == 1 && l.location.is_empty() => {
            l.sub.display().to_string()
        }
        _ => p.to_string(),
    }
}

/// Show just the file name of a source path (e.g. `hello.txt`), for the
/// "Copy <name> to:" prompt.
pub(super) fn display_basename(p: &VPath) -> String {
    p.last()
        .and_then(|l| l.sub.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| p.to_string())
}

/// Parse the destination string from the Copy/Move dialog into a `VPath`.
/// Accepts: full VPath strings (`local:/x`, `sftp://h/p`); absolute local
/// paths (`/x`); paths relative to `src_cwd` when that cwd is local.
pub(super) fn parse_dst(input: &str, src_cwd: &VPath) -> Option<VPath> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(p) = s.parse::<VPath>() {
        return Some(p);
    }
    let pb = PathBuf::from(s);
    if pb.is_absolute() {
        return Some(VPath::local(pb));
    }
    let layer = src_cwd.last()?;
    if layer.scheme == "local" && src_cwd.layers().len() == 1 {
        return Some(VPath::local(layer.sub.join(&pb)));
    }
    None
}

fn next_mount_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn parse_octal_mode(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim(), 8)
        .ok()
        .filter(|&m| m <= 0o7777)
}

/// Parse a `user:group` chown spec. Either side may be empty (returns `None`
/// for that field, meaning "don't change"). Numeric (uid/gid) and name forms
/// are both accepted on Unix; non-Unix only accepts numeric.
pub fn parse_chown(s: &str) -> Option<(Option<u32>, Option<u32>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (user, group) = match s.split_once(':') {
        Some((u, g)) => (u, g),
        None => (s, ""),
    };
    let uid = resolve_user(user.trim())?;
    let gid = resolve_group(group.trim())?;
    Some((uid, gid))
}

#[cfg(unix)]
fn resolve_user(x: &str) -> Option<Option<u32>> {
    if x.is_empty() {
        return Some(None);
    }
    if let Ok(n) = x.parse::<u32>() {
        return Some(Some(n));
    }
    nix::unistd::User::from_name(x)
        .ok()
        .flatten()
        .map(|u| Some(u.uid.as_raw()))
}

#[cfg(unix)]
fn resolve_group(x: &str) -> Option<Option<u32>> {
    if x.is_empty() {
        return Some(None);
    }
    if let Ok(n) = x.parse::<u32>() {
        return Some(Some(n));
    }
    nix::unistd::Group::from_name(x)
        .ok()
        .flatten()
        .map(|g| Some(g.gid.as_raw()))
}

#[cfg(not(unix))]
fn resolve_user(x: &str) -> Option<Option<u32>> {
    if x.is_empty() {
        return Some(None);
    }
    x.parse::<u32>().ok().map(Some)
}

#[cfg(not(unix))]
fn resolve_group(x: &str) -> Option<Option<u32>> {
    if x.is_empty() {
        return Some(None);
    }
    x.parse::<u32>().ok().map(Some)
}

/// Build a default link path for a hardlink/symlink dialog: the cursored
/// entry's local sibling with `_link` suffix. Returns the full path so the
/// user can edit just the trailing component.
pub(super) fn default_link_name(src: &VPath) -> String {
    let layer = match src.last() {
        Some(l) => l,
        None => return String::new(),
    };
    let mut sub = layer.sub.clone();
    let stem = sub
        .file_name()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = sub.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let link_name = format!("{stem}_link");
    sub = parent;
    sub.push(link_name);
    sub.to_string_lossy().into_owned()
}

pub(super) fn apply_panel_snapshot(p: &mut PanelState, s: &mc_config::PanelStateSnapshot) {
    p.sort_by = s.sort_by;
    p.reverse = s.reverse;
    p.show_hidden = s.show_hidden;
    p.mix_dirs = s.mix_dirs;
    p.filter = s.filter.clone();
    p.mode = match s.listing.as_str() {
        "Brief" => ListingMode::Brief,
        "Long" => ListingMode::Long,
        "Tree" => ListingMode::Tree,
        _ => ListingMode::Full,
    };
}

pub(super) fn panel_snapshot(p: &PanelState) -> mc_config::PanelStateSnapshot {
    mc_config::PanelStateSnapshot {
        sort_by: p.sort_by,
        reverse: p.reverse,
        listing: match p.mode {
            ListingMode::Full => "Full".into(),
            ListingMode::Brief => "Brief".into(),
            ListingMode::Long => "Long".into(),
            ListingMode::Tree => "Tree".into(),
        },
        show_hidden: p.show_hidden,
        mix_dirs: p.mix_dirs,
        filter: p.filter.clone(),
    }
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
