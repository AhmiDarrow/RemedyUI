//! The board on screen: its areas on the grid with their widgets, the same for the person
//! arranging it and for visitors. Editing adds selection, dragging, keys for every move, an
//! undo history, and a panel to add areas and widgets and edit their settings.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::Key;
use remedy_ui_layout::{Display, Length, Track};
use remedy_ui_reactive::{Effect, Signal, untrack};
use remedy_ui_script::SettingKind;
use remedy_ui_style::States;
use remedy_ui_view::{Align, Drag, DragData, DropEffect, El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_data::{Property, PropertyGroup, property_grid};
use serde_json::Value;

use crate::kinds::{Context, Registry, value};
use crate::model::{Board, BoardError};

/// What is selected while editing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// An area, by id.
    Area(String),
    /// A widget, by id.
    Widget(String),
}

/// An area being dragged.
struct AreaMove(String);
/// A widget being dragged.
struct WidgetMove(String);

/// A board being shown (and maybe edited): its document, the mode, the selection, and the
/// undo history. Cheap to clone.
#[derive(Clone)]
pub struct BoardState {
    /// The document. Set it to show another board (a peer sent a new version).
    pub doc: Signal<Board>,
    /// Editing (true) or viewing.
    pub editing: Signal<bool>,
    /// What is selected while editing.
    pub selected: Signal<Option<Selection>>,
    /// Why the last change was refused, until the next one.
    pub message: Signal<Option<String>>,
    undo: Rc<RefCell<Vec<Board>>>,
    redo: Rc<RefCell<Vec<Board>>>,
    /// The setting being typed into: edits to it share one undo step.
    typing: Rc<RefCell<Option<(String, String)>>>,
    /// Undo is possible.
    pub can_undo: Signal<bool>,
    /// Redo is possible.
    pub can_redo: Signal<bool>,
}

impl std::fmt::Debug for BoardState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoardState")
    }
}

impl BoardState {
    /// Shows `doc`, viewing.
    pub fn new(doc: Board) -> BoardState {
        BoardState {
            doc: Signal::new(doc),
            editing: Signal::new(false),
            selected: Signal::new(None),
            message: Signal::new(None),
            undo: Rc::default(),
            redo: Rc::default(),
            typing: Rc::default(),
            can_undo: Signal::new(false),
            can_redo: Signal::new(false),
        }
    }

    fn current(&self) -> Board {
        self.doc.get_untracked().unwrap_or_default()
    }

    fn sync(&self) {
        self.can_undo.set_if_changed(!self.undo.borrow().is_empty());
        self.can_redo.set_if_changed(!self.redo.borrow().is_empty());
    }

    /// Applies a change: on success it becomes the board (and can be undone); on refusal the
    /// reason is shown. True when it applied.
    pub fn apply(&self, change: Result<Board, BoardError>) -> bool {
        self.typing.borrow_mut().take();
        self.commit(change, true)
    }

    fn commit(&self, change: Result<Board, BoardError>, new_step: bool) -> bool {
        match change {
            Ok(b) => {
                if new_step {
                    self.undo.borrow_mut().push(self.current());
                }
                self.redo.borrow_mut().clear();
                self.doc.set(b);
                self.message.set_if_changed(None);
                self.sync();
                true
            }
            Err(e) => {
                self.message.set(Some(e.to_string()));
                false
            }
        }
    }

    /// Applies a typed edit to setting `key` of widget `id`: edits in a row to the same
    /// setting are one undo step.
    pub fn type_setting(&self, id: &str, key: &str, v: Value) -> bool {
        let token = (id.to_owned(), key.to_owned());
        let same = self.typing.borrow().as_ref() == Some(&token);
        *self.typing.borrow_mut() = Some(token);
        self.commit(self.current().set_setting(id, key, v), !same)
    }

    /// Back one change.
    pub fn undo(&self) {
        self.typing.borrow_mut().take();
        let prev = self.undo.borrow_mut().pop();
        if let Some(b) = prev {
            self.redo.borrow_mut().push(self.current());
            self.doc.set(b);
            self.forget_missing();
        }
        self.sync();
    }

