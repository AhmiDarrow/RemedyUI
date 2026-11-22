# Text and input methods

RemedyUI shapes text itself, with full bidi, variable and color fonts, and fallback to the
system's fonts for scripts the bundled Remedy families lack (feature `system-fonts`, on by
default; feature `complex-scripts` adds dictionary line breaking for Thai, Lao, Khmer and
Burmese). Glyphs are hinted and rasterized by the engine, in grayscale or LCD.

This page covers showing text, rich text, symbols, editable fields, secrets, input methods,
and the defenses against text that deceives.

The samples run in a window on the in-memory host with
[`TestApp`](crate::testing::TestApp), the way an app's own tests do (see
[`crate::guide::testing`]).

## Showing text

- [`widgets::text`](crate::widgets::text): body text that wraps to its width.
- [`widgets::text`](crate::widgets::text) with a [variant](crate::El::variant): `muted`,
  `small`, `error`, `strong` or `code` (monospace, in a box).
- [`widgets::heading`](crate::widgets::heading): a heading, level 1 to 3, which screen
  readers can jump between.
- [`widgets::link`](crate::widgets::link): text that does something when activated.

Any element shows text with [`El::text`](crate::El::text), or text that follows signals with
a closure or a signal given to [`El::text`](crate::El::text) (`.text(move || ..)`). [`El::nowrap`](crate::El::nowrap) keeps it on one
line.

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;

