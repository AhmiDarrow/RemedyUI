//! Tables: sortable, resizable and reorderable columns, frozen columns, virtual rows,
//! selection and inline editing, for any number of rows.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::base::{self as w, Icon, Rejection};
use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, Platform, PointerPhase};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::style::States;
use remedy_ui_core::tree::ElementId;
use remedy_ui_core::{Insets, Size, Vec2};
use remedy_ui_engine::view::{Align, Drag, DropEffect, El, FieldKind, Justify, Ui};

use crate::data::model::{SORT_OFF_THREAD, Selection, SortDirection, SortValue, sorted_order};
use crate::data::rows::{RowBuilder, Rows};

type CellFn<T> = Rc<dyn Fn(&Ui, &T) -> El>;
type SortKey<T> = Rc<dyn Fn(&T) -> SortValue>;
type EditGet<T> = Rc<dyn Fn(&T) -> String>;
type EditSet<T> = Rc<dyn Fn(&T, String) -> Result<(), Rejection>>;

/// A table column.
pub struct Column<T> {
    /// A stable id (sort, order and widths refer to it).
    pub id: String,
    /// The heading.
    pub title: String,
    /// The starting width in logical pixels.
    pub width: f32,
    /// The narrowest a resize may make it.
    pub min_width: f32,
    /// Numbers read better aligned to the end.
    pub align_end: bool,
    cell: CellFn<T>,
    sort: Option<SortKey<T>>,
    edit: Option<(EditGet<T>, EditSet<T>)>,
}

impl<T: 'static> Column<T> {
    /// A column whose cells `cell` builds.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        width: f32,
        cell: impl Fn(&Ui, &T) -> El + 'static,
    ) -> Self {
        Column {
            id: id.into(),
            title: title.into(),
            width,
            min_width: 40.0,
            align_end: false,
            cell: Rc::new(cell),
            sort: None,
            edit: None,
        }
    }

    /// A column of text from `value`, sortable by it.
    pub fn text(
        id: impl Into<String>,
        title: impl Into<String>,
        width: f32,
        value: impl Fn(&T) -> String + 'static,
    ) -> Self {
        let value = Rc::new(value);
        let v2 = value.clone();
        Column::new(id, title, width, move |ui, row| w::text(ui, value(row)).data_text().nowrap())
            .sort_by(move |row| SortValue::Text(v2(row)))
    }

    /// Makes the column sortable by `key`.
    pub fn sort_by(mut self, key: impl Fn(&T) -> SortValue + 'static) -> Self {
        self.sort = Some(Rc::new(key));
        self
    }

    /// Makes cells editable in place (F2 or a double click): `get` gives the text to edit,
    /// `set` stores it or says why not.
    pub fn editable(
        mut self,
        get: impl Fn(&T) -> String + 'static,
        set: impl Fn(&T, String) -> Result<(), Rejection> + 'static,
    ) -> Self {
        self.edit = Some((Rc::new(get), Rc::new(set)));
        self
    }

    /// The narrowest width.
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }

    /// Aligns cells to the end edge.
    pub fn align_end(mut self) -> Self {
        self.align_end = true;
        self
    }
}

/// What the user changed about a table, for the app to keep (and save): sort, column order
/// and widths, frozen columns, and the selection.
#[derive(Clone)]
pub struct TableState<K: Clone + Eq + Hash + 'static> {
    /// The sort column and direction.
    pub sort: Signal<Option<(String, SortDirection)>>,
    /// Column ids in display order (empty: the order they were given).
    pub order: Signal<Vec<String>>,
    /// Widths the user set, by column id.
    pub widths: Signal<HashMap<String, f32>>,
    /// How many leading columns stay in place when the table scrolls sideways.
    pub frozen: Signal<usize>,
    /// The selection and keyboard cursor.
    pub selection: Selection<K>,
    drag: Rc<RefCell<Option<DragFn<K>>>>,
    drop: Rc<RefCell<Option<DropHooks<K>>>>,
}

type DragFn<K> = Rc<dyn Fn(&[K]) -> Option<Drag>>;
type AcceptFn<K> = Rc<dyn Fn(&Drag, Option<&K>) -> DropEffect>;
type DropFn<K> = Rc<dyn Fn(&Drag, Option<&K>, DropEffect)>;
type DropHooks<K> = (AcceptFn<K>, DropFn<K>);

