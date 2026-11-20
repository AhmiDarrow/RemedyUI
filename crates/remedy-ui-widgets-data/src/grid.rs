//! Icon grids (a file explorer's large-icon view): fixed-size tiles that wrap to the width,
//! virtual rows for any number of items, the table's selection model, rubber-band selection,
//! keyboard movement in two dimensions, and drag and drop.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Insets, Point, Rect, Size, Vec2};
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, Platform, PointerPhase};
use remedy_ui_layout::{AUTO, Length, Position};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::States;
use remedy_ui_view::{Drag, DropEffect, El, Ui};

use crate::model::Selection;
use crate::rows::{RowBuilder, Rows};

type DragFn<K> = Rc<dyn Fn(&[K]) -> Option<Drag>>;
type AcceptFn<K> = Rc<dyn Fn(&Drag, Option<&K>) -> DropEffect>;
type DropFn<K> = Rc<dyn Fn(&Drag, Option<&K>, DropEffect)>;
type DropHooks<K> = (AcceptFn<K>, DropFn<K>);
type TileFn<T> = Rc<dyn Fn(&Ui, &T) -> El>;

/// How a grid draws its items: each in a box of `size`, by `draw`.
pub struct Tiles<T> {
    /// One tile's size.
    pub size: Size,
    draw: TileFn<T>,
}

impl<T> Tiles<T> {
    /// Tiles of `size` drawn by `draw`.
    pub fn new(size: Size, draw: impl Fn(&Ui, &T) -> El + 'static) -> Self {
        Tiles { size, draw: Rc::new(draw) }
    }
}

impl<T> std::fmt::Debug for Tiles<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tiles({:?})", self.size)
    }
}

/// What the user changed about a grid (the selection), and its drag and drop hooks.
#[derive(Clone)]
pub struct GridState<K: Clone + Eq + Hash + 'static> {
    /// The selection and keyboard cursor.
    pub selection: Selection<K>,
    drag: Rc<RefCell<Option<DragFn<K>>>>,
    drop: Rc<RefCell<Option<DropHooks<K>>>>,
}

impl<K: Clone + Eq + Hash + 'static> GridState<K> {
    /// Nothing selected; `multiple` allows selecting many.
    pub fn new(multiple: bool) -> Self {
        GridState { selection: Selection::new(multiple), drag: Rc::default(), drop: Rc::default() }
    }

    /// Lets tiles be dragged: `make` says what dragging these items (the selection, when the
    /// tile pressed is in it; else that tile) carries, or `None` to keep it a press. Set
    /// before the grid is built.
    pub fn set_drag(&self, make: impl Fn(&[K]) -> Option<Drag> + 'static) {
        *self.drag.borrow_mut() = Some(Rc::new(make));
    }

    /// Lets the grid take drops: `accept` says what a drop on a tile (`Some(key)`) or on the
    /// space around the tiles (`None`) would do, and `drop` receives it. A tile that refuses
    /// passes the drop to the grid. Set before the grid is built.
    pub fn set_drop(
        &self,
        accept: impl Fn(&Drag, Option<&K>) -> DropEffect + 'static,
        drop: impl Fn(&Drag, Option<&K>, DropEffect) + 'static,
    ) {
        *self.drop.borrow_mut() = Some((Rc::new(accept), Rc::new(drop)));
    }
}

impl<K: Clone + Eq + Hash + 'static> std::fmt::Debug for GridState<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GridState")
    }
}

fn toggle_key(m: Modifiers, platform: Platform) -> bool {
    if platform == Platform::Apple { m.meta } else { m.ctrl }
}

/// Where tiles sit: `cols` to a row, each `tile` big with `gap` between and `pad` around.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridGeometry {
    /// Tiles in a row.
    pub cols: usize,
    /// One tile's size.
    pub tile: Size,
    /// Space between tiles (both ways).
    pub gap: f32,
    /// Space around all the tiles.
    pub pad: f32,
}

impl GridGeometry {
    /// How many tiles of `tile` fit across `width` (at least one).
    pub fn columns_for(width: f32, tile: Size, gap: f32, pad: f32) -> usize {
        let room = width - pad * 2.0 + gap;
        ((room / (tile.width + gap)).floor() as usize).max(1)
    }

    /// The height of one row of tiles with the gap under it.
    pub fn row_height(&self) -> f32 {
        self.tile.height + self.gap
    }

