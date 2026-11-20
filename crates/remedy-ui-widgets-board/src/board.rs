//! The board on screen: its areas on the grid with their widgets, the same for the person
//! arranging it and for visitors. Editing adds selection, dragging, keys for every move, an
//! undo history, and a panel to add areas and widgets and edit their settings. The panel
//! can also stand apart ([`board_panel`]), for apps that show several boards with one panel
//! or none.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::Key;
use remedy_ui_layout::{Display, Length, Track};
use remedy_ui_reactive::{Effect, Signal, untrack};
use remedy_ui_style::States;
use remedy_ui_view::{Align, Drag, DragData, DropEffect, El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_data::{Property, PropertyGroup, property_grid};
use serde_json::Value;

use crate::kinds::{Context, Registry, value};
use crate::labels::{BoardLabels, fill};
use crate::model::{Area, Board, BoardError, Placed};
use crate::settings::setting_editor;

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

/// Where a board's editing panel goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Panel {
    /// Beside the grid while editing (the default).
    #[default]
    Beside,
    /// No panel with the board: the app shows [`board_panel`] where it likes (one for
    /// several boards on a page, say), or drives editing itself through [`BoardState`]'s
    /// operations. Selecting, dragging and the keys still work on the grid.
    Off,
}

/// The app's name for an area, for assistive technology; `None` leaves the board's own.
pub type AreaLabelFn = Rc<dyn Fn(&Area) -> Option<String>>;

/// How a board behaves: what an area may hold, how tall rows are, where its panel goes,
/// when typed changes count as settled, and what areas are called.
#[derive(Clone)]
#[non_exhaustive]
pub struct BoardOptions {
    /// Each area holds at most one widget (default false): the editor offers adding a
    /// widget only to an empty area, and refuses moving one into an area that holds another.
    pub one_widget_per_area: bool,
    /// Every row this tall, in logical pixels, with areas scrolling what does not fit; `None`
    /// (the default) sizes each row to its content.
    pub row_height: Option<f32>,
    /// Where the editing panel goes.
    pub panel: Panel,
    /// How long after the last keystroke typing in a setting counts as finished (default one
    /// second); typing also finishes when its field loses focus, on Enter, and with any other
    /// change. `None`: only those.
    pub settle_after: Option<Duration>,
    /// The app's names for areas. Without one (or where it says `None`), an area is called
    /// by its title, else by its one widget's label, else by its number.
    pub area_label: Option<AreaLabelFn>,
}

impl Default for BoardOptions {
    fn default() -> Self {
        BoardOptions {
            one_widget_per_area: false,
            row_height: None,
            panel: Panel::Beside,
            settle_after: Some(Duration::from_secs(1)),
            area_label: None,
        }
    }
}

impl std::fmt::Debug for BoardOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardOptions")
            .field("one_widget_per_area", &self.one_widget_per_area)
            .field("row_height", &self.row_height)
            .field("panel", &self.panel)
            .field("settle_after", &self.settle_after)
            .field("area_label", &self.area_label.is_some())
            .finish()
    }
}

impl BoardOptions {
    /// Sets [`one_widget_per_area`](BoardOptions::one_widget_per_area).
    pub fn one_widget_per_area(mut self, on: bool) -> Self {
        self.one_widget_per_area = on;
        self
    }

    /// Sets [`row_height`](BoardOptions::row_height).
    pub fn row_height(mut self, height: Option<f32>) -> Self {
        self.row_height = height;
        self
    }

    /// Sets [`panel`](BoardOptions::panel).
    pub fn panel(mut self, panel: Panel) -> Self {
        self.panel = panel;
        self
    }

    /// Sets [`settle_after`](BoardOptions::settle_after).
    pub fn settle_after(mut self, after: Option<Duration>) -> Self {
        self.settle_after = after;
        self
    }

    /// Sets [`area_label`](BoardOptions::area_label).
    pub fn area_label(mut self, f: impl Fn(&Area) -> Option<String> + 'static) -> Self {
        self.area_label = Some(Rc::new(f));
        self
    }
}

