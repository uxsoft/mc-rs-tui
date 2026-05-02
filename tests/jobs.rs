use std::path::Path;

use mc_rs::core::VPath;
use mc_rs::jobs::{self, CopyJob, CopyOptions, DeleteJob, Job, JobCtx, JobOutcome, JobUpdateKind};
use mc_rs::vfs::local::LocalVfs;
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

    let mut job = CopyJob::new(
        vfs.clone(),
        vfs.clone(),
        sources,
        dst_dir,
        CopyOptions::default(),
    );
    let (tx, mut rx) = mpsc::channel(64);
    let ctx = JobCtx {
        id: jobs::JobId(1),
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
    assert_eq!(std::fs::read(dst.path().join("a/x.txt")).unwrap(), b"hello");
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
        id: jobs::JobId(1),
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
        std::fs::write(
            src.path().join(format!("f{i:03}.bin")),
            vec![0u8; 16 * 1024],
        )
        .unwrap();
    }
    let vfs = LocalVfs::shared();
    let sources: Vec<VPath> = (0..50)
        .map(|i| VPath::local(src.path().join(format!("f{i:03}.bin"))))
        .collect();
    let dst_dir = VPath::local(dst.path().to_path_buf());

    let mut job = CopyJob::new(
        vfs.clone(),
        vfs.clone(),
        sources,
        dst_dir,
        CopyOptions::default(),
    );
    let (tx, mut rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let ctx = JobCtx {
        id: jobs::JobId(1),
        progress: tx,
        cancel: cancel.clone(),
    };

    cancel.cancel();
    let outcome = job.run(ctx).await.unwrap();
    assert!(matches!(outcome, JobOutcome::Cancelled));
    while rx.try_recv().is_ok() {}
}

/// Simulates the "copy to /usr/local/bin without root" failure: the destination
/// directory is read-only for the running process, so opening the destination
/// file for write must fail. The job should report this as `JobOutcome::Failed`
/// with a non-empty error string — that's what the UI relies on to surface an
/// error dialog.
#[cfg(unix)]
#[tokio::test]
async fn copy_to_readonly_dir_yields_failed_outcome() {
    use std::os::unix::fs::PermissionsExt;

    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    std::fs::write(src.path().join("foo.txt"), b"hello").unwrap();

    // Strip every write bit so the test process (non-root) cannot create
    // entries inside `dst`. Skip the test if we are root, since root bypasses
    // permission checks.
    let mut perms = std::fs::metadata(dst.path()).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(dst.path(), perms).unwrap();
    if nix::unistd::Uid::effective().is_root() {
        // Restore so TempDir cleanup works.
        let mut p = std::fs::metadata(dst.path()).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(dst.path(), p).unwrap();
        eprintln!("skipping: running as root bypasses permission checks");
        return;
    }

    let vfs = LocalVfs::shared();
    let sources = vec![VPath::local(src.path().join("foo.txt"))];
    let dst_dir = VPath::local(dst.path().to_path_buf());
    let mut job = CopyJob::new(
        vfs.clone(),
        vfs.clone(),
        sources,
        dst_dir,
        CopyOptions::default(),
    );
    let (tx, mut rx) = mpsc::channel(64);
    let ctx = JobCtx {
        id: jobs::JobId(1),
        progress: tx,
        cancel: CancellationToken::new(),
    };
    let outcome = job.run(ctx).await.unwrap();

    // Restore so TempDir cleanup works.
    let mut p = std::fs::metadata(dst.path()).unwrap().permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(dst.path(), p).unwrap();

    while rx.try_recv().is_ok() {}
    match outcome {
        JobOutcome::Failed(e) => assert!(!e.is_empty(), "Failed outcome should carry error text"),
        other => panic!("expected JobOutcome::Failed, got {other:?}"),
    }
}