    /// Item `i`'s box, in content coordinates.
    pub fn rect_of(&self, i: usize) -> Rect {
        let (r, c) = (i / self.cols, i % self.cols);
        let x = self.pad + c as f32 * (self.tile.width + self.gap);
        let y = self.pad + r as f32 * self.row_height();
        Rect::from_origin_size(Point::new(x, y), self.tile)
    }

    /// The items of `n` whose boxes meet `band` (content coordinates).
    pub fn hits(&self, band: Rect, n: usize) -> Vec<usize> {
        if n == 0 || band.width() <= 0.0 && band.height() <= 0.0 {
            return Vec::new();
        }
        let rh = self.row_height();
        let first = (((band.min.y - self.pad) / rh).floor().max(0.0)) as usize;
        let last = (((band.max.y - self.pad) / rh).floor().max(0.0)) as usize;
        let mut out = Vec::new();
        for r in first..=last {
            for c in 0..self.cols {
                let i = r * self.cols + c;
                if i >= n {
                    return out;
                }
                if self.rect_of(i).intersect(band).is_some_and(|x| !x.is_empty()) {
                    out.push(i);
                }
            }
        }
        out
    }
}

/// A grid of `items`, each drawn as one of `tiles`. `key` identifies an item
/// across changes; `state` holds the selection; `on_activate` runs for Enter or a double click.
///
/// Mouse: click selects (Ctrl or Command toggles, Shift selects a range), a drag on the space
/// between tiles draws a selection band (with Ctrl or Shift it adds to the selection), a
/// click on that space selects nothing. Keyboard: the arrow keys move in two dimensions,
/// Page Up and Page Down by a screenful, Home and End to the ends (Shift extends, Ctrl moves
/// without selecting, Space selects), Ctrl+A selects all, Enter activates.
#[track_caller]
#[allow(clippy::too_many_lines)]
pub fn grid_view<T: Clone + 'static, K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    tiles: Tiles<T>,
    state: &GridState<K>,
    on_activate: impl Fn(&K) + 'static,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let (gap, pad) = (unit * 2.0, unit * 2.0);
    let key: Rc<dyn Fn(&T) -> K> = Rc::new(key);
    let (tile, tile_size) = (tiles.draw, tiles.size);
    let on_activate: Rc<dyn Fn(&K)> = Rc::new(on_activate);
    let state = state.clone();
    let geo = Rc::new(Cell::new(GridGeometry { cols: 1, tile: tile_size, gap, pad }));
    // The keys in display order.
    let keys: Rc<RefCell<Vec<K>>> = Rc::default();
    let focused = Signal::new(false);
    let root = ui.el("grid").on_focus_change(move |on| focused.set(on));

    // ---- one tile
    let build_tile = {
        let (ui2, sel, keys, act, key, tile) = (
            ui.clone(),
            state.selection.clone(),
            keys.clone(),
            on_activate.clone(),
            key.clone(),
            tile.clone(),
        );
        let (drag_fn, drop_fns) = (state.drag.clone(), state.drop.clone());
        Rc::new(move |i: usize, item: &T| -> El {
            let k = key(item);
            let total = keys.borrow().len();
            let content = tile(&ui2, item);
            let (sel2, k2, keys2, ui3, act2) =
                (sel.clone(), k.clone(), keys.clone(), ui2.clone(), act.clone());
            // A press in a selection of several keeps it (so it can be dragged); a release
            // that was not a drag then selects just this tile.
            let kept: Rc<Cell<bool>> = Rc::default();
            let (kept2, sel5, k5) = (kept.clone(), sel.clone(), k.clone());
            let mut el = ui2
                .el("grid_tile")
                .column()
                .width(tile_size.width)
                .height(tile_size.height)
                .layout(|l| l.shrink = 0.0)
                .clip()
                .child(content)
                .on_pointer(move |e| {
                    if e.phase == PointerPhase::Up {
                        if kept2.replace(false) && e.modifiers == Modifiers::NONE {
                            sel5.select_only(k5.clone());
                        }
                        return false;
                    }
                    if e.phase != PointerPhase::Down || !e.buttons.contains(Buttons::PRIMARY) {
                        return false;
                    }
                    if ui3.click_count() >= 2 {
                        act2(&k2);
                        return true;
                    }
                    let order = keys2.borrow().clone();
                    let many = sel2.selected.with_untracked(|s| s.len() > 1).unwrap_or(false);
                    if many && sel2.contains(&k2) && e.modifiers == Modifiers::NONE {
                        sel2.cursor.set(Some(k2.clone()));
                        kept.set(true);
                    } else {
                        sel2.choose(k2.clone(), e.modifiers, toggle_key(e.modifiers, ui3.platform()), &order);
                    }
                    true
                })
                .keyboard_alternative(crate::tr_fn(&ui2, "grid-item-keys"));
            let drag = drag_fn.borrow().clone();
            if let Some(make) = drag {
                let (sel6, k6) = (sel.clone(), k.clone());
                el = el.draggable(
                    move || {
                        let picked = if sel6.contains(&k6) {
                            sel6.selected.get_untracked().unwrap_or_default()
                        } else {
                            vec![k6.clone()]
                        };
                        make(&picked)
                    },
                    |_| {},
                );
            }
            let drop = drop_fns.borrow().clone();
            if let Some((accept, on_drop)) = drop {
                let (k7, k8) = (k.clone(), k.clone());
                el = el.on_drop(move |d, _| accept(d, Some(&k7)), move |d, _, e| on_drop(d, Some(&k8), e));
            }
            let (e2, sel3) = (el.clone(), sel.clone());
            let el = el.bind(move || {
                let on = sel3.contains(&k);
                let cursor = sel3.cursor.get().as_ref() == Some(&k) && focused.get();
                e2.state(States::SELECTED, on);
                e2.state(States::FOCUS_VISIBLE, cursor);
            });
            let (sel4, k9) = (sel.clone(), key(item));
            el.semantics(move || {
                Semantics::new(Role::ListItem)
                    .set_position(i + 1, total)
                    .selected(sel4.contains(&k9))
                    .action(Action::Click)
                    .merge_descendants()
            })
        })
    };

    // ---- rows of tiles
    let build_row: RowBuilder = {
        let (ui2, geo, build_tile) = (ui.clone(), geo.clone(), build_tile.clone());
        Rc::new(move |r: usize| -> El {
            let g = geo.get();
            let mut row = ui2.el("grid_row").row().gap(g.gap).height(g.row_height()).padding(Insets {
                top: 0.0,
                bottom: g.gap,
                start: g.pad,
                end: g.pad,
            });
            let start = r * g.cols;
            let picked: Vec<T> = items
                .with_untracked(|v| v.iter().skip(start).take(g.cols).cloned().collect())
                .unwrap_or_default();
            for (j, item) in picked.iter().enumerate() {
                row = row.child(build_tile(start + j, item));
            }
            row
        })
    };
    let (rows, rows_column) = Rows::new(ui, geo.get().row_height(), build_row);
    let rows = Rc::new(RefCell::new(rows));

    // The band, drawn over the tiles in content coordinates.
    let band_el = ui.el("grid_band").hidden(true).pass_through().layout(|l| l.position = Position::Absolute);
    let content = ui
        .el("grid_content")
        .column()
        .fill_width()
        .layout(|l| l.shrink = 0.0)
        .padding(Insets { top: pad, bottom: pad, start: 0.0, end: 0.0 })
        .child(rows_column)
        .child(band_el.clone());
    let body = ui
        .el("scroll")
        .scroll()
        .column()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = Length::Px(0.0))
        .child(content.clone());
    let viewport = Rc::new(Cell::new(Size::new(600.0, 400.0)));

    let show = {
        let (rows, body, vp, ui2) = (rows.clone(), body.clone(), viewport.clone(), ui.clone());
        Rc::new(move || {
            let y = ui2.scroll_offset(body.id()).y;
            // Rows count from below the top padding.
            rows.borrow_mut().show((y - pad).max(0.0), vp.get().height);
        })
    };
    let rebuild = {
        let (rows, show, geo) = (rows.clone(), show.clone(), geo.clone());
        Rc::new(move |n: usize| {
            let g = geo.get();
            let mut r = rows.borrow_mut();
            r.set_row_height(g.row_height());
            r.set_count(n.div_ceil(g.cols), true);
            drop(r);
            show();
        })
    };

    // Items changed: the order of keys, a selection without the items that went, new tiles.
    let (keys2, sel, key2, rebuild2) = (keys.clone(), state.selection.clone(), key.clone(), rebuild.clone());
    let watcher = ui.el("grid_items").hidden(true).bind(move || {
        let list = items.get();
        untrack(|| {
            let ks: Vec<K> = list.iter().map(|t| key2(t)).collect();
            let present: HashSet<K> = ks.iter().cloned().collect();
            sel.retain(&present);
            *keys2.borrow_mut() = ks;
            rebuild2(list.len());
        });
    });

    // The width decides the columns.
    let (s1, vp2, geo2, rebuild3, s2) =
        (show.clone(), viewport.clone(), geo.clone(), rebuild.clone(), show.clone());
    let body = body.on_scroll(move |_| s1()).on_resize(move |sz: Size| {
        vp2.set(sz);
        let mut g = geo2.get();
        let cols = GridGeometry::columns_for(sz.width, g.tile, g.gap, g.pad);
        if cols == g.cols {
            s2();
        } else {
            g.cols = cols;
            geo2.set(g);
            rebuild3(items.with_untracked(Vec::len).unwrap_or(0));
        }
    });

    // ---- the band: a drag on the space between tiles
    let to_content = {
        let (body, ui2) = (body.clone(), ui.clone());
        move |p: Point| -> Point {
            let origin = body.bounds().map_or(Point::new(0.0, 0.0), |b| b.min);
            let off = ui2.scroll_offset(body.id());
            Point::new(p.x - origin.x + off.x, p.y - origin.y + off.y)
        }
    };
    let band: Rc<Cell<Option<(Point, bool)>>> = Rc::default();
    let base: Rc<RefCell<Vec<K>>> = Rc::default();
    let moved = Rc::new(Cell::new(false));
    let (band2, base2, moved2, keys3, sel2, geo3, band_el2, root2, ui3, body2) = (
        band.clone(),
        base.clone(),
        moved.clone(),
        keys.clone(),
        state.selection.clone(),
        geo.clone(),
        band_el.clone(),
        root.clone(),
        ui.clone(),
        body.clone(),
    );
    let _ = content
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                root2.focus();
                let adding = sel2.multiple && (e.modifiers.shift || toggle_key(e.modifiers, ui3.platform()));
                *base2.borrow_mut() =
                    if adding { sel2.selected.get_untracked().unwrap_or_default() } else { Vec::new() };
                band2.set(Some((to_content(e.position), adding)));
                moved2.set(false);
                true
            }
            PointerPhase::Move => {
                let Some((start, _)) = band2.get() else { return false };
                let now = to_content(e.position);
                if !moved2.get() && (now.x - start.x).abs().max((now.y - start.y).abs()) < 4.0 {
                    return true;
                }
                moved2.set(true);
                let r = Rect::from_points(start, now);
                let (x, y, wd, ht) = (r.min.x, r.min.y, r.width(), r.height());
                let _ = band_el2.clone().hidden(false).width(wd).height(ht).layout(move |l| {
                    l.inset = Insets { top: y, start: x, end: AUTO, bottom: AUTO };
                });
                // The band is in content coordinates; tiles start below the top padding.
                let g = geo3.get();
                let hit_band = r.translate(Vec2::new(0.0, -pad));
                let order = keys3.borrow().clone();
                let mut picked = base2.borrow().clone();
                for i in g.hits(hit_band, order.len()) {
                    if let Some(k) = order.get(i)
                        && !picked.contains(k)
                    {
                        picked.push(k.clone());
                    }
                }
                if !sel2.multiple {
                    picked.truncate(1);
                }
                sel2.selected.set(picked);
                // Near an edge, keep scrolling so the band can reach items out of view.
                if let Some(vb) = body2.bounds() {
                    let off = ui3.scroll_offset(body2.id());
                    let dy = if e.position.y < vb.min.y {
                        e.position.y - vb.min.y
                    } else if e.position.y > vb.max.y {
                        e.position.y - vb.max.y
                    } else {
                        0.0
                    };
                    if dy != 0.0 {
                        body2.scroll_to(Vec2::new(off.x, (off.y + dy).max(0.0)));
                    }
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let Some((_, adding)) = band2.take() else { return false };
                let _ = band_el2.clone().hidden(true);
                // A click on the space between tiles selects nothing.
                if !moved2.get() && !adding && e.phase == PointerPhase::Up {
                    sel2.clear();
                    sel2.cursor.set(None);
                }
                true
            }
            _ => false,
        })
        .keyboard_alternative(crate::tr_fn(ui, "grid-keys"));

    // ---- keyboard
    let (keys4, sel3, act, ui4, geo4, vp3, body3) = (
        keys.clone(),
        state.selection.clone(),
        on_activate.clone(),
        ui.clone(),
        geo.clone(),
        viewport.clone(),
        body.clone(),
    );
    let on_key = move |e: &KeyEvent| {
        if !e.pressed {
            return false;
        }
        let order = keys4.borrow().clone();
        if order.is_empty() {
            return false;
        }
        let g = geo4.get();
        let cur = sel3.cursor.get_untracked().ok().flatten().and_then(|c| order.iter().position(|k| *k == c));
        let last = order.len() - 1;
        let page = (((vp3.get().height / g.row_height()) as usize).max(1)) * g.cols;
        let target = match e.key {
            Key::ArrowRight => Some(cur.map_or(0, |c| (c + 1).min(last))),
            Key::ArrowLeft => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            Key::ArrowDown => Some(cur.map_or(0, |c| if c + g.cols <= last { c + g.cols } else { c })),
            Key::ArrowUp => Some(cur.map_or(0, |c| c.checked_sub(g.cols).unwrap_or(c))),
            Key::PageDown => Some(cur.map_or(0, |c| (c + page).min(last))),
            Key::PageUp => Some(cur.map_or(0, |c| c.saturating_sub(page))),
            Key::Home => Some(0),
            Key::End => Some(last),
            Key::Character(ref s) if s == "a" && toggle_key(e.modifiers, ui4.platform()) => {
                sel3.all(&order);
                return true;
            }
            Key::Space => {
                if let Some(k) = cur.and_then(|c| order.get(c)).cloned() {
                    sel3.choose(k, Modifiers { ctrl: true, meta: true, ..Modifiers::NONE }, true, &order);
                }
                return true;
            }
            Key::Enter => {
                if let Some(k) = cur.and_then(|c| order.get(c)) {
                    act(k);
                }
                return true;
            }
            _ => None,
        };
        let Some(t) = target else { return false };
        let Some(k) = order.get(t).cloned() else { return false };
        if toggle_key(e.modifiers, ui4.platform()) {
            sel3.cursor.set(Some(k));
        } else {
            sel3.choose(k, e.modifiers, false, &order);
        }
        // Keep the cursor's row in view.
        let r = g.rect_of(t);
        let (top, bottom) = (r.min.y + pad, r.max.y + pad + g.gap);
        let off = ui4.scroll_offset(body3.id());
        let vh = vp3.get().height;
        let y = if top < off.y {
            top - pad
        } else if bottom > off.y + vh {
            bottom - vh
        } else {
            off.y
        };
        if (y - off.y).abs() > f32::EPSILON {
            body3.scroll_to(Vec2::new(off.x, y.max(0.0)));
        }
        true
    };

    let drop = state.drop.borrow().clone();
    let root = match drop {
        Some((accept, on_drop)) => {
            root.on_drop(move |d, _| accept(d, None), move |d, _, e| on_drop(d, None, e))
        }
        None => root,
    };
    let (n_items, u) = (items, ui.clone());
    root.column()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = Length::Px(0.0))
        .focusable(true)
        .on_key(on_key)
        .child(body)
        .child(watcher)
        .semantics(move || {
            let count = n_items.with(Vec::len);
            Semantics::new(Role::List).label(label.clone()).description(crate::STRINGS.format(
                &u,
                "grid-count",
                &[("count", count.into())],
            ))
        })
}