/// A board being shown (and maybe edited): its document, the mode, the selection, and the
/// undo history. Cheap to clone.
#[derive(Clone)]
pub struct BoardState {
    /// The document. Set it to show another board (a peer sent a new version).
    pub doc: Signal<Board>,
    /// Editing (true) or viewing.
    pub editing: Signal<bool>,
    /// What is selected while editing. Set it to select from outside (an app's own panel).
    pub selected: Signal<Option<Selection>>,
    /// Why the last change was refused, until the next one.
    pub message: Signal<Option<String>>,
    undo: Rc<RefCell<Vec<Board>>>,
    redo: Rc<RefCell<Vec<Board>>>,
    /// The setting being typed into: edits to it share one undo step.
    typing: Rc<RefCell<Option<(String, String)>>>,
    /// Counts keystrokes, so a settle timer knows whether typing went on after it.
    keystrokes: Rc<Cell<u64>>,
    /// The document as the panel's own editor made it: the panel keeps its fields (and
    /// focus) rather than building again.
    panel_made: Rc<RefCell<Option<Board>>>,
    /// A list setting item to focus when the panel is next built (widget, key, index).
    focus_item: Rc<RefCell<Option<(String, String, usize)>>>,
    /// The element to focus once the panel is built.
    focus_el: Rc<RefCell<Option<El>>>,
    /// The words of the last board built with this state, for messages.
    words: Rc<RefCell<BoardLabels>>,
    /// Undo is possible.
    pub can_undo: Signal<bool>,
    /// Redo is possible.
    pub can_redo: Signal<bool>,
    /// How the board behaves. Change it at any time; the board follows.
    pub options: Signal<BoardOptions>,
    /// The document as of the last finished change: it follows [`doc`](BoardState::doc),
    /// except that typing in a setting reaches it once, when the typing finishes (its field
    /// loses focus, Enter, [`BoardOptions::settle_after`] without a keystroke, another
    /// change, or [`finish_typing`](BoardState::finish_typing)). Apps that send each change
    /// somewhere watch this instead of `doc`, to send one change rather than one a keystroke.
    pub settled: Signal<Board>,
}

impl std::fmt::Debug for BoardState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoardState")
    }
}

impl BoardState {
    /// Shows `doc`, viewing, with the default options.
    pub fn new(doc: Board) -> BoardState {
        BoardState::with_options(doc, BoardOptions::default())
    }

    /// Shows `doc`, viewing, behaving as `options` say.
    pub fn with_options(doc: Board, options: BoardOptions) -> BoardState {
        let st = BoardState {
            doc: Signal::new(doc.clone()),
            editing: Signal::new(false),
            selected: Signal::new(None),
            message: Signal::new(None),
            undo: Rc::default(),
            redo: Rc::default(),
            typing: Rc::default(),
            keystrokes: Rc::default(),
            panel_made: Rc::default(),
            focus_item: Rc::default(),
            focus_el: Rc::default(),
            words: Rc::default(),
            can_undo: Signal::new(false),
            can_redo: Signal::new(false),
            options: Signal::new(options),
            settled: Signal::new(doc),
        };
        // Whatever changes the document (an edit, undo, the app setting it) settles it,
        // except typing, which settles when it finishes.
        let (d, typing, settled) = (st.doc, st.typing.clone(), st.settled);
        let _ = Effect::new(move || {
            let b = d.get();
            untrack(|| {
                if typing.borrow().is_none() {
                    settled.set_if_changed(b);
                }
            });
        });
        st
    }

    /// The document now (untracked).
    pub fn current(&self) -> Board {
        self.doc.get_untracked().unwrap_or_default()
    }

    fn options_now(&self) -> BoardOptions {
        self.options.get_untracked().unwrap_or_default()
    }

    fn sync(&self) {
        self.can_undo.set_if_changed(!self.undo.borrow().is_empty());
        self.can_redo.set_if_changed(!self.redo.borrow().is_empty());
    }

