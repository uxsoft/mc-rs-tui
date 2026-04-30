use std::path::Path;

use mc_core::VPath;
use mc_jobs::{CopyJob, DeleteJob, Job, JobCtx, JobOutcome, JobUpdateKind};
use mc_vfs::local::LocalVfs;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn populate_tree(root: &Path) {
    use std::fs;
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::write(root.join("a/x.txt"), b"hello").unwrap();
    fs::write(root.join("a/b/y.bin"), vec![0u8; 1024]).unwrap();
    fs::write(root.join("top.txt"), b"top").unwrap();
}

#[tokio::test]
async fn copy_recursive_local() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    populate_tree(src.path());

    let vfs = LocalVfs::shared();
    let sources = vec![
        VPath::local(src.path().join("a")),
        VPath::local(src.path().join("top.txt")),
    ];
    let dst_dir = VPath::local(dst.path().to_path_buf());

    let mut job = CopyJob::new(vfs.clone(), vfs.clone(), sources, dst_dir);
    let (tx, mut rx) = mpsc::channel(64);
    let ctx = JobCtx {
        id: mc_jobs::JobId(1),
        progress: tx,
        cancel: CancellationToken::new(),
    };

    // Drain progress concurrently while running the job to completion.
    let drain = tokio::spawn(async move {
        let mut got_progress = false;
        while let Some(u) = rx.recv().await {
            if matches!(u.kind, JobUpdateKind::Progress(_)) {
                got_progress = true;
            }
        }
        got_progress
    });

    let outcome = job.run(ctx).await.unwrap();
    assert!(matches!(outcome, JobOutcome::Success));
    let got_progress = drain.await.unwrap();
    assert!(got_progress);

    // Ensure all expected files exist on dst.
    assert!(dst.path().join("a/x.txt").exists());
    assert!(dst.path().join("a/b/y.bin").exists());
    assert!(dst.path().join("top.txt").exists());
    assert_eq!(
        std::fs::read(dst.path().join("a/x.txt")).unwrap(),
        b"hello"
    );
}

#[tokio::test]
async fn delete_recursive_local() {
    let dir = TempDir::new().unwrap();
    populate_tree(dir.path());
    let vfs = LocalVfs::shared();
    let targets = vec![VPath::local(dir.path().join("a"))];
    let mut job = DeleteJob::new(vfs.clone(), targets);
    let (tx, mut rx) = mpsc::channel(64);
    let ctx = JobCtx {
        id: mc_jobs::JobId(1),
        progress: tx,
        cancel: CancellationToken::new(),
    };
    let outcome = job.run(ctx).await.unwrap();
    assert!(matches!(outcome, JobOutcome::Success));
    while rx.try_recv().is_ok() {}
    assert!(!dir.path().join("a").exists());
    assert!(dir.path().join("top.txt").exists());
}

#[tokio::test]
async fn copy_cancel_stops_early() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    // Make a moderate tree so we have time to cancel.
    for i in 0..50 {
        std::fs::write(src.path().join(format!("f{i:03}.bin")), vec![0u8; 16 * 1024]).unwrap();
    }
    let vfs = LocalVfs::shared();
    let sources: Vec<VPath> = (0..50)
        .map(|i| VPath::local(src.path().join(format!("f{i:03}.bin"))))
        .collect();
    let dst_dir = VPath::local(dst.path().to_path_buf());

    let mut job = CopyJob::new(vfs.clone(), vfs.clone(), sources, dst_dir);
    let (tx, mut rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let ctx = JobCtx {
        id: mc_jobs::JobId(1),
        progress: tx,
        cancel: cancel.clone(),
    };

    cancel.cancel();
    let outcome = job.run(ctx).await.unwrap();
    assert!(matches!(outcome, JobOutcome::Cancelled));
    while rx.try_recv().is_ok() {}
}
