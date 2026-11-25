//! The screen model, case by case as vttest exercises a terminal: recorded escape sequences
//! in, the grid asserted out. Then reflow on resize, the scrollback's budget, replies, OSC
//! strings, and hostile input that must stay bounded.

use remedy_ui_widgets_term::*;

fn emu(cols: usize, rows: usize, input: &str) -> Emulator {
    let mut e = Emulator::new(cols, rows);
    e.feed(input.as_bytes());
    e
}

fn rows(e: &Emulator) -> Vec<String> {
    (0..e.size().1).map(|r| e.line(r).map(Line::text).unwrap_or_default()).collect()
}

fn cell(e: &Emulator, row: usize, col: usize) -> Cell {
    e.cell(row, col).copied().unwrap_or_else(|| unreachable!("cell {row},{col} exists"))
}

#[test]
fn text_prints_wraps_and_returns() {
    let e = emu(10, 3, "abcdefghijKL\r\nxy");
    assert_eq!(rows(&e), ["abcdefghij", "KL", "xy"]);
    assert!(e.line(0).is_some_and(Line::is_wrapped), "the first line ran on");
    assert!(!e.line(1).is_some_and(Line::is_wrapped));
    let c = e.cursor();
    assert_eq!((c.row, c.col), (2, 2));
    // At the last column the cursor waits: a carriage return does not wrap first.
    let e = emu(5, 2, "abcde\rX");
    assert_eq!(rows(&e), ["Xbcde", ""]);
    // With autowrap off the last column is overwritten.
    let e = emu(5, 2, "\x1b[?7labcdefg");
    assert_eq!(rows(&e), ["abcdg", ""]);
}

