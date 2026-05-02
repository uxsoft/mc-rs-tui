use std::sync::Arc;

use crate::core::{Result, VPath};
use crate::vfs::Vfs;
use async_trait::async_trait;

use crate::jobs::ops::copy::{CopyJob, CopyOptions};
use crate::jobs::ops::delete::DeleteJob;
use crate::jobs::trait_::{Job, JobCtx, JobOutcome};

use super::child_of;

pub struct MoveJob {
    src_vfs: Arc<dyn Vfs>,
    dst_vfs: Arc<dyn Vfs>,
    sources: Vec<VPath>,
    dst_dir: VPath,
    /// When set with exactly one source, the entry is renamed to
    /// `dst_dir/target_name` rather than keeping the source's basename.
    /// Lets a single F6 dialog combine move + rename, matching mc.
    target_name: Option<String>,
    opts: CopyOptions,
}

impl MoveJob {
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
    /// exactly one source.
    #[must_use]
    pub fn with_target_name(mut self, name: Option<String>) -> Self {
        self.target_name = name;
        self
    }
}

#[async_trait]
impl Job for MoveJob {
    fn description(&self) -> String {
        if self.sources.len() == 1 {
            format!("Move {}", self.sources[0])
        } else {
            format!("Move {} items", self.sources.len())
        }
    }

    async fn run(&mut self, ctx: JobCtx) -> Result<JobOutcome> {
        // Try same-VFS rename fast path per source. Fall back to copy+delete.
        let same_vfs = Arc::ptr_eq(&self.src_vfs, &self.dst_vfs);
        let single = self.sources.len() == 1;
        let mut copy_sources: Vec<VPath> = Vec::new();
        for src in &self.sources {
            if same_vfs {
                let override_name = self.target_name.as_deref().filter(|_| single);
                let basename = override_name.map(str::to_string).or_else(|| {
                    src.last()
                        .and_then(|l| l.sub.file_name().map(|s| s.to_string_lossy().into_owned()))
                });
                let Some(name) = basename else {
                    copy_sources.push(src.clone());
                    continue;
                };
                let Ok(dst) = child_of(&self.dst_dir, &name) else {
                    copy_sources.push(src.clone());
                    continue;
                };
                ctx.report_status(format!("rename {name}")).await;
                if self.src_vfs.rename(src, &dst).await.is_ok() {
                    continue;
                }
            }
            copy_sources.push(src.clone());
        }

        if copy_sources.is_empty() {
            return Ok(JobOutcome::Success);
        }
        if ctx.cancelled() {
            return Ok(JobOutcome::Cancelled);
        }

        let copy_target_name = if single {
            self.target_name.clone()
        } else {
            None
        };
        let mut copy = CopyJob::new(
            self.src_vfs.clone(),
            self.dst_vfs.clone(),
            copy_sources.clone(),
            self.dst_dir.clone(),
            self.opts,
        )
        .with_target_name(copy_target_name);
        let copy_ctx = JobCtx {
            id: ctx.id,
            progress: ctx.progress.clone(),
            cancel: ctx.cancel.clone(),
        };
        match copy.run(copy_ctx).await? {
            JobOutcome::Success => {}
            other => return Ok(other),
        }
        if ctx.cancelled() {
            return Ok(JobOutcome::Cancelled);
        }

        let mut del = DeleteJob::new(self.src_vfs.clone(), copy_sources);
        let del_ctx = JobCtx {
            id: ctx.id,
            progress: ctx.progress.clone(),
            cancel: ctx.cancel.clone(),
        };
        del.run(del_ctx).await
    }
}