    /// Applies a change: on success it becomes the board (and can be undone); on refusal the
    /// reason is shown. True when it applied.
    pub fn apply(&self, change: Result<Board, BoardError>) -> bool {
        self.finish_typing();
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
                self.message.set(Some(self.say(&e)));
                false
            }
        }
    }

    /// Why a change was refused, in the board's words, naming areas by their titles.
    fn say(&self, e: &BoardError) -> String {
        let b = self.current();
        let area = |id: &str| match b.area(id) {
            Some(a) if !a.title.trim().is_empty() => a.title.clone(),
            _ => id.to_owned(),
        };
        self.words.borrow().error(e, area)
    }

    /// Applies a typed edit to setting `key` of widget `id`: edits in a row to the same
    /// setting are one undo step, and reach [`settled`](BoardState::settled) once, when the
    /// typing finishes.
    pub fn type_setting(&self, id: &str, key: &str, v: Value) -> bool {
        let token = (id.to_owned(), key.to_owned());
        let same = self.typing.borrow().as_ref() == Some(&token);
        if !same {
            self.finish_typing();
        }
        *self.typing.borrow_mut() = Some(token);
        self.keystrokes.set(self.keystrokes.get().wrapping_add(1));
        self.commit(self.current().set_setting(id, key, v), !same)
    }

    /// Ends the typing in a setting, if any: the next edit is a new undo step, and the
    /// document reaches [`settled`](BoardState::settled).
    pub fn finish_typing(&self) {
        if self.typing.borrow_mut().take().is_some() {
            self.settled.set_if_changed(self.current());
        }
    }

    /// A setting is being typed into (its change has not settled yet).
    pub fn is_typing(&self) -> bool {
        self.typing.borrow().is_some()
    }

    /// A keystroke from the panel: applied as typing, the panel keeps its fields, and a
    /// timer settles it after a pause.
    pub(crate) fn typed(&self, ui: &Ui, id: &str, key: &str, v: Value) {
        if let Ok(b) = self.current().set_setting(id, key, v.clone()) {
            *self.panel_made.borrow_mut() = Some(b);
        }
        self.type_setting(id, key, v);
        if let Some(after) = self.options_now().settle_after {
            let (st, n) = (self.clone(), self.keystrokes.get());
            let _ = ui.after(after, move || {
                if st.keystrokes.get() == n {
                    st.finish_typing();
                }
            });
        }
    }

    /// A change from the panel's own editor (a switch, a choice): one undo step, and the
    /// panel keeps its fields.
    pub(crate) fn panel_set(&self, id: &str, key: &str, v: Value) {
        let change = self.current().set_setting(id, key, v);
        if let Ok(b) = &change {
            *self.panel_made.borrow_mut() = Some(b.clone());
        }
        self.apply(change);
    }

    pub(crate) fn take_focus_item(&self, id: &str, key: &str, index: usize) -> bool {
        let mine =
            self.focus_item.borrow().as_ref().is_some_and(|(w, k, i)| w == id && k == key && *i == index);
        if mine {
            self.focus_item.borrow_mut().take();
        }
        mine
    }

    pub(crate) fn focus_item_next(&self, id: &str, key: &str, index: usize) {
        *self.focus_item.borrow_mut() = Some((id.to_owned(), key.to_owned(), index));
    }

    pub(crate) fn focus_after_build(&self, el: El) {
        *self.focus_el.borrow_mut() = Some(el);
    }

    /// Back one change.
    pub fn undo(&self) {
        self.finish_typing();
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
        self.finish_typing();
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

    /// Area `area` may take widget `widget` (`None`: a new one): always, unless areas hold
    /// one widget each and it holds another. A refusal is said.
    fn room_in(&self, b: &Board, area: &str, widget: Option<&str>) -> bool {
        if !self.options_now().one_widget_per_area {
            return true;
        }
        let free = b.area(area).is_none_or(|a| a.widgets.iter().all(|w| Some(w.id.as_str()) == widget));
        if !free {
            self.message.set(Some(self.words.borrow().one_widget.clone()));
        }
        free
    }

    /// Adds an empty area at the first free place, selects it, and returns its id.
    pub fn add_area(&self) -> Option<String> {
        let (b, id) = match self.current().add_area("", 6, 1) {
            Ok(r) => r,
            Err(e) => {
                self.message.set(Some(self.say(&e)));
                return None;
            }
        };
        self.apply(Ok(b));
        self.selected.set(Some(Selection::Area(id.clone())));
        Some(id)
    }

    /// Adds a widget of `kind` (with its defaults from `registry`) at the end of area `area`,
    /// selects it, and returns its id; refused (and said) where areas hold one widget and
    /// this one holds another.
    pub fn add_widget(&self, area: &str, kind: &str, registry: &Registry) -> Option<String> {
        let b = self.current();
        if !self.room_in(&b, area, None) {
            return None;
        }
        match b.add_widget(area, kind, registry.defaults(kind)) {
            Ok((b, id)) => {
                self.apply(Ok(b));
                self.selected.set(Some(Selection::Widget(id.clone())));
                Some(id)
            }
            Err(e) => {
                self.message.set(Some(self.say(&e)));
                None
            }
        }
    }

    /// Moves widget `id` into area `to` at `index` (clamped to its end); refused (and said)
    /// where areas hold one widget and `to` holds another. True when it moved.
    pub fn move_widget(&self, id: &str, to: &str, index: usize) -> bool {
        let b = self.current();
        self.room_in(&b, to, Some(id)) && self.apply(b.move_widget(id, to, index))
    }

    /// Moves the selected area by `columns` and `rows` cells.
    pub fn move_selected(&self, columns: i32, rows: i32) {
        self.nudge(columns, rows, false);
    }

    /// Resizes the selected area by `columns` and `rows` cells.
    pub fn resize_selected(&self, columns: i32, rows: i32) {
        self.nudge(columns, rows, true);
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

    /// Moves the selected widget `places` within its area, or to the area `areas` after
    /// (before, when negative) its own, at that area's end.
    pub fn move_selected_widget(&self, places: i32, areas: i32) {
        let Some(Selection::Widget(id)) = self.selected.get_untracked().ok().flatten() else { return };
        let b = self.current();
        let Some((ai, wi)) = b.widget_place(&id) else { return };
        let target = usize::try_from(i64::try_from(ai).unwrap_or(0) + i64::from(areas)).ok();
        let Some(dest) = target.and_then(|t| b.areas.get(t)) else { return };
        let index = if areas == 0 {
            match usize::try_from(i64::try_from(wi).unwrap_or(0) + i64::from(places)) {
                Ok(i) => i,
                Err(_) => return,
            }
        } else {
            dest.widgets.len()
        };
        let dest = dest.id.clone();
        self.move_widget(&id, &dest, index);
    }

    /// Removes what is selected; its neighbour is selected next (a widget's area, the area
    /// after a removed one, else the one before), so the keyboard keeps its place.
    pub fn remove_selected(&self) {
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
            Key::ArrowUp | Key::ArrowDown if m.alt => self.move_selected_widget(dy, 0),
            Key::ArrowLeft | Key::ArrowRight if m.alt => self.move_selected_widget(0, dx),
            _ => return false,
        }
        true
    }
}

