//! The terminal element on a real view with the theme sheet: sizing from the font, rows on a
//! virtual list, painting from theme colors, keys and pastes out, selection and copying,
//! links and clipboard writes under the app's control, the input method, the wheel, focus
//! reports, screen readers, a feeding thread, and the accessibility audit.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Color, Point, Size, Vec2};
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase, WheelEvent};
use remedy_ui_paint::Item;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};
use remedy_ui_widgets_term::*;

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn find(el: &El, widget: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

fn collect(layer: &remedy_ui_compositor::Layer, out: &mut Vec<Item>) {
    out.extend(layer.content.items().iter().cloned());
    for c in &layer.children {
        collect(c, out);
    }
}

struct H {
    ui: Ui,
    term: Terminal,
    out: Rc<RefCell<Vec<u8>>>,
    sizes: Rc<RefCell<Vec<GridSize>>>,
    requests: Vec<Request>,
    timers: Vec<u64>,
    items: Vec<Item>,
    size: Size,
    now: u64,
}

impl H {
    fn new() -> H {
        H::with(TerminalOptions::default())
    }

    fn with(options: TerminalOptions) -> H {
        H::on(ui(), options)
    }

    fn on(ui: Ui, options: TerminalOptions) -> H {
        let term = terminal(&ui, options);
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        term.on_output(move |b| o.borrow_mut().extend_from_slice(b));
        let sizes = Rc::new(RefCell::new(Vec::new()));
        let s = sizes.clone();
        term.on_resize(move |g| s.borrow_mut().push(g));
        ui.root().child(ui.el("col").column().height(600.0).fill_width().child(term.element()));
        let mut h = H {
            ui,
            term,
            out,
            sizes,
            requests: Vec::new(),
            timers: Vec::new(),
            items: Vec::new(),
            size: Size::new(800.0, 600.0),
            now: 1_000,
        };
        h.frame();
        h
    }

    fn frame(&mut self) {
        for _ in 0..6 {
            let _ = remedy_ui_reactive::run_posted();
            let f = self.ui.frame(self.size, 1.0);
            if let Some(t) = f.layers {
                for r in &t.roots {
                    collect(r, &mut self.items);
                }
            }
            for r in self.ui.take_requests() {
                match r {
                    Request::ScrollTo { scroller, offset } => self.ui.set_scroll_offset(scroller, offset),
                    Request::Timer { id, .. } => self.timers.push(id),
                    Request::Frame => {}
                    other => self.requests.push(other),
                }
            }
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.term.feeder().feed(bytes);
        self.frame();
    }

    fn fire_timers(&mut self) {
        for id in std::mem::take(&mut self.timers) {
            self.ui.fire_timer(id);
        }
        self.frame();
    }

    fn out(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.out.borrow_mut())).into_owned()
    }

    fn copies(&mut self) -> Vec<String> {
        let mut v = Vec::new();
        self.requests.retain(|r| match r {
            Request::Copy { text, .. } => {
                v.push(text.clone());
                false
            }
            _ => true,
        });
        v
    }

    fn focus(&mut self) {
        self.term.input_element().focus();
        self.frame();
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, text: Option<&str>) -> bool {
        let handled =
            self.ui.key(&KeyEvent::press(key).with_modifiers(modifiers).with_text(text.map(str::to_owned)));
        self.frame();
        handled
    }

    /// The middle of screen cell (`row`, `col`), in window coordinates.
    fn at(&self, row: usize, col: usize) -> Point {
        let scroller = find(&self.ui.root(), "terminal_scroll").into_iter().next();
        let b = scroller
            .as_ref()
            .and_then(El::bounds)
            .unwrap_or_else(|| unreachable!("the scroller is laid out"));
        let offset = scroller.map_or(0.0, |s| self.ui.scroll_offset(s.id()).y);
        let g = self.term.size();
        let first = self.term.with_emulator(|e| e.history_len());
        let (cw, lh) = (g.cell_width as f32, g.cell_height as f32);
        Point::new(b.min.x + (col as f32 + 0.5) * cw, b.min.y + ((first + row) as f32 + 0.5) * lh - offset)
    }

    fn pointer(&mut self, phase: PointerPhase, p: Point, buttons: Buttons, modifiers: Modifiers) -> bool {
        self.now += 20;
        let handled = self.ui.pointer(
            &PointerEvent::new(phase, p)
                .with_buttons(buttons)
                .with_time(Duration::from_millis(self.now))
                .with_modifiers(modifiers),
        );
        self.frame();
        handled
    }

    fn click(&mut self, p: Point, modifiers: Modifiers) {
        self.pointer(PointerPhase::Down, p, Buttons::PRIMARY, modifiers);
        self.pointer(PointerPhase::Up, p, Buttons::default(), modifiers);
    }

    fn rows_built(&self) -> usize {
        find(&self.ui.root(), "terminal_row").len()
    }
}

