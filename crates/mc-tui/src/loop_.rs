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
use mc_jobs::{CopyJob, DeleteJob, JobUpdateRx, MoveJob};
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

pub async fn run(mut app: App, mut job_rx: JobUpdateRx) -> Result<()> {
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
            let vfs = match targets
                .first()
                .and_then(|t| app.registry.root_for(t).ok())
            {
                Some(v) => v,
                None => return,
            };
            let job = DeleteJob::new(vfs, targets);
            let desc = job.description_for_dialog();
            let handle = app.jobs.submit(Box::new(job));
            app.show_progress(handle, desc);
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
    }
}

fn pick_src_dst(
    app: &App,
    src_example: Option<&VPath>,
    dst_dir: &VPath,
) -> Option<(std::sync::Arc<dyn mc_vfs::Vfs>, std::sync::Arc<dyn mc_vfs::Vfs>)> {
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

fn build_child(parent: &VPath, name: &str) -> Option<VPath> {
    let layer = parent.last().cloned()?;
    let mut new_layer = layer;
    new_layer.sub.push(name);
    let mut new = parent.clone();
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