/// What a placed widget is called: its kind's label for it, else the kind's name.
fn widget_label(registry: &Registry, placed: &Placed) -> String {
    match registry.get(&placed.kind) {
        Some(k) => {
            let mut settings = registry.defaults(&placed.kind);
            settings.extend(placed.settings.clone());
            k.label(&settings).unwrap_or_else(|| k.name())
        }
        None => placed.kind.clone(),
    }
}

/// What area `a` (the `index`th) is called for assistive technology: the app's name for
/// it, its title, its one widget's name, or its number. Never "Area area".
fn area_label(
    a: &Area,
    index: usize,
    registry: &Registry,
    words: &BoardLabels,
    options: &BoardOptions,
) -> String {
    if let Some(name) = options.area_label.as_ref().and_then(|f| f(a)) {
        return name;
    }
    if !a.title.trim().is_empty() {
        return fill(&words.area_named, &[("name", a.title.clone())]);
    }
    match a.widgets.as_slice() {
        [one] => widget_label(registry, one),
        _ => fill(&words.area_numbered, &[("number", (index + 1).to_string())]),
    }
}

/// The editing panel of the board `state`: undo and redo, adding areas and widgets, and
/// the selected area's place and title or the selected widget's settings. [`board`] shows
/// it beside the grid ([`Panel::Beside`]); with [`Panel::Off`] an app shows it where it
/// likes, such as one panel for the board being arranged among several on a page. Its
/// words are `cx.labels`, and picture settings use `cx.picture` and `cx.choose_picture`.
pub fn board_panel(ui: &Ui, state: &BoardState, registry: Rc<Registry>, cx: &Context) -> El {
    let mut cx = cx.clone();
    cx.labels = BoardLabels::or_locale(&cx.labels, &ui.locale());
    let cx = &cx;
    *state.words.borrow_mut() = cx.labels.clone();
    let unit = ui.theme().spacing.unit;
    let body = w::column(ui).gap(unit * 3.0).fill_width();
    let (b2, u2, st, reg, cx) = (body.clone(), ui.clone(), state.clone(), registry, cx.clone());
    let (sel, doc, opts) = (state.selected, state.doc, state.options);
    // What the panel was last built for: it keeps its fields while its own editors type.
    let shown: Rc<RefCell<Option<Option<Selection>>>> = Rc::default();
    body.bind(move || {
        let selected = sel.get();
        let board = doc.get();
        let options = opts.get();
        untrack(|| {
            let made = st.panel_made.borrow_mut().take();
            if made.as_ref() == Some(&board) && shown.borrow().as_ref() == Some(&selected) {
                return;
            }
            *shown.borrow_mut() = Some(selected.clone());
            b2.clear();
            build_panel(&u2, &b2, &st, &reg, &cx, &board, selected.as_ref(), &options);
            if let Some(el) = st.focus_el.borrow_mut().take() {
                el.focus();
            }
        });
    })
}

