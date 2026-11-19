# Moving from Qt or PyQt

RemedyPDF is the PyQt app moving over. This guide is for anyone coming from Qt Widgets,
PyQt or PySide.

Qt and RemedyUI are both retained: a tree of objects that lives between frames, with layouts
and a style sheet. The differences are in how state flows. Qt connects signals to slots by
hand and the widget keeps its own copy of the value. In RemedyUI the value lives in a
reactive signal the app owns, and the widget shows it.

## Concept map

| Qt / PyQt | RemedyUI |
|---|---|
| `QApplication`, `app.exec()` | [`crate::run`] with a setup closure |
| `QMainWindow`, `QWidget` as a window | [`crate::App::open`] with a [`crate::WindowSpec`] |
| A `QWidget` subclass | A function returning an [`crate::El`] |
| A widget tree with parents | Elements joined with `.child(..)` |
| `QHBoxLayout`, `QVBoxLayout`, `QGridLayout` | [`crate::widgets::row`], [`crate::widgets::column`], grid tracks in [`crate::El::layout`] |
| `addStretch`, `setSizePolicy` | [`crate::widgets::spacer`], [`crate::El::grow`], [`crate::El::fill_width`] |
| `Q_PROPERTY` with a `NOTIFY` signal | A [`crate::reactive::Signal`] |
| `connect(sender, signal, receiver, slot)` | A closure: `on_click(move \|\| ..)`, or a binding that reads a signal |
| `QAction` with a shortcut | A command in [`crate::nav::Commands`] |
| `QToolBar`, `QMenu` | [`crate::nav::toolbar`], [`crate::widgets::menu`], [`crate::widgets::context_menu`] |
| `QShortcut` | A command's chords; people can rebind them ([`crate::nav::Commands::rebind`]) |
| QSS (Qt style sheets) | A [`crate::style::StyleSheet`] over theme tokens; typed tokens (`Space::S3`, `Radius::R2`) in code |
| `QPalette` | A [`crate::style::Theme`] (colors by role, type scale, spacing, radii) |
| `QAbstractItemModel` + `QTableView` | [`crate::data::table`] over a `Signal<Vec<T>>` with [`crate::data::Column`]s |
| `QTreeView` | [`crate::data::tree_view`] |
| `QListView` | [`crate::widgets::list`], [`crate::widgets::virtual_list`] |
| `paintEvent` and `QPainter` | [`crate::El::paint`] or [`crate::widgets::canvas`] |
| `QThread`, `QRunnable` | [`crate::App::background`]; [`crate::App::background_cancellable`] for work that checks `isInterruptionRequested` |
| `QTimer::singleShot` | [`crate::App::after`], [`crate::Ui::after`] |
| `QSettings` for window geometry | [`crate::WindowSpec::persist_geometry`] |
| `QSettings` for other state | [`crate::reactive::Signal::persist`]: saved on change, restored at start |
| `setVisible`, `QStackedWidget` | [`crate::show`] (with a fallback), [`crate::El::hidden`], [`crate::nav::NavStack`] |
| `QPropertyAnimation` | [`crate::El::animate`] with a spring or a curve from [`crate::motion`] |
| A slot wrapped in `try`/`except` that shows an error | [`crate::catch_errors`] around the content |
| `QDialog::exec()` | [`crate::widgets::dialog`] or [`crate::widgets::modal`]; they do not block |
| `QFileDialog` | [`crate::App::file_dialog`] |
| `QSystemTrayIcon` | [`crate::App::set_tray`] and [`crate::App::on_tray`] |
| `QGraphicsView` of pages | [`crate::pages`]: a page view over pages the app renders |

## Widgets and slots

```python
class Counter(QWidget):
    def __init__(self):
        super().__init__()
        self.count = 0
        self.label = QLabel("Clicked 0 times")
        button = QPushButton("Add one")
        button.clicked.connect(self.add)
        layout = QVBoxLayout(self)
        layout.addWidget(self.label)
        layout.addWidget(button)

    def add(self):
        self.count += 1
        self.label.setText(f"Clicked {self.count} times")
```

