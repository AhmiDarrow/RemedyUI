//! A grid of color swatches to choose from.

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, Point};
use remedy_ui_input::Key;
use remedy_ui_layout::{Display, Length, Track};
use remedy_ui_paint::{Brush, Item, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_style::StylePatch;
use remedy_ui_view::{El, Ui};
use std::cell::Cell;
use std::rc::Rc;

/// One swatch: a color and the name screen readers and tooltips use for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    /// The color.
    pub color: Color,
    /// Its name ("Moss", "Signal red").
    pub name: String,
}

impl Swatch {
    /// A swatch of `color` called `name`.
    pub fn new(color: Color, name: impl Into<String>) -> Swatch {
        Swatch { color, name: name.into() }
    }
}

/// How a [`color_swatch_grid`] lays out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SwatchOptions {
    /// A fixed number of columns. `None` wraps swatches to the width the grid is given.
    pub columns: Option<usize>,
    /// The side of one swatch. `None` takes seven theme units.
    pub size: Option<f32>,
}

impl SwatchOptions {
    /// Sets `columns`: a fixed number of columns.
    pub fn columns(mut self, value: usize) -> Self {
        self.columns = Some(value);
        self
    }

    /// Sets `size`: the side of one swatch.
    pub fn size(mut self, value: f32) -> Self {
        self.size = Some(value);
        self
    }
}

fn hex(c: Color) -> String {
    let h = c.to_hex();
    if c.a >= 0.999 { h.chars().take(7).collect() } else { h }
}