#[allow(clippy::too_many_arguments)]
fn build_panel(
    ui: &Ui,
    body: &El,
    st: &BoardState,
    reg: &Rc<Registry>,
    cx: &Context,
    board: &Board,
    selected: Option<&Selection>,
    options: &BoardOptions,
) {
    let unit = ui.theme().spacing.unit;
    let words = &cx.labels;
    // Undo, redo, a new area.
    let (s1, s2, s3) = (st.clone(), st.clone(), st.clone());
    let undo = w::button(ui, words.undo.clone(), move || s1.undo()).variant("quiet");
    let redo = w::button(ui, words.redo.clone(), move || s2.redo()).variant("quiet");
    let (cu, cr) = (st.can_undo, st.can_redo);
    let (u3, r3) = (undo.clone(), redo.clone());
    let undo = undo.bind(move || {
        let _ = u3.clone().disabled(!cu.get());
    });
    let redo = redo.bind(move || {
        let _ = r3.clone().disabled(!cr.get());
    });
    let add_area = w::button(ui, words.add_area.clone(), move || {
        let _ = s3.add_area();
    });
    let _ =
        body.clone().child(w::row(ui).gap(unit).child(undo).child(redo).child(w::spacer(ui)).child(add_area));
    // Where new widgets go: the selected area, or the selected widget's.
    let area_id = match selected {
        Some(Selection::Area(id)) => board.area(id).map(|a| a.id.clone()),
        Some(Selection::Widget(id)) => {
            board.widget_place(id).and_then(|(ai, _)| board.areas.get(ai)).map(|a| a.id.clone())
        }
        None => None,
    };
    match selected {
        Some(Selection::Area(id)) => {
            let Some(a) = board.area(id) else { return };
            let title = w::text_input(ui, words.area_title.clone());
            title.set_value(&a.title);
            let (st4, aid, t2) = (st.clone(), id.clone(), title.clone());
            let title = title.on_submit(move || {
                st4.apply(st4.current().retitle(&aid, &t2.value()));
            });
            let (st5, aid) = (st.clone(), id.clone());
            let place = fill(
                &words.area_place,
                &[
                    ("column", (a.col + 1).to_string()),
                    ("row", (a.row + 1).to_string()),
                    ("columns", a.cols.to_string()),
                    ("rows", a.rows.to_string()),
                ],
            );
            let _ = body
                .clone()
                .child(w::heading(ui, 3, words.area_heading.clone()))
                .child(w::field(ui, words.area_title_hint.clone(), title.el().fill_width()))
                .child(w::text(ui, place).variant("muted"))
                .child(w::text(ui, words.area_keys_hint.clone()).variant("muted"))
                .child(
                    w::button(ui, words.remove_area.clone(), move || {
                        if st5.apply(st5.current().remove_area(&aid)) {
                            st5.selected.set(None);
                        }
                    })
                    .variant("danger"),
                );
        }
        Some(Selection::Widget(id)) => {
            let Some(placed) = board.widget(id) else { return };
            let groups = match reg.get(&placed.kind) {
                Some(kind) => {
                    let props = kind
                        .settings()
                        .iter()
                        .map(|spec| {
                            let now = value(&placed.settings, spec).clone();
                            Property::new(spec.label.clone(), setting_editor(ui, st, cx, id, spec, &now))
                        })
                        .collect();
                    vec![PropertyGroup::new(kind.name(), props)]
                }
                None => Vec::new(),
            };
            let (st6, wid) = (st.clone(), id.clone());
            let _ = body
                .clone()
                .child(w::heading(ui, 3, words.widget_heading.clone()))
                .child(property_grid(ui, words.settings.clone(), groups))
                .child(w::text(ui, words.widget_keys_hint.clone()).variant("muted"))
                .child(
                    w::button(ui, words.remove_widget.clone(), move || {
                        if st6.apply(st6.current().remove_widget(&wid)) {
                            st6.selected.set(None);
                        }
                    })
                    .variant("danger"),
                );
        }
        None => {
            let _ = body.clone().child(w::text(ui, words.nothing_selected.clone()).variant("muted"));
        }
    }
    // Adding a widget: the kinds this viewer may add, where the area has room.
    let room =
        |aid: &str| !options.one_widget_per_area || board.area(aid).is_none_or(|a| a.widgets.is_empty());
    let kinds = reg.addable();
    if let Some(aid) = area_id.filter(|aid| room(aid))
        && !kinds.is_empty()
    {
        let names: Vec<String> = kinds.iter().map(|k| k.name()).collect();
        let pick = Signal::new(0usize);
        let (st7, reg2) = (st.clone(), reg.clone());
        let add = w::button(ui, words.add_widget.clone(), move || {
            let Some(kind) = kinds.get(pick.get_untracked().unwrap_or(0)) else { return };
            let _ = st7.add_widget(&aid, kind.id(), &reg2);
        })
        .variant("primary");
        let _ = body.clone().child(
            w::row(ui)
                .gap(unit)
                .align(Align::Center)
                .child(w::select(ui, words.widget_kind.clone(), names, pick))
                .child(add),
        );
    }
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

/// What one build of the grid shares.
struct Build<'a> {
    ui: &'a Ui,
    state: &'a BoardState,
    registry: &'a Registry,
    cx: &'a Context,
    options: &'a BoardOptions,
    editing: bool,
    made: &'a Made,
}