    /// Forward one change.
    pub fn redo(&self) {
        self.typing.borrow_mut().take();
        let next = self.redo.borrow_mut().pop();
        if let Some(b) = next {
            self.undo.borrow_mut().push(self.current());
            self.doc.set(b);
            self.forget_missing();
        }
        self.sync();
    }

    /// Drops a selection whose item is gone.
    fn forget_missing(&self) {
        let b = self.current();
        let gone = match self.selected.get_untracked().ok().flatten() {
            Some(Selection::Area(id)) => b.area(&id).is_none(),
            Some(Selection::Widget(id)) => b.widget(&id).is_none(),
            None => false,
        };
        if gone {
            self.selected.set(None);
        }
    }

    /// Moves the selected area by `dc`, `dr` cells, or resizes it with `resize`.
    fn nudge(&self, dc: i32, dr: i32, resize: bool) {
        let Some(Selection::Area(id)) = self.selected.get_untracked().ok().flatten() else { return };
        let b = self.current();
        let Some(a) = b.area(&id) else { return };
        let add = |v: u32, d: i32| u32::try_from(i64::from(v) + i64::from(d)).ok();
        let r = if resize {
            match (add(a.cols, dc).filter(|c| *c >= 1), add(a.rows, dr).filter(|r| *r >= 1)) {
                (Some(c), Some(r)) => b.place_area(&id, a.col, a.row, c, r),
                _ => return,
            }
        } else {
            match (add(a.col, dc), add(a.row, dr)) {
                (Some(c), Some(r)) => b.place_area(&id, c, r, a.cols, a.rows),
                _ => Err(BoardError::OutsideGrid),
            }
        };
        self.apply(r);
    }

    /// Moves the selected widget within its area (`by` places) or to the next or previous
    /// area (`area`).
    fn shift_widget(&self, by: i32, area: i32) {
        let Some(Selection::Widget(id)) = self.selected.get_untracked().ok().flatten() else { return };
        let b = self.current();
        let Some((ai, wi)) = b.widget_place(&id) else { return };
        let target = usize::try_from(i64::try_from(ai).unwrap_or(0) + i64::from(area)).ok();
        let Some(dest) = target.and_then(|t| b.areas.get(t)) else { return };
        let index = if area == 0 {
            match usize::try_from(i64::try_from(wi).unwrap_or(0) + i64::from(by)) {
                Ok(i) => i,
                Err(_) => return,
            }
        } else {
            dest.widgets.len()
        };
        let dest = dest.id.clone();
        self.apply(b.move_widget(&id, &dest, index));
    }

    /// Removes what is selected; its neighbour is selected next (a widget's area, the area
    /// after a removed one, else the one before), so the keyboard keeps its place.
    fn remove_selected(&self) {
        let b = self.current();
        let (r, next) = match self.selected.get_untracked().ok().flatten() {
            Some(Selection::Area(id)) => {
                let at = b.areas.iter().position(|a| a.id == id).unwrap_or(0);
                let next = b
                    .areas
                    .get(at + 1)
                    .or_else(|| at.checked_sub(1).and_then(|p| b.areas.get(p)))
                    .map(|a| Selection::Area(a.id.clone()));
                (b.remove_area(&id), next)
            }
            Some(Selection::Widget(id)) => {
                let area = b
                    .widget_place(&id)
                    .and_then(|(ai, _)| b.areas.get(ai))
                    .map(|a| Selection::Area(a.id.clone()));
                (b.remove_widget(&id), area)
            }
            None => return,
        };
        if self.apply(r) {
            self.selected.set(next);
        }
    }