#[test]
fn cursor_movement_and_erasing() {
    // CUP, CUU/CUD/CUF/CUB, CHA, VPA, and erase in line and display.
    let e = emu(10, 4, "\x1b[2;3HX\x1b[AY\x1b[2BZ\x1b[3D!\x1b[8G@\x1b[1dT");
    assert_eq!(rows(&e), ["   Y    T", "  X", "  ! Z  @", ""]);
    let e = emu(6, 3, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[K");
    assert_eq!(rows(&e), ["aaaaaa", "bb", "cccccc"]);
    let e = emu(6, 3, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[1K");
    assert_eq!(rows(&e), ["aaaaaa", "   bbb", "cccccc"]);
    let e = emu(6, 3, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[J");
    assert_eq!(rows(&e), ["aaaaaa", "bb", ""]);
    let e = emu(6, 3, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[1J");
    assert_eq!(rows(&e), ["", "   bbb", "cccccc"]);
    let e = emu(6, 3, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2J");
    assert_eq!(rows(&e), ["", "", ""]);
    // ECH blanks in place.
    let e = emu(6, 1, "abcdef\x1b[1;2H\x1b[3X");
    assert_eq!(rows(&e), ["a   ef"]);
}

#[test]
fn insert_and_delete_characters_and_lines() {
    let e = emu(8, 1, "abcdef\x1b[1;3H\x1b[2@");
    assert_eq!(rows(&e), ["ab  cdef"]);
    let e = emu(8, 1, "abcdefgh\x1b[1;3H\x1b[2P");
    assert_eq!(rows(&e), ["abefgh"]);
    let e = emu(3, 4, "1\r\n2\r\n3\r\n4\x1b[2;1H\x1b[L");
    assert_eq!(rows(&e), ["1", "", "2", "3"]);
    let e = emu(3, 4, "1\r\n2\r\n3\r\n4\x1b[2;1H\x1b[2M");
    assert_eq!(rows(&e), ["1", "4", "", ""]);
    // Insert mode shifts what follows.
    let e = emu(8, 1, "abcd\x1b[1;2H\x1b[4hXY\x1b[4lZ");
    assert_eq!(rows(&e), ["aXYZcd"]);
    // REP repeats the last character.
    let e = emu(8, 1, "x\x1b[4b");
    assert_eq!(rows(&e), ["xxxxx"]);
}

#[test]
fn scroll_regions_scroll_only_inside() {
    // DECSTBM 2..3: a line feed at its bottom scrolls rows 2-3 and leaves 1 and 4.
    let e = emu(4, 4, "top\r\nr2\r\nr3\r\nbot\x1b[2;3r\x1b[3;1H\nnew");
    assert_eq!(rows(&e), ["top", "r3", "new", "bot"]);
    assert_eq!(e.history_len(), 0, "a region's lines never reach the scrollback");
    // Reverse index at the region's top scrolls it down.
    let e = emu(4, 4, "top\r\nr2\r\nr3\r\nbot\x1b[2;3r\x1b[2;1H\x1bMup");
    assert_eq!(rows(&e), ["top", "up", "r2", "bot"]);
    // SU and SD.
    let e = emu(3, 3, "a\r\nb\r\nc\x1b[S");
    assert_eq!(rows(&e), ["b", "c", ""]);
    let e = emu(3, 3, "a\r\nb\r\nc\x1b[T");
    assert_eq!(rows(&e), ["", "a", "b"]);
    // Origin mode addresses inside the region.
    let e = emu(4, 5, "\x1b[2;4r\x1b[?6h\x1b[1;1HO");
    assert_eq!(rows(&e), ["", "O", "", "", ""]);
}

#[test]
fn full_screen_scrolling_feeds_the_scrollback() {
    let mut e = Emulator::new(5, 3);
    for i in 0..10 {
        e.feed(format!("l{i}\r\n").as_bytes());
    }
    assert_eq!(rows(&e), ["l8", "l9", ""]);
    assert_eq!(e.history_len(), 8);
    assert_eq!(e.history_line(0).map(Line::text).as_deref(), Some("l0"));
    // ED 3 clears it.
    e.feed(b"\x1b[3J");
    assert_eq!(e.history_len(), 0);
}

#[test]
fn tabs_stop_where_set() {
    let e = emu(20, 1, "a\tb\tc");
    assert_eq!(rows(&e), ["a       b       c"]);
    // Clear all, set one at column 4, tab, back tab.
    let e = emu(20, 1, "\x1b[3g\x1b[1;5H\x1bH\x1b[1;1H\tX\x1b[Z\x1b[ZY");
    assert_eq!(rows(&e), ["Y   X"]);
}

#[test]
fn sgr_colors_and_styles() {
    let e = emu(
        20,
        1,
        "\x1b[31;1mA\x1b[38;5;208mB\x1b[38;2;1;2;3mC\x1b[48:2::9:8:7mD\x1b[0;3;4:3;7;2mE\x1b[0m F\x1b[9;53;58;5;4mG",
    );
    let a = cell(&e, 0, 0);
    assert_eq!(a.fg, TermColor::Indexed(1));
    assert!(a.attrs.contains(Attrs::BOLD));
    assert_eq!(cell(&e, 0, 1).fg, TermColor::Indexed(208));
    assert_eq!(cell(&e, 0, 2).fg, TermColor::Rgb(1, 2, 3));
    assert_eq!(cell(&e, 0, 3).bg, TermColor::Rgb(9, 8, 7));
    let ecell = cell(&e, 0, 4);
    assert!(
        ecell.attrs.contains(Attrs::ITALIC)
            && ecell.attrs.contains(Attrs::INVERSE)
            && ecell.attrs.contains(Attrs::DIM)
    );
    assert!(!ecell.attrs.contains(Attrs::BOLD));
    assert_eq!(ecell.attrs.underline(), Underline::Curly);
    assert_eq!(ecell.fg, TermColor::Default);
    let f = cell(&e, 0, 6);
    assert_eq!((f.fg, f.bg, f.attrs), (TermColor::Default, TermColor::Default, Attrs::NONE));
    let g = cell(&e, 0, 7);
    assert!(g.attrs.contains(Attrs::STRIKE) && g.attrs.contains(Attrs::OVERLINE));
    assert_eq!(g.underline_color, TermColor::Indexed(4));
    // Bright colors and resets of single attributes.
    let e = emu(8, 1, "\x1b[94;103;1;22;4;24mZ");
    let z = cell(&e, 0, 0);
    assert_eq!((z.fg, z.bg), (TermColor::Indexed(12), TermColor::Indexed(11)));
    assert_eq!(z.attrs, Attrs::NONE);
    // Erasing fills with the current background.
    let e = emu(4, 1, "\x1b[44m\x1b[K");
    assert_eq!(cell(&e, 0, 3).bg, TermColor::Indexed(4));
}

#[test]
fn the_alternate_screen_keeps_the_primary() {
    let mut e = emu(8, 3, "shell$ \x1b[?1049h\x1b[Hfull\x1b[2;1Hscreen");
    assert!(e.modes().alt_screen);
    assert_eq!(rows(&e), ["full", "screen", ""]);
    e.feed(b"\x1b[?1049l");
    assert!(!e.modes().alt_screen);
    assert_eq!(rows(&e), ["shell$", "", ""]);
    let c = e.cursor();
    assert_eq!((c.row, c.col), (0, 7), "the cursor came back where it was");
    // Scrolling on the alternate screen never reaches the scrollback.
    let mut e = Emulator::new(4, 2);
    e.feed(b"\x1b[?1049h1\r\n2\r\n3\r\n4\r\n");
    assert_eq!(e.history_len(), 0);
}

#[test]
fn modes_set_and_reset() {
    let e = emu(
        4,
        2,
        "\x1b[?1h\x1b=\x1b[?25l\x1b[?2004h\x1b[?1006h\x1b[?1002h\x1b[?1004h\x1b[?2026h\x1b[20h\x1b[5 q",
    );
    let m = e.modes();
    assert!(m.app_cursor && m.app_keypad && !m.cursor_visible && m.bracketed_paste && m.focus_events);
    assert!(m.synchronized && m.newline);
    assert_eq!(m.mouse, MouseMode::Drag);
    assert_eq!(m.mouse_encoding, MouseEncoding::Sgr);
    assert_eq!(e.cursor().shape, CursorShape::Bar);
    assert!(e.cursor().blink);
    let e = emu(4, 2, "\x1b[?1h\x1b[?1l\x1b[?2004h\x1b[?2004l\x1b[2 q");
    assert!(!e.modes().app_cursor && !e.modes().bracketed_paste);
    assert_eq!(e.cursor().shape, CursorShape::Block);
    assert!(!e.cursor().blink);
}

#[test]
fn line_drawing_and_the_uk_set() {
    let e = emu(8, 2, "\x1b(0lqqk\x1b(B ok\r\n\x1b)A\x0e#\x0f#");
    assert_eq!(rows(&e), ["┌──┐ ok", "£#"]);
}

#[test]
fn screen_alignment_fills_with_e() {
    let e = emu(3, 2, "\x1b#8");
    assert_eq!(rows(&e), ["EEE", "EEE"]);
}

#[test]
fn wide_characters_take_two_cells_and_combining_marks_join() {
    let e = emu(6, 2, "日本x");
    assert_eq!(rows(&e), ["日本x", ""]);
    assert!(cell(&e, 0, 0).attrs.contains(Attrs::WIDE));
    assert!(cell(&e, 0, 1).attrs.contains(Attrs::WIDE_SPACER));
    assert_eq!(e.cursor().col, 5);
    // Writing over half of a wide character blanks the other half.
    let e = emu(6, 1, "日本\x1b[1;2Hx");
    assert_eq!(rows(&e), [" x本"]);
    // A wide character that does not fit wraps whole.
    let e = emu(5, 2, "abcd日");
    assert_eq!(rows(&e), ["abcd", "日"]);
    // A combining accent stays with its letter.
    let e = emu(6, 1, "e\u{301}x");
    assert_eq!(rows(&e), ["e\u{301}x"]);
    assert_eq!(e.line(0).and_then(|l| l.grapheme(0)).as_deref(), Some("e\u{301}"));
    assert_eq!(e.cursor().col, 2);
}

#[test]
fn queries_are_answered() {
    let mut e = emu(10, 5, "\x1b[3;4H\x1b[6n\x1b[c\x1b[>c\x1b[5n\x1b[?2026$p\x1b[?9999$p\x1b[18t");
    let r = String::from_utf8(e.take_replies()).unwrap_or_default();
    assert_eq!(r, "\x1b[3;4R\x1b[?62;22c\x1b[>1;10;0c\x1b[0n\x1b[?2026;2$y\x1b[?9999;0$y\x1b[8;5;10t");
    // Color queries answer with the colors in use, as the query ended.
    e.feed(b"\x1b]4;1;rgb:12/34/56\x07\x1b]4;1;?\x07\x1b]10;?\x1b\\");
    let r = String::from_utf8(e.take_replies()).unwrap_or_default();
    assert!(r.starts_with("\x1b]4;1;rgb:1212/3434/5656\x07"), "{r:?}");
    assert!(r.contains("\x1b]10;rgb:") && r.ends_with("\x1b\\"), "{r:?}");
}

#[test]
fn titles_links_and_clipboard_requests() {
    let mut e = emu(20, 2, "\x1b]0;my \x07title\x07\x1b]2;build; ok\x1b\\");
    assert_eq!(e.title(), "build; ok");
    // Titles push and pop.
    e.feed(b"\x1b[22t\x1b]2;vim\x07\x1b[23t");
    assert_eq!(e.title(), "build; ok");
    // OSC 8: the cells carry the link; an empty URI ends it.
    e.feed(b"\x1b[2J\x1b[H\x1b]8;id=a;https://example.com/a;b\x1b\\link\x1b]8;;\x1b\\ plain");
    let link = cell(&e, 0, 0).link;
    assert_ne!(link, 0);
    assert_eq!(e.link(link), Some("https://example.com/a;b"));
    assert_eq!(cell(&e, 0, 3).link, link);
    assert_eq!(cell(&e, 0, 5).link, 0);
    // OSC 52 asks to write the clipboard; it is only a request. Reading is refused.
    e.feed(b"\x1b]52;c;aGVsbG8=\x07\x1b]52;c;?\x07");
    assert_eq!(e.take_clipboard_request().as_deref(), Some("hello"));
    assert!(e.take_replies().is_empty(), "the clipboard is never read to the program");
}

#[test]
fn soft_and_hard_resets() {
    let mut e = emu(6, 2, "\x1b[?25l\x1b[4h\x1b[31mab");
    e.feed(b"\x1b[!p");
    assert!(e.cursor().visible && !e.modes().insert);
    e.feed(b"x");
    assert_eq!(cell(&e, 0, 2).fg, TermColor::Default);
    e.feed(b"\x1bc");
    assert_eq!(rows(&e), ["", ""]);
}

#[test]
fn resizing_reflows_wrapped_lines() {
    let mut e = Emulator::new(10, 4);
    e.feed(b"0123456789abcdef\r\nnext");
    assert_eq!(rows(&e), ["0123456789", "abcdef", "next", ""]);
    // Narrower: the long line wraps again, the cursor stays after `next`.
    e.resize(6, 4);
    assert_eq!(rows(&e), ["012345", "6789ab", "cdef", "next"]);
    let c = e.cursor();
    assert_eq!((c.row, c.col), (3, 4));
    // Wider: it joins up again.
    e.resize(20, 4);
    assert_eq!(rows(&e), ["0123456789abcdef", "next", "", ""]);
    assert_eq!((e.cursor().row, e.cursor().col), (1, 4));
    // Shorter: lines above the cursor go to the scrollback; taller brings them back.
    e.resize(20, 1);
    assert_eq!(rows(&e), ["next"]);
    assert_eq!(e.history_len(), 1);
    e.resize(20, 3);
    assert_eq!(rows(&e), ["0123456789abcdef", "next", ""]);
    assert_eq!(e.history_len(), 0);
    // Wide characters never split across lines.
    let mut e = emu(6, 3, "ab日本語");
    e.resize(5, 3);
    assert_eq!(rows(&e), ["ab日", "本語", ""]);
    // The alternate screen is cut, not reflowed, and the primary under it still reflows.
    let mut e = emu(8, 2, "abcdefghij\x1b[?1049h\x1b[HALTALTALT");
    e.resize(4, 2);
    assert_eq!(rows(&e), ["ALTA", "T"]);
    e.feed(b"\x1b[?1049l");
    assert_eq!(rows(&e), ["efgh", "ij"]);
}

#[test]
fn the_scrollback_keeps_to_its_budget() {
    let mut e = Emulator::new(20, 5);
    e.set_scrollback(Scrollback::Lines(100));
    for i in 0..1000 {
        e.feed(format!("line {i}\r\n").as_bytes());
    }
    assert_eq!(e.history_len(), 100);
    assert_eq!(e.dropped_lines(), 896);
    assert_eq!(e.history_line(0).map(Line::text).as_deref(), Some("line 896"));
    // By bytes: the budget holds whatever the lines are made of.
    e.set_scrollback(Scrollback::Bytes(4096));
    assert!(e.history_bytes() <= 4096);
    for _ in 0..500 {
        e.feed(b"0123456789 0123456789 0123456789\r\n");
    }
    assert!(e.history_bytes() <= 4096, "{}", e.history_bytes());
    e.set_scrollback(Scrollback::Off);
    assert_eq!(e.history_len(), 0);
    // Auto sizes from the machine and is never zero.
    let mut e = Emulator::new(20, 5);
    for i in 0..2000 {
        e.feed(format!("auto {i}\r\n").as_bytes());
    }
    assert_eq!(e.history_len(), 1996);
}

#[test]
fn hostile_input_stays_bounded() {
    let mut e = Emulator::new(80, 24);
    e.set_string_limit(64 * 1024);
    // Huge parameters are clamped to the screen.
    e.feed(b"\x1b[65535;65535H\x1b[99999999999@\x1b[65535L\x1b[65535M\x1b[65535P\x1b[65535X\x1b[65535S\x1b[65535T");
    let c = e.cursor();
    assert!(c.row < 24 && c.col < 80);
    // A repeat count of 65535 costs at most a screenful.
    let before = e.history_len();
    e.feed(b"x\x1b[65535b");
    assert!(e.history_len() - before <= 25, "{} lines", e.history_len() - before);
    // An OSC that never ends is cut at the limit, and what follows its end shows.
    let mut e = Emulator::new(80, 24);
    e.set_string_limit(64 * 1024);
    e.feed(b"\x1b]2;");
    let chunk = vec![b'A'; 1 << 16];
    for _ in 0..64 {
        e.feed(&chunk);
    }
    e.feed(b"\x07after");
    assert_eq!(e.title(), "", "the over-long title was dropped");
    assert_eq!(e.line(0).map(Line::text).as_deref(), Some("after"));
    // Thousands of parameters, endless intermediates, nested and broken sequences.
    let mut e = Emulator::new(40, 10);
    let mut junk = b"\x1b[".to_vec();
    junk.extend(std::iter::repeat_n(b"1;".as_slice(), 100_000).flatten());
    junk.extend_from_slice(
        b"m\x1b[$$$$$$$$$$p\x1bP0;1;2qdata\x1b\\\x1b[?1;2;3;4;5;6;7;8;9h\x9b\xff\xfe\x1b]8;;",
    );
    e.feed(&junk);
    e.feed(b"\x07\r\nok");
    assert!(rows(&e).iter().any(|r| r == "ok"), "{:?}", rows(&e));
    // A flood of links is swept as they scroll away.
    let mut e = Emulator::new(20, 2);
    e.set_scrollback(Scrollback::Lines(10));
    for i in 0..20_000 {
        e.feed(format!("\x1b]8;;https://e.com/{i}\x07x\x1b]8;;\x07\r\n").as_bytes());
    }
    let last = (0..2).filter_map(|r| e.cell(r, 0)).map(|c| c.link).max().unwrap_or(0);
    assert!(last < 1_000, "links are renumbered as old ones go: {last}");
    // Random bytes never panic and leave a valid screen.
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut e = Emulator::new(50, 20);
    let mut buf = vec![0u8; 1 << 20];
    for _ in 0..8 {
        for b in &mut buf {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *b = if seed.is_multiple_of(5) { 0x1b } else { (seed >> 24) as u8 };
        }
        e.feed(&buf);
    }
    assert_eq!(e.size(), (50, 20));
    for r in 0..20 {
        assert_eq!(e.line(r).map(|l| l.cells().len()), Some(50));
    }
}

#[test]
fn keys_encode_as_xterm_does() {
    use remedy_ui_core::input::{Key, KeyEvent, Modifiers};
    let key = |k: Key, m: Modifiers, text: Option<&str>| {
        KeyEvent::press(k).with_modifiers(m).with_text(text.map(str::to_owned))
    };
    let normal = Modes::default();
    let app = {
        let mut v_ = Modes::default();
        v_.app_cursor = true;
        v_
    };
    let enc = |e: &KeyEvent, m: &Modes| encode_key(e, m).map(|b| String::from_utf8_lossy(&b).into_owned());
    let none = Modifiers::NONE;
    let ctrl = Modifiers { ctrl: true, ..none };
    let shift = Modifiers { shift: true, ..none };
    let alt = Modifiers { alt: true, ..none };
    assert_eq!(enc(&key(Key::ArrowUp, none, None), &normal).as_deref(), Some("\x1b[A"));
    assert_eq!(enc(&key(Key::ArrowUp, none, None), &app).as_deref(), Some("\x1bOA"));
    assert_eq!(enc(&key(Key::ArrowLeft, ctrl, None), &app).as_deref(), Some("\x1b[1;5D"));
    assert_eq!(enc(&key(Key::Home, shift, None), &normal).as_deref(), Some("\x1b[1;2H"));
    assert_eq!(enc(&key(Key::Delete, none, None), &normal).as_deref(), Some("\x1b[3~"));
    assert_eq!(enc(&key(Key::PageDown, alt, None), &normal).as_deref(), Some("\x1b[6;3~"));
    assert_eq!(enc(&key(Key::F(1), none, None), &normal).as_deref(), Some("\x1bOP"));
    assert_eq!(enc(&key(Key::F(5), none, None), &normal).as_deref(), Some("\x1b[15~"));
    assert_eq!(enc(&key(Key::F(12), ctrl, None), &normal).as_deref(), Some("\x1b[24;5~"));
    assert_eq!(enc(&key(Key::Character("c".into()), ctrl, None), &normal).as_deref(), Some("\x03"));
    assert_eq!(enc(&key(Key::Character("[".into()), ctrl, None), &normal).as_deref(), Some("\x1b"));
    assert_eq!(enc(&key(Key::Space, ctrl, None), &normal).as_deref(), Some("\0"));
    assert_eq!(enc(&key(Key::Character("x".into()), alt, Some("x")), &normal).as_deref(), Some("\x1bx"));
    assert_eq!(enc(&key(Key::Character("a".into()), shift, Some("A")), &normal).as_deref(), Some("A"));
    // AltGr arrives as Ctrl+Alt with the character it typed.
    let altgr = Modifiers { ctrl: true, alt: true, ..none };
    assert_eq!(enc(&key(Key::Character("q".into()), altgr, Some("@")), &normal).as_deref(), Some("@"));
    assert_eq!(enc(&key(Key::Backspace, none, None), &normal).as_deref(), Some("\x7f"));
    assert_eq!(enc(&key(Key::Backspace, ctrl, None), &normal).as_deref(), Some("\x08"));
    assert_eq!(enc(&key(Key::Tab, shift, None), &normal).as_deref(), Some("\x1b[Z"));
    assert_eq!(enc(&key(Key::Tab, ctrl, None), &normal), None, "Ctrl+Tab leaves the terminal");
    assert_eq!(enc(&key(Key::Enter, none, None), &normal).as_deref(), Some("\r"));
    let lnm = {
        let mut v_ = Modes::default();
        v_.newline = true;
        v_
    };
    assert_eq!(enc(&key(Key::Enter, none, None), &lnm).as_deref(), Some("\r\n"));
    assert_eq!(enc(&key(Key::Named("Insert".into()), none, None), &normal).as_deref(), Some("\x1b[2~"));
    // The keypad in application mode.
    let kp = {
        let mut v_ = Modes::default();
        v_.app_keypad = true;
        v_
    };
    let mut five = key(Key::Character("5".into()), none, Some("5"));
    five.physical = "Numpad5".into();
    assert_eq!(enc(&five, &kp).as_deref(), Some("\x1bOu"));
    assert_eq!(enc(&five, &normal).as_deref(), Some("5"));
    // Command and Super belong to the app; releases and composition send nothing.
    let meta = Modifiers { meta: true, ..none };
    assert_eq!(enc(&key(Key::Character("c".into()), meta, Some("c")), &normal), None);
    let mut up = key(Key::Character("a".into()), none, Some("a"));
    up.pressed = false;
    assert_eq!(enc(&up, &normal), None);
}

#[test]
fn pastes_pointer_reports_and_focus() {
    assert_eq!(encode_paste("a\r\nb\nc", false), b"a\rb\rc");
    // Bracketed, with escape sequences inside the paste removed: a paste cannot end the
    // bracket early or type keys the person did not see.
    assert_eq!(encode_paste("x\x1b[201~rm -rf\x03\ty", true), b"\x1b[200~x[201~rm -rf\ty\x1b[201~");
    let sgr = {
        let mut v_ = Modes::default();
        v_.mouse = MouseMode::Press;
        v_.mouse_encoding = MouseEncoding::Sgr;
        v_
    };
    let none = remedy_ui_core::input::Modifiers::NONE;
    let enc =
        |b, a, m: &Modes| encode_mouse(b, a, 4, 2, none, m).map(|v| String::from_utf8_lossy(&v).into_owned());
    assert_eq!(enc(MouseButton::Left, MouseAction::Press, &sgr).as_deref(), Some("\x1b[<0;5;3M"));
    assert_eq!(enc(MouseButton::Left, MouseAction::Release, &sgr).as_deref(), Some("\x1b[<0;5;3m"));
    assert_eq!(enc(MouseButton::WheelUp, MouseAction::Press, &sgr).as_deref(), Some("\x1b[<64;5;3M"));
    assert_eq!(enc(MouseButton::Left, MouseAction::Move, &sgr), None, "no moves in press mode");
    let drag = {
        let mut v_ = sgr;
        v_.mouse = MouseMode::Drag;
        v_
    };
    assert_eq!(enc(MouseButton::Left, MouseAction::Move, &drag).as_deref(), Some("\x1b[<32;5;3M"));
    let x10 = {
        let mut v_ = Modes::default();
        v_.mouse = MouseMode::Press;
        v_
    };
    assert_eq!(
        encode_mouse(MouseButton::Right, MouseAction::Press, 4, 2, none, &x10),
        Some(b"\x1b[M\"%#".to_vec())
    );
    assert_eq!(
        encode_mouse(MouseButton::Left, MouseAction::Release, 4, 2, none, &x10),
        Some(b"\x1b[M#%#".to_vec())
    );
    assert_eq!(
        encode_mouse(MouseButton::Left, MouseAction::Press, 300, 2, none, &x10),
        None,
        "too far to say in a byte"
    );
    assert_eq!(encode_mouse(MouseButton::Left, MouseAction::Press, 4, 2, none, &Modes::default()), None);
    assert_eq!(encode_focus(true), b"\x1b[I");
    assert_eq!(encode_focus(false), b"\x1b[O");
}

#[test]
fn several_clipboard_requests_in_one_chunk_all_arrive_in_order() {
    let mut e = emu(20, 2, "");
    e.feed(b"\x1b]52;c;b25l\x07\x1b]52;c;dHdv\x07\x1b]52;c;dGhyZWU=\x07");
    assert_eq!(e.take_clipboard_request().as_deref(), Some("one"));
    assert_eq!(e.take_clipboard_requests(), ["two", "three"]);
    assert!(e.take_clipboard_requests().is_empty());
    assert_eq!(e.take_clipboard_request(), None);
}

#[test]
fn screen_text_leaves_out_trailing_blank_rows_only() {
    let mut e = emu(10, 5, "");
    assert_eq!(e.screen_text(), "", "an empty screen has no text");
    e.feed(b"\x1b[2;1Hmid\x1b[3;1H   ");
    assert_eq!(e.screen_text(), "\nmid", "the blank row before stays, those after go");
}