The slot has to remember to update the label. In RemedyUI the label is bound to the count,
so every change to the count shows, wherever it came from:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
# fn names(ui: &Ui) -> Vec<String> {
#     ui.reader_view().into_iter().filter_map(|n| n.name).collect()
# }
# use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

fn counter(ui: &Ui) -> El {
    let count = Signal::new(0u32);
    w::column(ui)
        .gap(8.0)
        .child(w::text(ui, move || format!("Clicked {} times", count.get())))
        .child(w::button(ui, "Add one", move || count.update(|c| *c += 1)))
}

fn app(app: &App) {
    app.open(WindowSpec::new("Counter"), |ui, _| {
        ui.root().child(counter(ui));
    });
}

let mut app = TestApp::new(AppConfig::new("Counter"), app);
let Some(ui) = app.ui() else {
    unreachable!("the window is open")
};
assert!(app.press(Role::Button, "Add one"));
assert!(names(&ui).contains(&"Clicked 1 times".to_string()));
```

A Qt signal with several receivers becomes a signal several bindings read. A signal you
emit to tell other objects something happened becomes a closure argument (`on_open: impl
Fn(&Key)`), as the widgets themselves take them.

## Actions and shortcuts

`QAction` bundles a label, an icon, a shortcut and a slot. [`crate::nav::Commands`] does
the same, and also gives people a command palette, toolbars they arrange and shortcuts they
rebind. Agents run commands through the same table, with the person's consent.

```python
save = QAction(QIcon("save.png"), "Save", self)
save.setShortcut(QKeySequence.Save)
save.triggered.connect(self.save)
toolbar.addAction(save)
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use std::cell::Cell;
use std::rc::Rc;

use remedy_ui::host::HostEvent;
use remedy_ui::input::{Chord, Key, KeyEvent, Modifiers, Platform};
use remedy_ui::nav::{Commands, toolbar};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets::Icon;
use remedy_ui::{App, WindowSpec};

let saved = Rc::new(Cell::new(0));
let s = saved.clone();
let app = move |app: &App| {
    app.open(WindowSpec::new("Editor"), move |ui, _| {
        let commands = Commands::new();
        let ctrl_s = Chord::primary(ui.platform(), Key::Character("s".into()));
        commands.add("save", "Save", vec![ctrl_s], Some(Icon::Check), move || s.set(s.get() + 1));
        commands.attach(ui); // the window's keys reach the commands
        let slot = Signal::new(vec!["save".to_string()]);
        ui.root().child(toolbar(ui, "Document", &commands, slot));
    });
};
let mut app = TestApp::new(AppConfig::new("Editor"), app);
let Some(id) = app.host().window_ids().first().copied() else { unreachable!("the window is open") };
let key = KeyEvent::press(Key::Character("s".into()))
    .with_physical("KeyS")
    .with_modifiers(Modifiers::primary(Platform::default()));
app.host().send(HostEvent::Key(id, key));
app.frames(1);
assert_eq!(saved.get(), 1);
```

The toolbar shows the command's icon, with its label and shortcut in the tooltip.
`set_enabled` replaces `QAction::setEnabled`.

## Layouts

```python
row = QHBoxLayout()
row.addWidget(QLabel("Zoom"))
row.addStretch()
row.addWidget(zoom_box)
```

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::layout::Align;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    app.open(WindowSpec::new("Zoom"), |ui, _| {
        let zoom = Signal::new(100.0f64);
        let row = w::row(ui)
            .align(Align::Center)
            .fill_width()
            .child(w::text(ui, "Zoom"))
            .child(w::spacer(ui))
            .child(w::stepper(ui, "Zoom", zoom, 1.0, 6400.0, 10.0));
        ui.root().child(row);
    });
}
# let mut app = TestApp::new(AppConfig::new("Zoom"), app);
# let Some(ui) = app.ui() else { unreachable!("open") };
# assert!(ui.reader_view().iter().any(|n| n.value.as_deref() == Some("100")));
```

Layout is flex and grid (as on the web), run before paint in the same frame. Sizes are in
logical pixels; the engine snaps to device pixels. See [`crate::guide::layout`].

## Animations

