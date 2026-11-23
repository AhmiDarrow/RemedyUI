//! Icon grids (a file explorer's large-icon view, a media library's posters): fixed-size or
//! fluid tiles that wrap to the width, virtual rows for any number of items (in their own
//! scroll or the page's), the table's selection model, rubber-band selection,
//! keyboard movement in two dimensions, and drag and drop.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
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

/// How a grid draws its items: each in a box of `size` (or a fluid one), by `draw`.
pub struct Tiles<T> {
    /// One tile's size; for fluid tiles, the size at the narrowest.
    pub size: Size,
    /// Fluid tiles: (height over width of the part that scales, height added below it).
    fluid: Option<(f32, f32)>,
    draw: TileFn<T>,
    /// The space between tiles (`None`: the theme's).
    gap: Option<f32>,
    /// The variant the tiles' boxes (`grid_tile`) are styled by.
    variant: &'static str,
}

impl<T> Tiles<T> {
    /// Tiles of `size` drawn by `draw`.
    pub fn new(size: Size, draw: impl Fn(&Ui, &T) -> El + 'static) -> Self {
        Tiles { size, fluid: None, draw: Rc::new(draw), gap: None, variant: "" }
    }

    /// Tiles that fill the width, as CSS `repeat(auto-fill, minmax(min_width, 1fr))`: as many
    /// columns as fit at `min_width`, stretched to share the rest. A tile is `aspect` times
    /// its width tall (1.5 for a 2:3 poster) plus `extra_height` (a caption under it).
    /// Columns and sizes follow the width as it changes.
    pub fn fluid(
        min_width: f32,
        aspect: f32,
        extra_height: f32,
        draw: impl Fn(&Ui, &T) -> El + 'static,
    ) -> Self {
        let (w, a, e) = (min_width.max(1.0), aspect.max(0.0), extra_height.max(0.0));
        Tiles { size: Size::new(w, w * a + e), fluid: Some((a, e)), draw: Rc::new(draw), gap: None, variant: "" }
    }

    /// `gap` logical pixels between tiles, instead of the theme's spacing: a grid of cards
    /// and a list of rows in one app each keep their own.
    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = Some(gap.max(0.0));
        self
    }

    /// Styles the tiles' boxes as the `grid_tile` variant `variant`, so two grids in one
    /// app (cards, rows) can look different in every state.
    #[must_use]
    pub fn variant(mut self, variant: &'static str) -> Self {
        self.variant = variant;
        self
    }
}

/// How a grid sits in its page and what its tiles hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GridOptions {
    /// The grid scrolls by itself (true, the default) or grows to all its rows inside the
    /// page's scroll, building only the rows that scroll shows.
    pub own_scroll: bool,
    /// Tiles keep their own controls (Play, Details, a bookmark) for the keyboard and screen
    /// readers: Enter moves into the cursor's tile, the arrow keys between its controls, and
    /// Escape back to the grid. Off (the default), a tile reads as one list item.
    pub interactive_tiles: bool,
}

