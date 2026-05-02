use std::sync::Arc;

use async_trait::async_trait;
use mc_core::{Result, VPath};
use mc_vfs::Vfs;

use crate::ops::copy::{CopyJob, CopyOptions};
use crate::ops::delete::DeleteJob;
use crate::trait_::{Job, JobCtx, JobOutcome};

use super::child_of;

pub struct MoveJob {
    src_vfs: Arc<dyn Vfs>,
    dst_vfs: Arc<dyn Vfs>,
    sources: Vec<VPath>,
    dst_dir: VPath,
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
            opts,
        }
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
        let mut copy_sources: Vec<VPath> = Vec::new();
        for src in &self.sources {
            if same_vfs {
                let name = match src
                    .last()
                    .and_then(|l| l.sub.file_name().map(|s| s.to_string_lossy().into_owned()))
                {
                    Some(n) => n,
                    None => {
                        copy_sources.push(src.clone());
                        continue;
                    }
                };
                let dst = match child_of(&self.dst_dir, &name) {
                    Ok(d) => d,
                    Err(_) => {
                        copy_sources.push(src.clone());
                        continue;
                    }
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

        let mut copy = CopyJob::new(
            self.src_vfs.clone(),
            self.dst_vfs.clone(),
            copy_sources.clone(),
            self.dst_dir.clone(),
            self.opts,
        );
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