let mut app = TestApp::window(|ui, _| {
    ui.root()
        .child(w::heading(ui, 1, "Backups"))
        .child(w::text(ui, "Your files are copied every night."))
        .child(w::text(ui, "Last copy: 02:00").variant("muted"))
        .child(w::link(ui, "Show the log", || {}));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let names: Vec<String> = ui.reader_view().into_iter().filter_map(|n| n.name).collect();
assert_eq!(names, ["Backups", "Your files are copied every night.", "Last copy: 02:00", "Show the log"]);
```

## Your own fonts

An app ships its brand's typeface as files in its binary: TrueType (`.ttf`), OpenType
(`.otf`), or the web's WOFF and WOFF2 (`.woff`, `.woff2`), which are unwrapped once as they
are read. [`FontFile::new`](crate::text::FontFile::new) reads and checks one (bytes that are
not a font are an error, never a silent fallback), and
[`AppConfig::font`](crate::AppConfig::font) adds it to every window. Name its family first in
the theme's `font_families`; the Remedy families stay behind it for anything it lacks.

Add one file per weight and style. A web font split into subsets per script (Fontsource's
`inter-latin-400-normal.woff2`, `inter-cyrillic-400-normal.woff2`, …, which CSS joins with
`unicode-range`) is added file by file too: the parts join as one family, and each character
is drawn by whichever part covers it.

```
use remedy_ui::testing::TestApp;
use remedy_ui::text::FontFile;
use remedy_ui::widgets as w;
use remedy_ui::AppConfig;

# let latin: &'static [u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../remedy-ui-text/tests/fixtures/inter/inter-latin-400-normal.woff2"));
# let cyrillic: &'static [u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../remedy-ui-text/tests/fixtures/inter/inter-cyrillic-400-normal.woff2"));
// let latin = include_bytes!("../assets/fonts/inter-latin-400-normal.woff2");
let mut config = AppConfig::new("Brand")
    .font(FontFile::new(latin).expect("a WOFF2 font"))
    .font(FontFile::new(cyrillic).expect("a WOFF2 font"));
config.theme.font_families.insert(0, "Inter".into());

let mut app = TestApp::new(config, |app| {
    app.open(remedy_ui::WindowSpec::new("Brand"), |ui, _| {
        ui.root().child(w::text(ui, "Hello, Привет"));
    });
});
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(ui.font_families().iter().any(|f| f == "Inter"));
assert!(FontFile::new(b"not a font").is_err());
```

## Fallback faces and the web

With `script-fonts` (on by default) text in Arabic, Devanagari, Bengali, Chinese and
Japanese draws in the bundled Noto faces, so it never shows empty boxes where no such font
is installed. On the desktop they are built in; the Chinese and Japanese ones (8.6 MB of
WOFF2) are unpacked the first time such text shows. In a browser page none of them is in the
module: [`bundled_fonts`](crate::bundled_fonts) registers each as a file beside the page
(`fonts/NotoSansSC-Regular.woff2`), fetched the first time text needs it; the text draws
again as soon as the face arrives, and the service worker keeps it for offline use. An app
loading faces itself does the same with
[`FontCollection::add_on_demand`](crate::text::FontCollection::add_on_demand): a loader that
returns no faces yet is asked again, and
[`Ui::fonts_changed`](crate::view::Ui::fonts_changed) shapes the text again once they are
in.

## The system's font

CSS's generic families (`system-ui`, `sans-serif`, `monospace`, `emoji`) can be named in a
theme's `font_families` like any family. On the desktop, with `system-fonts`, they are the
system's choice. [`FontCollection::set_generic_family`](crate::text::FontCollection::set_generic_family)
points one at faces the app has, and [`generic_family`](crate::text::FontCollection::generic_family)
says what it draws in.

A browser page has no system fonts to reach, and no way to name the face the browser draws
`system-ui` text in: CSS's font loading never hands a page a font's file. So
[`App::load_system_ui_font`](crate::App::load_system_ui_font) reads it through the Local Font
Access API (Chrome and Edge on desktops): it lists the installed fonts, finds the one whose
text measures exactly as `system-ui` text does in that browser (Segoe UI on Windows, the
desktop's font on Linux, Roboto on ChromeOS), and adds its faces to every window with
`system-ui` pointing at them, so the text is the same face, weights included, that the
browser shows. The browser asks the person once whether the page may use their fonts: call
it in answer to the click that chooses the system font, then at start once it has been
allowed. Where the API is missing (Firefox, Safari, phones) or the person declines, it
reports why and `system-ui` falls through to the next family named, so name the app's own
face after it:

```no_run
# fn setup(app: &remedy_ui::App) {
let mut theme = remedy_ui::AppConfig::new("Notes").theme;
theme.font_families = vec!["system-ui".into(), "Remedy Sans UI".into()];
app.set_theme(theme);
app.load_system_ui_font(|got| match got {
    Ok(families) => println!("system-ui is {families:?}"),
    Err(why) => println!("keeping Remedy Sans UI: {why}"),
});
# }
```

## Rich text

[`RichText`](crate::view::RichText) is a string with styled spans and inline objects. Spans
take a [`TextStylePatch`](crate::view::TextStylePatch): size, weight, italic, color, underline,
strikethrough, letter spacing, OpenType features, families, and a link.

```
use remedy_ui::view::{RichText, TextStylePatch};

let line = RichText::new()
    .text("Saved to ")
    .styled("Documents", TextStylePatch::bold())
    .text(" just now");
assert_eq!(line.as_str(), "Saved to Documents just now");
assert_eq!(line.spans().len(), 1);
```

Pictures go inside a paragraph as inline objects: add one with
[`RichText::object`](crate::view::RichText::object) at the picture's size, and give the element
the picture for that object's id with [`El::inline_images`](crate::El::inline_images). The text
wraps around it like a character; screen readers and copying read its `alt` (`:wave:`).

```
use std::sync::Arc;
use remedy_ui::paint::Image;
use remedy_ui::testing::TestApp;
use remedy_ui::view::{InlineImage, InlineObject, RichText};

let mut app = TestApp::window(|ui, _| {
    let wave = Image::new(1, 1, Arc::from([255u8, 200, 0, 255].as_slice()));
    let text = RichText::new()
        .text("Hello ")
        .object(InlineObject { id: 1, width: 18.0, height: 18.0, baseline: None });
    ui.root().child(ui.el("text").text(text).inline_images(vec![InlineImage::new(1, wave, ":wave:")]));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Hello :wave:")));
```

Pass it to [`El::text`](crate::El::text). Markdown and a rich text editor are in the
`rich` feature; see [`crate::guide::documents`].

## Symbols

Arrows, shapes, stars, check marks and interface signs (✦ ◆ ◇ ◈ ◎ ⚠ ☰ ⌕ ⇄ ✓ ★ ⏸ …) belong
to no script (Unicode's Common script), so the script-based fallback above does not look for
them. When a character comes out without a glyph, layout tries the collection's *symbol
fallbacks* for that character: Remedy Symbols, a small face of interface symbols the Remedy
text families lack (9 KB, in `remedy-ui-fonts`, on by default), then the platform's symbol,
math and emoji fonts (Segoe UI Symbol on Windows, Apple Symbols on macOS, Noto Sans Symbols
and DejaVu Sans on Linux and Android). So these symbols draw in a browser page and on systems
with no symbol fonts too, and an app writes them as plain text, with no icon font of its own.

[`FontCollection::set_symbol_fallbacks`](crate::text::FontCollection::set_symbol_fallbacks)
changes the list (an app's own icon font first, say), and an empty list turns it off. Apps set
it in [`AppConfig::fonts`](crate::AppConfig::fonts).

```
use remedy_ui::text::{FontCollection, DEFAULT_SYMBOL_FALLBACKS};

let mut fonts = FontCollection::new();
assert_eq!(fonts.symbol_fallbacks().first().map(String::as_str), Some("Remedy Symbols"));
assert_eq!(fonts.symbol_fallbacks().len(), DEFAULT_SYMBOL_FALLBACKS.len());
fonts.set_symbol_fallbacks(["My Icons", "Remedy Symbols"]);
assert_eq!(fonts.symbol_fallbacks(), ["My Icons", "Remedy Symbols"]);
```

## Editable fields

A field is a [`TextField`](crate::TextField): a handle to an element with an editor inside.
It keeps its own text, selection, undo history and input-method composition. Read it with
[`TextField::value`](crate::TextField::value), set it with
[`TextField::set_value`](crate::TextField::set_value), and hear about edits with
[`TextField::on_change`](crate::TextField::on_change). The widgets give each kind its look and
label:

- [`widgets::text_input`](crate::widgets::text_input): one line; Enter calls
  [`TextField::on_submit`](crate::TextField::on_submit).
- [`widgets::text_area`](crate::widgets::text_area): several lines; Enter breaks the line.
- [`widgets::search`](crate::widgets::search): a search box; Escape clears it.
- [`widgets::secret_input`](crate::widgets::secret_input): passwords and keys (below).
- [`widgets::field`](crate::widgets::field): a label above any control.

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let typed = Signal::new(String::new());
let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let name = w::text_input(ui, "Name").placeholder("Ada Lovelace");
    let n = name.clone();
    let name = name.on_change(move || typed.set(n.value()));
    let _ = s.set(name.clone());
    ui.root().child(name.el());
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let Some(name) = slot.get() else { unreachable!("built") };
let Some(window) = app.host().window_ids().first().copied() else { unreachable!("open") };

// A person types "Grace" (here committed in one go, as an input method or paste would).
name.el().focus();
app.host().compose_and_commit(window, "", "Grace");
app.frames(1);
assert_eq!(name.value(), "Grace");
assert_eq!(typed.get(), "Grace", "on_change heard the edit");

// The field is a text input to screen readers, named by its label, valued by its text.
let node = ui.reader_view().into_iter().find(|n| n.name.as_deref() == Some("Name"));
assert_eq!(node.and_then(|n| n.value), Some("Grace".to_string()));

// The app's own changes do not call `on_change`; they start a fresh undo history.
name.set_value("Ada");
assert_eq!((name.value(), typed.get()), ("Ada".to_string(), "Grace".to_string()));
```

A field is not bound to a signal: it owns its text, so typing never round-trips through
app state. Copy its value out in `on_change` (as above) or read it when you need it, such as
in `on_submit`. [`TextField::on_change`](crate::TextField::on_change) hears what people do;
[`TextField::observe_text`](crate::TextField::observe_text) hears every change, the app's
included.

## Input methods

People who write Chinese, Japanese, Korean, Vietnamese and many other languages type
through an input method: they compose a few keystrokes, pick a candidate, and commit it. A
focused field opens an input-method session with the platform (TSF on Windows, the platform
equivalents elsewhere), shows the composition in place, and places the candidate window at
the caret. Nothing in the app has to change for this.

The in-memory host plays an input method, so tests can check what a person typing through
one would get:

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let message = w::text_input(ui, "Message");
    ui.root().child(message.el());
    message.el().focus();
    let _ = s.set(message);
});
let Some(window) = app.host().window_ids().first().copied() else { unreachable!("open") };
assert!(app.host().window(window).is_some_and(|w| w.text_input().is_some()), "focus opened a session");

app.host().compose_and_commit(window, "nihao", "你好");
app.frames(1);
assert_eq!(slot.get().map(|f| f.value()), Some("你好".to_string()));
```

## Secrets

[`widgets::secret_input`](crate::widgets::secret_input) is for passwords, keys and recovery
phrases. Its text:

- is masked, unless the person turns on [`widgets::reveal_toggle`](crate::widgets::reveal_toggle);
- lives in a buffer that is wiped when it is dropped, and is never copied to the clipboard;
- goes to the platform only as a private input session: no composition, no learning, no
  prediction, and none of the text around the caret;
- is never given to screen readers, agents or the devtools: they see a password input and
  its label, never its value.

Take the value with [`TextField::take_secret`](crate::TextField::take_secret), which empties
the field and hands over a [`SecretString`](crate::text::SecretString) that wipes itself.

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let password = w::secret_input(ui, "Master password");
    let row = w::row(ui).child(password.el()).child(w::reveal_toggle(ui, &password));
    ui.root().child(row);
    password.el().focus();
    let _ = s.set(password);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let Some(password) = slot.get() else { unreachable!("built") };
let Some(window) = app.host().window_ids().first().copied() else { unreachable!("open") };
let private = app.host().window(window).and_then(|w| w.text_input()).is_some_and(|s| s.private && s.surrounding.is_empty());
assert!(private, "the platform is told: no learning, no prediction, and no text around the caret");
// The person types it, key by key: secret fields take no input method.
app.type_text("correct horse");

let reader = ui.reader_view();
let Some(node) = reader.iter().find(|n| n.name.as_deref() == Some("Master password")) else {
    unreachable!("the field is in the tree")
};
assert_eq!(node.value, None, "a screen reader never gets the password");

let secret = password.take_secret();
assert_eq!(secret.expose(), "correct horse");
assert!(password.is_empty(), "taking it empties the field");
```

### Fields that hold secrets in the clear

Some text is secret but must be seen and edited: a note of passwords, an API key being
written, a recovery phrase being checked. Keep it in an ordinary field (or a multi-line, rich
or code field) and call [`TextField::copy_as_secret`](crate::TextField::copy_as_secret):
what the person copies or cuts out of it goes to the clipboard as a secret, as
[`App::copy_secret`](crate::App::copy_secret) does. It is kept out of clipboard history and
cloud sync, and cleared after the time given if the clipboard still holds it (anything the
person copied since is left alone). A rich field copies plain text only.
[`TextField::on_copy`](crate::TextField::on_copy) is told after each copy, to say so.

```
use remedy_ui::input::{Key, KeyEvent, Modifiers};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

let status = Signal::new(String::new());
let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let note = w::text_area(ui, "Note")
        .copy_as_secret(Duration::from_secs(30))
        .on_copy(move || status.set("Copied · clears in 30s".into()));
    note.set_value("api-key-123");
    ui.root().child(note.el());
    let _ = s.set(note);
});
let Some(note) = slot.get() else { unreachable!("built") };
note.el().focus();
note.select_all();
app.frames(1);
let copy = KeyEvent::press(Key::Character("c".into())).with_modifiers(Modifiers::CTRL);
app.key(copy);
app.frames(1);
assert_eq!(app.host().clipboard(), Some(("api-key-123", true)), "kept out of history and sync");
assert_eq!(status.get(), "Copied · clears in 30s");

app.host().advance(Duration::from_secs(31));
app.frames(1);
assert!(app.host().clipboard().is_none_or(|(text, _)| text.is_empty()), "cleared after 30 s");
```

## Text that deceives

Text from other people can lie about itself. Bidi control characters can make a file name
display as something it is not, and look-alike letters from other scripts can make one name
pass for another. [`text::reveal_controls`](crate::text::reveal_controls) shows hidden
controls in text the app marks as security-sensitive, and
[`text::looks_like`](crate::text::looks_like) tells when two different names would look the
same on screen:

```
use remedy_ui::text::{Reveal, looks_like, reveal_controls};

let attachment = "invoice\u{202E}fdp.exe"; // displays as "invoiceexe.pdf"
assert_eq!(reveal_controls(attachment, Reveal::default()), "invoice⟨RLO⟩fdp.exe");

assert!(looks_like("paypal", "раураl"), "Cyrillic р, а and у");
assert!(!looks_like("paypal", "paypal"));
```

Use them on usernames, room names, domains and file names before people act on them.

## Locales and pseudo-locales

[`Ui::set_locale`](crate::Ui::set_locale) (or [`App::set_locale`](crate::App::set_locale)
for every window) sets the language text is shaped and broken in. Two pseudo-locales check
layouts before translations exist: `en-XA` makes the app's own strings accented, longer and
bracketed, and `ar-XB` forces them right to left. They rewrite labels, headings and
placeholders, never what people type, and never text marked as data with
[`El::data_text`](crate::El::data_text) (names, dates, file names).

```
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slots = Rc::new((OnceCell::new(), OnceCell::new()));
let s = slots.clone();
let mut app = TestApp::window(move |ui, _| {
    let label = w::text(ui, "Settings").nowrap();
    let file = w::text(ui, "report.pdf").nowrap().data_text();
    let _ = (s.0.set(label.clone()), s.1.set(file.clone()));
    ui.root().child(w::column(ui).align(remedy_ui::layout::Align::Start).child(label).child(file));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let width = |slot: &OnceCell<remedy_ui::El>| slot.get().and_then(|e| e.bounds()).map_or(0.0, |b| b.width());
let before = (width(&slots.0), width(&slots.1));

ui.set_locale("en-XA");
app.frames(1);
assert!(width(&slots.0) > before.0, "the app's string got longer, as translations do");
assert_eq!(width(&slots.1), before.1, "data is left alone");
```

Pseudo-locales change only what is drawn. Screen readers and agents still get the real
strings.

For translations themselves, use Fluent message files through [`i18n`](crate::i18n)
(feature `i18n`, on with `forms`); `cargo remedy new` sets this up.

## The engine's own words

The words RemedyUI shows itself (an icon button's "Close", the password field's "Show
password", the spelling menu, the media player's controls, the board editor, the
customization pages, the agent prompts, what screen readers hear) follow the window's locale
with built-in translations. Your app does nothing for this: set the locale as above, and those
words change with it. RemedyUI ships them in English, Spanish, Simplified Chinese, Hindi,
Arabic, French, Brazilian Portuguese, Russian, Japanese, German and Bengali. A locale the
engine does not ship takes the closest one it does (`es-MX` reads Spanish, `pt-PT` Brazilian
Portuguese), then English.

Each crate keeps its words in Fluent files (`i18n/<locale>.ftl`), compiled in as its
`STRINGS` catalog. Reword a built-in message, or add a language, with
`remedy_ui::view::strings::add_messages(catalog, locale, ftl)`. Structs of words such as
`board::BoardLabels` keep English as their `Default`, give a locale's words with
`localized`, and take your own wording through `translated`. The translations were written
with machine help and wait for review by native speakers. `docs/I18N.md` has the details,
including how to add a language to the engine.

## Spell checking

Feature `spell` adds [`spell`](crate::spell): the platform's checker or Hunspell
dictionaries, run off the UI thread, with misspellings underlined in a field as people type
and corrections in its context menu ([`spell::SpellCheckField`](crate::spell::SpellCheckField)).

## Limits

Text works within bounds that each have a default and a public way to change them.

| Limit | Default | How to change it |
| --- | --- | --- |
| Shortest paragraph laid out a piece at a time | 32 KiB | `text::LazyLimits::min_bytes` |
| About how much text one piece holds | 8 KiB | `text::LazyLimits::chunk_bytes` |
| Pieces of one paragraph kept laid out | 48 | `text::LazyLimits::kept_pieces` |
| Side of a glyph atlas page | 1024 px | `text::GlyphAtlas::set_page_size` |
| Deepest list level (rich editor, HTML and document readers) | 5 | `text::set_max_list_level` |
| Typing pause that starts a new undo step | 1000 ms | `text::set_undo_pause_ms` |

`LazyLimits` holds for the whole process: pass one to `text::set_lazy_limits`; paragraphs
laid out from then on use it, and one already laid out keeps its own. `usize::MAX` as
`min_bytes` lays every paragraph out whole, which costs time and memory with its length.

```no_run
use remedy_ui::text::{LazyLimits, set_lazy_limits, set_max_list_level};

// A log viewer that scrolls back and forth over huge lines: keep more of them laid out.
set_lazy_limits(LazyLimits::default().kept_pieces(256));
set_max_list_level(8);
```

A WOFF 1.0 table may not claim to unpack to more than 1032 times its stored size: that is
the most a DEFLATE stream can expand, so a table claiming more is damaged, not large. It is
a fact of the format, not a limit.
