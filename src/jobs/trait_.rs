use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

impl JobId {
    pub fn next() -> Self {
        static N: AtomicU64 = AtomicU64::new(1);
        Self(N.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub items_total: u64,
    pub items_done: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
}

#[derive(Debug, Clone)]
pub struct JobUpdate {
    pub id: JobId,
    pub kind: JobUpdateKind,
}

#[derive(Debug, Clone)]
pub enum JobUpdateKind {
    Started {
        description: String,
    },
    Progress(Progress),
    /// User-readable status (e.g., "copying foo.txt").
    Status(String),
    Log(String),
    Finished(JobOutcome),
}

#[derive(Debug, Clone)]
pub enum JobOutcome {
    Success,
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub struct JobCtx {
    pub id: JobId,
    pub progress: mpsc::Sender<JobUpdate>,
    pub cancel: CancellationToken,
}

impl JobCtx {
    pub fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub async fn report_progress(&self, p: Progress) {
        let _ = self
            .progress
            .send(JobUpdate {
                id: self.id,
                kind: JobUpdateKind::Progress(p),
            })
            .await;
    }

    pub async fn report_status(&self, s: impl Into<String>) {
        let _ = self
            .progress
            .send(JobUpdate {
                id: self.id,
                kind: JobUpdateKind::Status(s.into()),
            })
            .await;
    }
}

#[async_trait]
pub trait Job: Send {
    fn description(&self) -> String;
    async fn run(&mut self, ctx: JobCtx) -> Result<JobOutcome>;
}

/// Handle returned by [`crate::JobQueue::submit`]; can be used to cancel.
#[derive(Debug, Clone)]
pub struct JobHandle {
    pub id: JobId,
    pub cancel: CancellationToken,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
