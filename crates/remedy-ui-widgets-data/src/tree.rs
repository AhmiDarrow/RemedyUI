//! Tree views: lazily loaded children, expand and collapse, type-ahead, selection, and
//! moving items by dragging or by cut and paste, with virtual rows for any size.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Insets, Size, Vec2};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, Platform, PointerPhase};
use remedy_ui_core::layout::Length;
use remedy_ui_core::reactive::{Memo, Signal, untrack};
use remedy_ui_core::style::States;
use remedy_ui_engine::view::{Align, Drag, DragData, DropEffect, El, Ui};
use remedy_ui_widgets_core::{self as w, Icon, Rejection};

use crate::model::Selection;
use crate::rows::{RowBuilder, Rows};
use crate::table::row_height;

/// One item of a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeItem<K> {
    /// Identifies the item across changes.
    pub key: K,
    /// What the row says.
    pub label: String,
    /// The icon before the label.
    pub icon: Option<Icon>,
    /// It can be expanded: it has children, or may have (they load when it opens).
    pub has_children: bool,
}

impl<K> TreeItem<K> {
    /// A leaf.
    pub fn leaf(key: K, label: impl Into<String>) -> Self {
        TreeItem { key, label: label.into(), icon: None, has_children: false }
    }

    /// An item with children.
    pub fn branch(key: K, label: impl Into<String>) -> Self {
        TreeItem { key, label: label.into(), icon: None, has_children: true }
    }

    /// With an icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A tree's items, which are open, and the selection. Children are loaded on demand: when an
/// item opens and its children are not known, the tree asks the app (`load`), which answers
/// with [`TreeState::set_children`] now or later.
#[derive(Clone)]
pub struct TreeState<K: Clone + Eq + Hash + 'static> {
    /// The top-level items.
    pub roots: Signal<Vec<TreeItem<K>>>,
    children: Signal<HashMap<K, Vec<TreeItem<K>>>>,
    /// The open items.
    pub expanded: Signal<HashSet<K>>,
    /// The selection and keyboard cursor.
    pub selection: Selection<K>,
    /// Items marked to move by cut (Ctrl+X), waiting for a paste.
    pub cut: Signal<Vec<K>>,
    /// What dragged items are to other apps (files, text, links), when set.
    export: Rc<RefCell<Option<ExportFn<K>>>>,
    /// Takes what other apps drop on an item that has children, when set.
    import: Rc<RefCell<Option<ImportFn<K>>>>,
}

type ExportFn<K> = Rc<dyn Fn(&[K]) -> DragData>;
type ImportFn<K> = Rc<dyn Fn(&DragData, &K) -> Result<(), Rejection>>;

/// Items of this tree being dragged.
struct Moving<K>(Vec<K>);

impl<K: Clone + Eq + Hash + 'static> std::fmt::Debug for TreeState<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TreeState")
    }
}

impl<K: Clone + Eq + Hash + 'static> TreeState<K> {
    /// A tree of `roots`; `multiple` allows selecting many items.
    pub fn new(roots: Vec<TreeItem<K>>, multiple: bool) -> Self {
        TreeState {
            roots: Signal::new(roots),
            children: Signal::new(HashMap::new()),
            expanded: Signal::new(HashSet::new()),
            selection: Selection::new(multiple),
            cut: Signal::new(Vec::new()),
            export: Rc::default(),
            import: Rc::default(),
        }
    }

    /// Lets items be dragged out to other apps: `f` says what they are there (a file
    /// explorer gives their paths).
    pub fn set_export(&self, f: impl Fn(&[K]) -> DragData + 'static) {
        *self.export.borrow_mut() = Some(Rc::new(f));
    }

    /// Takes files, text or links dropped from other apps on an item with children: `f`
    /// stores them in it, or says why not (the tree shows why).
    pub fn set_import(&self, f: impl Fn(&DragData, &K) -> Result<(), Rejection> + 'static) {
        *self.import.borrow_mut() = Some(Rc::new(f));
    }

    /// The children of `key` are `items`.
    pub fn set_children(&self, key: K, items: Vec<TreeItem<K>>) {
        self.children.update(|m| {
            m.insert(key, items);
        });
    }

    /// Forgets the children of `key`, so they load again the next time it is shown open.
    pub fn reload(&self, key: &K) {
        self.children.update(|m| {
            m.remove(key);
        });
    }

    /// The children of `key`, when loaded (tracked).
    pub fn children_of(&self, key: &K) -> Option<Vec<TreeItem<K>>> {
        self.children.with(|m| m.get(key).cloned())
    }

    /// Opens `key`.
    pub fn expand(&self, key: K) {
        self.expanded.update(|s| {
            s.insert(key);
        });
    }

    /// Closes `key`.
    pub fn collapse(&self, key: &K) {
        self.expanded.update(|s| {
            s.remove(key);
        });
    }
}