/// A grid of color swatches, one of which can be chosen. The grid takes keyboard focus as one
/// stop: arrow keys move the cursor over the swatches (Home and End to the row's ends, Ctrl
/// with them to the first and last swatch) and Enter or Space chooses the one under it.
/// Clicking a swatch chooses it. Each swatch is a grid cell named for its color, so screen
/// readers read "Moss, selected", and the grid reads where the cursor is. The grid mirrors
/// in right-to-left windows and the Left and Right arrow keys follow.
#[track_caller]
pub fn color_swatch_grid(
    ui: &Ui,
    label: impl Into<String>,
    swatches: Signal<Vec<Swatch>>,
    selected: Signal<Option<usize>>,
    options: SwatchOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let size = options.size.unwrap_or(unit * 7.0).max(8.0);
    let gap = unit * 1.5;
    let cursor = Signal::new(0usize);
    let focused = Signal::new(false);

    let root = ui.el("color_swatch_grid").focusable(true).gap(gap);
    let root = match options.columns {
        Some(n) => {
            let n = n.max(1);
            root.layout(move |l| {
                l.display = Display::Grid;
                l.columns = (0..n).map(|_| Track::Px(size)).collect();
                l.gap = (gap, gap);
            })
        }
        None => root.row().layout(|l| {
            l.wrap = true;
            l.width = Length::Percent(1.0);
        }),
    };

    // The columns the cursor moves over: fixed, or the swatches on the first line.
    let columns = {
        let root = root.clone();
        move || columns_of(&options, &root)
    };

    // Rebuild the cells when the swatches change.
    let host = root.clone();
    let ui2 = ui.clone();
    let rebuilt = Rc::new(Cell::new(false));
    let build = move || {
        let list = swatches.get();
        let _ = rebuilt.replace(true);
        host.clear();
        let count = list.len();
        if cursor.get_untracked().unwrap_or(0) >= count {
            cursor.set(count.saturating_sub(1));
        }
        for (i, sw) in list.into_iter().enumerate() {
            let cell = ui2
                .el("swatch_cell")
                .width(size)
                .height(size)
                .layout(|l| l.shrink = 0.0)
                .style(StylePatch::default().background(sw.color));
            let mark_color = sw.color;
            let cell = cell.paint(move |cx| {
                let on = selected.get() == Some(i);
                let at_cursor = focused.get() && cursor.get() == i;
                let c = &cx.theme.colors;
                let ink =
                    if c.text.contrast(mark_color) >= c.bg.contrast(mark_color) { c.text } else { c.bg };
                if at_cursor {
                    cx.list.border(
                        remedy_ui_core::Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
                        remedy_ui_core::CornerRadii::all(cx.style.radius),
                        2.0,
                        cx.theme.derive().focus,
                    );
                }
                if on {
                    let s = cx.size.width.min(cx.size.height);
                    let mut p = Path::new();
                    p.move_to(Point::new(s * 0.28, s * 0.52));
                    p.line_to(Point::new(s * 0.45, s * 0.68));
                    p.line_to(Point::new(s * 0.73, s * 0.34));
                    let mut stroke = Stroke::new((s * 0.1).max(2.0));
                    stroke.cap = StrokeCap::Round;
                    stroke.join = StrokeJoin::Round;
                    cx.list.push(Item::StrokePath { path: p, stroke, brush: Brush::Solid(ink) });
                }
            });
            let name = sw.name.clone();
            let code = hex(sw.color);
            let sem_cell = cell.clone();
            let focus_root = host.clone();
            let cell = cell
                .on_click(move || {
                    selected.set_if_changed(Some(i));
                    cursor.set_if_changed(i);
                    focus_root.focus();
                })
                // The grid is the one keyboard stop; a swatch is only a target for the pointer.
                .focusable(false);
            let cell = cell.semantics(move || {
                Semantics::new(Role::GridCell)
                    .label(name.clone())
                    .description(code.clone())
                    .selected(selected.get() == Some(i))
                    .set_position(i + 1, count)
                    .action(remedy_ui_a11y::Action::Click)
            });
            remedy_ui_widgets_core::tooltip(&sem_cell, sw.name);
            let _ = host.clone().child(cell);
        }
    };
    let root = root.bind(build);

    let key_el = root.clone();
    let sem_el = root.clone();
    let l2 = label.clone();
    let u = ui.clone();
    root.on_focus_change(move |f| focused.set_if_changed(f))
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            let n = swatches.get_untracked().map_or(0, |v| v.len());
            if n == 0 {
                return false;
            }
            let rtl = key_el.is_rtl();
            let cols = columns();
            let cur = cursor.get_untracked().unwrap_or(0).min(n - 1);
            let row_start = cur / cols * cols;
            let next = match &e.key {
                Key::ArrowRight => {
                    if rtl {
                        cur.saturating_sub(1)
                    } else {
                        (cur + 1).min(n - 1)
                    }
                }
                Key::ArrowLeft => {
                    if rtl {
                        (cur + 1).min(n - 1)
                    } else {
                        cur.saturating_sub(1)
                    }
                }
                Key::ArrowDown => {
                    if cur + cols < n {
                        cur + cols
                    } else {
                        cur
                    }
                }
                Key::ArrowUp => cur.saturating_sub(cols).min(cur),
                Key::Home if e.modifiers.ctrl => 0,
                Key::End if e.modifiers.ctrl => n - 1,
                Key::Home => row_start,
                Key::End => (row_start + cols - 1).min(n - 1),
                Key::Enter | Key::Space => {
                    selected.set_if_changed(Some(cur));
                    return true;
                }
                _ => return false,
            };
            cursor.set_if_changed(next);
            true
        })
        .semantics(move || {
            let list = swatches.get();
            let cur = cursor.get().min(list.len().saturating_sub(1));
            let mut s = Semantics::new(Role::Grid).label(l2.clone());
            if let Some(sw) = list.get(cur) {
                let cols = columns_of(&options, &sem_el);
                let picked = selected.get() == Some(cur);
                let template = if picked {
                    u.word("{name}, {color}, row {row} column {column}, selected")
                } else {
                    u.word("{name}, {color}, row {row} column {column}")
                };
                s = s.value(remedy_ui_view::words::fill(&template, &[
                    ("name", &sw.name),
                    ("color", &hex(sw.color)),
                    ("row", &(cur / cols + 1).to_string()),
                    ("column", &(cur % cols + 1).to_string()),
                ]));
            }
            s
        })
}

fn columns_of(options: &SwatchOptions, root: &El) -> usize {
    if let Some(n) = options.columns {
        return n.max(1);
    }
    let kids = root.child_elements();
    match kids.first().and_then(El::bounds) {
        Some(f) => {
            kids.iter().filter_map(El::bounds).filter(|b| (b.min.y - f.min.y).abs() < 1.0).count().max(1)
        }
        None => 1,
    }
}