    /// Keys while editing; true when handled.
    pub fn key(&self, e: &remedy_ui_input::KeyEvent) -> bool {
        if !e.pressed || !self.editing.get_untracked().unwrap_or(false) {
            return false;
        }
        let m = e.modifiers;
        let cmd = m.ctrl || m.meta;
        let area = matches!(self.selected.get_untracked().ok().flatten(), Some(Selection::Area(_)));
        let (dx, dy) = match e.key {
            Key::ArrowLeft => (-1, 0),
            Key::ArrowRight => (1, 0),
            Key::ArrowUp => (0, -1),
            Key::ArrowDown => (0, 1),
            _ => (0, 0),
        };
        match &e.key {
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("z") && m.shift => self.redo(),
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("z") => self.undo(),
            Key::Character(c) if cmd && c.eq_ignore_ascii_case("y") => self.redo(),
            Key::Delete | Key::Backspace => self.remove_selected(),
            Key::Escape => self.selected.set(None),
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown if area => {
                self.nudge(dx, dy, m.shift)
            }
            Key::ArrowUp | Key::ArrowDown if m.alt => self.shift_widget(dy, 0),
            Key::ArrowLeft | Key::ArrowRight if m.alt => self.shift_widget(0, dx),
            _ => return false,
        }
        true
    }
}

fn setting_editor(
    ui: &Ui,
    state: &BoardState,
    id: &str,
    spec: &remedy_ui_script::SettingSpec,
    now: &Value,
) -> El {
    let (st, wid, key) = (state.clone(), id.to_owned(), spec.key.clone());
    match &spec.kind {
        SettingKind::Bool => {
            let on = Signal::new(now.as_bool().unwrap_or(false));
            let first = std::cell::Cell::new(true);
            let _ = Effect::new(move || {
                let v = on.get();
                if !first.replace(false) {
                    untrack(|| st.apply(st.current().set_setting(&wid, &key, Value::Bool(v))));
                }
            });
            w::check(ui, spec.label.clone(), on)
        }
        SettingKind::Choice(options) => {
            let at = options.iter().position(|o| Some(o.as_str()) == now.as_str()).unwrap_or(0);
            let idx = Signal::new(at);
            let (opts, first) = (options.clone(), std::cell::Cell::new(true));
            let _ = Effect::new(move || {
                let i = idx.get();
                if !first.replace(false)
                    && let Some(o) = opts.get(i)
                {
                    untrack(|| st.apply(st.current().set_setting(&wid, &key, Value::String(o.clone()))));
                }
            });
            w::select(ui, spec.label.clone(), options.clone(), idx)
        }
        kind => {
            let long = matches!(kind, SettingKind::LongText);
            let field = if long {
                w::text_area(ui, spec.label.clone())
            } else {
                w::text_input(ui, spec.label.clone())
            };
            field.set_value(&match now {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            });
            let (f2, kind) = (field.clone(), kind.clone());
            let el = field
                .on_change(move || {
                    let text = f2.value();
                    let v = match &kind {
                        SettingKind::Number { min, max } => match text.trim().parse::<f64>() {
                            Ok(n) => {
                                let n = min.map_or(n, |m| n.max(m));
                                Value::from(max.map_or(n, |m| n.min(m)))
                            }
                            // Not a number yet: nothing changes.
                            Err(_) => return,
                        },
                        _ => Value::String(text),
                    };
                    st.type_setting(&wid, &key, v);
                })
                .el()
                .fill_width();
            if long { el.height(96.0) } else { el }
        }
    }
}