const NONE: Modifiers = Modifiers::NONE;
const CTRL_SHIFT: Modifiers = Modifiers { ctrl: true, shift: true, alt: false, meta: false };

#[test]
fn the_grid_fits_the_view_and_the_app_hears_its_size() {
    let mut h = H::new();
    let g = h.term.size();
    assert!(g.cell_width > 0 && g.cell_height > 0, "{g:?}");
    // The cells fill the view: one more column or row would not fit.
    let scroller = find(&h.ui.root(), "terminal_scroll").into_iter().next().and_then(|s| s.bounds());
    let b = scroller.unwrap_or_else(|| unreachable!("laid out"));
    assert_eq!(g.cols, (b.width() / g.cell_width as f32).floor() as usize, "{g:?} in {b:?}");
    assert_eq!(g.rows, (b.height() / g.cell_height as f32).floor() as usize);
    assert_eq!(h.sizes.borrow().last().copied(), Some(g));
    assert_eq!(h.term.with_emulator(|e| e.size()), (g.cols, g.rows));
    // A narrower window: fewer columns, and the app is told.
    h.size = Size::new(400.0, 300.0);
    let narrow = find(&h.ui.root(), "col").into_iter().next().unwrap_or_else(|| unreachable!("the column"));
    let _ = narrow.height(300.0);
    h.frame();
    let g2 = h.term.size();
    assert!(g2.cols < g.cols && g2.rows < g.rows, "{g2:?} after {g:?}");
    assert_eq!(h.sizes.borrow().last().copied(), Some(g2));
}

#[test]
fn output_shows_in_theme_colors_on_rows_built_only_near_the_view() {
    let mut h = H::new();
    h.items.clear();
    h.feed(b"plain \x1b[41mred bg\x1b[0m \x1b[38;2;10;200;30mgreen\x1b[0m");
    assert_eq!(h.term.with_emulator(|e| e.line(0).map(Line::text)).as_deref(), Some("plain red bg green"));
    let theme = h.ui.theme();
    let red = ansi(&theme, 1);
    let fills = |items: &[Item], c: Color| {
        items
            .iter()
            .filter(
                |i| matches!(i, Item::FillRect { brush: remedy_ui_paint::Brush::Solid(b), .. } if *b == c),
            )
            .count()
    };
    assert!(fills(&h.items, red) >= 1, "the red background is drawn in the theme's term-1");
    let glyph_colors: Vec<Color> =
        h.items.iter().filter_map(|i| if let Item::Glyphs(g) = i { Some(g.color) } else { None }).collect();
    assert!(glyph_colors.contains(&Color::rgb8(10, 200, 30)), "true color reaches the glyphs");
    assert!(glyph_colors.contains(&foreground(&theme)), "default text is the theme's term foreground");
    // A thousand lines: the view follows the end, and only the rows near it exist.
    let mut many = Vec::new();
    for i in 0..1000 {
        many.extend_from_slice(format!("\r\nline {i}").as_bytes());
    }
    h.feed(&many);
    let rows = h.term.size().rows;
    assert!(h.rows_built() <= rows + 10, "{} rows built for {rows} shown", h.rows_built());
    let scroller =
        find(&h.ui.root(), "terminal_scroll").into_iter().next().unwrap_or_else(|| unreachable!("scroller"));
    let offset = h.ui.scroll_offset(scroller.id()).y;
    assert!(offset > 0.0, "it followed the output down");
    // Scrolled to the top it stays there while output arrives, and shows the oldest lines.
    h.ui.set_scroll_offset(scroller.id(), Vec2::ZERO);
    h.frame();
    h.feed(b"\r\nmore");
    assert_eq!(h.ui.scroll_offset(scroller.id()).y, 0.0);
    let texts: Vec<String> =
        h.term.with_emulator(|e| (0..3).filter_map(|i| e.history_line(i).map(Line::text)).collect());
    assert_eq!(texts.first().map(String::as_str), Some("plain red bg green"));
    // Typing brings the view back to the end.
    h.focus();
    h.key(Key::Character("x".into()), NONE, Some("x"));
    assert!(h.ui.scroll_offset(scroller.id()).y > 0.0);
}

