//! Background job queue for file operations.
//!
//! - [`Job`] is an async trait describing a unit of work.
//! - [`JobQueue`] is an actor that owns the running set; submit jobs via
//!   [`JobQueue::submit`] and observe their lifecycle through a shared
//!   [`tokio::sync::mpsc::Receiver`] of [`JobUpdate`].
//!
//! Jobs are cooperatively cancellable via [`tokio_util::sync::CancellationToken`].

pub mod ops;
pub mod queue;
pub mod trait_;

pub use queue::{JobQueue, JobUpdateRx};
pub use ops::{CopyJob, DeleteJob, MoveJob};
pub use trait_::{Job, JobCtx, JobHandle, JobId, JobOutcome, JobUpdate, JobUpdateKind, Progress};
