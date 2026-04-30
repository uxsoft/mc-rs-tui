use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
};
use futures::StreamExt;
use mc_core::VPath;
use mc_vfs::Vfs;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::time::interval;

use crate::app::{App, Disposition, PendingOp};
use crate::editor_spawn::{resolve_editor, spawn_editor};
use crate::event::chord_from_crossterm;

pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enable raw mode")?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)
            .context("enter alternate screen")?;
        Ok(Self { active: true })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
            let _ = disable_raw_mode();
        }
    }
}

pub async fn run(mut app: App) -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("init terminal")?;

    app.refresh_both().await;
    update_title(&app);

    let mut events = EventStream::new();
    let mut tick = interval(Duration::from_millis(33));

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
                            match app.handle_key(chord) {
                                Disposition::Quit => break,
                                Disposition::Redraw => redraw = true,
                                Disposition::ReloadActive => {
                                    app.refresh_active().await;
                                    update_title(&app);
                                    redraw = true;
                                }
                                Disposition::RunOp(op) => {
                                    run_op(&mut app, &mut terminal, op).await;
                                    update_title(&app);
                                    redraw = true;
                                }
                                Disposition::None => {}
                            }
                        }
                    }
                    Some(Ok(Event::Resize(_, _))) => redraw = true,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!("input stream error: {e}");
                        break;
                    }
                    None => break,
                }
            }
            _ = tick.tick() => {}
        }
    }

    Ok(())
}

fn update_title(app: &App) {
    let cwd = if app.active_left { &app.left.cwd } else { &app.right.cwd };
    let title = format!("mc-rs: {cwd}");
    let _ = execute!(io::stdout(), SetTitle(title));
}

async fn run_op<B: ratatui::backend::Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    op: PendingOp,
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
            let target = match build_child(&in_dir, &name) {
                Some(t) => t,
                None => return,
            };
            if let Err(e) = vfs.mkdir(&target).await {
                tracing::warn!("mkdir failed: {e}");
            }
            app.refresh_active().await;
        }
        PendingOp::Delete { targets } => {
            for t in targets {
                let vfs = match app.registry.root_for(&t) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("delete: no backend: {e}");
                        continue;
                    }
                };
                let res = match stat_kind(vfs.as_ref(), &t).await {
                    Some(true) => recursive_remove(vfs.as_ref(), &t).await,
                    Some(false) => vfs.unlink(&t).await,
                    None => continue,
                };
                if let Err(e) = res {
                    tracing::warn!("delete {} failed: {e}", t);
                }
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
            let dst = match build_child(&parent, &new_name) {
                Some(t) => t,
                None => return,
            };
            if let Err(e) = vfs.rename(&src, &dst).await {
                tracing::warn!("rename failed: {e}");
            }
            app.refresh_active().await;
        }
        PendingOp::Copy { src, dst_dir } => {
            let src_vfs = match app.registry.root_for(&src) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("copy src: no backend: {e}");
                    return;
                }
            };
            let dst_vfs = match app.registry.root_for(&dst_dir) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("copy dst: no backend: {e}");
                    return;
                }
            };
            let name = match src.last().and_then(|l| l.sub.file_name().map(|s| s.to_string_lossy().into_owned())) {
                Some(n) => n,
                None => return,
            };
            let dst = match build_child(&dst_dir, &name) {
                Some(t) => t,
                None => return,
            };
            if let Err(e) = stream_copy(src_vfs.as_ref(), &src, dst_vfs.as_ref(), &dst).await {
                tracing::warn!("copy failed: {e}");
            }
            app.refresh_panel(false).await;
            app.refresh_panel(true).await;
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
        PendingOp::RunEditor { file, line } => {
            let editor = resolve_editor(app.config.editor.command.as_deref());
            // Drop terminal so the child sees a clean stdout.
            let _ = terminal.show_cursor();
            if let Err(e) = spawn_editor(&editor, &file, line) {
                tracing::warn!("editor failed: {e}");
            }
            // Force a full redraw on return.
            let _ = terminal.clear();
            app.refresh_active().await;
        }
    }
}

async fn stat_kind(vfs: &dyn Vfs, p: &VPath) -> Option<bool> {
    match vfs.stat(p).await {
        Ok(e) => Some(e.is_dir()),
        Err(e) => {
            tracing::warn!("stat {p}: {e}");
            None
        }
    }
}

async fn recursive_remove(vfs: &dyn Vfs, p: &VPath) -> mc_core::Result<()> {
    // Depth-first remove. For Phase 1 this is fine; Phase 2 will move it into a Job.
    let entries = vfs.read_dir(p).await?;
    for child in entries {
        if child.name == "." || child.name == ".." {
            continue;
        }
        let mut child_path = p.clone();
        if let Some(layer) = child_path.last().cloned() {
            let mut new_layer = layer;
            new_layer.sub.push(&child.name);
            child_path.pop_layer();
            child_path.push_layer(new_layer);
        }
        if child.is_dir() {
            Box::pin(recursive_remove(vfs, &child_path)).await?;
        } else {
            vfs.unlink(&child_path).await?;
        }
    }
    vfs.rmdir(p).await
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

async fn stream_copy(
    src_vfs: &dyn Vfs,
    src: &VPath,
    dst_vfs: &dyn Vfs,
    dst: &VPath,
) -> mc_core::Result<()> {
    use mc_vfs::WriteOpts;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut reader = src_vfs.open_read(src).await?;
    let mut writer = dst_vfs
        .open_write(
            dst,
            WriteOpts {
                create: true,
                truncate: true,
                append: false,
            },
        )
        .await?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
    }
    writer.flush().await?;
    Ok(())
}

fn build_child(parent: &VPath, name: &str) -> Option<VPath> {
    let layer = parent.last().cloned()?;
    let mut new_layer = layer;
    new_layer.sub.push(name);
    let mut new = parent.clone();
    new.pop_layer();
    new.push_layer(new_layer);
    Some(new)
}
