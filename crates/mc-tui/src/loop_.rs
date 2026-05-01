use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyEventKind, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use mc_core::VPath;
use mc_find::{FindEvent, Query};
use mc_jobs::{CopyJob, DeleteJob, JobUpdateRx, MoveJob};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use crate::app::{App, Disposition, PendingOp};
use crate::editor_spawn::{resolve_editor, spawn_editor};
use crate::event::chord_from_crossterm;
use crate::watcher::PanelWatcher;

pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        let mut out = io::stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )
        .context("enter alternate screen")?;
        Ok(Self { active: true })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Err(e) = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        ) {
            // Log + stderr — the tracing layer may itself be torn down at this
            // point (typical for panics), so a bare eprintln is a useful belt
            // for the suspenders.
            tracing::error!("TerminalGuard cleanup failed: {e}");
            eprintln!("[mc-rs] terminal cleanup failed: {e}");
        }
        if let Err(e) = disable_raw_mode() {
            tracing::error!("disable_raw_mode failed: {e}");
            eprintln!("[mc-rs] disable_raw_mode failed: {e}");
        }
    }
}

/// Wraps a tokio mpsc receiver of `FindEvent` so the loop can poll it alongside
/// other event sources. `None` means no find is running.
struct FindStream {
    rx: tokio::sync::mpsc::Receiver<FindEvent>,
    cancel: CancellationToken,
    /// When true, the active panel is replaced with the hit list on
    /// completion (Find-and-panelize). Otherwise the FindResults modal
    /// stays open for the user.
    panelize: bool,
}

/// Outcome of a TUI session; the binary may serialize it (`-P FILE`).
#[derive(Debug, Default)]
pub struct ExitInfo {
    /// Final cwd of the active panel, if it points at a local path.
    pub final_cwd: Option<std::path::PathBuf>,
}

