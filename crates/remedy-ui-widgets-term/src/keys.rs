//! What the terminal sends the program: keys as xterm encodes them, pastes, pointer reports
//! and focus changes.

use remedy_ui_core::input::{Key, KeyEvent, Modifiers};

use crate::term::{Modes, MouseEncoding, MouseMode};

/// xterm's modifier parameter: 1 plus Shift 1, Alt 2, Ctrl 4, Meta 8.
fn modifier_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl) + 8 * u8::from(m.meta)
}

/// A cursor-style key (arrows, Home, End, F1–F4): `CSI 1;m X` with modifiers, `SS3 X` in
/// application mode (or always, for F1–F4), `CSI X` otherwise.
fn cursor_key(letter: char, m: u8, ss3: bool) -> Vec<u8> {
    if m > 1 {
        format!("\x1b[1;{m}{letter}").into_bytes()
    } else if ss3 {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

/// A key sending `CSI n ~`, with `;m` for modifiers.
fn tilde_key(n: u8, m: u8) -> Vec<u8> {
    if m > 1 { format!("\x1b[{n};{m}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
}

/// The control character Ctrl with `c` types (Ctrl+A is 1, Ctrl+[ is ESC, Ctrl+Space is NUL).
fn control(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | '2' | ' ' | '`' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' | '~' => 0x1e,
        '_' | '-' | '7' | '/' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

/// The keypad's application sequences (DECKPAM), by physical key.
fn keypad(physical: &str) -> Option<u8> {
    Some(match physical {
        "Numpad0" => b'p',
        "Numpad1" => b'q',
        "Numpad2" => b'r',
        "Numpad3" => b's',
        "Numpad4" => b't',
        "Numpad5" => b'u',
        "Numpad6" => b'v',
        "Numpad7" => b'w',
        "Numpad8" => b'x',
        "Numpad9" => b'y',
        "NumpadDecimal" => b'n',
        "NumpadAdd" => b'k',
        "NumpadSubtract" => b'm',
        "NumpadMultiply" => b'j',
        "NumpadDivide" => b'o',
        "NumpadEnter" => b'M',
        _ => return None,
    })
}

/// The bytes xterm sends for key event `e` under `modes`; `None` when the key sends nothing
/// (a release, a key with Command or Super held, which belong to the app's shortcuts).
pub fn encode_key(e: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    if modes.kitty_flags != 0 {
        return encode_kitty(e, modes);
    }
    if !e.pressed || e.composing {
        return None;
    }
    let mods = e.modifiers;
    if mods.meta {
        return None;
    }
    let m = modifier_param(mods);
    let esc_if_alt = |mut v: Vec<u8>| {
        if mods.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    if modes.app_keypad
        && !mods.shift
        && !mods.ctrl
        && !mods.alt
        && let Some(c) = keypad(&e.physical)
    {
        return Some(vec![0x1b, b'O', c]);
    }
    let out = match &e.key {
        Key::ArrowUp => cursor_key('A', m, modes.app_cursor),
        Key::ArrowDown => cursor_key('B', m, modes.app_cursor),
        Key::ArrowRight => cursor_key('C', m, modes.app_cursor),
        Key::ArrowLeft => cursor_key('D', m, modes.app_cursor),
        Key::Home => cursor_key('H', m, modes.app_cursor),
        Key::End => cursor_key('F', m, modes.app_cursor),
        Key::PageUp => tilde_key(5, m),
        Key::PageDown => tilde_key(6, m),
        Key::Delete => tilde_key(3, m),
        Key::Named(n) if n == "Insert" => tilde_key(2, m),
        Key::F(n @ 1..=4) => {
            cursor_key(['P', 'Q', 'R', 'S'].get(usize::from(*n) - 1).copied().unwrap_or('P'), m, true)
        }
        Key::F(n @ 5..=20) => {
            const CODES: [u8; 16] = [15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34];
            tilde_key(CODES.get(usize::from(*n) - 5).copied().unwrap_or(15), m)
        }
        Key::F(_) => return None,
        Key::Enter => esc_if_alt(if modes.newline { b"\r\n".to_vec() } else { b"\r".to_vec() }),
        Key::Tab => {
            if mods.ctrl {
                // Ctrl+Tab moves keyboard focus out of the terminal.
                return None;
            }
            if mods.shift { b"\x1b[Z".to_vec() } else { esc_if_alt(b"\t".to_vec()) }
        }
        Key::Backspace => esc_if_alt(if mods.ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Escape => esc_if_alt(vec![0x1b]),
        Key::Space => esc_if_alt(if mods.ctrl { vec![0] } else { b" ".to_vec() }),
        Key::Character(k) => {
            let text = e.text.as_deref().unwrap_or(k.as_str());
            // AltGr reaches us as Ctrl+Alt with the character it typed: send that.
            let altgr = mods.ctrl && mods.alt && text.chars().next().is_some_and(|c| !c.is_control());
            if altgr {
                return Some(text.as_bytes().to_vec());
            }
            if mods.ctrl {
                let typed = text.chars().next().filter(|c| (*c as u32) < 0x20 || *c == '\x7f');
                let byte = match typed {
                    Some(c) => c as u8,
                    None => control(k.chars().next()?)?,
                };
                return Some(esc_if_alt(vec![byte]));
            }
            if text.is_empty() {
                return None;
            }
            esc_if_alt(text.as_bytes().to_vec())
        }
        // The system's back action is the app's, not the program's.
        Key::Back => return None,
        // Keys with no sequence of their own (named by the platform, or newer than this
        // terminal) send the text they typed, if any.
        _ => return e.text.as_ref().filter(|t| !t.is_empty()).map(|t| t.as_bytes().to_vec()),
    };
    Some(out)
}

/// A key under the kitty keyboard protocol (progressive enhancement): `CSI code[:shifted]
/// ; mods[:event] [; text] u` for what the flags ask to disambiguate or report, the legacy
/// bytes for the rest.
fn encode_kitty(e: &KeyEvent, modes: &Modes) -> Option<Vec<u8>> {
    let flags = modes.kitty_flags;
    let (events, alternates, all, text_flag) =
        (flags & 2 != 0, flags & 4 != 0, flags & 8 != 0, flags & 16 != 0);
    let m = e.modifiers;
    if e.composing || m.meta {
        return None;
    }
    let event = if !e.pressed {
        3
    } else if e.repeat {
        2
    } else {
        1
    };
    if event == 3 && !events {
        return None;
    }
    let mods = u32::from(modifier_param(m));
    // `;mods[:event]`, left out when both are the defaults.
    let tail = |with_text: bool| -> String {
        match (mods, event) {
            (1, 1) if !with_text => String::new(),
            (_, 1) => format!(";{mods}"),
            _ => format!(";{mods}:{event}"),
        }
    };
    let legacy_ok = !all && event == 1;
    let letter = |l: char| -> Vec<u8> {
        let t = tail(false);
        if t.is_empty() { format!("\x1b[{l}").into_bytes() } else { format!("\x1b[1{t}{l}").into_bytes() }
    };
    let tilde = |n: u32| -> Vec<u8> { format!("\x1b[{n}{}~", tail(false)).into_bytes() };
    let plain = mods == 1;
    let out = match &e.key {
        Key::ArrowUp | Key::ArrowDown | Key::ArrowRight | Key::ArrowLeft | Key::Home | Key::End
            if legacy_ok && plain =>
        {
            return encode_key(e, &Modes { kitty_flags: 0, ..*modes });
        }
        Key::ArrowUp => letter('A'),
        Key::ArrowDown => letter('B'),
        Key::ArrowRight => letter('C'),
        Key::ArrowLeft => letter('D'),
        Key::Home => letter('H'),
        Key::End => letter('F'),
        Key::F(1) => letter('P'),
        Key::F(2) => letter('Q'),
        Key::F(3) => tilde(13),
        Key::F(4) => letter('S'),
        Key::F(n @ 5..=20) => {
            const CODES: [u32; 16] = [15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34];
            tilde(CODES.get(usize::from(*n) - 5).copied().unwrap_or(15))
        }
        Key::F(_) | Key::Back => return None,
        Key::Named(n) if n == "Insert" => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        // Ctrl+Tab moves keyboard focus out of the terminal, as without the protocol.
        Key::Tab if m.ctrl => return None,
        // Enter, Tab and Backspace stay as they were unless every key is reported (or a
        // modifier makes them ambiguous); Escape is always disambiguated.
        Key::Enter | Key::Tab | Key::Backspace if legacy_ok && plain => {
            return encode_key(e, &Modes { kitty_flags: 0, ..*modes });
        }
        Key::Enter => format!("\x1b[13{}u", tail(false)).into_bytes(),
        Key::Tab => format!("\x1b[9{}u", tail(false)).into_bytes(),
        Key::Backspace => format!("\x1b[127{}u", tail(false)).into_bytes(),
        Key::Escape => format!("\x1b[27{}u", tail(false)).into_bytes(),
        Key::Space | Key::Character(_) => {
            let base = match &e.key {
                Key::Character(k) => k.chars().next()?,
                _ => ' ',
            };
            let text = e.text.as_deref().filter(|t| t.chars().all(|c| !c.is_control()));
            // Text typed with no modifier but Shift goes as text, unless every key is
            // reported; a release of such a key is reported only then.
            let only_shift = !m.ctrl && !m.alt;
            if !all && only_shift {
                return (event == 1).then(|| text.unwrap_or(" ").as_bytes().to_vec());
            }
            let mut code = format!("{}", u32::from(base.to_lowercase().next().unwrap_or(base)));
            if alternates
                && m.shift
                && let Some(shifted) = text.and_then(|t| t.chars().next()).filter(|c| *c != base)
            {
                code.push_str(&format!(":{}", u32::from(shifted)));
            }
            let with_text = all && text_flag && event != 3 && text.is_some();
            let mut s = format!("\x1b[{code}{}", tail(with_text));
            if with_text && let Some(t) = text {
                let cps: Vec<String> = t.chars().map(|c| u32::from(c).to_string()).collect();
                s.push(';');
                s.push_str(&cps.join(":"));
            }
            s.push('u');
            s.into_bytes()
        }
        // Keys the protocol has no code for here are not reported.
        _ => return None,
    };
    Some(out)
}

/// Pasted text as the program gets it. Control characters other than tab and line breaks
/// are dropped, so a paste cannot carry escape sequences or interrupt keys the person did not
/// see. With bracketed paste the text is wrapped in `CSI 200~` … `CSI 201~`; without it,
/// line breaks become carriage returns, as Enter sends.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r')).collect();
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        clean.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// What focus reporting sends (DECSET 1004).
pub fn encode_focus(focused: bool) -> &'static [u8] {
    if focused { b"\x1b[I" } else { b"\x1b[O" }
}

/// A pointer button, as reports name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The middle button.
    Middle,
    /// The secondary button.
    Right,
    /// The wheel turned up (away from the person).
    WheelUp,
    /// The wheel turned down.
    WheelDown,
    /// No button (a move with nothing held).
    None,
}

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseAction {
    /// A button went down (or the wheel turned).
    Press,
    /// A button came up.
    Release,
    /// The pointer moved to another cell.
    Move,
}

/// The report for a pointer event at cell (`col`, `row`) (from 0), when the program asked for
/// this kind of event; `None` otherwise.
pub fn encode_mouse(
    button: MouseButton,
    action: MouseAction,
    col: usize,
    row: usize,
    mods: Modifiers,
    modes: &Modes,
) -> Option<Vec<u8>> {
    let wheel = matches!(button, MouseButton::WheelUp | MouseButton::WheelDown);
    let wanted = match modes.mouse {
        MouseMode::Off => false,
        MouseMode::X10 => action == MouseAction::Press,
        MouseMode::Press => action != MouseAction::Move,
        MouseMode::Drag => action != MouseAction::Move || button != MouseButton::None,
        MouseMode::Motion => true,
    };
    if !wanted || (wheel && action == MouseAction::Release) {
        return None;
    }
    let base: u32 = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    let sgr = modes.mouse_encoding == MouseEncoding::Sgr;
    let mut b = if action == MouseAction::Release && !sgr { 3 } else { base };
    if action == MouseAction::Move {
        b += 32;
    }
    if modes.mouse != MouseMode::X10 {
        b += 4 * u32::from(mods.shift) + 8 * u32::from(mods.alt) + 16 * u32::from(mods.ctrl);
    }
    let (x, y) = (u32::try_from(col).ok()?.saturating_add(1), u32::try_from(row).ok()?.saturating_add(1));
    match modes.mouse_encoding {
        MouseEncoding::Sgr => {
            let end = if action == MouseAction::Release { 'm' } else { 'M' };
            Some(format!("\x1b[<{b};{x};{y}{end}").into_bytes())
        }
        MouseEncoding::Default => {
            // Positions past 223 cannot be said in one byte: the report is not sent.
            let enc = |v: u32| v.checked_add(32).and_then(|v| u8::try_from(v).ok());
            Some(vec![0x1b, b'[', b'M', enc(b)?, enc(x)?, enc(y)?])
        }
        MouseEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [b, x, y] {
                let c = v.checked_add(32).filter(|v| *v < 0x800).and_then(char::from_u32)?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Some(out)
        }
    }
}