fn panel(ui: &Ui, state: &BoardState, registry: &Rc<Registry>) -> El {
    let unit = ui.theme().spacing.unit;
    let body = w::column(ui).gap(unit * 3.0).fill_width();
    let (b2, u2, st, reg) = (body.clone(), ui.clone(), state.clone(), registry.clone());
    let (sel, doc) = (state.selected, state.doc);
    body.bind(move || {
        let selected = sel.get();
        let board = doc.get();
        untrack(|| {
            b2.clear();
            let ui = &u2;
            // Undo, redo, a new area.
            let (s1, s2, s3) = (st.clone(), st.clone(), st.clone());
            let undo = w::button(ui, "Undo", move || s1.undo()).variant("quiet");
            let redo = w::button(ui, "Redo", move || s2.redo()).variant("quiet");
            let (cu, cr) = (st.can_undo, st.can_redo);
            let (u3, r3) = (undo.clone(), redo.clone());
            let undo = undo.bind(move || {
                let _ = u3.clone().disabled(!cu.get());
            });
            let redo = redo.bind(move || {
                let _ = r3.clone().disabled(!cr.get());
            });
            let add_area = w::button(ui, "Add area", move || {
                if let Ok((b, id)) = s3.current().add_area("", 6, 1) {
                    s3.apply(Ok(b));
                    s3.selected.set(Some(Selection::Area(id)));
                }
            });
            let _ = b2
                .clone()
                .child(w::row(ui).gap(unit).child(undo).child(redo).child(w::spacer(ui)).child(add_area));
            // Where new widgets go: the selected area, or the selected widget's.
            let area_id = match &selected {
                Some(Selection::Area(id)) => board.area(id).map(|a| a.id.clone()),
                Some(Selection::Widget(id)) => board.widget_place(id).and_then(|(ai, _)| board.areas.get(ai)).map(|a| a.id.clone()),
                None => None,
            };
            match (&selected, &area_id) {
                (Some(Selection::Area(id)), _) => {
                    let Some(a) = board.area(id) else { return };
                    let title = w::text_input(ui, "Area title");
                    title.set_value(&a.title);
                    let (st4, aid, t2) = (st.clone(), id.clone(), title.clone());
                    let title = title.on_submit(move || {
                        st4.apply(st4.current().retitle(&aid, &t2.value()));
                    });
                    let (st5, aid) = (st.clone(), id.clone());
                    let _ = b2
                        .clone()
                        .child(w::heading(ui, 3, "Area"))
                        .child(w::field(ui, "Title (Enter to apply)", title.el().fill_width()))
                        .child(w::label(
                            ui,
                            format!("Column {}, row {}, {} wide, {} high", a.col + 1, a.row + 1, a.cols, a.rows),
                            "muted",
                        ))
                        .child(w::label(ui, "Arrow keys move it; Shift with arrows resizes it.", "muted"))
                        .child(
                            w::button(ui, "Remove area", move || {
                                if st5.apply(st5.current().remove_area(&aid)) {
                                    st5.selected.set(None);
                                }
                            })
                            .variant("danger"),
                        );
                }
                (Some(Selection::Widget(id)), _) => {
                    let Some(placed) = board.widget(id) else { return };
                    let groups = match reg.get(&placed.kind) {
                        Some(kind) => {
                            let props = kind
                                .settings()
                                .iter()
                                .map(|spec| {
                                    let now = value(&placed.settings, spec).clone();
                                    Property::new(spec.label.clone(), setting_editor(ui, &st, id, spec, &now))
                                })
                                .collect();
                            vec![PropertyGroup::new(kind.name(), props)]
                        }
                        None => Vec::new(),
                    };
                    let (st6, wid) = (st.clone(), id.clone());
                    let _ = b2
                        .clone()
                        .child(w::heading(ui, 3, "Widget"))
                        .child(property_grid(ui, "Settings", groups))
                        .child(w::label(ui, "Alt with Up and Down moves it; Alt with Left and Right moves it to the next area.", "muted"))
                        .child(
                            w::button(ui, "Remove widget", move || {
                                if st6.apply(st6.current().remove_widget(&wid)) {
                                    st6.selected.set(None);
                                }
                            })
                            .variant("danger"),
                        );
                }
                _ => {
                    let _ = b2.clone().child(w::label(ui, "Select an area or a widget to change it.", "muted"));
                }
            }
            // Adding a widget.
            if let Some(aid) = area_id {
                let kinds = reg.all();
                let names: Vec<String> = kinds.iter().map(|k| k.name()).collect();
                let pick = Signal::new(0usize);
                let (st7, reg2) = (st.clone(), reg.clone());
                let add = w::button(ui, "Add widget", move || {
                    let Some(kind) = kinds.get(pick.get_untracked().unwrap_or(0)) else { return };
                    let defaults = reg2.defaults(kind.id());
                    if let Ok((b, id)) = st7.current().add_widget(&aid, kind.id(), defaults) {
                        st7.apply(Ok(b));
                        st7.selected.set(Some(Selection::Widget(id)));
                    }
                })
                .variant("primary");
                let _ = b2.clone().child(w::row(ui).gap(unit).align(Align::Center).child(w::select(ui, "Widget kind", names, pick)).child(add));
            }
        });
    })
}