`QPropertyAnimation` animates a property between values over a duration. [`crate::El::animate`]
moves an element's opacity, colors, padding, radius, offset or scale to a target with a spring
or an easing curve, on the window's frame clock. A new target starts from where the value is,
with its speed, so an interrupted animation never jumps, and under reduced motion it is
instant:

```python
anim = QPropertyAnimation(effect, b"opacity")
anim.setDuration(200)
anim.setEndValue(0.0)
anim.start()
```

```
# use remedy_ui::testing::TestApp;
use remedy_ui::motion::{Motion, Pace, Target};
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let banner = w::text(ui, "Saved");
    let _ = s.set(banner.clone());
    ui.root().child(banner);
});
let (Some(banner), Some(ui)) = (slot.get(), app.ui()) else { unreachable!("built") };
// The theme's standard motion, as a spring.
banner.animate(Target::Opacity(0.0), ui.motion(Pace::Standard));
app.frames(1);
assert!(banner.is_animating());
app.frames(120);
assert!(!banner.is_animating(), "it settled, and asks for no more frames");
# let _ = Motion::Instant;
```

## QSS becomes rules over tokens

QSS names colors directly, so a dark mode or a high-contrast mode means another sheet.
RemedyUI rules name theme tokens, so the same sheet works for all twelve Remedy palettes,
light and dark, and high contrast ([`crate::themes::high_contrast`]).

```css
QPushButton#danger { background: #c0392b; border-radius: 6px; }
QPushButton#danger:hover { background: #e74c3c; }
```

```
use remedy_ui::style::{States, StylePatch};
use remedy_ui::{AppConfig, themes};

let config = AppConfig::new("Reader").theme(themes::ThemeChoice::default().resolve(None, false), |t| {
    let mut sheet = themes::sheet(t);
    let patch = StylePatch::new().background(t.colors.danger).radius(t.radii.medium);
    sheet.rule("button", Some("remove"), States::NONE, patch);
    let hover = StylePatch::new().background(t.colors.danger.mix_oklab(t.colors.text, 0.15));
    sheet.rule("button", Some("remove"), States::HOVER, hover);
    sheet
});
# let _ = config;
```

`QPushButton#danger` becomes `button(..).variant("remove")`. Pseudo-states (`:hover`,
`:pressed`, `:focus`, `:disabled`, `:checked`) are [`crate::style::States`].

## Models and views

Qt separates the model (`QAbstractTableModel`: `rowCount`, `data`, `dataChanged`) from the
view. In RemedyUI the model is a `Signal<Vec<T>>` and a key per row; columns say how to show
and sort a row. Setting the signal is `dataChanged`, `rowsInserted` and `layoutChanged` at
once. The table is virtual: only rows near the viewport exist, and large sorts run off the UI
thread.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::data::{Column, SortValue, TableState, table};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

#[derive(Clone)]
struct Page { number: u32, title: String }

let pages = Signal::new(vec![
    Page { number: 1, title: "Cover".into() },
    Page { number: 2, title: "Contents".into() },
]);
let app = move |app: &App| {
    app.open(WindowSpec::new("Outline").size(480.0, 320.0), move |ui, _| {
        let state = TableState::new(false);
        let columns = vec![
            Column::new("page", "Page", 80.0, |ui, p: &Page| w::text(ui, p.number.to_string()).data_text())
                .sort_by(|p| SortValue::Number(f64::from(p.number)))
                .align_end(),
            Column::text("title", "Title", 240.0, |p: &Page| p.title.clone()),
        ];
        let outline = table(ui, "Outline", pages, |p: &Page| p.number, columns, &state, |_number| {});
        ui.root().child(outline.grow(1.0));
    });
};
let mut app = TestApp::new(AppConfig::new("Outline"), app);
app.frames(1);
let Some(ui) = app.ui() else { unreachable!("open") };
let reader = ui.reader_view();
assert!(reader.iter().any(|n| n.name.as_deref().is_some_and(|s| s.contains("Contents"))));
```

[`crate::data::TableState`] holds what the user changed (sort, column order and widths,
selection) for the app to save, like `QHeaderView::saveState`.

## QPainter becomes a painter

`paintEvent` runs whenever Qt decides to repaint. A RemedyUI painter runs when the element is
first drawn and again only when a signal it read changes (or its size or theme changes):

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use remedy_ui::core::{Point, Vec2};
use remedy_ui::reactive::Signal;
use remedy_ui::view::TextPaint;
use remedy_ui::widgets as w;
use remedy_ui::{App, WindowSpec};

fn app(app: &App) {
    app.open(WindowSpec::new("Page number"), |ui, _| {
        let page = Signal::new(1u32);
        let badge = w::canvas(ui, move |cx| {
            let p = TextPaint { size: 13.0, weight: 600.0, color: cx.theme.colors.muted, mono: true };
            let corner = Point::new(cx.size.width - 8.0, cx.size.height - 8.0);
            cx.text(&format!("Page {}", page.get()), corner, Vec2::new(1.0, 1.0), p);
        });
        ui.root().child(badge.width(200.0).height(60.0));
    });
}
# let mut app = TestApp::new(AppConfig::new("Page"), app);
# assert!(!app.host().window_ids().is_empty());
```