fn widget_el(b: &Build<'_>, placed: &Placed) -> El {
    let (ui, state, registry, words) = (b.ui, b.state, b.registry, &b.cx.labels);
    let built = match registry.get(&placed.kind) {
        Some(k) => {
            // Settings the board does not hold take the kind's defaults.
            let mut settings = registry.defaults(&placed.kind);
            settings.extend(placed.settings.clone());
            k.build(ui, &placed.id, &settings, b.cx)
        }
        None => w::text(ui, fill(&words.unknown_kind, &[("kind", placed.kind.clone())])).variant("muted"),
    };
    if !b.editing {
        return ui.el("board_widget").column().fill_width().child(built);
    }
    let name = widget_label(registry, placed);
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
        .keyboard_alternative(words.widget_keys.clone())
        .semantics(
            Semantics::new(Role::Button)
                .label(fill(&words.widget_name, &[("name", name)]))
                .action(remedy_ui_a11y::Action::Click),
        );
    // A widget dropped on another goes before it (where its area has room).
    let (st4, target, st5) = (state.clone(), placed.id.clone(), state.clone());
    let target2 = target.clone();
    let el = el.on_drop(
        move |d, _| match d.local::<WidgetMove>() {
            Some(WidgetMove(id)) if st5.fits_beside(id, &target2) => DropEffect::Move,
            _ => DropEffect::None,
        },
        move |d, _, _| {
            let Some(WidgetMove(id)) = d.local::<WidgetMove>() else { return };
            let b = st4.current();
            if let Some((ai, wi)) = b.widget_place(&target)
                && let Some(a) = b.areas.get(ai)
            {
                let dest = a.id.clone();
                if st4.move_widget(id, &dest, wi) {
                    st4.selected.set(Some(Selection::Widget(id.clone())));
                }
            }
        },
    );
    item_states(el, state, Selection::Widget(placed.id.clone()), b.made)
}

impl BoardState {
    /// Widget `id` may be dropped beside widget `target` (into its area): always, unless
    /// areas hold one widget and that one is another's.
    fn fits_beside(&self, id: &str, target: &str) -> bool {
        id != target
            && (!self.options_now().one_widget_per_area || {
                let b = self.current();
                b.widget_place(id).map(|(a, _)| a) == b.widget_place(target).map(|(a, _)| a)
            })
    }

    /// Widget `id` may be dropped on area `area`: always, unless areas hold one widget and
    /// it holds another.
    fn fits_in(&self, id: &str, area: &str) -> bool {
        !self.options_now().one_widget_per_area
            || self.current().area(area).is_none_or(|a| a.widgets.iter().all(|w| w.id == id))
    }
}