#[test]
fn keys_go_to_the_program_in_the_modes_it_asked_for() {
    let mut h = H::new();
    h.focus();
    assert!(h.key(Key::Character("l".into()), NONE, Some("l")));
    assert!(h.key(Key::Character("s".into()), NONE, Some("s")));
    h.key(Key::Enter, NONE, None);
    h.key(Key::ArrowUp, NONE, None);
    h.key(Key::Character("c".into()), Modifiers { ctrl: true, ..NONE }, None);
    assert_eq!(h.out(), "ls\r\x1b[A\x03");
    h.feed(b"\x1b[?1h");
    h.key(Key::ArrowUp, NONE, None);
    assert_eq!(h.out(), "\x1bOA");
    // Ctrl+Tab is not the program's: it moves focus on.
    h.key(Key::Tab, Modifiers { ctrl: true, ..NONE }, None);
    assert_eq!(h.out(), "");
}

#[test]
fn queries_are_answered_through_the_app() {
    let mut h = H::new();
    h.feed(b"\x1b[2;5H\x1b[6n\x1b]11;?\x07");
    let out = h.out();
    assert!(out.starts_with("\x1b[2;5R"), "{out:?}");
    // The background answer is the theme's terminal background.
    let [r, g, b, _] = background(&h.ui.theme()).to_rgba8();
    assert!(
        out.contains(&format!("\x1b]11;rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")),
        "{out:?}"
    );
}

#[test]
fn dragging_selects_and_copy_joins_wrapped_lines() {
    let mut h = H::new();
    let cols = h.term.size().cols;
    // A line long enough to wrap, then another.
    let long: String = (0..cols + 5).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    h.feed(format!("{long}\r\nsecond line").as_bytes());
    h.focus();
    let (from, to) = (h.at(0, 0), h.at(2, 5));
    h.pointer(PointerPhase::Down, from, Buttons::PRIMARY, NONE);
    h.pointer(PointerPhase::Move, to, Buttons::PRIMARY, NONE);
    h.pointer(PointerPhase::Up, to, Buttons::default(), NONE);
    assert_eq!(h.term.selected_text(), Some(format!("{long}\nsecond")));
    h.key(Key::Character("c".into()), CTRL_SHIFT, None);
    assert_eq!(h.copies(), [format!("{long}\nsecond")]);
    assert_eq!(h.out(), "", "copying sends nothing to the program");
    // A double click selects a word (a path whole), a triple click the line.
    h.feed(b"\x1b[2J\x1b[Hsee /usr/local/bin now");
    let at = h.at(0, 8);
    h.click(at, NONE);
    h.click(at, NONE);
    assert_eq!(h.term.selected_text().as_deref(), Some("/usr/local/bin"));
    h.click(at, NONE);
    h.pointer(PointerPhase::Down, at, Buttons::PRIMARY, NONE);
    h.pointer(PointerPhase::Up, at, Buttons::default(), NONE);
    // Typing clears the selection.
    h.key(Key::Character("q".into()), NONE, Some("q"));
    assert_eq!(h.term.selected_text(), None);
    // Select all takes everything.
    h.key(Key::Character("a".into()), CTRL_SHIFT, None);
    assert!(h.term.selected_text().is_some_and(|t| t.contains("see /usr/local/bin now")));
}

