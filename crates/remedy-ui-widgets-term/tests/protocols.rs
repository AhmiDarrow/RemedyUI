//! The screen model's richer protocols: device control strings (DECRQSS, sixel), left and
//! right margins, 132 columns, the kitty keyboard protocol, the shell's working directory
//! and prompt marks, emoji sequences, and scrollback given back under memory pressure.

use remedy_ui_core::input::{Key, KeyEvent, Modifiers};
use remedy_ui_widgets_term::*;

fn emu(cols: usize, rows: usize, input: &str) -> Emulator {
    let mut e = Emulator::new(cols, rows);
    e.feed(input.as_bytes());
    e
}

fn rows(e: &Emulator) -> Vec<String> {
    (0..e.size().1).map(|r| e.line(r).map(Line::text).unwrap_or_default()).collect()
}

fn replies(e: &mut Emulator) -> String {
    String::from_utf8_lossy(&e.take_replies()).into_owned()
}

#[test]
fn decrqss_reports_settings_and_other_dcs_is_dropped_safely() {
    let mut e = emu(20, 5, "\x1b[1;31;48;5;200m\x1bP$qm\x1b\\");
    assert_eq!(replies(&mut e), "\x1bP1$r0;1;31;48;5;200m\x1b\\");
    e.feed(b"\x1b[0m\x1b[2;4r\x1bP$qr\x1b\\\x1b[5 q\x1bP$q q\x1b\\\x1bP$q\"p\x1b\\");
    assert_eq!(replies(&mut e), "\x1bP1$r2;4r\x1b\\\x1bP1$r5 q\x1b\\\x1bP1$r62;1\"p\x1b\\");
    e.feed(b"\x1b[?69h\x1b[3;9s\x1bP$qs\x1b\\\x1bP$qzz\x1b\\");
    assert_eq!(replies(&mut e), "\x1bP1$r3;9s\x1b\\\x1bP0$r\x1b\\");
    // A long request is not a setting: no reply beyond "not valid", nothing kept.
    e.feed(format!("\x1bP$q{}\x1b\\", "x".repeat(100_000)).as_bytes());
    assert_eq!(replies(&mut e), "\x1bP0$r\x1b\\");
    // Other device control strings (DECUDK, ReGIS, …) are parsed and dropped.
    let mut e = emu(20, 2, "\x1bP1;1|17/616263\x1b\\\x1bP1pS(E)P[100,100]V[200,200]\x1b\\ok");
    assert_eq!(rows(&e).first().map(String::as_str), Some("ok"));
    assert!(e.take_replies().is_empty());
}

