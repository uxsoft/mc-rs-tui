//! Key chords and sequences in mc-compatible syntax.
//!
//! Examples:
//! - `F1`, `F10`, `F20`
//! - `C-x`, `M-?`, `S-Tab`, `C-M-Delete`
//! - `Insert`, `Delete`, `PageUp`, `Enter`, `Esc`, `Tab`, `Backspace`
//! - `+`, `\`, `?`, single printable characters

use std::fmt;
use std::str::FromStr;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct KeyMods: u8 {
        const SHIFT = 1 << 0;
        const ALT   = 1 << 1; // mc's "M-" (Meta)
        const CTRL  = 1 << 2; // mc's "C-"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    F(u8),
    Enter,
    Escape,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyMods,
}

impl KeyChord {
    #[must_use]
    pub fn new(code: KeyCode, mods: KeyMods) -> Self {
        Self { code, mods }
    }

    #[must_use]
    pub fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyMods::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeySequence(pub Vec<KeyChord>);

impl KeySequence {
    #[must_use]
    pub fn single(c: KeyChord) -> Self {
        Self(vec![c])
    }
}

// -- parsing --------------------------------------------------------------

impl FromStr for KeyChord {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut mods = KeyMods::empty();
        let mut rest = s;
        loop {
            if let Some(r) = rest.strip_prefix("C-") {
                mods |= KeyMods::CTRL;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("M-") {
                mods |= KeyMods::ALT;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("S-") {
                mods |= KeyMods::SHIFT;
                rest = r;
            } else {
                break;
            }
        }
        let code = parse_code(rest)?;
        Ok(KeyChord { code, mods })
    }
}

fn parse_code(s: &str) -> Result<KeyCode> {
    let invalid = || Error::InvalidKey(s.to_owned());
    let lower = s.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        other if other.starts_with('f') && other.len() <= 3 => {
            let n: u8 = other[1..].parse().map_err(|_| invalid())?;
            if !(1..=24).contains(&n) {
                return Err(invalid());
            }
            KeyCode::F(n)
        }
        _ => {
            let mut chars = s.chars();
            let c = chars.next().ok_or_else(invalid)?;
            if chars.next().is_some() {
                return Err(invalid());
            }
            KeyCode::Char(c)
        }
    };
    Ok(code)
}

impl FromStr for KeySequence {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let chords: Result<Vec<_>> = s.split_whitespace().map(KeyChord::from_str).collect();
        Ok(Self(chords?))
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyMods::CTRL) {
            f.write_str("C-")?;
        }
        if self.mods.contains(KeyMods::ALT) {
            f.write_str("M-")?;
        }
        if self.mods.contains(KeyMods::SHIFT) {
            f.write_str("S-")?;
        }
        match self.code {
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::Enter => f.write_str("Enter"),
            KeyCode::Escape => f.write_str("Esc"),
            KeyCode::Tab => f.write_str("Tab"),
            KeyCode::BackTab => f.write_str("BackTab"),
            KeyCode::Backspace => f.write_str("Backspace"),
            KeyCode::Delete => f.write_str("Delete"),
            KeyCode::Insert => f.write_str("Insert"),
            KeyCode::Home => f.write_str("Home"),
            KeyCode::End => f.write_str("End"),
            KeyCode::PageUp => f.write_str("PageUp"),
            KeyCode::PageDown => f.write_str("PageDown"),
            KeyCode::Up => f.write_str("Up"),
            KeyCode::Down => f.write_str("Down"),
            KeyCode::Left => f.write_str("Left"),
            KeyCode::Right => f.write_str("Right"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_function_keys() {
        let f1: KeyChord = "F1".parse().unwrap();
        assert_eq!(f1.code, KeyCode::F(1));
        assert!(f1.mods.is_empty());
        assert_eq!("F20".parse::<KeyChord>().unwrap().code, KeyCode::F(20));
        assert!("F0".parse::<KeyChord>().is_err());
        assert!("F25".parse::<KeyChord>().is_err());
    }

    #[test]
    fn parse_modifiers() {
        let c: KeyChord = "C-x".parse().unwrap();
        assert_eq!(c.code, KeyCode::Char('x'));
        assert_eq!(c.mods, KeyMods::CTRL);

        let m: KeyChord = "M-?".parse().unwrap();
        assert_eq!(m.code, KeyCode::Char('?'));
        assert_eq!(m.mods, KeyMods::ALT);

        let cms: KeyChord = "C-M-S-Delete".parse().unwrap();
        assert_eq!(cms.code, KeyCode::Delete);
        assert_eq!(cms.mods, KeyMods::CTRL | KeyMods::ALT | KeyMods::SHIFT);
    }

    #[test]
    fn parse_named_keys() {
        for (s, expected) in [
            ("Insert", KeyCode::Insert),
            ("Tab", KeyCode::Tab),
            ("Enter", KeyCode::Enter),
            ("Esc", KeyCode::Escape),
            ("PageUp", KeyCode::PageUp),
            ("Backspace", KeyCode::Backspace),
        ] {
            assert_eq!(s.parse::<KeyChord>().unwrap().code, expected);
        }
    }

    #[test]
    fn parse_chord_sequence() {
        let seq: KeySequence = "C-x C-c".parse().unwrap();
        assert_eq!(seq.0.len(), 2);
        assert_eq!(seq.0[0].mods, KeyMods::CTRL);
        assert_eq!(seq.0[1].code, KeyCode::Char('c'));
    }

    #[test]
    fn display_round_trip() {
        for s in ["F1", "C-x", "M-?", "S-Tab", "C-M-Delete", "Insert"] {
            let c: KeyChord = s.parse().unwrap();
            assert_eq!(c.to_string(), s);
        }
    }
}