pub async fn run(mut app: App, mut job_rx: JobUpdateRx) -> Result<ExitInfo> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("init terminal")?;

    app.ensure_remote_mount().await;
    app.refresh_both().await;
    update_title(&app);

    let mut events = EventStream::new();
    let mut tick = interval(Duration::from_millis(33));

    let mut find: Option<FindStream> = None;
    let mut watcher = PanelWatcher::new();
    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    rearm_watcher(&app, &mut watcher, &watch_tx, &mut watch_rx);
    let mut redraw = true;

    loop {
        if redraw {
            terminal.draw(|f| app.render(f)).context("draw")?;
            redraw = false;
        }

        tokio::select! {
            biased;
            maybe_ev = events.next() => {
                match maybe_ev {
                    Some(Ok(Event::Key(ev))) if ev.kind == KeyEventKind::Press => {
                        if let Some(chord) = chord_from_crossterm(ev) {
                            let disp = app.handle_key(chord);
                            match apply_disposition(
                                disp, &mut app, &mut terminal, &mut find,
                                &mut watcher, &watch_tx, &mut watch_rx,
                            ).await {
                                LoopOutcome::Break => break,
                                LoopOutcome::Continue { redraw: r } => redraw |= r,
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => redraw = true,
                    Some(Ok(Event::Paste(text))) => {
                        app.handle_paste(text);
                        redraw = true;
                    }
                    Some(Ok(Event::Mouse(ev))) => {
                        if matches!(
                            ev.kind,
                            MouseEventKind::Down(MouseButton::Left)
                                | MouseEventKind::Down(MouseButton::Right)
                                | MouseEventKind::ScrollUp
                                | MouseEventKind::ScrollDown
                        ) {
                            let disp = app.handle_mouse(ev);
                            match apply_disposition(
                                disp, &mut app, &mut terminal, &mut find,
                                &mut watcher, &watch_tx, &mut watch_rx,
                            ).await {
                                LoopOutcome::Break => break,
                                LoopOutcome::Continue { redraw: r } => redraw |= r,
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!("input stream error: {e}");
                        break;
                    }
                    None => break,
                }
            }
            Some(()) = watch_rx.recv() => {
                // Drain coalesced events.
                while watch_rx.try_recv().is_ok() {}
                app.refresh_active().await;
                redraw = true;
            }
            Some(ev) = recv_find(&mut find) => {
                match ev {
                    FindEvent::Scanned(p) => app.find_set_status(format!("scanning {p}")),
                    FindEvent::Matched(m) => app.find_push(m.path),
                    FindEvent::Done => {
                        app.find_finish();
                        let was_panelize = find.as_ref().map(|f| f.panelize).unwrap_or(false);
                        find = None;
                        if was_panelize {
                            let items = app.find_results_items();
                            if !items.is_empty() {
                                app.panelize_active(items);
                            }
                        }
                    }
                }
                redraw = true;
            }
            Some(update) = job_rx.recv() => {
                let mut finished = false;
                if matches!(update.kind, mc_jobs::JobUpdateKind::Finished(_)) {
                    finished = true;
                }
                app.handle_job_update(update);
                if finished {
                    // Reload both panels in case the job changed them.
                    app.refresh_both().await;
                }
                redraw = true;
            }
            _ = tick.tick() => {}
        }
    }

    let final_cwd = {
        let cwd = if app.active_left {
            &app.left.cwd
        } else {
            &app.right.cwd
        };
        cwd.last().and_then(|l| {
            if l.scheme == "local" {
                Some(l.sub.clone())
            } else {
                None
            }
        })
    };
    Ok(ExitInfo { final_cwd })
}

fn rearm_watcher(
    app: &App,
    watcher: &mut PanelWatcher,
    tx: &tokio::sync::mpsc::UnboundedSender<()>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    let cwd = if app.active_left {
        &app.left.cwd
    } else {
        &app.right.cwd
    };
    let layer = match cwd.last() {
        Some(l) => l,
        None => return,
    };
    if layer.scheme != "local" {
        watcher.shutdown();
    } else {
        watcher.watch(&layer.sub, tx.clone());
    }
    // Drain any stale events queued by the previous watcher before its
    // callback was dropped — otherwise a notification from the directory
    // we just left would trigger a redundant refresh of the new directory.
    while rx.try_recv().is_ok() {}
}

fn update_title(app: &App) {
    let cwd = if app.active_left {
        &app.left.cwd
    } else {
        &app.right.cwd
    };
    let title = format!("mc-rs: {cwd}");
    let _ = execute!(io::stdout(), SetTitle(title));
}

/// Suspend our crossterm raw/alt-screen state, run `sh -c <cmd>` synchronously
/// in `cwd`, and restore the TUI on return.
fn run_shell_command(cwd: &std::path::Path, cmd: &str) -> anyhow::Result<()> {
    use std::io;
    use std::process::Command;

    use crossterm::execute;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };

    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let status = Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .status();

    // After the child exits, ask the user to press Enter so they have a chance
    // to read its output before we redraw.
    println!("\n[mc-rs] press Enter to continue…");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _ = status?;
    Ok(())
}

async fn recv_find(find: &mut Option<FindStream>) -> Option<FindEvent> {
    match find.as_mut() {
        Some(fs) => fs.rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Outcome of applying a [`Disposition`] to the run-loop. `Break` exits the
/// outer event loop (Quit); `Continue` returns whether a redraw is needed.
enum LoopOutcome {
    Break,
    Continue { redraw: bool },
}

/// Single dispatcher for `Disposition` — shared by the key, mouse, and any
/// future event paths. Handles `ensure_remote_mount`, panel refresh, title
/// updates, watcher re-arm, and `RunOp` execution.
async fn apply_disposition<B: ratatui::backend::Backend>(
    disp: Disposition,
    app: &mut App,
    terminal: &mut Terminal<B>,
    find: &mut Option<FindStream>,
    watcher: &mut PanelWatcher,
    watch_tx: &tokio::sync::mpsc::UnboundedSender<()>,
    watch_rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
) -> LoopOutcome {
    match disp {
        Disposition::Quit => LoopOutcome::Break,
        Disposition::None => LoopOutcome::Continue { redraw: false },
        Disposition::Redraw => LoopOutcome::Continue { redraw: true },
        Disposition::ReloadActive => {
            app.ensure_remote_mount().await;
            app.refresh_active().await;
            update_title(app);
            rearm_watcher(app, watcher, watch_tx, watch_rx);
            LoopOutcome::Continue { redraw: true }
        }
        Disposition::ReloadBoth => {
            app.ensure_remote_mount().await;
            app.refresh_both().await;
            update_title(app);
            rearm_watcher(app, watcher, watch_tx, watch_rx);
            LoopOutcome::Continue { redraw: true }
        }
        Disposition::RebuildTree => {
            app.rebuild_tree().await;
            LoopOutcome::Continue { redraw: true }
        }
        Disposition::TreeToggle => {
            app.tree_toggle().await;
            LoopOutcome::Continue { redraw: true }
        }
        Disposition::RunOp(op) => {
            run_op(app, terminal, op, find).await;
            update_title(app);
            rearm_watcher(app, watcher, watch_tx, watch_rx);
            LoopOutcome::Continue { redraw: true }
        }
    }
}

async fn run_op<B: ratatui::backend::Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    op: PendingOp,
    find: &mut Option<FindStream>,
) {
    match op {
        PendingOp::Mkdir { in_dir, name } => {
            let vfs = match app.registry.root_for(&in_dir) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("mkdir: no backend: {e}");
                    return;
                }
            };
            let target = match VPath::child(&in_dir, &name) {
                Some(t) => t,
                None => return,
            };
            if let Err(e) = vfs.mkdir(&target).await {
                tracing::warn!("mkdir failed: {e}");
            }
            app.refresh_active().await;
        }
        PendingOp::Rename { src, new_name } => {
            let vfs = match app.registry.root_for(&src) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("rename: no backend: {e}");
                    return;
                }
            };
            let parent = match parent_of(&src) {
                Some(p) => p,
                None => return,
            };
            let dst = match VPath::child(&parent, &new_name) {
                Some(t) => t,
                None => return,
            };
            if let Err(e) = vfs.rename(&src, &dst).await {
                tracing::warn!("rename failed: {e}");
            }
            app.refresh_active().await;
        }
        PendingOp::Chmod { targets, mode } => {
            for t in targets {
                let vfs = match app.registry.root_for(&t) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("chmod: no backend: {e}");
                        continue;
                    }
                };
                if let Err(e) = vfs.chmod(&t, mode).await {
                    tracing::warn!("chmod {t} failed: {e}");
                }
            }
            app.refresh_active().await;
        }
        PendingOp::SubmitCopy { sources, dst_dir } => {
            let Some((src_vfs, dst_vfs)) = pick_src_dst(app, sources.first(), &dst_dir) else {
                return;
            };
            let job = CopyJob::new(src_vfs, dst_vfs, sources, dst_dir);
            let desc = job.description_for_dialog();
            let handle = app.jobs.submit(Box::new(job));
            app.show_progress(handle, desc);
        }
        PendingOp::SubmitMove { sources, dst_dir } => {
            let Some((src_vfs, dst_vfs)) = pick_src_dst(app, sources.first(), &dst_dir) else {
                return;
            };
            let job = MoveJob::new(src_vfs, dst_vfs, sources, dst_dir);
            let desc = job.description_for_dialog();
            let handle = app.jobs.submit(Box::new(job));
            app.show_progress(handle, desc);
        }
        PendingOp::SubmitDelete { targets } => {
            let vfs = match targets.first().and_then(|t| app.registry.root_for(t).ok()) {
                Some(v) => v,
                None => return,
            };
            let job = DeleteJob::new(vfs, targets);
            let desc = job.description_for_dialog();
            let handle = app.jobs.submit(Box::new(job));
            app.show_progress(handle, desc);
        }
        PendingOp::StartFind { start, params } => {
            // Cancel any prior find.
            if let Some(prev) = find.take() {
                prev.cancel.cancel();
            }
            let vfs = match app.registry.root_for(&start) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("find: no backend: {e}");
                    return;
                }
            };
            let name_glob = if params.name_pattern.is_empty() || params.name_pattern == "*" {
                None
            } else {
                match mc_find::build_name_glob(&params.name_pattern) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        tracing::warn!("bad glob {:?}: {e}", params.name_pattern);
                        None
                    }
                }
            };
            let content = if params.content_pattern.is_empty() {
                None
            } else {
                match mc_find::build_content_regex(
                    &params.content_pattern,
                    params.whole_word,
                    !params.case_sensitive,
                ) {
                    Ok(r) => Some(mc_find::ContentQuery { regex: r }),
                    Err(e) => {
                        tracing::warn!("bad regex {:?}: {e}", params.content_pattern);
                        None
                    }
                }
            };
            let ignore_dirs = params
                .ignore_dirs
                .split(':')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let q = Query {
                start: start.clone(),
                name_glob,
                content,
                ignore_dirs,
                max_matches: 5000,
            };
            let summary = format!(
                "name={} content={} in={}",
                if params.name_pattern.is_empty() {
                    "*".into()
                } else {
                    params.name_pattern.clone()
                },
                if params.content_pattern.is_empty() {
                    "—".into()
                } else {
                    params.content_pattern.clone()
                },
                start,
            );
            app.show_find_results(summary);
            let cancel = CancellationToken::new();
            let panelize = params.panelize;
            let rx = mc_find::run(vfs, q, cancel.clone());
            *find = Some(FindStream {
                rx,
                cancel,
                panelize,
            });
        }
        PendingOp::RetryRemoteWithPassword {
            scheme,
            location,
            password,
        } => match scheme.as_str() {
            "sftp" => {
                if let Ok(endpoint) = mc_vfs_net::sftp::SftpEndpoint::parse(&location) {
                    match mc_vfs_net::SftpVfs::connect_with_password("sftp", endpoint, &password)
                        .await
                    {
                        Ok(vfs) => {
                            app.registry.register_mount(
                                "sftp",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                            app.set_status(format!("connected to sftp://{location}"));
                            app.refresh_active().await;
                        }
                        Err(e) => {
                            tracing::warn!("sftp password retry {location}: {e}");
                            app.set_status(format!("auth failed: {e}"));
                        }
                    }
                }
            }
            "ftp" => {
                if let Ok(endpoint) = mc_vfs_net::ftp::FtpEndpoint::parse(&location) {
                    match mc_vfs_net::FtpVfs::connect_with_password("ftp", endpoint, &password)
                        .await
                    {
                        Ok(vfs) => {
                            app.registry.register_mount(
                                "ftp",
                                location.clone(),
                                std::sync::Arc::new(vfs),
                            );
                            app.set_status(format!("connected to ftp://{location}"));
                            app.refresh_active().await;
                        }
                        Err(e) => {
                            tracing::warn!("ftp password retry {location}: {e}");
                            app.set_status(format!("auth failed: {e}"));
                        }
                    }
                }
            }
            _ => {}
        },
        PendingOp::AcceptHostKeyAndRetry {
            scheme,
            location,
            algorithm,
            fingerprint,
        } => {
            if scheme != "sftp" {
                app.set_status(format!("host-key trust not supported for scheme {scheme}"));
                return;
            }
            let endpoint = match mc_vfs_net::sftp::SftpEndpoint::parse(&location) {
                Ok(e) => e,
                Err(e) => {
                    app.set_status(format!("bad sftp location: {e}"));
                    return;
                }
            };
            match mc_vfs_net::SftpVfs::connect_trusting(
                "sftp",
                endpoint,
                algorithm,
                fingerprint,
                None,
            )
            .await
            {
                Ok(vfs) => {
                    app.registry
                        .register_mount("sftp", location.clone(), std::sync::Arc::new(vfs));
                    app.set_status(format!("connected to sftp://{location}"));
                    app.refresh_active().await;
                }
                Err(mc_core::Error::HostKeyUnknown { .. }) => {
                    // Server presented a different fingerprint than the user
                    // confirmed — refuse outright (don't loop).
                    app.set_status(format!(
                        "host key changed for {location}; refusing connection"
                    ));
                }
                Err(mc_core::Error::Vfs(msg)) if msg.contains("auth") => {
                    // Auth failure after host accepted: prompt for a password.
                    app.prompt_password("sftp".into(), location);
                }
                Err(e) => {
                    tracing::warn!("sftp accept-host retry {location}: {e}");
                    app.set_status(format!("connect failed: {e}"));
                }
            }
        }
        PendingOp::DropToShell { cwd } => {
            let _ = terminal.show_cursor();
            match crate::subshell::drop_to_shell_with_sync(&cwd) {
                Ok(Some(new_cwd)) => {
                    let target = mc_core::VPath::local(new_cwd.clone());
                    if app.active_left {
                        app.left.navigate(target);
                    } else {
                        app.right.navigate(target);
                    }
                    app.set_status(format!("subshell synced cwd to {}", new_cwd.display()));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("subshell: {e}"),
            }
            let _ = terminal.clear();
            app.refresh_both().await;
        }
        PendingOp::RunShell { cwd, cmd } => {
            let _ = terminal.show_cursor();
            if let Err(e) = run_shell_command(&cwd, &cmd) {
                tracing::warn!("shell {cmd:?}: {e}");
            }
            let _ = terminal.clear();
            app.refresh_both().await;
        }
        PendingOp::RunEditor { file, line } => {
            let editor = resolve_editor(app.config.editor.command.as_deref());
            let _ = terminal.show_cursor();
            if let Err(e) = spawn_editor(&editor, &file, line) {
                tracing::warn!("editor failed: {e}");
            }
            let _ = terminal.clear();
            app.refresh_active().await;
        }
        PendingOp::Chown { targets, uid, gid } => {
            #[cfg(unix)]
            {
                use nix::unistd::{Gid, Uid, chown};
                for t in targets {
                    let Some(local) = crate::app::vpath_to_local(&t) else {
                        app.set_status("chown: remote not supported");
                        continue;
                    };
                    let u = uid.map(Uid::from_raw);
                    let g = gid.map(Gid::from_raw);
                    if let Err(e) = chown(&local, u, g) {
                        tracing::warn!("chown {}: {e}", local.display());
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (targets, uid, gid);
                app.set_status("chown: unix-only");
            }
            app.refresh_active().await;
        }
        PendingOp::Hardlink { src, link } => {
            if let Err(e) = std::fs::hard_link(&src, &link) {
                tracing::warn!("hardlink {} -> {}: {e}", src.display(), link.display());
                app.set_status(format!("hardlink failed: {e}"));
            }
            app.refresh_active().await;
        }
        PendingOp::Symlink {
            target,
            link,
            relative,
        } => {
            #[cfg(unix)]
            {
                let actual = if relative {
                    let parent = link.parent().unwrap_or(std::path::Path::new(""));
                    pathdiff::diff_paths(&target, parent).unwrap_or_else(|| target.clone())
                } else {
                    target.clone()
                };
                if let Err(e) = std::os::unix::fs::symlink(&actual, &link) {
                    tracing::warn!("symlink {} -> {}: {e}", actual.display(), link.display());
                    app.set_status(format!("symlink failed: {e}"));
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (target, link, relative);
                app.set_status("symlink: unix-only");
            }
            app.refresh_active().await;
        }
        PendingOp::EditSymlink { link, new_target } => {
            // Best-effort: remove + recreate. There is a brief window where
            // the symlink doesn't exist; acceptable for an interactive op.
            if let Err(e) = std::fs::remove_file(&link) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("edit symlink remove {}: {e}", link.display());
                    app.set_status(format!("edit symlink: {e}"));
                    return;
                }
            }
            #[cfg(unix)]
            {
                if let Err(e) = std::os::unix::fs::symlink(&new_target, &link) {
                    tracing::warn!(
                        "edit symlink create {} -> {}: {e}",
                        new_target.display(),
                        link.display()
                    );
                    app.set_status(format!("edit symlink: {e}"));
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (link, new_target);
            }
            app.refresh_active().await;
        }
        PendingOp::ComputeSizes { cwd } => {
            let local = match crate::app::vpath_to_local(&cwd) {
                Some(p) => p,
                None => {
                    app.set_status("directory sizes: local panels only");
                    return;
                }
            };
            let entries = if app.active_left {
                app.left.entries.clone()
            } else {
                app.right.entries.clone()
            };
            let mut sizes: Vec<(String, u64)> = Vec::new();
            for e in entries.iter().filter(|e| e.name != "..") {
                if !matches!(e.kind, mc_core::EntryKind::Dir) {
                    continue;
                }
                let dir = local.join(&e.name);
                let mut total: u64 = 0;
                for entry in walkdir::WalkDir::new(&dir)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(Result::ok)
                {
                    if entry.file_type().is_file() {
                        if let Ok(md) = entry.metadata() {
                            total = total.saturating_add(md.len());
                        }
                    }
                }
                sizes.push((e.name.clone(), total));
            }
            let panel = if app.active_left {
                &mut app.left
            } else {
                &mut app.right
            };
            for entry in &mut panel.entries {
                if let Some((_, sz)) = sizes.iter().find(|(n, _)| n == &entry.name) {
                    entry.size = *sz;
                }
            }
            panel.sizes_computed = true;
            app.set_status("directory sizes computed");
        }
        PendingOp::ExternalPanelize { cwd, cmd } => {
            use tokio::process::Command;
            let out = match Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .current_dir(&cwd)
                .output()
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("external panelize: {e}");
                    app.set_status(format!("external panelize: {e}"));
                    return;
                }
            };
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut entries: Vec<mc_core::Entry> = Vec::new();
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let p = std::path::PathBuf::from(line);
                let abs = if p.is_absolute() { p } else { cwd.join(p) };
                let md = match std::fs::symlink_metadata(&abs) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let kind = if md.is_dir() {
                    mc_core::EntryKind::Dir
                } else if md.file_type().is_symlink() {
                    mc_core::EntryKind::Symlink
                } else {
                    mc_core::EntryKind::File
                };
                entries.push(mc_core::Entry {
                    name: abs.to_string_lossy().into_owned(),
                    kind,
                    size: md.len(),
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
            let panel = if app.active_left {
                &mut app.left
            } else {
                &mut app.right
            };
            panel.entries = entries;
            panel.cursor = 0;
            panel.view_offset = 0;
            panel.is_virtual_panelized = true;
            app.set_status("panel populated by external command");
        }
    }
}

fn pick_src_dst(
    app: &App,
    src_example: Option<&VPath>,
    dst_dir: &VPath,
) -> Option<(
    std::sync::Arc<dyn mc_vfs::Vfs>,
    std::sync::Arc<dyn mc_vfs::Vfs>,
)> {
    let src = src_example?;
    let src_vfs = match app.registry.root_for(src) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("src: no backend: {e}");
            return None;
        }
    };
    let dst_vfs = match app.registry.root_for(dst_dir) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("dst: no backend: {e}");
            return None;
        }
    };
    Some((src_vfs, dst_vfs))
}

fn parent_of(p: &VPath) -> Option<VPath> {
    let last = p.last()?.clone();
    let mut sub = last.sub.clone();
    if !sub.pop() {
        return None;
    }
    let mut new_layer = last;
    new_layer.sub = sub;
    let mut new = p.clone();
    new.pop_layer();
    new.push_layer(new_layer);
    Some(new)
}

// -- Convenience: jobs expose `description()` as &str via the trait, but we want
//    a `String` for the dialog without holding a borrow across the submit.
trait DescForDialog {
    fn description_for_dialog(&self) -> String;
}

impl<T: mc_jobs::Job> DescForDialog for T {
    fn description_for_dialog(&self) -> String {
        self.description()
    }
}