#[test]
fn the_program_gets_the_pointer_when_it_asks_and_shift_keeps_selecting() {
    let mut h = H::new();
    h.feed(b"\x1b[?1000h\x1b[?1006hhello");
    h.focus();
    let p = h.at(0, 2);
    h.click(p, NONE);
    assert_eq!(h.out(), "\x1b[<0;3;1M\x1b[<0;3;1m");
    assert_eq!(h.term.selected_text(), None);
    // Shift+drag selects instead.
    h.pointer(PointerPhase::Down, h.at(0, 0), Buttons::PRIMARY, Modifiers { shift: true, ..NONE });
    h.pointer(PointerPhase::Move, h.at(0, 4), Buttons::PRIMARY, Modifiers { shift: true, ..NONE });
    h.pointer(PointerPhase::Up, h.at(0, 4), Buttons::default(), Modifiers { shift: true, ..NONE });
    assert_eq!(h.out(), "");
    assert_eq!(h.term.selected_text().as_deref(), Some("hello"));
    // The wheel is reported too.
    let lh = h.term.size().cell_height as f32;
    let wheel =
        WheelEvent::new(h.at(0, 1), Vec2::new(0.0, -2.0 * lh)).with_precise(true).with_modifiers(NONE);
    assert!(h.ui.wheel(&wheel));
    assert_eq!(h.out(), "\x1b[<64;2;1M\x1b[<64;2;1M");
}

#[test]
fn the_wheel_scrolls_the_scrollback_or_sends_arrows_on_the_alternate_screen() {
    let mut h = H::new();
    let lh = h.term.size().cell_height as f32;
    let wheel = |h: &H, dy: f32| {
        WheelEvent::new(h.at(0, 0), Vec2::new(0.0, dy)).with_precise(true).with_modifiers(NONE)
    };
    // On the primary screen the wheel is the scroll view's.
    assert!(!h.ui.wheel(&wheel(&h, -lh)));
    h.feed(b"\x1b[?1049h");
    assert!(h.ui.wheel(&wheel(&h, -3.0 * lh)));
    assert_eq!(h.out(), "\x1b[A\x1b[A\x1b[A");
    // Half a line, twice: one line.
    assert!(h.ui.wheel(&wheel(&h, lh * 0.5)));
    assert!(h.ui.wheel(&wheel(&h, lh * 0.5)));
    assert_eq!(h.out(), "\x1b[B");
}

#[test]
fn links_open_only_through_the_app_on_ctrl_click() {
    let mut h = H::new();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let o = opened.clone();
    h.term.on_link(move |u| o.borrow_mut().push(u.to_owned()));
    h.feed(b"see \x1b]8;;https://example.com/docs\x1b\\the docs\x1b]8;;\x1b\\ here");
    h.focus();
    h.click(h.at(0, 6), NONE);
    assert!(opened.borrow().is_empty(), "a plain click never opens a link");
    h.click(h.at(0, 6), Modifiers { ctrl: true, ..NONE });
    assert_eq!(*opened.borrow(), ["https://example.com/docs"]);
    h.click(h.at(0, 1), Modifiers { ctrl: true, ..NONE });
    assert_eq!(opened.borrow().len(), 1, "ctrl+click off the link opens nothing");
}

#[test]
fn programs_write_the_clipboard_only_with_the_apps_consent() {
    let mut h = H::new();
    h.feed(b"\x1b]52;c;c2VjcmV0\x07");
    assert!(h.copies().is_empty(), "no consent hook: no clipboard write");
    let asked = Rc::new(RefCell::new(Vec::new()));
    let a = asked.clone();
    h.term.on_clipboard_write(move |t| {
        a.borrow_mut().push(t.to_owned());
        t != "no"
    });
    h.feed(b"\x1b]52;c;bm8=\x07");
    assert!(h.copies().is_empty(), "refused");
    h.feed(b"\x1b]52;c;aGVsbG8=\x07");
    assert_eq!(h.copies(), ["hello"]);
    assert_eq!(*asked.borrow(), ["no", "hello"]);
}

