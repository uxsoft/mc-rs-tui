# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`mc-rs-tui` is a Rust port of GNU Midnight Commander — a dual-panel terminal file manager. The crate is built on `tokio` (async runtime) and `ratatui` + `crossterm` (TUI). The package is `mc-rs` and produces an executable named `mc-rs`.

## Common commands

```bash
# Build / run
cargo build                              # debug build
cargo build --release                    # release build (LTO + strip, see [profile.release])
cargo run                                # launch the TUI

# Tests
cargo test                               # all tests
cargo test --test jobs                   # integration job tests
cargo test <name>                        # run a specific test by name
cargo test -- --nocapture                # show stdout from tests

# Lint / format
cargo clippy --all-targets               # must be clean of pedantic warnings (see below)
cargo fmt
```

Toolchain: stable Rust pinned via `rust-toolchain.toml`; `mise.toml` declares `rust = "latest"`. MSRV is **1.85**, Rust **edition 2024**.

Crate lints (from `Cargo.toml`):
- `unsafe_code = "deny"` — no `unsafe` anywhere.
- `clippy::all` + `clippy::pedantic` at `warn` — code is expected to satisfy pedantic. The following are explicitly allowed: `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc`.

## Architecture

The crate is layered by modules. `core` is leaf (pure types, no I/O). `vfs` defines the storage abstraction every other module uses to touch files. `tui` owns the UI and stitches everything together. `src/main.rs` is just the binary entry point.

```
src/main.rs (binary; produces `mc-rs`)
  └── tui (App, event loop, panels, dialogs)
        ├── core        (pure types: Action, Entry, VPath, KeyChord, Theme, errors; no I/O)
        ├── config      (TOML config, keymaps, themes, hotlist, history)
        ├── jobs        (async job queue: copy/move/delete with progress + cancel)
        ├── find        (recursive name + content search over any Vfs)
        ├── diff        (side-by-side line diff via imara_diff)
        └── vfs         (Vfs trait + Registry + local FS backend)
              ├── vfs_archive  (tar, zip, 7z, cpio, rar — read-only)
              └── vfs_net      (SFTP via russh, FTP, WebDAV)
```

### VFS — the central abstraction

The `Vfs` trait lives in `src/vfs/trait_.rs`. It is `async-trait` and exposes the operations a panel needs: `stat`, `read_dir`, `open_read`/`open_write`, `mkdir`, `rename`, `chmod`, `chown`, `symlink`, `mount_as_vfs`. Backends advertise what they can do via a `Capabilities` bitflags (`READ`, `WRITE`, `STAT`, `SYMLINK`, `CHMOD`, `CHOWN`, `RANDOM_READ`, `WATCH`).

Backends are mounted by scheme through a `Registry`. Local filesystem ships in `vfs`; archives and network protocols plug in via `vfs_archive` and `vfs_net`. **Anything that touches files goes through `Vfs`** — `core` deliberately has zero I/O.

### `tui` internals

The files you'll most often need to read or change when working on the UI:

- `src/tui/app/` — `App` state machine; `PendingOp` (queued actions); `Disposition` (redraw hints).
- `src/tui/loop_.rs` — event loop and `TerminalGuard` (raw-mode lifetime); crossterm event → ratatui frame dispatch.
- `src/tui/panel/` — dual panel rendering, `ListingMode`, `PanelState`.
- `src/tui/dialog/` — modals: menubar, progress, find, confirm, input, hotlist, jobs log.
- `src/tui/event.rs` — keymap chord resolver.
- `src/tui/editor_spawn.rs`, `subshell.rs`, `watcher.rs`, `clipboard.rs` — terminal/shell integration. These suspend or interact with the raw-mode TUI; touch them carefully when changing the event loop.

### Async / job model

Long-running I/O (copy, move, delete) is submitted to `jobs` as `Job` trait impls. Progress flows back to the UI through a `JobUpdateRx` mpsc channel; cancellation is cooperative via `CancellationToken`. **The UI task never blocks on I/O** — if you find yourself adding a blocking call in `tui`, route it through `jobs` or a `tokio::spawn` instead.

### Errors

- `thiserror` for domain error types (e.g. `core::Error`).
- `anyhow` at the binary boundary (`src/main.rs`).
- `color-eyre` for human-readable terminal reports.

### Tests

Most tests are unit tests embedded in modules. The main integration test today is `tests/jobs.rs`. When adding async tests, follow the patterns there (tokio `#[tokio::test]`, channels, cancellation tokens).