impl<K: Clone + Eq + Hash + 'static> TableState<K> {
    /// Unsorted, given order and widths, nothing frozen; `multiple` allows selecting many.
    pub fn new(multiple: bool) -> Self {
        TableState {
            sort: Signal::new(None),
            order: Signal::new(Vec::new()),
            widths: Signal::new(HashMap::new()),
            frozen: Signal::new(0),
            selection: Selection::new(multiple),
            drag: Rc::default(),
            drop: Rc::default(),
        }
    }

    /// Lets rows be dragged: `make` says what dragging these rows (the selection, when the
    /// row pressed is in it; else that row) carries, or `None` to keep it a press. Set
    /// before the table is built.
    pub fn set_drag(&self, make: impl Fn(&[K]) -> Option<Drag> + 'static) {
        *self.drag.borrow_mut() = Some(Rc::new(make));
    }

    /// Lets the table take drops: `accept` says what a drop on a row (`Some(key)`) or on the
    /// table around the rows (`None`) would do, and `drop` receives it. A row that refuses
    /// passes the drop to the table. Set before the table is built.
    pub fn set_drop(
        &self,
        accept: impl Fn(&Drag, Option<&K>) -> DropEffect + 'static,
        drop: impl Fn(&Drag, Option<&K>, DropEffect) + 'static,
    ) {
        *self.drop.borrow_mut() = Some((Rc::new(accept), Rc::new(drop)));
    }
}

impl<K: Clone + Eq + Hash + 'static> std::fmt::Debug for TableState<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TableState")
    }
}

/// The row height of tables and trees: the theme's control height.
pub(crate) fn row_height(ui: &Ui) -> f32 {
    ui.theme().spacing.control_height.max(w::MIN_TARGET)
}

fn toggle_key(m: Modifiers, platform: Platform) -> bool {
    if platform == Platform::Apple { m.meta } else { m.ctrl }
}