#[test]
fn input_methods_compose_at_the_cursor_and_pastes_are_bracketed() {
    let mut h = H::new();
    h.feed(b"$ ");
    h.requests.clear();
    h.focus();
    let session = h.requests.iter().find_map(|r| match r {
        Request::TextInput(Some(s)) => Some(s.clone()),
        _ => None,
    });
    let session = session.unwrap_or_else(|| unreachable!("focus starts an input-method session"));
    let at = h.at(0, 2);
    assert!(
        session.caret.contains(at),
        "the candidate window goes at the cursor: {:?} vs {at:?}",
        session.caret
    );
    assert!(!session.private && session.surrounding.is_empty());
    assert!(h.ui.ime(&ImeEvent::Preedit { text: "にほ".into(), cursor: None }));
    h.frame();
    assert_eq!(h.out(), "", "composition is not sent");
    assert!(h.ui.ime(&ImeEvent::Commit("日本".into())));
    assert_eq!(h.out(), "日本");
    // Pastes: plain, then bracketed once the program asks.
    h.ui.paste("a\nb");
    assert_eq!(h.out(), "a\rb");
    h.feed(b"\x1b[?2004h");
    h.ui.paste("x\x1b[201~y");
    assert_eq!(h.out(), "\x1b[200~x[201~y\x1b[201~");
    // Ctrl+Shift+V asks the host for the clipboard.
    h.requests.clear();
    h.key(Key::Character("v".into()), CTRL_SHIFT, None);
    assert!(h.requests.contains(&Request::Paste));
}

#[test]
fn focus_changes_are_reported_when_asked() {
    let mut h = H::new();
    h.feed(b"\x1b[?1004h");
    h.focus();
    assert_eq!(h.out(), "\x1b[I");
    h.term.input_element().blur();
    h.frame();
    assert_eq!(h.out(), "\x1b[O");
}

#[test]
fn screen_readers_get_the_screen_and_hear_new_output_politely() {
    let mut h = H::new();
    h.feed(b"build started\r\ncompiling\r\nbuild ok");
    h.fire_timers();
    let root = h.term.input_element();
    let s = root.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("semantics"));
    assert_eq!(s.role(), remedy_ui_a11y::Role::MultilineTextInput);
    assert_eq!(s.name(), Some("Terminal"));
    assert_eq!(s.value_text(), Some("build started\ncompiling\nbuild ok"));
    let announcer = find(&h.ui.root(), "terminal_announcer")
        .into_iter()
        .next()
        .unwrap_or_else(|| unreachable!("announcer"));
    let a = announcer.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!("announcer semantics"));
    assert_eq!(a.live_state(), remedy_ui_a11y::Live::Polite);
    assert_eq!(a.name(), Some("build started\ncompiling\nbuild ok"));
    // Output in quick succession is read in one go, after a pause (one timer, not one each).
    h.term.feeder().feed(b"\r\none");
    h.frame();
    h.term.feeder().feed(b"\r\ntwo");
    h.frame();
    assert_eq!(h.timers.len(), 1, "announcements are rate-limited");
    h.fire_timers();
    let a = announcer.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    assert_eq!(a.as_deref(), Some("one\ntwo"));
    // The title names it.
    h.feed(b"\x1b]2;cargo build\x07");
    h.fire_timers();
    let name = || {
        root.info()
            .and_then(|i| i.semantics)
            .and_then(|s| s.name().map(|n| n.replace(['\u{2068}', '\u{2069}'], "")))
    };
    assert_eq!(name().as_deref(), Some("Terminal: cargo build"));
    assert_eq!(h.term.title().get_untracked().ok().as_deref(), Some("cargo build"));
    // The default name follows the window's language.
    h.ui.set_locale("ru");
    h.frame();
    assert_eq!(name().as_deref(), Some("Терминал: cargo build"));
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(TerminalOptions::localized("ja").label, "ターミナル");
}

