use std::sync::Arc;

use crate::core::{Error, Result, VPath};
use crate::vfs::Vfs;
use async_trait::async_trait;

use crate::jobs::trait_::{Job, JobCtx, JobOutcome, Progress};

use super::child_of;

pub struct DeleteJob {
    vfs: Arc<dyn Vfs>,
    targets: Vec<VPath>,
}

impl DeleteJob {
    #[must_use]
    pub fn new(vfs: Arc<dyn Vfs>, targets: Vec<VPath>) -> Self {
        Self { vfs, targets }
    }
}

#[async_trait]
impl Job for DeleteJob {
    fn description(&self) -> String {
        if self.targets.len() == 1 {
            format!("Delete {}", self.targets[0])
        } else {
            format!("Delete {} items", self.targets.len())
        }
    }

    async fn run(&mut self, ctx: JobCtx) -> Result<JobOutcome> {
        let mut totals = Progress::default();
        for t in &self.targets {
            walk_count(self.vfs.as_ref(), t, &mut totals).await?;
        }
        ctx.report_progress(totals).await;
        let mut state = totals;
        state.items_done = 0;

        for t in self.targets.clone() {
            if let Err(e) = delete_recursive(self.vfs.as_ref(), &t, &ctx, &mut state).await {
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

async fn walk_count(vfs: &dyn Vfs, p: &VPath, out: &mut Progress) -> Result<()> {
    let entry = vfs.stat(p).await?;
    out.items_total += 1;
    if entry.is_dir() {
        let entries = vfs.read_dir(p).await?;
        for e in entries {
            if e.name == "." || e.name == ".." {
                continue;
            }
            let child = child_of(p, &e.name)?;
            Box::pin(walk_count(vfs, &child, out)).await?;
        }
    }
    Ok(())
}

async fn delete_recursive(
    vfs: &dyn Vfs,
    p: &VPath,
    ctx: &JobCtx,
    state: &mut Progress,
) -> Result<()> {
    if ctx.cancelled() {
        return Err(Error::Cancelled);
    }
    let entry = vfs.stat(p).await?;
    if entry.is_dir() {
        let kids = vfs.read_dir(p).await?;
        for k in kids {
            if k.name == "." || k.name == ".." {
                continue;
            }
            let child = child_of(p, &k.name)?;
            Box::pin(delete_recursive(vfs, &child, ctx, state)).await?;
        }
        ctx.report_status(format!("rmdir {p}")).await;
        vfs.rmdir(p).await?;
    } else {
        ctx.report_status(format!("rm {}", entry.name)).await;
        vfs.unlink(p).await?;
    }
    state.items_done += 1;
    ctx.report_progress(*state).await;
    Ok(())
}