/// The editable items of the last build, for keyboard focus to follow the selection.
type Made = RefCell<Vec<(Selection, El)>>;

fn item_states(el: El, state: &BoardState, mine: Selection, made: &Made) -> El {
    made.borrow_mut().push((mine.clone(), el.clone()));
    let (e2, sel) = (el.clone(), state.selected);
    el.bind(move || {
        let on = sel.get().as_ref() == Some(&mine);
        e2.state(States::SELECTED, on);
    })
}

fn widget_el(
    ui: &Ui,
    state: &BoardState,
    registry: &Registry,
    cx: &Context,
    placed: &crate::model::Placed,
    editing: bool,
    made: &Made,
) -> El {
    let built = match registry.get(&placed.kind) {
        Some(k) => {
            // Settings the board does not hold take the kind's defaults.
            let mut settings = registry.defaults(&placed.kind);
            settings.extend(placed.settings.clone());
            k.build(ui, &placed.id, &settings, cx)
        }
        None => w::label(
            ui,
            format!("This board uses a widget this app does not have ({}).", placed.kind),
            "muted",
        ),
    };
    if !editing {
        return ui.el("board_widget").column().fill_width().child(built);
    }
    let name = registry.get(&placed.kind).map_or_else(|| placed.kind.clone(), |k| k.name());
    let (st, id, st2, id2, id3) =
        (state.clone(), placed.id.clone(), state.clone(), placed.id.clone(), placed.id.clone());
    let el = ui
        .el("board_item")
        .column()
        .fill_width()
        .layout(|l| l.min_height = Length::Px(32.0))
        .padding(Insets::all(ui.theme().spacing.unit))
        // While arranging, the widget is shown but not used: clicks select it.
        .child(built.pass_through())
        .focusable(true)
        .on_click(move || st.selected.set(Some(Selection::Widget(id.clone()))))
        .on_focus_change(move |on| {
            if on {
                st2.selected.set_if_changed(Some(Selection::Widget(id2.clone())));
            }
        })
        .draggable(
            move || {
                Some(
                    Drag::new(DragData::default())
                        .with_local(WidgetMove(id3.clone()))
                        .allow(&[DropEffect::Move]),
                )
            },
            |_| {},
        )
        .keyboard_alternative("Alt with the arrow keys moves it")
        .semantics(Semantics::new(Role::Button).label(format!("{name} widget")));
    // A widget dropped on another goes before it.
    let (st4, target) = (state.clone(), placed.id.clone());
    let el = el.on_drop(
        |d, _| if d.local::<WidgetMove>().is_some() { DropEffect::Move } else { DropEffect::None },
        move |d, _, _| {
            let Some(WidgetMove(id)) = d.local::<WidgetMove>() else { return };
            let b = st4.current();
            if let Some((ai, wi)) = b.widget_place(&target)
                && let Some(a) = b.areas.get(ai)
            {
                let dest = a.id.clone();
                st4.apply(b.move_widget(id, &dest, wi));
                st4.selected.set(Some(Selection::Widget(id.clone())));
            }
        },
    );
    item_states(el, state, Selection::Widget(placed.id.clone()), made)
}

