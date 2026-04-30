//! Clipboard helpers: arboard for native clipboards, OSC52 fallback for SSH.

use std::io::Write;

use base64::Engine;

/// Best-effort copy: try the system clipboard first, then OSC52 to the host
/// terminal. Returns the strategy actually used (for status messages), or
/// `None` if both failed.
pub fn copy(text: &str) -> Option<&'static str> {
    if try_arboard(text) {
        return Some("clipboard");
    }
    if write_osc52(text).is_ok() {
        return Some("OSC52");
    }
    None
}

fn try_arboard(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut c) => match c.set_text(text.to_string()) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!("arboard set_text: {e}");
                false
            }
        },
        Err(e) => {
            tracing::debug!("arboard new: {e}");
            false
        }
    }
}

/// Write the OSC 52 escape so the *terminal* (e.g. iTerm2, alacritty, kitty,
/// wezterm, recent xterm) copies the bytes — works through SSH because the
/// sequence is part of the terminal stream.
fn write_osc52(text: &str) -> std::io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let seq = format!("\x1b]52;c;{encoded}\x1b\\");
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(seq.as_bytes())?;
    stdout.flush()?;
    Ok(())
}