#[cfg(test)]
mod tests {
    use remedy_ui_core::{Point, Rect, Size};

    use super::GridGeometry;

    fn g(cols: usize) -> GridGeometry {
        GridGeometry { cols, tile: Size::new(100.0, 80.0), gap: 10.0, pad: 5.0 }
    }

    #[test]
    fn tiles_wrap_to_the_width() {
        let t = Size::new(100.0, 80.0);
        assert_eq!(GridGeometry::columns_for(10.0, t, 10.0, 5.0), 1, "never fewer than one");
        assert_eq!(GridGeometry::columns_for(110.0, t, 10.0, 5.0), 1);
        // 3 tiles: 3*100 + 2*10 + 2*5 = 330.
        assert_eq!(GridGeometry::columns_for(330.0, t, 10.0, 5.0), 3);
        assert_eq!(GridGeometry::columns_for(329.0, t, 10.0, 5.0), 2);
    }

    #[test]
    fn boxes_and_band_hits() {
        let geo = g(3);
        assert_eq!(geo.rect_of(0), Rect::new(5.0, 5.0, 105.0, 85.0));
        assert_eq!(geo.rect_of(4), Rect::new(115.0, 95.0, 215.0, 175.0));
        // A band over the gap between the first two rows meets nothing.
        assert!(geo.hits(Rect::new(0.0, 86.0, 400.0, 94.0), 9).is_empty());
        // From inside item 1 to inside item 5.
        assert_eq!(
            geo.hits(Rect::from_points(Point::new(150.0, 50.0), Point::new(300.0, 120.0)), 9),
            vec![1, 2, 4, 5]
        );
        // Past the last item stops at it.
        assert_eq!(geo.hits(Rect::new(0.0, 0.0, 400.0, 1000.0), 4), vec![0, 1, 2, 3]);
    }
}