#[test]
fn sixel_pictures_decode_and_show_in_cells() {
    // Red, two bands of six rows (the second half drawn), four pixels wide.
    let mut e = emu(20, 6, "ab\x1bPq#0;2;100;0;0~~~~-~~~~\x1b\\");
    let piece =
        e.line(0).and_then(|l| l.pictures().first().cloned()).unwrap_or_else(|| unreachable!("a picture"));
    let pic = &piece.picture;
    assert_eq!((pic.width, pic.height, piece.col, piece.band, piece.bands), (4, 12, 2, 0, 1));
    assert_eq!(pic.rgba.get(0..4), Some(&[255u8, 0, 0, 255][..]));
    // The cursor goes below the picture, at its column.
    let c = e.cursor();
    assert_eq!((c.row, c.col), (1, 2));
    // Raster attributes give the size: 30 × 45 covers 3 columns and 3 bands of 20-pixel cells.
    e.feed(b"\x1b[3;1H\x1bP0;1q\"1;1;30;45#1;2;0;0;100!30~\x1b\\");
    let pieces: Vec<usize> =
        (2..6).filter_map(|r| e.line(r)).flat_map(|l| l.pictures().iter().map(|p| p.band)).collect();
    assert_eq!(pieces, [0, 1, 2]);
    let big =
        e.line(2).and_then(|l| l.pictures().first().cloned()).unwrap_or_else(|| unreachable!("picture"));
    assert_eq!((big.picture.width, big.picture.height), (30, 45));
    // P2 = 1: what was not drawn is transparent; HLS colors (DEC hue 120 is red).
    let mut e = emu(20, 4, "\x1bP0;1q#1;1;120;50;100#1~\x1b\\");
    let pic = e
        .line(0)
        .and_then(|l| l.pictures().first().cloned())
        .unwrap_or_else(|| unreachable!("picture"))
        .picture;
    assert_eq!(pic.rgba.get(0..4), Some(&[255u8, 0, 0, 255][..]));
    e.feed(b"\x1b[2J");
    assert!(e.line(0).is_some_and(|l| l.pictures().is_empty()), "erasing the line removes the picture");
    // Hostile sizes stay inside the screen in pixels (80 × 24 cells of 10 × 20 here).
    let mut e = Emulator::new(80, 24);
    e.feed(b"\x1bPq\"1;1;99999;99999#0!65535~-!65535~\x1b\\");
    let pic = e
        .line(0)
        .and_then(|l| l.pictures().first().cloned())
        .unwrap_or_else(|| unreachable!("picture"))
        .picture;
    assert!(pic.width <= 800 && pic.height <= 480, "{}x{}", pic.width, pic.height);
    // Garbage in a sixel string never panics.
    e.feed(b"\x1bPq#99999999;7;1;2;3!!!!-$$$---#;;;;~~~\xff\x1b\\");
}

#[test]
fn left_and_right_margins_bound_scrolling_wrapping_and_editing() {
    // Columns 3–6 (from 1) scroll; the rest of each row stays.
    let e = emu(8, 3, "AAAAAAAA\r\nBBBBBBBB\r\nCCCCCCCC\x1b[?69h\x1b[3;6s\x1b[3;3H\n");
    assert_eq!(rows(&e), ["AABBBBAA", "BBCCCCBB", "CC    CC"]);
    assert_eq!(e.history_len(), 0, "a band never reaches the scrollback");
    // Text wraps at the right margin to the left one.
    let e = emu(8, 2, "\x1b[?69h\x1b[3;6s\x1b[1;3HXYZWVU");
    assert_eq!(rows(&e), ["  XYZW", "  VU"]);
    // Insert and delete characters shift only up to the right margin.
    let e = emu(8, 1, "abcdefgh\x1b[?69h\x1b[3;6s\x1b[1;3H\x1b[@");
    assert_eq!(rows(&e), ["ab cdegh"]);
    let e = emu(8, 1, "abcdefgh\x1b[?69h\x1b[3;6s\x1b[1;3H\x1b[P");
    assert_eq!(rows(&e), ["abdef gh"]);
    // Origin mode counts from the margins; insert line works inside them only.
    let e = emu(8, 3, "\x1b[?69h\x1b[3;6s\x1b[?6h\x1b[1;1HO");
    assert_eq!(rows(&e).first().map(String::as_str), Some("  O"));
    let e = emu(8, 3, "11111111\r\n22222222\r\n33333333\x1b[?69h\x1b[3;6s\x1b[1;3H\x1b[L");
    assert_eq!(rows(&e), ["11    11", "22111122", "33222233"]);
    // Without DECLRMM, CSI s saves the cursor as before.
    let e = emu(8, 2, "\x1b[2;4H\x1b[s\x1b[1;1H\x1b[uX");
    assert_eq!(rows(&e), ["", "   X"]);
    // Turning the mode off lifts the margins.
    let e = emu(8, 2, "\x1b[?69h\x1b[3;6s\x1b[?69l\x1b[1;1Habcdefghij");
    assert_eq!(rows(&e), ["abcdefgh", "ij"]);
}