impl Default for GridOptions {
    fn default() -> Self {
        GridOptions { own_scroll: true, interactive_tiles: false }
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

    /// Fluid tiles across `width` ([`Tiles::fluid`]): as many columns as fit at `min_width`,
    /// stretched to fill it, `aspect` times their width tall plus `extra_height`.
    pub fn fluid(width: f32, min_width: f32, aspect: f32, extra_height: f32, gap: f32, pad: f32) -> Self {
        let cols = Self::columns_for(width, Size::new(min_width, 0.0), gap, pad);
        let w = ((width - pad * 2.0 - gap * (cols - 1) as f32) / cols as f32).max(1.0);
        GridGeometry { cols, tile: Size::new(w, w * aspect + extra_height), gap, pad }
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
pub fn grid_view<T: Clone + 'static, K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    tiles: Tiles<T>,
    state: &GridState<K>,
    on_activate: impl Fn(&K) + 'static,
) -> El {
    grid_view_with(ui, label, items, key, tiles, state, GridOptions::default(), on_activate)
}

/// [`grid_view`] with `options`: in the page's scroll, and tiles whose controls the keyboard
/// and screen readers reach ([`GridOptions`]).
#[track_caller]
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn grid_view_with<T: Clone + 'static, K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    tiles: Tiles<T>,
    state: &GridState<K>,
    options: GridOptions,
    on_activate: impl Fn(&K) + 'static,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let (gap, pad) = (tiles.gap.unwrap_or(unit * 2.0), unit * 2.0);
    let key: Rc<dyn Fn(&T) -> K> = Rc::new(key);
    let tile_variant = tiles.variant;
    let (tile, tile_size, fluid) = (tiles.draw, tiles.size, tiles.fluid);
    let GridOptions { own_scroll, interactive_tiles: interactive } = options;
    // Built tiles by index, for Enter to move into (cleared when the rows rebuild).
    let built: Rc<RefCell<HashMap<usize, El>>> = Rc::default();
    let on_activate: Rc<dyn Fn(&K)> = Rc::new(on_activate);
    let state = state.clone();
    let geo = Rc::new(Cell::new(GridGeometry { cols: 1, tile: tile_size, gap, pad }));
    // The keys in display order.
    let keys: Rc<RefCell<Vec<K>>> = Rc::default();
    let focused = Signal::new(false);
    let root = ui.el("grid").on_focus_change(move |on| focused.set(on));
    // The part of the grid that shows: (offset from its top, height).
    let view = Rc::new(Cell::new((0.0f32, 400.0f32)));

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
        let (geo, built, root) = (geo.clone(), built.clone(), root.clone());
        Rc::new(move |i: usize, item: &T| -> El {
            let tile_size = geo.get().tile;
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
                .variant(tile_variant)
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
            if interactive {
                // Inside a tile: arrows between its controls, Escape back to the grid; focus
                // on a control puts the grid's cursor on its tile.
                let (me, root2, sel7, k10) = (el.clone(), root.clone(), sel.clone(), k.clone());
                el = el
                    .on_key(move |e| {
                        if !e.pressed {
                            return false;
                        }
                        match e.key {
                            Key::Escape => {
                                root2.focus();
                                true
                            }
                            Key::ArrowRight | Key::ArrowDown => me.focus_step(true),
                            Key::ArrowLeft | Key::ArrowUp => me.focus_step(false),
                            _ => false,
                        }
                    })
                    .on_focus_within(move |on| {
                        if on && sel7.cursor.get_untracked().ok().flatten().as_ref() != Some(&k10) {
                            sel7.cursor.set(Some(k10.clone()));
                        }
                    });
            }
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
            let el = el.semantics(move || {
                let s = Semantics::new(Role::ListItem)
                    .set_position(i + 1, total)
                    .selected(sel4.contains(&k9))
                    .action(Action::Click);
                // Interactive tiles keep their buttons in the tree.
                if interactive { s } else { s.merge_descendants() }
            });
            built.borrow_mut().insert(i, el.clone());
            el
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
    let body = if own_scroll {
        ui.el("scroll").scroll().column().grow(1.0).fill_width().layout(|l| l.min_height = Length::Px(0.0))
    } else {
        ui.el("grid_body").column().fill_width().layout(|l| l.shrink = 0.0)
    }
    .child(content.clone());

    let show = {
        let (rows, view) = (rows.clone(), view.clone());
        Rc::new(move || {
            let (y, h) = view.get();
            // Rows count from below the top padding.
            rows.borrow_mut().show((y - pad).max(0.0), h);
        })
    };
    let rebuild = {
        let (rows, show, geo, built) = (rows.clone(), show.clone(), geo.clone(), built.clone());
        Rc::new(move |n: usize| {
            let g = geo.get();
            built.borrow_mut().clear();
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

    // The width decides the columns (and a fluid tile's size).
    let relayout = {
        let (geo, rebuild, show) = (geo.clone(), rebuild.clone(), show.clone());
        move |width: f32| {
            let old = geo.get();
            let g = match fluid {
                Some((aspect, extra)) => GridGeometry::fluid(width, tile_size.width, aspect, extra, gap, pad),
                None => GridGeometry { cols: GridGeometry::columns_for(width, tile_size, gap, pad), ..old },
            };
            if g == old {
                show();
            } else {
                geo.set(g);
                rebuild(items.with_untracked(Vec::len).unwrap_or(0));
            }
        }
    };
    let body = if own_scroll {
        let (s1, v1, v2, body2, ui2) = (show.clone(), view.clone(), view.clone(), body.clone(), ui.clone());
        body.on_scroll(move |o| {
            v1.set((o.y, v1.get().1));
            s1();
        })
        .on_resize(move |sz: Size| {
            v2.set((ui2.scroll_offset(body2.id()).y, sz.height));
            relayout(sz.width);
        })
    } else {
        // In the page's scroll: the rows the window shows of the grid, wherever it scrolls.
        let (s1, v1) = (show.clone(), view.clone());
        body.on_placed(move |p| {
            if let Some(p) = p {
                v1.set((p.visible.min.y - p.bounds.min.y, p.visible.height()));
                s1();
            }
        })
        .on_resize(move |sz: Size| relayout(sz.width))
    };

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
                    // Content points: left is left in either direction.
                    l.inset_physical = true;
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
    let (keys4, sel3, act, ui4, geo4, vp3, body3, built2) = (
        keys.clone(),
        state.selection.clone(),
        on_activate.clone(),
        ui.clone(),
        geo.clone(),
        view.clone(),
        body.clone(),
        built.clone(),
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
        let page = (((vp3.get().1 / g.row_height()) as usize).max(1)) * g.cols;
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
                // An interactive tile takes the focus onto its first control.
                let entered = interactive
                    && cur.and_then(|c| built2.borrow().get(&c).cloned()).is_some_and(|t| t.focus_first());
                if !entered && let Some(k) = cur.and_then(|c| order.get(c)) {
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
        if !own_scroll {
            // Its tile may not be built yet: show its box through the page's scroll.
            body3.scroll_rect_into_view(Rect::new(
                r.min.x,
                r.min.y + pad - g.gap,
                r.max.x,
                r.max.y + pad + g.gap,
            ));
            return true;
        }
        let (top, bottom) = (r.min.y + pad, r.max.y + pad + g.gap);
        let off = ui4.scroll_offset(body3.id());
        let vh = vp3.get().1;
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
    let root = if own_scroll {
        root.grow(1.0).layout(|l| l.min_height = Length::Px(0.0))
    } else {
        root.layout(|l| l.shrink = 0.0)
    };
    root.column().fill_width().focusable(true).on_key(on_key).child(body).child(watcher).semantics(
        move || {
            let count = n_items.with(Vec::len);
            Semantics::new(Role::List).label(label.clone()).description(crate::STRINGS.format(
                &u,
                "grid-count",
                &[("count", count.into())],
            ))
        },
    )
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