fn area_el(
    ui: &Ui,
    state: &BoardState,
    registry: &Registry,
    cx: &Context,
    a: &crate::model::Area,
    editing: bool,
    made: &Made,
) -> El {
    let unit = ui.theme().spacing.unit;
    let (col, cols, row, rows) = (a.col, a.cols, a.row, a.rows);
    let mut el =
        ui.el("board_area").column().gap(unit * 3.0).padding(Insets::all(unit * 4.0)).layout(move |l| {
            l.grid_column = Some((i16::try_from(col + 1).unwrap_or(1), u16::try_from(cols).unwrap_or(1)));
            l.grid_row = Some((i16::try_from(row + 1).unwrap_or(1), u16::try_from(rows).unwrap_or(1)));
            l.min_width = Length::Px(0.0);
        });
    if !a.title.is_empty() {
        el = el.child(w::heading(ui, 2, a.title.clone()));
    }
    for p in &a.widgets {
        el = el.child(widget_el(ui, state, registry, cx, p, editing, made));
    }
    let label = if a.title.is_empty() { "Area".to_owned() } else { a.title.clone() };
    if !editing {
        return el.semantics(Semantics::new(Role::Group).label(label));
    }
    if a.widgets.is_empty() {
        el = el.child(w::label(ui, "Empty area: add a widget or drop one here.", "muted"));
    }
    let (st, id, st2, id2, id3) = (state.clone(), a.id.clone(), state.clone(), a.id.clone(), a.id.clone());
    let el = el
        .focusable(true)
        .on_click(move || st.selected.set(Some(Selection::Area(id.clone()))))
        .on_focus_change(move |on| {
            if on {
                st2.selected.set_if_changed(Some(Selection::Area(id2.clone())));
            }
        })
        .draggable(
            move || {
                Some(
                    Drag::new(DragData::default())
                        .with_local(AreaMove(id3.clone()))
                        .allow(&[DropEffect::Move]),
                )
            },
            |_| {},
        )
        .keyboard_alternative("The arrow keys move it; Shift with the arrow keys resizes it")
        .semantics(Semantics::new(Role::Group).label(format!("{label} area")));
    // Widgets dropped on an area go to its end; an area dropped on another trades places.
    let (st3, target) = (state.clone(), a.id.clone());
    let el = el.on_drop(
        |d, _| {
            if d.local::<WidgetMove>().is_some() || d.local::<AreaMove>().is_some() {
                DropEffect::Move
            } else {
                DropEffect::None
            }
        },
        move |d, _, _| {
            let b = st3.current();
            if let Some(WidgetMove(id)) = d.local::<WidgetMove>() {
                st3.apply(b.move_widget(id, &target, usize::MAX));
                return;
            }
            if let Some(AreaMove(id)) = d.local::<AreaMove>()
                && *id != target
            {
                st3.apply(swap(&b, id, &target));
            }
        },
    );
    item_states(el, state, Selection::Area(a.id.clone()), made)
}

/// Two areas trade places: each takes the other's cells (its place and size), which were
/// free of everything else, so a trade always fits.
fn swap(b: &Board, one: &str, other: &str) -> Result<Board, BoardError> {
    let (Some(x), Some(y)) = (b.area(one).cloned(), b.area(other).cloned()) else {
        return Err(BoardError::Missing(one.into()));
    };
    // Lift one below everything first, so the two never overlap in between.
    let below = b.rows();
    let lifted = b.place_area(one, 0, below, 1, 1)?;
    let moved = lifted.place_area(other, x.col, x.row, x.cols, x.rows)?;
    moved.place_area(one, y.col, y.row, y.cols, y.rows)
}