/// What a row of the open tree is.
#[derive(Clone, Debug, PartialEq)]
struct Line<K> {
    item: TreeItem<K>,
    depth: usize,
    parent: Option<K>,
    /// Its place among its siblings (1-based) and how many there are.
    pos: (usize, usize),
    open: bool,
    /// Open, but its children have not arrived yet.
    loading: bool,
}

/// An item waiting to be laid out: the item, its depth, parent and sibling position.
type Pending<K> = (TreeItem<K>, usize, Option<K>, (usize, usize));

fn flatten<K: Clone + Eq + Hash>(
    roots: &[TreeItem<K>],
    children: &HashMap<K, Vec<TreeItem<K>>>,
    expanded: &HashSet<K>,
) -> Vec<Line<K>> {
    let mut out = Vec::new();
    // An explicit stack, so deep trees cannot overflow the call stack.
    let mut stack: Vec<Pending<K>> = Vec::new();
    let n = roots.len();
    for (i, r) in roots.iter().enumerate().rev() {
        stack.push((r.clone(), 0, None, (i + 1, n)));
    }
    while let Some((item, depth, parent, pos)) = stack.pop() {
        let open = item.has_children && expanded.contains(&item.key);
        let kids = if open { children.get(&item.key) } else { None };
        let loading = open && kids.is_none();
        if let Some(kids) = kids {
            let n = kids.len();
            for (i, c) in kids.iter().enumerate().rev() {
                stack.push((c.clone(), depth + 1, Some(item.key.clone()), (i + 1, n)));
            }
        }
        out.push(Line { item, depth, parent, pos, open, loading });
    }
    out
}

/// Why a move did not happen: this pack's message, or what the app said.
#[derive(Clone, Debug, PartialEq)]
enum MoveProblem {
    Message(&'static str),
    App(String),
}

impl MoveProblem {
    /// What the person is told, in the window's language for this pack's own messages.
    fn message(&self, ui: &Ui) -> String {
        match self {
            MoveProblem::Message(id) => crate::tr(ui, id),
            MoveProblem::App(m) => m.clone(),
        }
    }
}

fn toggle_key(m: Modifiers, platform: Platform) -> bool {
    if platform == Platform::Apple { m.meta } else { m.ctrl }
}

type MoveFn<K> = Rc<dyn Fn(&[K], &K) -> Result<(), Rejection>>;
/// Moves items into a target item, or says why not.
pub type MoveBox<K> = Box<dyn Fn(&[K], &K) -> Result<(), Rejection>>;

/// A tree view over `state`. `load(key)` is asked for the children of an item that opens
/// before they are known; `on_activate` runs for Enter or a double click; `on_move`, when
/// given, moves items into another item (by dragging, or Ctrl+X then Ctrl+V on the target)
/// and says why not when it cannot.
///
/// Keyboard: Up and Down move, Right opens (or goes to the first child), Left closes (or
/// goes to the parent), Home and End, Page Up and Page Down; Shift extends the selection,
/// Ctrl moves without selecting and Space selects; typing jumps to the next item starting
/// with what was typed.
#[track_caller]
#[allow(clippy::too_many_lines)]
pub fn tree_view<K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    state: &TreeState<K>,
    load: impl Fn(&K) + 'static,
    on_activate: impl Fn(&K) + 'static,
    on_move: Option<MoveBox<K>>,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let h = row_height(ui);
    let indent = unit * 5.0;
    let state = state.clone();
    let load: Rc<dyn Fn(&K)> = Rc::new(load);
    let on_activate: Rc<dyn Fn(&K)> = Rc::new(on_activate);
    let on_move: Option<MoveFn<K>> = on_move.map(Rc::from);
    let focused = Signal::new(false);
    let error: Signal<Option<MoveProblem>> = Signal::new(None);

