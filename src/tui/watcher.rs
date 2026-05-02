//! Local-FS watcher that notifies the loop when the active panel cwd changes.
//!
//! We use `notify`'s recommended-watcher with non-recursive mode and re-arm
//! it whenever the panel navigates somewhere new.

use std::path::{Path, PathBuf};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

pub struct PanelWatcher {
    watcher: Option<RecommendedWatcher>,
    current: Option<PathBuf>,
}

impl PanelWatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            watcher: None,
            current: None,
        }
    }

    /// Re-arm the watcher to observe `path`. No-op if `path` is already the
    /// observed dir. Sends a unit `()` on the given channel whenever the watched
    /// dir changes (events are debounced naturally by the loop's tick).
    pub fn watch(&mut self, path: &Path, tx: mpsc::UnboundedSender<()>) {
        if self.current.as_deref() == Some(path) {
            return;
        }
        self.shutdown();

        let path_buf = path.to_path_buf();
        let res = RecommendedWatcher::new(
            move |ev: notify::Result<Event>| {
                if ev.is_ok() {
                    let _ = tx.send(());
                }
            },
            notify::Config::default(),
        );
        let mut w = match res {
            Ok(w) => w,
            Err(e) => {
                tracing::debug!("notify init: {e}");
                return;
            }
        };
        if let Err(e) = w.watch(&path_buf, RecursiveMode::NonRecursive) {
            tracing::debug!("notify watch {}: {e}", path_buf.display());
            return;
        }
        self.watcher = Some(w);
        self.current = Some(path_buf);
    }

    pub fn shutdown(&mut self) {
        self.watcher = None;
        self.current = None;
    }
}

impl Default for PanelWatcher {
    fn default() -> Self {
        Self::new()
    }
}
