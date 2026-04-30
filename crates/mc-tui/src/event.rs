use crossterm::event::{KeyCode as CtKeyCode, KeyEvent, KeyModifiers};
use mc_core::key::{KeyChord, KeyCode, KeyMods};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyChord),
    Resize(u16, u16),
    Tick,
}

#[must_use]
pub fn chord_from_crossterm(ev: KeyEvent) -> Option<KeyChord> {
    let mut mods = KeyMods::empty();
    if ev.modifiers.contains(KeyModifiers::SHIFT) {
        mods |= KeyMods::SHIFT;
    }
    if ev.modifiers.contains(KeyModifiers::ALT) {
        mods |= KeyMods::ALT;
    }
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        mods |= KeyMods::CTRL;
    }
    let code = match ev.code {
        CtKeyCode::Char(c) => KeyCode::Char(c),
        CtKeyCode::F(n) => KeyCode::F(n),
        CtKeyCode::Enter => KeyCode::Enter,
        CtKeyCode::Esc => KeyCode::Escape,
        CtKeyCode::Tab => KeyCode::Tab,
        CtKeyCode::BackTab => KeyCode::BackTab,
        CtKeyCode::Backspace => KeyCode::Backspace,
        CtKeyCode::Delete => KeyCode::Delete,
        CtKeyCode::Insert => KeyCode::Insert,
        CtKeyCode::Home => KeyCode::Home,
        CtKeyCode::End => KeyCode::End,
        CtKeyCode::PageUp => KeyCode::PageUp,
        CtKeyCode::PageDown => KeyCode::PageDown,
        CtKeyCode::Up => KeyCode::Up,
        CtKeyCode::Down => KeyCode::Down,
        CtKeyCode::Left => KeyCode::Left,
        CtKeyCode::Right => KeyCode::Right,
        _ => return None,
    };
    Some(KeyChord { code, mods })
}