#[test]
fn a_thread_feeds_while_the_ui_keeps_drawing() {
    let mut h = H::new();
    let feeder = h.term.feeder();
    let t = std::thread::spawn(move || {
        for i in 0..2000 {
            feeder.feed(format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 8).as_bytes());
        }
        feeder.feed(b"done");
    });
    for _ in 0..2000 {
        h.frame();
        if h.term.with_emulator(|e| e.screen_text().contains("done")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = t.join();
    h.frame();
    let shown = h.term.with_emulator(Emulator::screen_text);
    assert!(shown.contains("line 1999") && shown.ends_with("done"), "{shown}");
}

#[test]
fn a_view_scrolled_back_stays_on_its_lines_as_old_ones_leave() {
    let mut h = H::with({
        let mut v_ = TerminalOptions::default();
        v_.scrollback = Scrollback::Lines(200);
        v_
    });
    let mut lines = Vec::new();
    for i in 0..300 {
        lines.extend_from_slice(format!("n{i}\r\n").as_bytes());
    }
    h.feed(&lines);
    let lh = h.term.size().cell_height as f32;
    let scroller =
        find(&h.ui.root(), "terminal_scroll").into_iter().next().unwrap_or_else(|| unreachable!("scroller"));
    let top_line = |h: &H| {
        let i = (h.ui.scroll_offset(scroller.id()).y / lh).round() as usize;
        h.term.with_emulator(|e| e.history_line(i).map(Line::text))
    };
    h.ui.set_scroll_offset(scroller.id(), Vec2::new(0.0, 50.0 * lh));
    h.frame();
    let before = top_line(&h);
    assert!(before.is_some());
    // Twenty more lines push twenty old ones out of the scrollback.
    h.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\r\nk\r\nl\r\nm\r\nn\r\no\r\np\r\nq\r\nr\r\ns\r\nt\r\n");
    assert_eq!(top_line(&h), before, "the view kept its lines");
}

#[test]
fn theme_tokens_color_the_terminal() {
    let mut theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let derived = ansi(&theme, 1);
    let pink = Color::rgb8(255, 0, 170);
    theme.colors.custom.insert("term-1".into(), pink);
    assert_eq!(ansi(&theme, 1), pink);
    assert_ne!(derived, pink);
    // The derived colors read on the terminal's background.
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let bg = background(&theme);
    for i in 1..=6u8 {
        assert!(ansi(&theme, i).contrast(bg) >= 4.4, "term-{i}");
    }
    assert_eq!(ansi(&theme, 196), Color::rgb8(255, 0, 0));
}

#[test]
fn the_terminal_passes_the_accessibility_audit() {
    let mut h = H::new();
    h.feed(b"\x1b[1mhello\x1b[0m world");
    h.focus();
    let audit = remedy_ui_devtools::audit(&h.ui);
    assert!(audit.is_empty(), "{:#?}", audit.iter().map(|p| p.message()).collect::<Vec<_>>());
}

/// The text of display line `i` (scrollback first, then the screen).
fn line_text(h: &H, i: usize) -> String {
    h.term
        .with_emulator(|e| match i.checked_sub(e.history_len()) {
            None => e.history_line(i).map(Line::text),
            Some(r) => e.line(r).map(Line::text),
        })
        .unwrap_or_default()
}

fn top_line(h: &H) -> String {
    let scroller =
        find(&h.ui.root(), "terminal_scroll").into_iter().next().unwrap_or_else(|| unreachable!("scroller"));
    let lh = h.term.size().cell_height as f32;
    line_text(h, (h.ui.scroll_offset(scroller.id()).y / lh).round() as usize)
}

#[test]
fn the_working_directory_and_prompts_are_followed() {
    let mut h = H::new();
    h.feed(b"\x1b]7;file://box/tmp/my%20work\x07");
    assert_eq!(h.term.cwd().get_untracked().ok().flatten().as_deref(), Some("/tmp/my work"));
    let mut out = Vec::new();
    for i in 0..3 {
        out.extend_from_slice(format!("\x1b]133;A\x07$ \x1b]133;B\x07cmd{i}\r\n\x1b]133;C\x07").as_bytes());
        for j in 0..30 {
            out.extend_from_slice(format!("out{i}-{j}\r\n").as_bytes());
        }
        out.extend_from_slice(b"\x1b]133;D;0\x07");
    }
    out.extend_from_slice(b"\x1b]133;A\x07$ ");
    h.feed(&out);
    h.focus();
    // Ctrl+Shift+Up: the previous prompt comes to the top; again, the one before.
    h.key(Key::ArrowUp, CTRL_SHIFT, None);
    assert_eq!(top_line(&h), "$ cmd2");
    h.key(Key::ArrowUp, CTRL_SHIFT, None);
    assert_eq!(top_line(&h), "$ cmd1");
    h.key(Key::ArrowDown, CTRL_SHIFT, None);
    assert_eq!(top_line(&h), "$ cmd2");
    assert_eq!(h.out(), "", "prompt jumps are the terminal's, not the program's");
    // The last command's output, selected whole.
    assert!(h.term.select_command_output());
    let want: Vec<String> = (0..30).map(|j| format!("out2-{j}")).collect();
    assert_eq!(h.term.selected_text(), Some(want.join("\n")));
}

#[test]
fn the_middle_button_pastes_the_primary_selection() {
    let mut h = H::new();
    h.feed(b"hello world");
    h.focus();
    h.pointer(PointerPhase::Down, h.at(0, 0), Buttons::PRIMARY, NONE);
    h.pointer(PointerPhase::Move, h.at(0, 4), Buttons::PRIMARY, NONE);
    h.pointer(PointerPhase::Up, h.at(0, 4), Buttons::default(), NONE);
    assert!(
        h.requests.contains(&Request::SetPrimary("hello".into())),
        "selecting sets the primary selection"
    );
    h.requests.clear();
    h.pointer(PointerPhase::Down, h.at(0, 8), Buttons::MIDDLE, NONE);
    assert!(
        h.requests.contains(&Request::PastePrimary { fallback: Some("hello".into()) }),
        "{:?}",
        h.requests
    );
    // The host answers with the primary selection (or the fallback) as a paste.
    h.ui.paste("hello");
    assert_eq!(h.out(), "hello");
}

#[test]
fn dragging_past_the_edge_scrolls_and_extends_the_selection() {
    let mut h = H::new();
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    h.feed(&lines);
    let scroller =
        find(&h.ui.root(), "terminal_scroll").into_iter().next().unwrap_or_else(|| unreachable!("scroller"));
    let b = scroller.bounds().unwrap_or_else(|| unreachable!("laid out"));
    let lh = h.term.size().cell_height as f32;
    let start = h.ui.scroll_offset(scroller.id()).y;
    let rows = h.term.size().rows;
    h.pointer(PointerPhase::Down, h.at(rows - 2, 2), Buttons::PRIMARY, NONE);
    // Two and a half lines above the top edge: three lines a tick, upward.
    h.timers.clear();
    let above = Point::new(b.min.x + 20.0, b.min.y - 2.5 * lh);
    h.pointer(PointerPhase::Move, above, Buttons::PRIMARY, NONE);
    h.fire_timers();
    h.fire_timers();
    let now = h.ui.scroll_offset(scroller.id()).y;
    assert!((start - now - 6.0 * lh).abs() < 0.5, "scrolled {} lines", (start - now) / lh);
    let selected = h.term.selected_text().unwrap_or_default();
    assert!(
        selected.lines().count() > rows,
        "the selection grew with the scroll: {}",
        selected.lines().count()
    );
    // Letting go stops it.
    h.pointer(PointerPhase::Up, above, Buttons::default(), NONE);
    h.fire_timers();
    assert_eq!(h.ui.scroll_offset(scroller.id()).y, now);
}

#[test]
fn key_releases_reach_programs_that_ask_for_them() {
    let mut h = H::new();
    h.feed(b"\x1b[>3u");
    h.focus();
    let ctrl = Modifiers { ctrl: true, ..NONE };
    h.key(Key::Character("c".into()), ctrl, None);
    let release = h.ui.key(
        &KeyEvent::press(Key::Character("c".into())).with_modifiers(ctrl).with_pressed(false).with_text(None),
    );
    assert!(release);
    assert_eq!(h.out(), "\x1b[99;5u\x1b[99;5:3u");
}

#[test]
fn a_program_switching_to_132_columns_gets_them() {
    let mut h = H::new();
    let fit = h.term.size().cols;
    h.feed(b"\x1b[?40h\x1b[?3h");
    assert_eq!(h.term.size().cols, 132);
    assert_eq!(h.sizes.borrow().last().map(|g| g.cols), Some(132), "the app's PTY hears");
    let rows_box = find(&h.ui.root(), "terminal_rows").into_iter().next().and_then(|r| r.bounds());
    let cw = h.term.size().cell_width as f32;
    assert!(rows_box.is_some_and(|b| (b.width() - 132.0 * cw).abs() < 1.0), "{rows_box:?}");
    h.feed(b"\x1b[?3l");
    assert_eq!(h.term.size().cols, 80);
    h.feed(b"\x1bc");
    assert_eq!(h.term.size().cols, fit, "a reset gives the view its own width back");
}

#[test]
fn sixel_pictures_are_drawn() {
    let mut h = H::new();
    h.items.clear();
    h.feed(b"\x1bPq#0;2;0;100;0!20~-!20~\x1b\\");
    let images = h.items.iter().filter(|i| matches!(i, Item::Image { .. })).count();
    assert!(images >= 1, "the picture is drawn");
}

#[test]
fn memory_pressure_empties_the_scrollback() {
    let mut h = H::new();
    let mut lines = Vec::new();
    for i in 0..500 {
        lines.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    h.feed(&lines);
    assert!(h.term.with_emulator(Emulator::history_len) > 400);
    remedy_ui_view::device::memory_pressure(remedy_ui_view::device::MemoryPressure::Critical);
    h.frame();
    assert_eq!(h.term.with_emulator(Emulator::history_len), 0);
    assert!(h.term.with_emulator(Emulator::screen_text).contains("line 499"));
}

#[test]
fn a_line_rewritten_in_place_is_read_again_now_and_then() {
    let mut h = H::new();
    let announcer = find(&h.ui.root(), "terminal_announcer")
        .into_iter()
        .next()
        .unwrap_or_else(|| unreachable!("announcer"));
    let said = |a: &El| a.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    h.feed(b"progress 10%");
    h.fire_timers();
    assert_eq!(said(&announcer).as_deref(), Some("progress 10%"));
    h.feed(b"\rprogress 20%");
    h.fire_timers();
    assert_eq!(said(&announcer).as_deref(), Some("progress 20%"));
    // Too soon after: the latest state waits for its turn, then is read.
    h.feed(b"\rprogress 30%");
    h.fire_timers();
    assert_eq!(said(&announcer).as_deref(), Some("progress 20%"));
    h.feed(b"\rprogress 40%");
    for _ in 0..4 {
        h.fire_timers();
    }
    assert_eq!(said(&announcer).as_deref(), Some("progress 40%"));
}

#[test]
fn a_real_monospace_face_is_preferred() {
    // Only the bundled faces (none monospace): characters are centered to line up.
    let h = H::new();
    assert!(!h.term.is_monospace());
    // With the system's fonts, the system's monospace face is used.
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::with_system_fonts();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
    let h = H::on(ui, TerminalOptions::default());
    assert!(h.term.is_monospace(), "this machine has a monospace font");
}

#[test]
fn every_clipboard_request_of_one_chunk_asks_the_app_in_order() {
    let mut h = H::new();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let a = asked.clone();
    h.term.on_clipboard_write(move |t| {
        a.borrow_mut().push(t.to_owned());
        t != "two"
    });
    h.feed(b"\x1b]52;c;b25l\x07\x1b]52;c;dHdv\x07\x1b]52;c;dGhyZWU=\x07");
    assert_eq!(*asked.borrow(), ["one", "two", "three"]);
    assert_eq!(h.copies(), ["one", "three"]);
}

#[test]
fn selecting_everything_leaves_out_the_blank_rows_below_the_output() {
    let mut h = H::new();
    h.feed(b"first\r\n\r\nsecond");
    h.term.select_all();
    assert_eq!(h.term.selected_text().as_deref(), Some("first\n\nsecond"));
}
