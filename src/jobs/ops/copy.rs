use std::sync::Arc;

use crate::core::{Entry, Error, Result, VPath};
use crate::vfs::{Vfs, WriteOpts};
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::jobs::trait_::{Job, JobCtx, JobOutcome, Progress};

use super::child_of;

/// Per-job options for copy / move. Surfaced to the UI as a settings
/// dialog before the operation begins.
#[derive(Debug, Clone, Copy)]
pub struct CopyOptions {
    pub overwrite: bool,
    /// Placeholder: not yet consumed by the recursive walker.
    pub preserve_attrs: bool,
    /// Placeholder: not yet consumed by the recursive walker.
    pub follow_symlinks: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            overwrite: true,
            preserve_attrs: true,
            follow_symlinks: false,
        }
    }
}

pub struct CopyJob {
    src_vfs: Arc<dyn Vfs>,
    dst_vfs: Arc<dyn Vfs>,
    sources: Vec<VPath>,
    dst_dir: VPath,
    /// When set with exactly one source, the file is copied to
    /// `dst_dir/target_name` instead of `dst_dir/<src_basename>`. Allows
    /// a single Copy/Move dialog to combine copy + rename, matching mc.
    target_name: Option<String>,
    opts: CopyOptions,
}

impl CopyJob {
    #[must_use]
    pub fn new(
        src_vfs: Arc<dyn Vfs>,
        dst_vfs: Arc<dyn Vfs>,
        sources: Vec<VPath>,
        dst_dir: VPath,
        opts: CopyOptions,
    ) -> Self {
        Self {
            src_vfs,
            dst_vfs,
            sources,
            dst_dir,
            target_name: None,
            opts,
        }
    }

    /// Override the destination basename. Only honored when there is
    /// exactly one source — for multi-source copies the destination must
    /// be a directory and per-source basenames are kept.
    #[must_use]
    pub fn with_target_name(mut self, name: Option<String>) -> Self {
        self.target_name = name;
        self
    }
}

#[async_trait]
impl Job for CopyJob {
    fn description(&self) -> String {
        if self.sources.len() == 1 {
            format!("Copy {}", self.sources[0])
        } else {
            format!("Copy {} items", self.sources.len())
        }
    }

    async fn run(&mut self, ctx: JobCtx) -> Result<JobOutcome> {
        // Pre-walk: total items + bytes for progress reporting.
        let mut totals = Progress::default();
        for src in &self.sources {
            walk_totals(self.src_vfs.as_ref(), src, &mut totals).await?;
        }
        ctx.report_progress(totals).await;

        let mut state = totals;
        state.items_done = 0;
        state.bytes_done = 0;

        let single = self.sources.len() == 1;
        for src in self.sources.clone() {
            let name = match &self.target_name {
                Some(n) if single => n.clone(),
                _ => file_name(&src)?,
            };
            let dst = child_of(&self.dst_dir, &name)?;
            if let Err(e) = copy_recursive(
                self.src_vfs.as_ref(),
                &src,
                self.dst_vfs.as_ref(),
                &dst,
                self.opts.overwrite,
                &ctx,
                &mut state,
            )
            .await
            {
                if ctx.cancelled() {
                    return Ok(JobOutcome::Cancelled);
                }
                return Ok(JobOutcome::Failed(e.to_string()));
            }
            if ctx.cancelled() {
                return Ok(JobOutcome::Cancelled);
            }
        }
        Ok(JobOutcome::Success)
    }
}

async fn walk_totals(vfs: &dyn Vfs, p: &VPath, out: &mut Progress) -> Result<()> {
    let entry = vfs.stat(p).await?;
    if entry.is_dir() {
        out.items_total += 1;
        let entries = vfs.read_dir(p).await?;
        for e in entries {
            if e.name == "." || e.name == ".." {
                continue;
            }
            let child = child_of(p, &e.name)?;
            Box::pin(walk_totals(vfs, &child, out)).await?;
        }
    } else {
        out.items_total += 1;
        out.bytes_total += entry.size;
    }
    Ok(())
}

async fn copy_recursive(
    src_vfs: &dyn Vfs,
    src: &VPath,
    dst_vfs: &dyn Vfs,
    dst: &VPath,
    overwrite: bool,
    ctx: &JobCtx,
    state: &mut Progress,
) -> Result<()> {
    if ctx.cancelled() {
        return Err(Error::Cancelled);
    }
    let entry = src_vfs.stat(src).await?;
    if entry.is_dir() {
        // Best-effort mkdir; ignore EEXIST.
        if let Err(e) = dst_vfs.mkdir(dst).await {
            tracing::debug!("mkdir {dst}: {e}");
        }
        state.items_done += 1;
        ctx.report_status(format!("dir {dst}")).await;
        ctx.report_progress(*state).await;

        let kids = src_vfs.read_dir(src).await?;
        for k in kids {
            if k.name == "." || k.name == ".." {
                continue;
            }
            let s = child_of(src, &k.name)?;
            let d = child_of(dst, &k.name)?;
            Box::pin(copy_recursive(
                src_vfs, &s, dst_vfs, &d, overwrite, ctx, state,
            ))
            .await?;
        }
    } else {
        copy_one(src_vfs, src, dst_vfs, dst, &entry, overwrite, ctx, state).await?;
    }
    Ok(())
}

async fn copy_one(
    src_vfs: &dyn Vfs,
    src: &VPath,
    dst_vfs: &dyn Vfs,
    dst: &VPath,
    entry: &Entry,
    overwrite: bool,
    ctx: &JobCtx,
    state: &mut Progress,
) -> Result<()> {
    ctx.report_status(format!("copy {}", entry.name)).await;
    let mut reader = src_vfs.open_read(src).await?;
    let opts = WriteOpts {
        create: true,
        truncate: overwrite,
        append: false,
    };
    let mut writer = dst_vfs.open_write(dst, opts).await?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if ctx.cancelled() {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
        state.bytes_done += n as u64;
        // Throttle: only report every ~1 MiB.
        if state.bytes_done % (1024 * 1024) < n as u64 {
            ctx.report_progress(*state).await;
        }
    }
    writer.flush().await?;
    state.items_done += 1;
    ctx.report_progress(*state).await;
    Ok(())
}

fn file_name(p: &VPath) -> Result<String> {
    let layer = p
        .last()
        .ok_or_else(|| Error::InvalidPath("empty vpath".into()))?;
    layer
        .sub
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| Error::InvalidPath(format!("no file name in {}", layer.sub.display())))
}