fn area_el(b: &Build<'_>, a: &Area, index: usize) -> El {
    let (ui, state, words) = (b.ui, b.state, &b.cx.labels);
    let unit = ui.theme().spacing.unit;
    let (col, cols, row, rows) = (a.col, a.cols, a.row, a.rows);
    let mut el =
        ui.el("board_area").column().gap(unit * 3.0).padding(Insets::all(unit * 4.0)).layout(move |l| {
            l.grid_column = Some((i16::try_from(col + 1).unwrap_or(1), u16::try_from(cols).unwrap_or(1)));
            l.grid_row = Some((i16::try_from(row + 1).unwrap_or(1), u16::try_from(rows).unwrap_or(1)));
            l.min_width = Length::Px(0.0);
        });
    if b.options.row_height.is_some() {
        // Rows of a fixed height: what does not fit scrolls inside the area.
        el = el.layout(|l| l.min_height = Length::Px(0.0)).scroll();
    }
    if !a.title.is_empty() {
        el = el.child(w::heading(ui, 2, a.title.clone()));
    }
    for p in &a.widgets {
        el = el.child(widget_el(b, p));
    }
    let label = area_label(a, index, b.registry, words, b.options);
    if !b.editing {
        return el.semantics(Semantics::new(Role::Group).label(label));
    }
    if a.widgets.is_empty() {
        el = el.child(w::text(ui, words.empty_area.clone()).variant("muted"));
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
        .keyboard_alternative(words.area_keys.clone())
        .semantics(Semantics::new(Role::Group).label(label).action(remedy_ui_a11y::Action::Click));
    // Widgets dropped on an area go to its end (where it has room); an area dropped on
    // another trades places.
    let (st3, target, st4, target2) = (state.clone(), a.id.clone(), state.clone(), a.id.clone());
    let el = el.on_drop(
        move |d, _| match (d.local::<WidgetMove>(), d.local::<AreaMove>()) {
            (Some(WidgetMove(id)), _) if st4.fits_in(id, &target2) => DropEffect::Move,
            (_, Some(_)) => DropEffect::Move,
            _ => DropEffect::None,
        },
        move |d, _, _| {
            let b = st3.current();
            if let Some(WidgetMove(id)) = d.local::<WidgetMove>() {
                st3.move_widget(id, &target, usize::MAX);
                return;
            }
            if let Some(AreaMove(id)) = d.local::<AreaMove>()
                && *id != target
            {
                st3.apply(swap(&b, id, &target));
            }
        },
    );
    item_states(el, state, Selection::Area(a.id.clone()), b.made)
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

/// A board: its areas and widgets, editable while `state.editing` is on, with its panel
/// beside it unless [`BoardOptions::panel`] says otherwise. Its words are `cx.labels`.
pub fn board(ui: &Ui, state: &BoardState, registry: Rc<Registry>, mut cx: Context) -> El {
    cx.labels = BoardLabels::or_locale(&cx.labels, &ui.locale());
    *state.words.borrow_mut() = cx.labels.clone();
    let unit = ui.theme().spacing.unit;
    let grid = ui.el("board").fill_width();
    let (g2, u2, st, reg, cx2) = (grid.clone(), ui.clone(), state.clone(), registry.clone(), cx.clone());
    let (doc, editing, opts) = (state.doc, state.editing, state.options);
    let grid = grid.bind(move || {
        let b = doc.get();
        let edit = editing.get();
        let options = opts.get();
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
            let track = options.row_height.map_or(Track::MaxContent, Track::Px);
            let _ = g2.clone().layout(move |l| {
                l.display = Display::Grid;
                l.columns = vec![Track::Fr(1.0); usize::try_from(columns).unwrap_or(12)];
                l.rows = vec![track.clone(); usize::try_from(rows.max(1)).unwrap_or(1)];
                l.gap = (unit * 4.0, unit * 4.0);
            });
            let build = Build {
                ui: &u2,
                state: &st,
                registry: &reg,
                cx: &cx2,
                options: &options,
                editing: edit,
                made: &made,
            };
            for (i, a) in b.areas.iter().enumerate() {
                let _ = g2.clone().child(area_el(&build, a, i));
            }
            let selected = st.selected.get_untracked().ok().flatten();
            if focused_inside
                && let Some(sel) = selected
                && let Some((_, el)) = made.borrow().iter().find(|(s, _)| *s == sel)
            {
                el.focus();
            }
            if b.areas.is_empty() {
                let words = &cx2.labels;
                let note = if edit { &words.empty_board_editing } else { &words.empty_board_viewing };
                let _ = g2.clone().child(
                    w::empty_state(&u2, words.empty_board.clone(), note.clone()).layout(move |l| {
                        l.grid_column = Some((1, u16::try_from(columns).unwrap_or(12)));
                    }),
                );
            }
        });
    });
    let message = w::text(ui, "").variant("error");
    let (m2, msg) = (message.clone(), state.message);
    let message = message.bind(move || {
        let m = msg.get();
        let _ = m2
            .clone()
            .text(remedy_ui_view::RichText::plain(m.clone().unwrap_or_default()))
            .hidden(m.is_none())
            .semantics(Semantics::new(Role::Alert).label(m.unwrap_or_default()));
    });
    // The panel, built the first time it shows.
    let side = w::column(ui).width(320.0).layout(|l| l.shrink = 0.0);
    let (s2, ed, u3, st3, reg3) = (side.clone(), state.editing, ui.clone(), state.clone(), registry);
    let side = side.bind(move || {
        let show = ed.get() && opts.with(|o| o.panel == Panel::Beside);
        untrack(|| {
            if show && s2.child_elements().is_empty() {
                let _ = s2.clone().child(board_panel(&u3, &st3, reg3.clone(), &cx));
            }
            let _ = s2.clone().hidden(!show);
        });
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
        // A refused change says why (naming the area by its title) and changes nothing.
        let before = st.current();
        let (crowded, _) = before.add_area("Next door", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(st.apply(Ok(crowded)));
        let before = st.current();
        assert!(!st.apply(before.place_area(&area, 3, 0, 6, 1)));
        assert_eq!(st.current(), before);
        assert_eq!(
            st.message.get_untracked().ok().flatten().as_deref(),
            Some("that would cover the area Next door")
        );
    }

    #[test]
    fn typing_settles_once_when_it_finishes() {
        let st = BoardState::new(Board::default());
        let (b, area) = st.current().add_area("A", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, wid) =
            b.add_widget(&area, "text", serde_json::Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(st.apply(Ok(b)));
        let changes = Rc::new(Cell::new(0));
        let (c2, settled) = (changes.clone(), st.settled);
        let _ = Effect::new(move || {
            let _ = settled.get();
            c2.set(c2.get() + 1);
        });
        let start = changes.get();
        for t in ["H", "Hi", "Hi!"] {
            st.type_setting(&wid, "title", t.into());
        }
        assert!(st.is_typing());
        assert_eq!(changes.get(), start, "nothing settles while typing");
        assert_eq!(
            st.settled.get_untracked().ok().and_then(|b| b.widget(&wid).map(|w| w.settings.len())),
            Some(0)
        );
        st.finish_typing();
        assert_eq!(changes.get(), start + 1, "one change for the whole typing");
        let title = st
            .settled
            .get_untracked()
            .ok()
            .and_then(|b| b.widget(&wid).and_then(|w| w.settings.get("title").cloned()));
        assert_eq!(title, Some("Hi!".into()));
        // Typing into another setting finishes the first; any other change settles at once.
        st.type_setting(&wid, "title", "Yo".into());
        st.type_setting(&wid, "body", "x".into());
        assert_eq!(changes.get(), start + 2);
        assert!(st.apply(st.current().retitle(&area, "B")));
        assert_eq!(changes.get(), start + 4, "the typing settles, then the new title");
        assert!(!st.is_typing());
        // The app setting the document settles it too.
        st.doc.set(Board::default());
        assert_eq!(st.settled.get_untracked().ok(), Some(Board::default()));
    }

    #[test]
    fn one_widget_per_area_refuses_a_second() {
        let st =
            BoardState::with_options(Board::default(), BoardOptions::default().one_widget_per_area(true));
        let reg = Registry::with_builtins();
        let one = st.add_area().unwrap_or_else(|| unreachable!());
        let two = st.add_area().unwrap_or_else(|| unreachable!());
        let w1 = st.add_widget(&one, "text", &reg).unwrap_or_else(|| unreachable!());
        assert!(st.add_widget(&one, "links", &reg).is_none(), "the area is full");
        assert!(st.message.get_untracked().ok().flatten().is_some_and(|m| m.contains("already holds")));
        let w2 = st.add_widget(&two, "links", &reg).unwrap_or_else(|| unreachable!());
        assert!(!st.move_widget(&w1, &two, 0), "not into a full area");
        assert!(st.move_widget(&w1, &one, 0), "within its own area is fine");
        st.selected.set(Some(Selection::Widget(w2.clone())));
        st.move_selected_widget(0, -1);
        assert_eq!(st.current().widget_place(&w2).map(|(a, _)| a), Some(1), "Alt+Left refused too");
        assert!(!st.fits_in(&w2, &one) && st.fits_in(&w2, &two) && !st.fits_beside(&w2, &w1));
    }

    #[test]
    fn areas_are_named_without_saying_area_twice() {
        let reg = Registry::with_builtins();
        let words = BoardLabels::default();
        let opts = BoardOptions::default();
        let (b, titled) = Board::default().add_area("News", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, bare) = b.add_area("", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, lone) = b.add_area("", 6, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let mut s = serde_json::Map::new();
        s.insert("title".into(), "Useful links".into());
        let (b, _) = b.add_widget(&lone, "links", s).unwrap_or_else(|e| unreachable!("{e}"));
        let name = |id: &str, o: &BoardOptions| {
            let i = b.areas.iter().position(|a| a.id == id).unwrap_or(0);
            b.area(id).map(|a| area_label(a, i, &reg, &words, o)).unwrap_or_default()
        };
        assert_eq!(name(&titled, &opts), "News area");
        assert_eq!(name(&bare, &opts), "Area 2");
        assert_eq!(name(&lone, &opts), "Useful links", "named by its one widget");
        let app = BoardOptions::default().area_label(|a| (a.id == "area2").then(|| "Lobby".to_owned()));
        assert_eq!(name(&bare, &app), "Lobby");
        assert_eq!(name(&titled, &app), "News area");
    }
}