Painters record into a display list ([`crate::PaintCx::list`]) in the element's own
coordinates: rectangles, rounded rectangles, borders, paths, gradients, images and clips.
Those types ([`crate::paint::Brush`], [`crate::paint::Item`], [`crate::paint::Path`]) are in
[`crate::paint`]. Text goes through [`crate::PaintCx::text`] and
[`crate::PaintCx::layout_text`], which use the window's fonts and hinting.

## Pages: RemedyPDF's viewer

RemedyPDF draws pages with MuPDF into a `QGraphicsView` or a scroll area of labels. With
[`crate::pages`], MuPDF stays in the app behind a [`crate::pages::PageSource`]: the page
count, each page's size in points, and a render callback that draws a region at a scale into
pixels. The engine does the rest: virtual layout, tiles on worker threads, deep zoom from 1%
to 6400%, rotation, spreads, selection, search, thumbnails, annotations and screen readers.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::AppConfig;
use std::sync::Arc;

use remedy_ui::core::Size;
use remedy_ui::pages::{Cancel, PageSource, PageView, RenderError, RenderRequest, page_view};
use remedy_ui::{App, WindowSpec};
use remedy_ui::paint::Image;

/// Stands in for the MuPDF document: blank A4 pages.
struct Blank;

impl PageSource for Blank {
    fn page_count(&self) -> usize {
        3
    }
    fn page_size(&self, _page: usize) -> Size {
        Size::new(595.0, 842.0)
    }
    fn render(&self, request: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError> {
        if cancel.is_cancelled() {
            return Err(RenderError::Cancelled);
        }
        let pixels = vec![255u8; request.width as usize * request.height as usize * 4];
        Ok(Image::new(request.width, request.height, pixels.into()))
    }
}

fn app(app: &App) {
    app.open(WindowSpec::new("Reader").size(800.0, 600.0), |ui, _| {
        let view = PageView::new(Arc::new(Blank));
        ui.root().child(page_view(ui, &view, "Report.pdf"));
        view.go_to_page(1);
    });
}
let mut app = TestApp::new(AppConfig::new("Reader"), app);
app.frames(1);
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Report.pdf")));
```

The render callback runs on page workers, so the source is `Send + Sync`. Check
[`crate::pages::Cancel`] in long renders: tiles that scrolled away are cancelled. See
[`crate::guide::documents`].

## Pitfalls

- **No blocking dialogs.** `QDialog::exec()` runs a nested event loop. RemedyUI dialogs are
  overlays with handlers; continue the work in the button's closure.
- **No `setText` from slots.** Calling methods on elements from handlers works, but it is
  the Qt habit. Set the signal and let the bound property follow, so every path that changes
  the value updates the screen.
- **No object ownership tree to manage.** Removing an element removes its children and
  disposes its bindings. Nothing needs `deleteLater`.
- **Threads hand results back.** Do not touch elements or signals from a worker thread.
  Return the result from [`crate::App::background`]'s work closure and set signals in `done`.
- **No `QStyle` subclassing.** Look comes from the theme and the style sheet; drawing that
  is not a widget's look goes in a painter.