    let (roots, children, expanded) = (state.roots, state.children, state.expanded);
    let lines: Memo<Rc<Vec<Line<K>>>> =
        Memo::new(move || Rc::new(roots.with(|r| children.with(|c| expanded.with(|e| flatten(r, c, e))))));
    let current: Rc<RefCell<Rc<Vec<Line<K>>>>> = Rc::default();

    let is_within = |lines: &[Line<K>], k: &K, ancestor: &K| -> bool {
        // True when `k` is `ancestor` or inside it.
        let parents: HashMap<&K, Option<&K>> =
            lines.iter().map(|l| (&l.item.key, l.parent.as_ref())).collect();
        let mut cur = Some(k);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = parents.get(c).copied().flatten();
        }
        false
    };
    let do_move = {
        let (on_move, current, sel, cut) =
            (on_move.clone(), current.clone(), state.selection.clone(), state.cut);
        Rc::new(move |items: Vec<K>, target: K| {
            let Some(mv) = &on_move else { return };
            let ls = current.borrow().clone();
            let into_self = items.iter().any(|k| is_within(&ls, &target, k));
            let target_ok = ls.iter().any(|l| l.item.key == target && l.item.has_children);
            if into_self || !target_ok {
                error.set(Some(MoveProblem::Message(if into_self {
                    "tree-into-itself"
                } else {
                    "tree-not-folder"
                })));
                return;
            }
            match mv(&items, &target) {
                Ok(()) => {
                    error.set(None);
                    cut.set(Vec::new());
                    sel.cursor.set(Some(target));
                }
                Err(e) => error.set(Some(MoveProblem::App(e.to_string()))),
            }
        })
    };

    // ---- a row
    let build_row = {
        let (ui2, sel, current, act, do_move, state) = (
            ui.clone(),
            state.selection.clone(),
            current.clone(),
            on_activate.clone(),
            do_move.clone(),
            state.clone(),
        );
        let movable = on_move.is_some();
        Rc::new(move |i: usize| -> El {
            let line = current.borrow().get(i).cloned();
            let Some(line) = line else { return ui2.el("tree_item") };
            let k = line.item.key.clone();
            let chevron = if line.item.has_children {
                w::icon(
                    &ui2,
                    if line.open { Icon::ChevronDown } else { Icon::ChevronRight },
                    ui2.theme().type_scale.small + 2.0,
                )
            } else {
                ui2.el("tree_spacer").width(ui2.theme().type_scale.small + 2.0)
            };
            let mut row = ui2
                .el("tree_item")
                .row()
                .align(Align::Center)
                .gap(unit)
                .height(h)
                .padding(Insets {
                    top: 0.0,
                    bottom: 0.0,
                    start: unit * 2.0 + line.depth as f32 * indent,
                    end: unit * 2.0,
                })
                .child(chevron);
            if let Some(icon) = line.item.icon {
                row = row.child(w::icon(&ui2, icon, ui2.theme().type_scale.body));
            }
            row = row.child(w::text(&ui2, line.item.label.clone()).nowrap().grow(1.0));
            if line.loading {
                row = row.child(w::text(&ui2, crate::tr_fn(&ui2, "tree-loading")).variant("muted").nowrap());
            }
            let sem = {
                let (k, sel, line, u) = (k.clone(), sel.clone(), line.clone(), ui2.clone());
                move || {
                    let mut s = Semantics::new(Role::TreeItem)
                        .label(line.item.label.clone())
                        .level(line.depth + 1)
                        .set_position(line.pos.0, line.pos.1)
                        .selected(sel.contains(&k))
                        .action(Action::Click);
                    if line.item.has_children {
                        s = s.expanded(line.open).action(if line.open {
                            Action::Collapse
                        } else {
                            Action::Expand
                        });
                    }
                    if line.loading {
                        s = s.description(crate::tr(&u, "tree-loading-spoken"));
                    }
                    s
                }
            };
            // Press selects; a drag (with moving allowed) carries the selection to a folder.
            // A press selects (keeping a selection it lands in, so the selection can be
            // dragged); a release that was not a drag then selects just this item.
            let pressed: Rc<Cell<bool>> = Rc::default();
            let (ui3, sel2, k2, cur2, act2, p2) =
                (ui2.clone(), sel.clone(), k.clone(), current.clone(), act.clone(), pressed.clone());
            let mut row = row
                .on_pointer(move |e| match e.phase {
                    PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                        if ui3.click_count() >= 2 {
                            act2(&k2);
                            return true;
                        }
                        let order: Vec<K> = cur2.borrow().iter().map(|l| l.item.key.clone()).collect();
                        if sel2.contains(&k2) && e.modifiers == Modifiers::NONE {
                            sel2.cursor.set(Some(k2.clone()));
                            p2.set(true);
                        } else {
                            sel2.choose(
                                k2.clone(),
                                e.modifiers,
                                toggle_key(e.modifiers, ui3.platform()),
                                &order,
                            );
                        }
                        true
                    }
                    PointerPhase::Up => {
                        if p2.replace(false) && e.modifiers == Modifiers::NONE {
                            sel2.select_only(k2.clone());
                        }
                        true
                    }
                    PointerPhase::Cancel => {
                        p2.set(false);
                        false
                    }
                    _ => false,
                })
                .keyboard_alternative(crate::tr_fn(&ui2, "tree-keys"))
                .semantics(sem)
                .on_accessibility_action({
                    // Screen readers and agents select, open and close rows as the keyboard does.
                    let (sel, k, state, has_children) =
                        (sel.clone(), k.clone(), state.clone(), line.item.has_children);
                    move |a| match a {
                        Action::Click => {
                            sel.select_only(k.clone());
                            true
                        }
                        Action::Expand if has_children => {
                            state.expand(k.clone());
                            true
                        }
                        Action::Collapse if has_children => {
                            state.collapse(&k);
                            true
                        }
                        _ => false,
                    }
                });
            if movable || state.export.borrow().is_some() {
                // Dragging carries the selection (or this item), as the app's export says
                // other apps see it, and as itself to targets in this tree.
                let (sel4, k4, p4, export) = (sel.clone(), k.clone(), pressed.clone(), state.export.clone());
                row = row.draggable(
                    move || {
                        p4.set(false);
                        let items = if sel4.contains(&k4) {
                            sel4.selected.get_untracked().unwrap_or_default()
                        } else {
                            vec![k4.clone()]
                        };
                        let data = export.borrow().as_ref().map(|f| f(&items)).unwrap_or_default();
                        let allow: &[DropEffect] =
                            if movable { &[DropEffect::Move, DropEffect::Copy] } else { &[DropEffect::Copy] };
                        Some(Drag::new(data).with_local(Moving(items)).allow(allow))
                    },
                    |_| {},
                );
            }
            if line.item.has_children {
                // Items of this tree move into folders; files from elsewhere are imported.
                let (k5, dm, import) = (k.clone(), do_move.clone(), state.import.clone());
                let (k6, cur6, import2) = (k.clone(), current.clone(), state.import.clone());
                row = row.on_drop(
                    move |d, _| match d.local::<Moving<K>>() {
                        Some(Moving(items)) if movable => {
                            let ls = cur6.borrow().clone();
                            if items.iter().any(|i| is_within(&ls, &k6, i)) {
                                DropEffect::None
                            } else {
                                DropEffect::Move
                            }
                        }
                        Some(_) => DropEffect::None,
                        None if import2.borrow().is_some() && !d.data.is_empty() => DropEffect::Copy,
                        None => DropEffect::None,
                    },
                    move |d, _, _| match d.local::<Moving<K>>() {
                        Some(Moving(items)) => dm(items.clone(), k5.clone()),
                        None => {
                            let f = import.borrow().clone();
                            if let Some(f) = f {
                                match f(&d.data, &k5) {
                                    Ok(()) => error.set(None),
                                    Err(e) => error.set(Some(MoveProblem::App(e.to_string()))),
                                }
                            }
                        }
                    },
                );
            }
            let (r2, sel3, k3) = (row.clone(), sel.clone(), k.clone());
            row.bind(move || {
                r2.state(States::SELECTED, sel3.contains(&k3));
                r2.state(States::FOCUS_VISIBLE, sel3.cursor.get().as_ref() == Some(&k3) && focused.get());
                let cut = state.cut.with(|c| c.contains(&k3));
                let _ = r2.clone().variant(if cut { "cut" } else { "" });
            })
        })
    };

    let (rows, column) = Rows::new(ui, h, build_row as RowBuilder);
    let rows = Rc::new(RefCell::new(rows));
    let viewport = Rc::new(Cell::new(400.0f32));
    let body = ui
        .el("scroll")
        .scroll()
        .column()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = Length::Px(0.0))
        .child(column);
    let show = {
        let (rows, body, vp, ui2) = (rows.clone(), body.clone(), viewport.clone(), ui.clone());
        Rc::new(move || rows.borrow_mut().show(ui2.scroll_offset(body.id()).y, vp.get()))
    };
    let (s1, s2, vp2) = (show.clone(), show.clone(), viewport.clone());
    let body = body.on_scroll(move |_| s1()).on_resize(move |sz: Size| {
        vp2.set(sz.height);
        s2();
    });

    // Rebuild when the open tree changes; ask for children an open item still needs.
    let sync = {
        let (rows, show, current, sel, load) =
            (rows.clone(), show.clone(), current.clone(), state.selection.clone(), load.clone());
        let asked: Rc<RefCell<HashSet<K>>> = Rc::default();
        ui.el("tree_sync").hidden(true).bind(move || {
            let ls = lines.get();
            untrack(|| {
                *current.borrow_mut() = ls.clone();
                let present: HashSet<K> = ls.iter().map(|l| l.item.key.clone()).collect();
                sel.retain(&present);
                let mut asked = asked.borrow_mut();
                asked.retain(|k| ls.iter().any(|l| l.loading && l.item.key == *k));
                for l in ls.iter().filter(|l| l.loading) {
                    if asked.insert(l.item.key.clone()) {
                        load(&l.item.key);
                    }
                }
                drop(asked);
                rows.borrow_mut().set_count(ls.len(), true);
                show();
            });
        })
    };

    // ---- keyboard
    let typed: Rc<RefCell<(String, Option<Instant>)>> = Rc::default();
    let (current, sel, body2, act, ui2, vp3, dm) = (
        current.clone(),
        state.selection.clone(),
        body.clone(),
        on_activate.clone(),
        ui.clone(),
        viewport.clone(),
        do_move.clone(),
    );
    let movable = on_move.is_some();
    let on_key = move |e: &KeyEvent| {
        if !e.pressed {
            return false;
        }
        let ls = current.borrow().clone();
        if ls.is_empty() {
            return false;
        }
        let order: Vec<K> = ls.iter().map(|l| l.item.key.clone()).collect();
        let cur = sel.cursor.get_untracked().ok().flatten().and_then(|c| order.iter().position(|k| *k == c));
        let page = ((vp3.get() / h) as usize).max(1);
        let last = ls.len() - 1;
        let toggle = toggle_key(e.modifiers, ui2.platform());
        let at = |i: usize| ls.get(i);
        let target = match &e.key {
            Key::ArrowDown => Some(cur.map_or(0, |c| (c + 1).min(last))),
            Key::ArrowUp => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            Key::PageDown => Some(cur.map_or(0, |c| (c + page).min(last))),
            Key::PageUp => Some(cur.map_or(0, |c| c.saturating_sub(page))),
            Key::Home => Some(0),
            Key::End => Some(last),
            Key::ArrowRight => {
                let Some(c) = cur else { return false };
                let Some(l) = at(c) else { return false };
                if !l.item.has_children {
                    return false;
                }
                if l.open {
                    // To the first child, once loaded.
                    if l.loading { None } else { Some((c + 1).min(last)) }
                } else {
                    state.expand(l.item.key.clone());
                    return true;
                }
            }
            Key::ArrowLeft => {
                let Some(c) = cur else { return false };
                let Some(l) = at(c) else { return false };
                if l.open {
                    state.collapse(&l.item.key);
                    return true;
                }
                l.parent.as_ref().and_then(|p| order.iter().position(|k| k == p))
            }
            Key::Enter => {
                if let Some(k) = cur.and_then(|c| order.get(c)) {
                    act(k);
                }
                return true;
            }
            Key::Space => {
                if let Some(k) = cur.and_then(|c| order.get(c)).cloned() {
                    sel.choose(k, Modifiers { ctrl: true, meta: true, ..Modifiers::NONE }, true, &order);
                }
                return true;
            }
            Key::Escape if state.cut.with_untracked(|c| !c.is_empty()).unwrap_or(false) => {
                state.cut.set(Vec::new());
                return true;
            }
            Key::Character(s) if toggle && s == "a" => {
                sel.all(&order);
                return true;
            }
            Key::Character(s) if toggle && s == "x" && movable => {
                state.cut.set(sel.selected.get_untracked().unwrap_or_default());
                return true;
            }
            Key::Character(s) if toggle && s == "v" && movable => {
                let items = state.cut.get_untracked().unwrap_or_default();
                if let Some(t) = cur.and_then(|c| order.get(c)).cloned()
                    && !items.is_empty()
                {
                    dm(items, t);
                }
                return true;
            }
            Key::Character(_) if !toggle && !e.modifiers.alt => {
                // Type-ahead: what was typed within a second, matched from the next item on.
                let Some(text) = e.text.as_deref().filter(|t| !t.trim().is_empty()) else { return false };
                let mut t = typed.borrow_mut();
                let now = Instant::now();
                if t.1.is_none_or(|last| now.duration_since(last) > Duration::from_secs(1)) {
                    t.0.clear();
                }
                t.0.push_str(&text.to_lowercase());
                t.1 = Some(now);
                let prefix = t.0.clone();
                drop(t);
                let start = cur.map_or(0, |c| if prefix.chars().count() == 1 { c + 1 } else { c });
                (0..ls.len())
                    .map(|i| (start + i) % ls.len())
                    .find(|&i| at(i).is_some_and(|l| l.item.label.to_lowercase().starts_with(&prefix)))
            }
            _ => None,
        };
        let Some(t) = target else { return matches!(e.key, Key::ArrowRight | Key::Character(_)) };
        let Some(k) = order.get(t).cloned() else { return false };
        if toggle {
            sel.cursor.set(Some(k));
        } else {
            sel.choose(k, e.modifiers, false, &order);
        }
        let off = ui2.scroll_offset(body2.id());
        let top = t as f32 * h;
        let y = if top < off.y {
            top
        } else if top + h > off.y + vp3.get() {
            top + h - vp3.get()
        } else {
            off.y
        };
        if (y - off.y).abs() > f32::EPSILON {
            body2.scroll_to(Vec2::new(off.x, y));
        }
        true
    };

    let status = {
        // Why a move did not happen, announced as it appears.
        let (u1, u2) = (ui.clone(), ui.clone());
        let el = w::text(ui, "")
            .variant("danger")
            .text(move || error.get().map(|p| p.message(&u1)).unwrap_or_default());
        let e2 = el.clone();
        el.bind(move || {
            let msg = error.get().map(|p| p.message(&u2));
            let _ = e2
                .clone()
                .hidden(msg.is_none())
                .semantics(Semantics::new(Role::Alert).label(msg.unwrap_or_default()));
        })
    };
    let l2 = label.clone();
    ui.el("tree")
        .column()
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = Length::Px(0.0))
        .focusable(true)
        .on_focus_change(move |on| focused.set(on))
        .on_key(on_key)
        .child(body)
        .child(status)
        .child(sync)
        .semantics(Semantics::new(Role::Tree).label(l2))
}
