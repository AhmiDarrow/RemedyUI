# The terminal

[`crate::term`] (feature `term`) is a terminal element: VT and xterm emulation on a screen of
cells, scrollback on a virtual list, selection and copying, links, input methods and true
color. It is what a terminal app, an IDE's console or a build log uses.

The app owns the program and its PTY. RemedyUI never starts a process. The app:

1. feeds the program's output in through a [`crate::term::Feeder`], from any thread;
2. writes what [`Terminal::on_output`](crate::term::Terminal::on_output) gives it (typed
   keys, pastes, pointer reports, answers to the program's queries) to the program;
3. resizes the PTY when [`Terminal::on_resize`](crate::term::Terminal::on_resize) says the
   rows and columns changed.

## A terminal on screen

[`crate::term::terminal`] makes the terminal; put [`Terminal::element`](crate::term::Terminal::element)
in the tree. [`crate::term::TerminalOptions`] sets the name screen readers use, the
scrollback, the longest string a program may send in one sequence, the font families and
which characters count as part of a word when double-clicking.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::term::{Emulator, TerminalOptions, terminal};
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let term = terminal(&ui, TerminalOptions::default().label("Build output".into()));
ui.root().child(ui.el("panel").column().height(400.0).fill_width().child(term.element()));

// The program prints, in color, and sets the window title.
term.feeder().feed(b"\x1b]2;cargo build\x07\x1b[32mCompiling\x1b[0m remedy-ui\r\n");
term.sync(); // show it now; otherwise it shows on the next frame
# app.frames(2);

assert!(term.with_emulator(Emulator::screen_text).starts_with("Compiling remedy-ui"));
assert_eq!(term.title().get(), "cargo build");
assert!(term.size().cols > 0 && term.size().rows > 0);
```

[`Terminal::with_emulator`](crate::term::Terminal::with_emulator) reads the screen, the
scrollback and the modes, for tests, recordings and agents reading the session. The
[`crate::term::Emulator`] also works on its own, with no element, for headless use:

```
use remedy_ui::term::Emulator;

let mut emu = Emulator::new(20, 3);
emu.feed(b"one\r\ntwo\r\nthree\r\nfour");
assert_eq!(emu.screen_text(), "two\nthree\nfour");
assert_eq!(emu.history_len(), 1, "\"one\" scrolled into the scrollback");
emu.resize(10, 3);
assert_eq!(emu.size(), (10, 3));
```

## Wiring a program

Output is parsed on the thread that feeds it, so a busy program never stalls the window: the
UI thread only takes what changed. Here the "program" is any reader and writer pair: a PTY's
two ends in a real app (ConPTY on Windows, a pseudo-terminal elsewhere, through the PTY
crate the app chooses), an in-memory pair in this sample.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::term::{Emulator, GridSize, Terminal, TerminalOptions, terminal};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

/// Connects `term` to a program: its output in, the terminal's bytes out, sizes to the PTY.
fn connect(
    term: &Terminal,
    mut from_program: impl Read + Send + 'static,
    to_program: Rc<RefCell<dyn Write>>,
    resize_pty: impl Fn(GridSize) + 'static,
) -> std::thread::JoinHandle<()> {
    term.on_output(move |bytes| {
        let _ = to_program.borrow_mut().write_all(bytes);
    });
    term.on_resize(resize_pty);
    let feeder = term.feeder();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = from_program.read(&mut buf) {
            if n == 0 {
                break;
            }
            feeder.feed(buf.get(..n).unwrap_or_default());
        }
    })
}

let term = terminal(&ui, TerminalOptions::default());
ui.root().child(ui.el("panel").column().height(400.0).fill_width().child(term.element()));
let typed: Rc<RefCell<Vec<u8>>> = Rc::default();
let sizes: Rc<RefCell<Vec<GridSize>>> = Rc::default();
let s = sizes.clone();
let program = connect(&term, std::io::Cursor::new(b"$ ".to_vec()), typed.clone(), move |g| s.borrow_mut().push(g));
let _ = program.join();
# app.frames(2);
assert_eq!(term.with_emulator(Emulator::screen_text), "$");

// A paste goes to the program, bracketed when it asked for that.
term.paste("ls -la");
assert_eq!(String::from_utf8_lossy(&typed.borrow()), "ls -la");

// The terminal told the app its size once it was laid out.
assert!(sizes.borrow().last().is_some_and(|g| g.cols == term.size().cols));
```

Keys typed while the terminal has focus are encoded as xterm does, following the modes the
program set (application cursor keys, the kitty keyboard protocol), and reach the program
through `on_output`. [`crate::term::encode_key`], [`crate::term::encode_paste`] and
[`crate::term::encode_mouse`] are those encoders on their own. The input method composes at
the cursor, so its candidate window opens where the text will go.

## Scrollback and memory

[`crate::term::Scrollback`] says how much history to keep: `Auto` (the default) sizes it from
the machine (1/256 of its memory), or `Lines`, `Bytes` or `Off`. Under memory pressure the
terminal gives scrollback back. Lines reflow when the width changes. The scrollback is a
virtual list, so a long history costs only the lines on screen.

While the view follows the output, new lines scroll it; scrolled back, it stays on its lines
as new output arrives and old lines leave.
[`Terminal::scroll_to_bottom`](crate::term::Terminal::scroll_to_bottom) follows again. With a
shell that marks its prompts (OSC 133),
[`Terminal::jump_to_prompt`](crate::term::Terminal::jump_to_prompt) moves between commands
and [`Terminal::select_command_output`](crate::term::Terminal::select_command_output)
selects the last command's output. [`Terminal::cwd`](crate::term::Terminal::cwd) is the
working directory the shell reported (OSC 7).

## Selection and copying

Dragging selects, with auto-scroll past the edges; double-click selects a word (with the
characters in `word_chars`, so a path or URL selects whole); triple-click a line. On X11 and
Wayland the selection becomes the primary selection, and the middle button pastes it.
[`Terminal::selected_text`](crate::term::Terminal::selected_text) joins wrapped lines and
leaves trailing blanks out, and [`Terminal::copy_selection`](crate::term::Terminal::copy_selection)
puts it on the clipboard.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::term::{TerminalOptions, terminal};
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let term = terminal(&ui, TerminalOptions::default());
ui.root().child(ui.el("panel").column().height(400.0).fill_width().child(term.element()));
term.feeder().feed(b"first line   \r\nsecond");
term.sync();
# app.frames(1);

term.select_all(); // the scrollback and the whole screen
// Trailing spaces and the blank rows below the output are left out.
assert_eq!(term.selected_text().as_deref(), Some("first line\nsecond"));
assert!(term.copy_selection());
# app.frames(1);
assert_eq!(app.host().clipboard(), Some(("first line\nsecond", false)));
```

## Links and the clipboard, with consent

Programs print links (OSC 8) and ask to write the clipboard (OSC 52). Neither happens unless
the app says so:

- [`Terminal::on_link`](crate::term::Terminal::on_link) gets a link's address when someone
  Ctrl+clicks it (Command+click on Apple platforms). Links never open by themselves; without
  this hook they do nothing. The app decides what opening means, for example
  [`App::open_url`](crate::App::open_url).
- [`Terminal::on_clipboard_write`](crate::term::Terminal::on_clipboard_write) gets the text a
  program wants on the clipboard and returns true to allow it, by asking the person or by a
  setting. Without this hook programs cannot write the clipboard. Programs can never read it.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::term::{TerminalOptions, terminal};
use std::cell::RefCell;
use std::rc::Rc;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(800.0, 600.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let term = terminal(&ui, TerminalOptions::default());
ui.root().child(ui.el("panel").column().height(400.0).fill_width().child(term.element()));

// No hook: the program's request is dropped.
term.feeder().feed(b"\x1b]52;c;c2VjcmV0\x07"); // "secret", base64
term.sync();
# app.frames(1);
assert_eq!(app.host().clipboard(), None);

// The app allows what the person agrees to (here: anything but "no").
let asked = Rc::new(RefCell::new(Vec::new()));
let a = asked.clone();
term.on_clipboard_write(move |text| {
    a.borrow_mut().push(text.to_owned());
    text != "no"
});
// Every request reaches the hook, in order, however many arrive before the screen updates.
term.feeder().feed(b"\x1b]52;c;bm8=\x07"); // "no": refused
term.feeder().feed(b"\x1b]52;c;aGVsbG8=\x07"); // "hello"
term.sync();
# app.frames(1);
assert_eq!(*asked.borrow(), ["no", "hello"]);
assert_eq!(app.host().clipboard(), Some(("hello", false)));
```

## Screen readers and agents

The terminal reads as a terminal named for its label and the program's title ("Terminal:
cargo build"). Screen readers hear the screen, new output (announced at a measured rate, so a
flood of output does not drown the reader) and rewritten lines. Agents read the session the
same way, through the window's semantics (see the [agents guide](crate::guide::agents)).

## Colors

The 16 ANSI colors, the default foreground and background and the cursor come from the
theme's `term-*` color tokens ([`crate::term::ansi`], [`crate::term::foreground`],
[`crate::term::background`], [`crate::term::cursor`]), so a theme colors the terminal like
everything else. Programs may still use 256 colors and true color.