/// A table of `rows` with `columns`. `key` identifies a row across changes; `state` holds
/// what the user changed; `on_activate` runs for Enter or a double click on a row.
///
/// Keyboard: arrows, Page Up and Page Down, Home and End move the cursor (Shift extends,
/// Ctrl moves without selecting, Space selects), Ctrl+A selects all, Enter activates, F2
/// edits the first editable cell. On a column heading: Enter sorts, Ctrl+Left and
/// Ctrl+Right resize, Alt+Shift+Left and Alt+Shift+Right move the column.
#[track_caller]
#[allow(clippy::too_many_lines)]
pub fn table<T: Clone + 'static, K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    rows: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    columns: Vec<Column<T>>,
    state: &TableState<K>,
    on_activate: impl Fn(&K) + 'static,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let h = row_height(ui);
    let key: Rc<dyn Fn(&T) -> K> = Rc::new(key);
    let on_activate: Rc<dyn Fn(&K)> = Rc::new(on_activate);
    let cols: Rc<Vec<Column<T>>> = Rc::new(columns);
    let state = state.clone();
    if state.order.get_untracked().is_ok_and(|o| o.is_empty()) {
        state.order.set(cols.iter().map(|c| c.id.clone()).collect());
    }

    // Display order (indices into rows) and the keys in that order.
    let view: Signal<Vec<usize>> = Signal::new(Vec::new());
    let shown_keys: Rc<RefCell<Vec<K>>> = Rc::default();
    let editing: Signal<Option<(K, String)>> = Signal::new(None);
    let edit_error: Signal<Option<String>> = Signal::new(None);
    let generation = Arc::new(AtomicU64::new(0));
    // The table has keyboard focus: its cursor row shows it.
    let focused = Signal::new(false);
    let root = ui.el("table").on_focus_change(move |on| focused.set(on));
    // The editor built for the cell being edited, focused once its row is in the tree.
    let editor: Rc<RefCell<Option<El>>> = Rc::default();

    let col_by_id = {
        let cols = cols.clone();
        move |id: &str| cols.iter().position(|c| c.id == id)
    };
    let width_of = {
        let cols = cols.clone();
        let widths = state.widths;
        move |c: usize| {
            let col = cols.get(c);
            let set = col.and_then(|col| widths.with(|m| m.get(&col.id).copied()));
            set.or(col.map(|c| c.width)).unwrap_or(120.0)
        }
    };
    // The columns in each pane, as indices into `cols`.
    let panes = {
        let col_by_id = col_by_id.clone();
        move || -> (Vec<usize>, Vec<usize>) {
            let order = state.order.get();
            let frozen = state.frozen.get();
            let idx: Vec<usize> = order.iter().filter_map(|id| col_by_id(id)).collect();
            let split = frozen.min(idx.len());
            (
                idx.get(..split).map(<[usize]>::to_vec).unwrap_or_default(),
                idx.get(split..).map(<[usize]>::to_vec).unwrap_or_default(),
            )
        }
    };
    let panes = Rc::new(panes);
    let width_of = Rc::new(width_of);

    // ---- one row's cells for a pane
    let make_cells = {
        let (cols, rows_s, key, width_of, ui2, editor) =
            (cols.clone(), rows, key.clone(), width_of.clone(), ui.clone(), editor.clone());
        Rc::new(move |row_el: El, idx: usize, which: &[usize]| -> El {
            let Some(row) = rows_s.with_untracked(|r| r.get(idx).cloned()).ok().flatten() else {
                return row_el;
            };
            let k = key(&row);
            let edit_now = editing.get_untracked().ok().flatten();
            let mut row_el = row_el;
            for &c in which {
                let Some(col) = cols.get(c) else { continue };
                let width = width_of(c);
                let editing_here = edit_now.as_ref().is_some_and(|(ek, cid)| *ek == k && *cid == col.id);
                let content = if editing_here && let Some((get, set)) = &col.edit {
                    let field = ui2.text_field("text_input", FieldKind::Single).label(
                        crate::data::format_now(&ui2, "table-edit", &[("column", col.title.as_str().into())]),
                    );
                    field.set_value(&get(&row));
                    let (f2, set, row2) = (field.clone(), set.clone(), row.clone());
                    let commit = Rc::new(move || match set(&row2, f2.value()) {
                        Ok(()) => {
                            edit_error.set(None);
                            editing.set(None);
                        }
                        Err(e) => edit_error.set(Some(e.to_string())),
                    });
                    let c2 = commit.clone();
                    let field = field.on_submit(move || c2());
                    let fe = field.el().fill_width().on_key(move |e| {
                        if e.pressed && e.key == Key::Escape {
                            edit_error.set(None);
                            editing.set(None);
                            return true;
                        }
                        false
                    });
                    let fe2 = fe.clone();
                    let fe = fe.bind(move || {
                        let bad = edit_error.get();
                        fe2.state(States::INVALID, bad.is_some());
                    });
                    *editor.borrow_mut() = Some(fe.clone());
                    fe
                } else {
                    (col.cell)(&ui2, &row)
                };
                let cell = ui2
                    .el("table_cell")
                    .row()
                    .align(Align::Center)
                    .justify(if col.align_end { Justify::End } else { Justify::Start })
                    .width(width)
                    // Cells keep their column's width; a narrow table scrolls sideways.
                    .layout(|l| l.shrink = 0.0)
                    .height(h)
                    .padding(Insets::symmetric(0.0, unit * 2.0))
                    .clip()
                    .child(content)
                    .semantics(Semantics::new(Role::Cell).label(col.title.clone()));
                row_el = row_el.child(cell);
            }
            row_el
        })
    };

    // ---- a row (one pane's part of it)
    let build_row = {
        let (rows_s, key, ui2, sel, keys, act, cols2) = (
            rows,
            key.clone(),
            ui.clone(),
            state.selection.clone(),
            shown_keys.clone(),
            on_activate.clone(),
            cols.clone(),
        );
        let (make_cells, width_of) = (make_cells.clone(), width_of.clone());
        let (drag_fn, drop_fns) = (state.drag.clone(), state.drop.clone());
        Rc::new(move |display: usize, which: Vec<usize>| -> El {
            let idx = view.with_untracked(|v| v.get(display).copied()).ok().flatten().unwrap_or(display);
            let Some(row) = rows_s.with_untracked(|r| r.get(idx).cloned()).ok().flatten() else {
                return ui2.el("table_row");
            };
            let k = key(&row);
            let total = keys.borrow().len();
            let el =
                ui2.el("table_row").row().height(h).semantics(
                    Semantics::new(Role::Row).set_position(display + 1, total).action(Action::Click),
                );
            let el = make_cells(el, idx, &which);
            let (sel2, k2, keys2, ui3, act2) =
                (sel.clone(), k.clone(), keys.clone(), ui2.clone(), act.clone());
            let (cols3, ui4, k3, which2, wo) =
                (cols2.clone(), ui2.clone(), k.clone(), which.clone(), width_of.clone());
            // A press in a selection of several keeps it (so it can be dragged); a release
            // that was not a drag then selects just this row.
            let kept: Rc<Cell<bool>> = Rc::default();
            let (kept2, sel5, k5) = (kept.clone(), sel.clone(), k.clone());
            let el = el
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
                        // A double click on an editable cell edits it; anywhere else it activates.
                        let mut row_el = ui4.element_at(e.position);
                        while let Some(r) =
                            row_el.as_ref().filter(|r| r.info().is_none_or(|i| i.widget != "table_row"))
                        {
                            row_el = r.parent();
                        }
                        let x = row_el.and_then(|r| r.bounds()).map(|b| e.position.x - b.min.x);
                        let mut at = 0.0;
                        let mut col = None;
                        for &c in &which2 {
                            let wd = wo(c);
                            if x.is_some_and(|x| x >= at && x < at + wd) {
                                col = cols3.get(c);
                                break;
                            }
                            at += wd;
                        }
                        match col.filter(|c| c.edit.is_some()) {
                            Some(c) => editing.set(Some((k3.clone(), c.id.clone()))),
                            None => act2(&k2),
                        }
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
                .keyboard_alternative(crate::data::tr_fn(&ui2, "table-row-keys"));
            let mut el = el;
            let drag = drag_fn.borrow().clone();
            if let Some(make) = drag {
                let (sel6, k6) = (sel.clone(), k.clone());
                el = el.draggable(
                    move || {
                        let rows = if sel6.contains(&k6) {
                            sel6.selected.get_untracked().unwrap_or_default()
                        } else {
                            vec![k6.clone()]
                        };
                        make(&rows)
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
            el.bind(move || {
                let on = sel3.contains(&k);
                let cursor = sel3.cursor.get().as_ref() == Some(&k) && focused.get();
                e2.state(States::SELECTED, on);
                e2.state(States::FOCUS_VISIBLE, cursor);
            })
        })
    };

    // ---- the panes
    let header_frozen = ui.el("table_header").row();
    let header_main_row = ui.el("table_header").row().layout(|l| l.shrink = 0.0);
    let main_body = ui
        .el("scroll")
        .scroll()
        .scroll_x()
        .column()
        .grow(1.0)
        .layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(0.0));
    let frozen_body = ui
        .el("scroll")
        .scroll()
        .column()
        .scroll_follows(&main_body, false, true)
        .layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(0.0));
    let header_main = ui
        .el("scroll")
        .scroll()
        .scroll_x()
        .row()
        .grow(1.0)
        .scroll_follows(&main_body, true, false)
        .child(header_main_row.clone());

    let main_cols = Rc::new(RefCell::new(Vec::<usize>::new()));
    let frozen_cols = Rc::new(RefCell::new(Vec::<usize>::new()));
    let (b1, mc) = (build_row.clone(), main_cols.clone());
    let main_builder: RowBuilder = Rc::new(move |i| b1(i, mc.borrow().clone()));
    let (b2, fc) = (build_row.clone(), frozen_cols.clone());
    let frozen_builder: RowBuilder = Rc::new(move |i| b2(i, fc.borrow().clone()));
    let (main_rows, main_column) = Rows::new(ui, h, main_builder);
    let main_column_el = main_column.clone();
    let (frozen_rows, frozen_column) = Rows::new(ui, h, frozen_builder);
    let main_rows = Rc::new(RefCell::new(main_rows));
    let frozen_rows = Rc::new(RefCell::new(frozen_rows));
    let main_body = main_body.child(main_column);
    let frozen_body = frozen_body.child(frozen_column);
    let viewport = Rc::new(Cell::new(600.0f32));

    let show = {
        let (mr, fr, mb, vp, ui2) =
            (main_rows.clone(), frozen_rows.clone(), main_body.clone(), viewport.clone(), ui.clone());
        Rc::new(move || {
            let y = ui2.scroll_offset(mb.id()).y;
            mr.borrow_mut().show(y, vp.get());
            fr.borrow_mut().show(y, vp.get());
        })
    };
    let rebuild = {
        let (mr, fr, show) = (main_rows.clone(), frozen_rows.clone(), show.clone());
        let fc = frozen_cols.clone();
        Rc::new(move |n: usize| {
            mr.borrow_mut().set_count(n, true);
            // With nothing frozen the frozen pane is hidden and builds no rows.
            fr.borrow_mut().set_count(if fc.borrow().is_empty() { 0 } else { n }, true);
            show();
        })
    };
    let (s1, vp2, s2) = (show.clone(), viewport.clone(), show.clone());
    let main_body = main_body.on_scroll(move |_| s1()).on_resize(move |sz: Size| {
        vp2.set(sz.height);
        s2();
    });

    // ---- headings; drops onto a heading find its column through `heading_ids`
    let heading_ids: Rc<RefCell<HashMap<ElementId, String>>> = Rc::default();
    let heading = {
        let (cols, ui2, width_of, heading_ids) =
            (cols.clone(), ui.clone(), width_of.clone(), heading_ids.clone());
        Rc::new(move |c: usize| -> El {
            let Some(col) = cols.get(c) else { return ui2.el("table_heading") };
            let id = col.id.clone();
            let sorted = state.sort.get_untracked().ok().flatten().filter(|(s, _)| *s == id).map(|(_, d)| d);
            let mut cell = ui2
                .el("table_heading")
                .row()
                .align(Align::Center)
                .gap(unit)
                .width(width_of(c))
                .layout(|l| l.shrink = 0.0)
                .height(h)
                .padding(Insets::symmetric(0.0, unit * 2.0))
                .child(w::text(&ui2, col.title.clone()).variant("strong").nowrap().grow(1.0));
            if let Some(d) = sorted {
                cell = cell.child(w::icon(
                    &ui2,
                    if d == SortDirection::Ascending { Icon::ChevronUp } else { Icon::ChevronDown },
                    ui2.theme().type_scale.small + 2.0,
                ));
            }
            let sortable = col.sort.is_some();
            let id2 = id.clone();
            // Reorder by dragging the heading sideways onto another column; a drag is not a
            // click, so it does not sort.
            let drag: Rc<Cell<Option<(f32, bool)>>> = Rc::default();
            let d0 = drag.clone();
            let cycle = move || {
                if !sortable || d0.get().is_some_and(|(_, moved)| moved) {
                    return;
                }
                let next = match state.sort.get_untracked().ok().flatten() {
                    Some((s, SortDirection::Ascending)) if s == id2 => {
                        Some((id2.clone(), SortDirection::Descending))
                    }
                    Some((s, SortDirection::Descending)) if s == id2 => None,
                    _ => Some((id2.clone(), SortDirection::Ascending)),
                };
                state.sort.set(next);
            };
            // Resize: drag the end edge (the press is captured, so the drag can go anywhere).
            let min = col.min_width;
            let (id3, wo) = (id.clone(), width_of.clone());
            let start: Rc<Cell<Option<(f32, f32)>>> = Rc::default();
            let grip = ui2
                .el("column_grip")
                .width(6.0)
                .height(h)
                .on_pointer(move |e| match e.phase {
                    PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                        start.set(Some((e.position.x, wo(c))));
                        true
                    }
                    PointerPhase::Move => match start.get() {
                        Some((x0, w0)) => {
                            let nw = (w0 + e.position.x - x0).max(min);
                            state.widths.update(|m| {
                                m.insert(id3.clone(), nw);
                            });
                            true
                        }
                        None => false,
                    },
                    PointerPhase::Up | PointerPhase::Cancel => start.take().is_some(),
                    _ => false,
                })
                .keyboard_alternative(crate::data::tr_fn(&ui2, "table-resize-keys"));
            let (id4, wo2, id5) = (id.clone(), width_of.clone(), id.clone());
            let keys = move |e: &KeyEvent| {
                if !e.pressed {
                    return false;
                }
                let m = e.modifiers;
                match e.key {
                    Key::ArrowLeft | Key::ArrowRight if m.ctrl && !m.alt => {
                        let by = if e.key == Key::ArrowRight { 16.0 } else { -16.0 };
                        let nw = (wo2(c) + by).max(min);
                        state.widths.update(|mm| {
                            mm.insert(id4.clone(), nw);
                        });
                        true
                    }
                    Key::ArrowLeft | Key::ArrowRight if m.alt && m.shift => {
                        let right = e.key == Key::ArrowRight;
                        state.order.update(|o| {
                            if let Some(i) = o.iter().position(|x| *x == id5) {
                                let j = if right {
                                    (i + 1).min(o.len().saturating_sub(1))
                                } else {
                                    i.saturating_sub(1)
                                };
                                o.swap(i, j);
                            }
                        });
                        true
                    }
                    _ => false,
                }
            };
            let desc = match sorted {
                Some(SortDirection::Ascending) => Some("table-sorted-ascending"),
                Some(SortDirection::Descending) => Some("table-sorted-descending"),
                None if sortable => Some("table-not-sorted"),
                None => None,
            };
            // Reorder by dragging the heading sideways onto another column.
            let (d2, id6, ui3, hids) = (drag.clone(), id.clone(), ui2.clone(), heading_ids.clone());
            let cell = cell
                .child(grip)
                .on_click(cycle)
                .on_key(keys)
                .on_pointer(move |e| match e.phase {
                    PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                        // Taking the press keeps the drag even when it leaves the heading.
                        d2.set(Some((e.position.x, false)));
                        true
                    }
                    PointerPhase::Move => match d2.get() {
                        Some((x0, moved)) => {
                            d2.set(Some((x0, moved || (e.position.x - x0).abs() > 12.0)));
                            true
                        }
                        None => false,
                    },
                    PointerPhase::Cancel => d2.take().is_some(),
                    PointerPhase::Up => {
                        if let Some((_, true)) = d2.take() {
                            // The heading under the pointer is where it goes.
                            let mut cur = ui3.element_at(e.position);
                            let mut over = None;
                            while let Some(c) = cur {
                                if let Some(t) = hids.borrow().get(&c.id()) {
                                    over = Some(t.clone());
                                    break;
                                }
                                cur = c.parent();
                            }
                            if let Some(t) = over.filter(|t| *t != id6) {
                                state.order.update(|o| {
                                    if let (Some(i), Some(j)) =
                                        (o.iter().position(|x| *x == id6), o.iter().position(|x| *x == t))
                                    {
                                        let moved = o.remove(i);
                                        o.insert(j, moved);
                                    }
                                });
                            }
                            return true;
                        }
                        false
                    }
                    _ => false,
                })
                .semantics({
                    let (u, title) = (ui2.clone(), col.title.clone());
                    move || {
                        let desc = desc.map(|id| crate::data::tr(&u, id)).unwrap_or_default();
                        Semantics::new(Role::ColumnHeader)
                            .label(title.clone())
                            .description(desc)
                            .action(Action::Click)
                    }
                })
                .keyboard_alternative(crate::data::tr_fn(&ui2, "table-move-keys"));
            heading_ids.borrow_mut().insert(cell.id(), id);
            cell
        })
    };

    let (hf, hm, mc2, fc2, panes2, heading2, rebuild2, fb2) = (
        header_frozen.clone(),
        header_main_row.clone(),
        main_cols.clone(),
        frozen_cols.clone(),
        panes.clone(),
        heading.clone(),
        rebuild.clone(),
        frozen_body.clone(),
    );
    let hids2 = heading_ids.clone();
    let mcol = main_column_el.clone();
    let width_sum = {
        let wo = width_of.clone();
        move |which: &[usize]| which.iter().map(|c| wo(*c)).sum::<f32>()
    };
    let layout_host = ui.el("table_layout").hidden(true);
    let layout_host = layout_host.bind(move || {
        let (frozen, main) = panes2();
        let _ = state.sort.get();
        let _ = state.widths.get();
        let total_f = width_sum(&frozen);
        let total_m = width_sum(&main);
        untrack(|| {
            *fc2.borrow_mut() = frozen.clone();
            *mc2.borrow_mut() = main.clone();
            hf.clear();
            hm.clear();
            hids2.borrow_mut().clear();
            for c in &frozen {
                let _ = hf.clone().child(heading2(*c));
            }
            for c in &main {
                let _ = hm.clone().child(heading2(*c));
            }
            let _ = fb2.clone().width(total_f).hidden(frozen.is_empty());
            // The scrolling part is as wide as its columns, so its rows and headings line up
            // and it scrolls sideways when the table is narrower.
            let _ = mcol.clone().layout(move |l| l.min_width = remedy_ui_core::layout::Length::Px(total_m));
            let _ = hm.clone().layout(move |l| l.min_width = remedy_ui_core::layout::Length::Px(total_m));
            let _ = hf.clone().hidden(frozen.is_empty());
            let n = view.get_untracked().map(|v| v.len()).unwrap_or(0);
            rebuild2(n);
        });
    });

    // ---- sorting: the display order, off the UI thread for large sets
    let sorter = {
        let (cols, key, keys, rebuild, generation, sel) = (
            cols.clone(),
            key.clone(),
            shown_keys.clone(),
            rebuild.clone(),
            generation.clone(),
            state.selection.clone(),
        );
        let col_by_id = col_by_id.clone();
        ui.el("table_sort").hidden(true).bind(move || {
            let list = rows.get();
            let sort = state.sort.get();
            untrack(|| {
                let gen_now = generation.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                let present: HashSet<K> = list.iter().map(|r| key(r)).collect();
                sel.retain(&present);
                let apply = {
                    let (keys, rebuild, list2, key2) =
                        (keys.clone(), rebuild.clone(), list.clone(), key.clone());
                    move |order: Vec<usize>| {
                        *keys.borrow_mut() =
                            order.iter().filter_map(|i| list2.get(*i)).map(|r| key2(r)).collect();
                        view.set(order.clone());
                        rebuild(order.len());
                    }
                };
                let sorter = sort.as_ref().and_then(|(id, d)| {
                    col_by_id(id).and_then(|c| cols.get(c)).and_then(|c| c.sort.clone()).map(|f| (f, *d))
                });
                match sorter {
                    None => apply((0..list.len()).collect()),
                    Some((f, dir)) => {
                        let values: Vec<SortValue> = list.iter().map(|r| f(r)).collect();
                        if values.len() <= SORT_OFF_THREAD {
                            apply(sorted_order(&values, dir));
                        } else {
                            // Keep showing the previous order until the sorted one arrives.
                            let waker = remedy_ui_core::reactive::waker().ok();
                            let result: Signal<Option<Vec<usize>>> = Signal::new(None);
                            let apply = Rc::new(apply);
                            let gen2 = generation.clone();
                            let _ = remedy_ui_core::reactive::Effect::new(move || {
                                if let Some(order) = result.get()
                                    && gen2.load(AtomicOrdering::SeqCst) == gen_now
                                {
                                    untrack(|| apply(order));
                                }
                            });
                            match waker {
                                Some(waker) => {
                                    let _ = remedy_ui_core::exec::spawn("remedy-table-sort", move || {
                                        let order = sorted_order(&values, dir);
                                        // A newer sort replaces this one's scope; its result then has nowhere to go.
                                        let _ = waker.post(move || {
                                            let _ = result.try_set(Some(order));
                                        });
                                    });
                                }
                                None => result.set(Some(sorted_order(&values, dir))),
                            }
                        }
                    }
                }
            });
        })
    };

    // ---- keyboard on the table
    let (keys, sel, mb, act, cols3, ui2) = (
        shown_keys.clone(),
        state.selection.clone(),
        main_body.clone(),
        on_activate.clone(),
        cols.clone(),
        ui.clone(),
    );
    let vp3 = viewport.clone();
    let on_key = move |e: &KeyEvent| {
        if !e.pressed || editing.get_untracked().ok().flatten().is_some() {
            return false;
        }
        let order = keys.borrow().clone();
        if order.is_empty() {
            return false;
        }
        let cur = sel.cursor.get_untracked().ok().flatten().and_then(|c| order.iter().position(|k| *k == c));
        let page = ((vp3.get() / h) as usize).max(1);
        let last = order.len() - 1;
        let target = match e.key {
            Key::ArrowDown => Some(cur.map_or(0, |c| (c + 1).min(last))),
            Key::ArrowUp => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            Key::PageDown => Some(cur.map_or(0, |c| (c + page).min(last))),
            Key::PageUp => Some(cur.map_or(0, |c| c.saturating_sub(page))),
            Key::Home => Some(0),
            Key::End => Some(last),
            Key::Character(ref s) if s == "a" && toggle_key(e.modifiers, ui2.platform()) => {
                sel.all(&order);
                return true;
            }
            Key::Space => {
                if let Some(k) = cur.and_then(|c| order.get(c)).cloned() {
                    sel.choose(k, Modifiers { ctrl: true, meta: true, ..Modifiers::NONE }, true, &order);
                }
                return true;
            }
            Key::Enter => {
                if let Some(k) = cur.and_then(|c| order.get(c)) {
                    act(k);
                }
                return true;
            }
            Key::F(2) => {
                // With nothing editable, F2 is the app's (a rename dialog, say).
                let col = cols3.iter().find(|c| c.edit.is_some()).map(|c| c.id.clone());
                if let (Some(k), Some(col)) = (cur.and_then(|c| order.get(c)).cloned(), col) {
                    editing.set(Some((k, col)));
                    return true;
                }
                return false;
            }
            _ => None,
        };
        let Some(t) = target else { return false };
        let Some(k) = order.get(t).cloned() else { return false };
        if toggle_key(e.modifiers, ui2.platform()) {
            sel.cursor.set(Some(k));
        } else {
            sel.choose(k, e.modifiers, false, &order);
        }
        // Keep the cursor row in view.
        let off = ui2.scroll_offset(mb.id());
        let top = t as f32 * h;
        let y = if top < off.y {
            top
        } else if top + h > off.y + vp3.get() {
            top + h - vp3.get()
        } else {
            off.y
        };
        if (y - off.y).abs() > f32::EPSILON {
            mb.scroll_to(Vec2::new(off.x, y));
        }
        true
    };

    let redraw = {
        let (rebuild, root, editor) = (rebuild.clone(), root.clone(), editor.clone());
        let was_editing = Cell::new(false);
        ui.el("table_edit").hidden(true).bind(move || {
            let now = editing.get().is_some();
            untrack(|| {
                editor.borrow_mut().take();
                rebuild(view.get_untracked().map(|v| v.len()).unwrap_or(0));
                match editor.borrow_mut().take() {
                    Some(field) => field.focus(),
                    // The editor went away with focus in it: the table takes focus back.
                    None if was_editing.get() => root.focus(),
                    None => {}
                }
            });
            was_editing.set(now);
        })
    };
    let header = ui.el("table_header_bar").row().fill_width().child(header_frozen).child(header_main);
    let bodies = ui
        .el("table_bodies")
        .row()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(0.0))
        .child(frozen_body)
        .child(main_body);
    let (n_rows, u) = (rows, ui.clone());
    let l2 = label.clone();
    // Drops around the rows go to the table itself.
    let drop = state.drop.borrow().clone();
    let root = match drop {
        Some((accept, on_drop)) => {
            root.on_drop(move |d, _| accept(d, None), move |d, _, e| on_drop(d, None, e))
        }
        None => root,
    };
    root.column()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(0.0))
        .focusable(true)
        .on_key(on_key)
        .child(header)
        .child(bodies)
        .child(layout_host)
        .child(sorter)
        .child(redraw)
        .semantics(move || {
            let count = n_rows.with(Vec::len);
            Semantics::new(Role::Table).label(l2.clone()).description(crate::data::STRINGS.format(
                &u,
                "table-count",
                &[("count", count.into())],
            ))
        })
}
