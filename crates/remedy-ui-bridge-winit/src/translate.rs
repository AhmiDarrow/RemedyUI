//! winit events into RemedyUI's event model.

use remedy_ui_input::{Key, Modifiers};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey, PhysicalKey};

/// Logical pixels one wheel notch (one "line") scrolls: three lines of 16 px interface text,
/// the long-standing desktop default. A tuning constant, not a cap.
pub(crate) const LINE_PIXELS: f32 = 48.0;

pub(crate) fn modifiers(m: ModifiersState) -> Modifiers {
    Modifiers { shift: m.shift_key(), ctrl: m.control_key(), alt: m.alt_key(), meta: m.super_key() }
}

pub(crate) fn key(k: &WKey) -> Key {
    match k {
        WKey::Character(s) => Key::Character(s.to_lowercase()),
        WKey::Named(n) => match n {
            NamedKey::Enter => Key::Enter,
            NamedKey::Tab => Key::Tab,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Space => Key::Space,
            NamedKey::ArrowLeft => Key::ArrowLeft,
            NamedKey::ArrowRight => Key::ArrowRight,
            NamedKey::ArrowUp => Key::ArrowUp,
            NamedKey::ArrowDown => Key::ArrowDown,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            other => match function_key(*other) {
                Some(f) => Key::F(f),
                None => Key::Named(format!("{other:?}")),
            },
        },
        WKey::Unidentified(native) => Key::Named(format!("{native:?}")),
        WKey::Dead(c) => Key::Named(c.map_or_else(|| "Dead".to_owned(), |c| format!("Dead{c}"))),
    }
}

fn function_key(n: NamedKey) -> Option<u8> {
    use NamedKey::*;
    let all = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22,
        F23, F24,
    ];
    all.iter().position(|k| *k == n).and_then(|i| u8::try_from(i + 1).ok())
}

pub(crate) fn physical(p: &PhysicalKey) -> String {
    match p {
        PhysicalKey::Code(c) => format!("{c:?}"),
        PhysicalKey::Unidentified(n) => format!("{n:?}"),
    }
}

/// A byte range inside `text` from winit's preedit cursor, kept on character boundaries.
pub(crate) fn preedit_cursor(text: &str, cursor: Option<(usize, usize)>) -> Option<std::ops::Range<usize>> {
    let (a, b) = cursor?;
    let (a, b) = (a.min(b), a.max(b));
    (text.is_char_boundary(a) && text.is_char_boundary(b) && b <= text.len()).then_some(a..b)
}

/// Premultiplied RGBA8 to softbuffer's 0RGB words (alpha already applied).
pub(crate) fn pack(p: &[u8]) -> u32 {
    match p {
        [r, g, b, _] => (u32::from(*r) << 16) | (u32::from(*g) << 8) | u32::from(*b),
        _ => 0,
    }
}