/// A board: its areas and widgets, editable while `state.editing` is on.
pub fn board(ui: &Ui, state: &BoardState, registry: Rc<Registry>, cx: Context) -> El {
    let unit = ui.theme().spacing.unit;
    let grid = ui.el("board").fill_width();
    let (g2, u2, st, reg, cx2) = (grid.clone(), ui.clone(), state.clone(), registry.clone(), cx);
    let (doc, editing) = (state.doc, state.editing);
    let grid = grid.bind(move || {
        let b = doc.get();
        let edit = editing.get();
        untrack(|| {
            // Keyboard focus inside the board comes back to the selected item after the rebuild.
            let focused_inside = u2.tree().focused().is_some_and(|f| {
                let mut cur = Some(u2.element(f));
                while let Some(e) = cur {
                    if e.id() == g2.id() {
                        return true;
                    }
                    cur = e.parent();
                }
                false
            });
            let made: Made = RefCell::new(Vec::new());
            g2.clear();
            // While editing, one more row waits below for areas to move into.
            let rows = b.rows() + u32::from(edit);
            let columns = b.columns;
            let _ = g2.clone().layout(move |l| {
                l.display = Display::Grid;
                l.columns = vec![Track::Fr(1.0); usize::try_from(columns).unwrap_or(12)];
                l.rows = vec![Track::MaxContent; usize::try_from(rows.max(1)).unwrap_or(1)];
                l.gap = (unit * 4.0, unit * 4.0);
            });
            for a in &b.areas {
                let _ = g2.clone().child(area_el(&u2, &st, &reg, &cx2, a, edit, &made));
            }
            let selected = st.selected.get_untracked().ok().flatten();
            if focused_inside
                && let Some(sel) = selected
                && let Some((_, el)) = made.borrow().iter().find(|(s, _)| *s == sel)
            {
                el.focus();
            }
            if b.areas.is_empty() {
                let note = if edit { "This board is empty: add an area." } else { "Nothing here yet." };
                let _ = g2.clone().child(w::empty_state(&u2, "Empty board", note).layout(move |l| {
                    l.grid_column = Some((1, u16::try_from(columns).unwrap_or(12)));
                }));
            }
        });
    });
    let message = w::label(ui, "", "error");
    let (m2, msg) = (message.clone(), state.message);
    let message = message.bind(move || {
        let m = msg.get();
        let _ = m2
            .clone()
            .text(remedy_ui_view::RichText::plain(m.clone().unwrap_or_default()))
            .hidden(m.is_none())
            .semantics(Semantics::new(Role::Alert).label(m.unwrap_or_default()));
    });
    let side = panel(ui, state, &registry).width(320.0).layout(|l| l.shrink = 0.0);
    let (s2, ed) = (side.clone(), state.editing);
    let side = side.bind(move || {
        let _ = s2.clone().hidden(!ed.get());
    });
    let st = state.clone();
    w::row(ui)
        .gap(unit * 4.0)
        .fill_width()
        .align(Align::Start)
        .child(
            w::column(ui)
                .grow(1.0)
                .gap(unit * 2.0)
                .layout(|l| l.min_width = Length::Px(0.0))
                .child(message)
                .child(grid),
        )
        .child(side)
        .on_key(move |e| st.key(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_trade_places_and_cells() {
        let (b, a1) = Board::default().add_area("A", 8, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, a2) = b.add_area("B", 4, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let s = swap(&b, &a1, &a2).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(s.area(&a2).map(|a| (a.col, a.row, a.cols)), Some((0, 0, 8)));
        assert_eq!(s.area(&a1).map(|a| (a.col, a.row, a.cols)), Some((8, 0, 4)));
    }

    #[test]
    fn edits_undo_and_redo_and_typing_is_one_step() {
        let st = BoardState::new(Board::default());
        let (b, area) = st.current().add_area("A", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(st.apply(Ok(b)));
        let (b, wid) = st
            .current()
            .add_widget(&area, "text", serde_json::Map::new())
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert!(st.apply(Ok(b)));
        for t in ["H", "Hi", "Hi!"] {
            st.type_setting(&wid, "title", t.into());
        }
        st.undo();
        assert_eq!(
            st.current().widget(&wid).map(|w| w.settings.is_empty()),
            Some(true),
            "the typing was one step"
        );
        st.undo();
        assert!(st.current().widget(&wid).is_none());
        st.redo();
        st.redo();
        assert_eq!(
            st.current().widget(&wid).and_then(|w| w.settings.get("title")).cloned(),
            Some("Hi!".into())
        );
        // A refused change says why and changes nothing.
        let before = st.current();
        assert!(!st.apply(before.place_area(&area, 10, 0, 6, 1)));
        assert_eq!(st.current(), before);
        assert!(st.message.get_untracked().ok().flatten().is_some());
    }
}
