use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::jobs::trait_::{Job, JobCtx, JobHandle, JobId, JobOutcome, JobUpdate, JobUpdateKind};

pub type JobUpdateRx = mpsc::Receiver<JobUpdate>;
pub type JobUpdateTx = mpsc::Sender<JobUpdate>;

pub struct JobQueue {
    update_tx: JobUpdateTx,
}

impl JobQueue {
    /// Returns a queue plus a receiver for job updates. The caller polls
    /// the receiver from its event loop.
    #[must_use]
    pub fn new(buffer: usize) -> (Self, JobUpdateRx) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { update_tx: tx }, rx)
    }

    /// Spawn a job onto the runtime; returns a cancel handle.
    pub fn submit(&self, mut job: Box<dyn Job>) -> JobHandle {
        let id = JobId::next();
        let cancel = CancellationToken::new();
        let handle = JobHandle {
            id,
            cancel: cancel.clone(),
        };
        let tx = self.update_tx.clone();
        let description = job.description();
        let started = JobUpdate {
            id,
            kind: JobUpdateKind::Started { description },
        };
        let started_tx = tx.clone();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            // Best-effort started signal.
            if started_tx.send(started).await.is_err() {
                return;
            }
            let ctx = JobCtx {
                id,
                progress: tx.clone(),
                cancel: cancel_for_task,
            };
            let outcome = match job.run(ctx).await {
                Ok(outcome) => outcome,
                Err(e) => JobOutcome::Failed(e.to_string()),
            };
            if let Err(e) = tx
                .send(JobUpdate {
                    id,
                    kind: JobUpdateKind::Finished(outcome),
                })
                .await
            {
                warn!("job {id:?} finished update lost: {e}");
            }
        });
        handle
    }
}
