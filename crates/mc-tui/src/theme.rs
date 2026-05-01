//! Bridge between [`mc_config::ThemeColor`] and ratatui's [`Color`].
//!
//! Lives here (not in `mc-config`) so the config crate stays
//! backend-neutral.

use mc_config::ThemeColor;
use ratatui::style::Color;

/// Convert a theme color to ratatui's color type. `Reset` becomes
/// `Color::Reset`, which uses the terminal's default fg/bg.
#[must_use]
pub fn rtc(c: ThemeColor) -> Color {
    match c {
        ThemeColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ThemeColor::Reset => Color::Reset,
    }
}
