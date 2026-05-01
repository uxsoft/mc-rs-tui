# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`mc-rs-tui` is a Rust port of GNU Midnight Commander — a dual-panel terminal file manager. The workspace is built on `tokio` (async runtime) and `ratatui` + `crossterm` (TUI). The binary crate is `mc` and produces an executable named `mc-rs`.

## Common commands

```bash
# Build / run
cargo build                              # debug build, whole workspace
cargo build --release                    # release build (LTO + strip, see [profile.release])
cargo run -p mc                          # launch the TUI

# Tests
cargo test                               # all workspace tests
cargo test -p <crate>                    # tests in a single crate (e.g. -p mc-vfs)
cargo test -p mc-jobs --test jobs        # the one integration test currently in the repo
cargo test <name>                        # run a specific test by name
cargo test -- --nocapture                # show stdout from tests

# Lint / format
cargo clippy --workspace --all-targets   # must be clean of pedantic warnings (see below)
cargo fmt --all
```

Toolchain: stable Rust pinned via `rust-toolchain.toml`; `mise.toml` declares `rust = "latest"`. MSRV is **1.85**, Rust **edition 2024**.

Workspace lints (from `Cargo.toml`):
- `unsafe_code = "deny"` — no `unsafe` anywhere.
- `clippy::all` + `clippy::pedantic` at `warn` — code is expected to satisfy pedantic. The following are explicitly allowed: `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc`.

## Architecture

The workspace is layered. `mc-core` is leaf (pure types, no I/O). `mc-vfs` defines the storage abstraction every other crate uses to touch files. `mc-tui` owns the UI and stitches everything together. `mc` is just the binary entry point.

```
mc (binary; produces `mc-rs`)
  └── mc-tui (App, event loop, panels, dialogs)
        ├── mc-core    (pure types: Action, Entry, VPath, KeyChord, Theme, errors; no I/O)
        ├── mc-config  (TOML config, keymaps, themes, hotlist, history)
        ├── mc-jobs    (async job queue: copy/move/delete with progress + cancel)
        ├── mc-find    (recursive name + content search over any Vfs)
        ├── mc-diff    (side-by-side line diff via imara_diff)
        └── mc-vfs     (Vfs trait + Registry + local FS backend)
              ├── mc-vfs-archive  (tar, zip, 7z, cpio, rar — read-only)
              └── mc-vfs-net      (SFTP via russh, FTP, WebDAV)
```

### VFS — the central abstraction

The `Vfs` trait lives in `crates/mc-vfs/src/trait_.rs`. It is `async-trait` and exposes the operations a panel needs: `stat`, `read_dir`, `open_read`/`open_write`, `mkdir`, `rename`, `chmod`, `chown`, `symlink`, `mount_as_vfs`. Backends advertise what they can do via a `Capabilities` bitflags (`READ`, `WRITE`, `STAT`, `SYMLINK`, `CHMOD`, `CHOWN`, `RANDOM_READ`, `WATCH`).

Backends are mounted by scheme through a `Registry`. Local filesystem ships in `mc-vfs`; archives and network protocols plug in via `mc-vfs-archive` and `mc-vfs-net`. **Anything that touches files goes through `Vfs`** — `mc-core` deliberately has zero I/O.

### `mc-tui` internals

The files you'll most often need to read or change when working on the UI:

- `crates/mc-tui/src/app.rs` — `App` state machine; `PendingOp` (queued actions); `Disposition` (redraw hints).
- `crates/mc-tui/src/loop_.rs` — event loop and `TerminalGuard` (raw-mode lifetime); crossterm event → ratatui frame dispatch.
- `crates/mc-tui/src/panel.rs` — dual panel rendering, `ListingMode`, `PanelState`.
- `crates/mc-tui/src/dialog/` — modals: menubar, progress, find, confirm, input, hotlist, jobs log.
- `crates/mc-tui/src/event.rs` — keymap chord resolver.
- `crates/mc-tui/src/editor_spawn.rs`, `subshell.rs`, `watcher.rs`, `clipboard.rs` — terminal/shell integration. These suspend or interact with the raw-mode TUI; touch them carefully when changing the event loop.

### Async / job model

Long-running I/O (copy, move, delete) is submitted to `mc-jobs` as `Job` trait impls. Progress flows back to the UI through a `JobUpdateRx` mpsc channel; cancellation is cooperative via `CancellationToken`. **The UI task never blocks on I/O** — if you find yourself adding a blocking call in `mc-tui`, route it through `mc-jobs` or a `tokio::spawn` instead.

### Errors

- `thiserror` for domain error types (e.g. `mc-core::Error`).
- `anyhow` at the binary boundary (`crates/mc/src/main.rs`).
- `color-eyre` for human-readable terminal reports.

### Tests

Most tests are unit tests embedded in modules. The only integration test today is `crates/mc-jobs/tests/jobs.rs`. When adding async tests, follow the patterns there (tokio `#[tokio::test]`, channels, cancellation tokens).