#[test]
fn deccolm_switches_columns_only_when_allowed() {
    let mut e = emu(80, 5, "text\x1b[?3h");
    assert_eq!(e.size(), (80, 5), "not allowed without DECSET 40");
    assert_eq!(e.forced_columns(), None);
    e.feed(b"\x1b[?40h\x1b[?3h");
    assert_eq!(e.size(), (132, 5));
    assert_eq!(e.forced_columns(), Some(132));
    assert_eq!(rows(&e).concat(), "", "the screen clears");
    assert_eq!((e.cursor().row, e.cursor().col), (0, 0));
    e.feed(b"\x1b[?3$p");
    assert_eq!(replies(&mut e), "\x1b[?3;1$y");
    e.feed(b"\x1b[?3l");
    assert_eq!(e.size(), (80, 5));
    e.feed(b"\x1bc");
    assert_eq!(e.forced_columns(), None, "a full reset gives the columns back to the view");
}

fn key(k: Key, m: Modifiers, text: Option<&str>, pressed: bool, repeat: bool) -> KeyEvent {
    KeyEvent::press(k)
        .with_modifiers(m)
        .with_pressed(pressed)
        .with_repeat(repeat)
        .with_text(text.map(str::to_owned))
}

fn enc(e: &KeyEvent, flags: u8) -> Option<String> {
    let modes = {
        let mut v_ = Modes::default();
        v_.kitty_flags = flags;
        v_
    };
    encode_key(e, &modes).map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn the_kitty_keyboard_protocol_stacks_and_encodes() {
    let mut e = emu(20, 3, "\x1b[>1u");
    assert_eq!(e.modes().kitty_flags, 1);
    e.feed(b"\x1b[?u");
    assert_eq!(replies(&mut e), "\x1b[?1u");
    e.feed(b"\x1b[>3u");
    assert_eq!(e.modes().kitty_flags, 3);
    e.feed(b"\x1b[<u");
    assert_eq!(e.modes().kitty_flags, 1);
    e.feed(b"\x1b[=8;2u");
    assert_eq!(e.modes().kitty_flags, 9);
    e.feed(b"\x1b[=1;3u");
    assert_eq!(e.modes().kitty_flags, 8);
    // Each screen has its own stack.
    e.feed(b"\x1b[?1049h");
    assert_eq!(e.modes().kitty_flags, 0);
    e.feed(b"\x1b[?1049l");
    assert_eq!(e.modes().kitty_flags, 8);
    e.feed(b"\x1b[<10u");
    assert_eq!(e.modes().kitty_flags, 0);

    let none = Modifiers::NONE;
    let ctrl = Modifiers { ctrl: true, ..none };
    let shift = Modifiers { shift: true, ..none };
    // Disambiguate (1).
    assert_eq!(enc(&key(Key::Escape, none, None, true, false), 1).as_deref(), Some("\x1b[27u"));
    assert_eq!(
        enc(&key(Key::Character("c".into()), ctrl, None, true, false), 1).as_deref(),
        Some("\x1b[99;5u")
    );
    assert_eq!(enc(&key(Key::Character("a".into()), none, Some("a"), true, false), 1).as_deref(), Some("a"));
    assert_eq!(enc(&key(Key::ArrowUp, none, None, true, false), 1).as_deref(), Some("\x1b[A"));
    assert_eq!(enc(&key(Key::ArrowUp, shift, None, true, false), 1).as_deref(), Some("\x1b[1;2A"));
    assert_eq!(enc(&key(Key::Enter, none, None, true, false), 1).as_deref(), Some("\r"));
    assert_eq!(enc(&key(Key::F(3), none, None, true, false), 1).as_deref(), Some("\x1b[13~"));
    // Event types (2): releases and repeats.
    assert_eq!(
        enc(&key(Key::Character("c".into()), ctrl, None, false, false), 3).as_deref(),
        Some("\x1b[99;5:3u")
    );
    assert_eq!(enc(&key(Key::Character("a".into()), none, Some("a"), false, false), 3), None);
    assert_eq!(enc(&key(Key::ArrowUp, none, None, true, true), 3).as_deref(), Some("\x1b[1;1:2A"));
    assert_eq!(
        enc(&key(Key::Character("c".into()), ctrl, None, false, false), 1),
        None,
        "no releases without flag 2"
    );
    // Every key as an escape code (8), with the shifted key (4) and the text (16).
    assert_eq!(
        enc(&key(Key::Character("a".into()), none, Some("a"), true, false), 8).as_deref(),
        Some("\x1b[97u")
    );
    assert_eq!(enc(&key(Key::Enter, none, None, true, false), 8).as_deref(), Some("\x1b[13u"));
    assert_eq!(
        enc(&key(Key::Character("a".into()), shift, Some("A"), true, false), 12).as_deref(),
        Some("\x1b[97:65;2u")
    );
    assert_eq!(
        enc(&key(Key::Character("a".into()), none, Some("a"), true, false), 24).as_deref(),
        Some("\x1b[97;1;97u")
    );
    assert_eq!(
        enc(&key(Key::Character("a".into()), none, Some("a"), false, false), 10).as_deref(),
        Some("\x1b[97;1:3u")
    );
    // Ctrl+Tab still leaves the terminal.
    assert_eq!(enc(&key(Key::Tab, ctrl, None, true, false), 9), None);
}

#[test]
fn the_shell_reports_its_directory_and_marks_prompts() {
    let mut e = emu(30, 10, "\x1b]7;file://host/home/me/my%20dir\x07");
    assert_eq!(e.cwd(), Some("/home/me/my dir"));
    e.feed(
        b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out1\r\nout2\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ",
    );
    let marks = e.prompt_lines();
    let at = |k: u64| marks.iter().find(|(l, _)| *l == k).map(|(_, m)| m.clone()).unwrap_or_default();
    assert_eq!(at(0), [PromptMark::Prompt, PromptMark::Command]);
    assert_eq!(at(1), [PromptMark::Output]);
    assert_eq!(at(3), [PromptMark::Done(Some(0)), PromptMark::Prompt]);
    // The marks travel with their lines through a resize.
    e.resize(10, 10);
    assert_eq!(e.prompt_lines().len(), 3);
    // And through the scrollback.
    for _ in 0..20 {
        e.feed(b"\r\nmore");
    }
    assert!(e.prompt_lines().iter().any(|(_, m)| m.contains(&PromptMark::Output)));
}

#[test]
fn emoji_sequences_are_one_grapheme() {
    let e = emu(10, 1, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x");
    let line = e.line(0).unwrap_or_else(|| unreachable!("line"));
    assert_eq!(line.grapheme(0).as_deref(), Some("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"));
    assert!(line.cells().first().is_some_and(|c| c.attrs.contains(Attrs::WIDE)));
    assert_eq!(line.cells().get(2).map(|c| c.ch), Some('x'));
    // Flags: two regional indicators each.
    let e = emu(10, 1, "\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}x");
    let line = e.line(0).unwrap_or_else(|| unreachable!("line"));
    assert_eq!(line.grapheme(0).as_deref(), Some("\u{1F1E9}\u{1F1EA}"));
    assert_eq!(line.text(), "\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}x");
}

#[test]
fn memory_pressure_gives_scrollback_back() {
    let mut e = Emulator::new(20, 5);
    for i in 0..1000 {
        e.feed(format!("line {i}\r\n").as_bytes());
    }
    let before = e.history_len();
    e.relieve(remedy_ui_view::device::MemoryPressure::Moderate);
    assert_eq!(e.history_len(), before - before / 2);
    assert_eq!(e.history_line(e.history_len() - 1).map(Line::text).as_deref(), Some("line 995"));
    e.relieve(remedy_ui_view::device::MemoryPressure::Critical);
    assert_eq!(e.history_len(), 0);
    assert_eq!(rows(&e).first().map(String::as_str), Some("line 996"), "the screen stays");
}
