//! Lists: keyed children, selectable lists and virtualized lists.
//!
//! Each list takes its items as a `Signal<Vec<T>>` or a [`SignalVec`]. A `SignalVec`
//! reports each change as an operation (push, insert, remove, move, set, clear, replace)
//! and the list applies it directly: a new message is one `Push` and one row mounts, with
//! nothing compared. A `Signal<Vec<T>>` is compared by key on each change instead (rows
//! whose key stays keep their element and its state).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::Key;
use remedy_ui_core::layout::{Length, VirtualList};
use remedy_ui_core::reactive::{Scope, Signal, SignalVec, VecDiff, VecReader, untrack};
use remedy_ui_core::style::States;
use remedy_ui_engine::view::motion::Motion;
use remedy_ui_engine::view::{El, Transition, Ui};

/// Where a list's items come from: a signal holding a vector, or a [`SignalVec`] whose
/// changes arrive as operations.
#[non_exhaustive]
pub enum ListItems<T: Clone + 'static> {
    /// A vector replaced as a whole; lists compare it by key.
    Signal(Signal<Vec<T>>),
    /// A reactive vector; lists apply its operations.
    Vec(SignalVec<T>),
}

impl<T: Clone + 'static> Clone for ListItems<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone + 'static> Copy for ListItems<T> {}

impl<T: Clone + 'static> std::fmt::Debug for ListItems<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListItems::Signal(s) => write!(f, "ListItems::Signal({s:?})"),
            ListItems::Vec(v) => write!(f, "ListItems::Vec({v:?})"),
        }
    }
}

impl<T: Clone + 'static> From<Signal<Vec<T>>> for ListItems<T> {
    fn from(s: Signal<Vec<T>>) -> Self {
        ListItems::Signal(s)
    }
}

impl<T: Clone + 'static> From<SignalVec<T>> for ListItems<T> {
    fn from(v: SignalVec<T>) -> Self {
        ListItems::Vec(v)
    }
}

impl<T: Clone + 'static> ListItems<T> {
    /// The items now, without subscribing.
    pub fn snapshot(&self) -> Vec<T> {
        match self {
            ListItems::Signal(s) => s.get_untracked().unwrap_or_default(),
            ListItems::Vec(v) => untrack(|| v.with(<[T]>::to_vec)).unwrap_or_default(),
        }
    }

    /// The items now, subscribing the running binding or effect to every change (for a
    /// widget that compares the whole contents by key, as a chat list does).
    pub fn get(&self) -> Vec<T> {
        match self {
            ListItems::Signal(s) => s.get(),
            ListItems::Vec(v) => v.with(<[T]>::to_vec).unwrap_or_default(),
        }
    }

    /// How many items there are now, without subscribing.
    pub fn len(&self) -> usize {
        match self {
            ListItems::Signal(s) => s.with_untracked(Vec::len).unwrap_or(0),
            ListItems::Vec(v) => untrack(|| v.len()).unwrap_or(0),
        }
    }

    /// True when there are no items now.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What changed since the last read, for a list applying it.
enum Change<T> {
    /// Compare everything by key (a `Signal<Vec<T>>` changed, or a vector was replaced).
    All(Vec<T>),
    /// These operations, in order.
    Ops(Vec<VecDiff<T>>),
}

/// Reads a [`ListItems`] inside a binding: subscribes it and says what changed.
struct Feed<T: Clone + 'static> {
    items: ListItems<T>,
    reader: Option<VecReader<T>>,
}

impl<T: Clone + 'static> Feed<T> {
    fn new(items: ListItems<T>) -> Self {
        let reader = match items {
            ListItems::Vec(v) => v.reader().ok(),
            ListItems::Signal(_) => None,
        };
        Feed { items, reader }
    }

    fn read(&self) -> Change<T> {
        match (&self.items, &self.reader) {
            (ListItems::Signal(s), _) => Change::All(s.get()),
            (ListItems::Vec(v), Some(r)) => {
                let ops = r.take_diffs().unwrap_or_default();
                // An operation this list does not know (a newer `SignalVec`) is applied by
                // comparing the whole contents by key instead.
                if ops.iter().all(known) {
                    Change::Ops(ops)
                } else {
                    Change::All(v.with(<[T]>::to_vec).unwrap_or_default())
                }
            }
            (ListItems::Vec(v), None) => Change::All(v.with(<[T]>::to_vec).unwrap_or_default()),
        }
    }
}

/// A row and the scope it was built in: removing it disposes what building it made (its
/// signals, effects and resources), and nothing else.
struct Row {
    el: El,
    scope: Option<Scope>,
}

impl std::ops::Deref for Row {
    type Target = El;
    fn deref(&self) -> &El {
        &self.el
    }
}

impl Row {
    /// Builds `f` in a scope of its own under `owner`'s, outside any binding (so a list's
    /// next change does not dispose it).
    fn build(owner: &El, f: impl FnOnce() -> El) -> Row {
        let mut f = Some(f);
        let mut run = || Scope::new(|| untrack(|| f.take().map(|f| f())));
        if let Some(s) = owner.scope()
            && let Ok((Some(el), scope)) = s.run(&mut run)
        {
            return Row { el, scope: Some(scope) };
        }
        // The owner's scope is gone (it is being removed): the row is built unowned and
        // removed with the list.
        match f.take() {
            Some(f) => Row { el: untrack(f), scope: None },
            None => unreachable!("the build ran inside the owner's scope and gave its row"),
        }
    }

    fn remove(&self) {
        self.el.remove();
        if let Some(s) = self.scope {
            s.dispose();
        }
    }
}

/// True for the operations lists apply one by one.
fn known<T>(op: &VecDiff<T>) -> bool {
    matches!(
        op,
        VecDiff::Replace(_)
            | VecDiff::Insert { .. }
            | VecDiff::Remove { .. }
            | VecDiff::Set { .. }
            | VecDiff::Move { .. }
            | VecDiff::Push(_)
            | VecDiff::Pop
            | VecDiff::Clear
    )
}

/// Rows kept in step with a list's items: one element per item, in order, with its key.
struct Keyed<T, K> {
    parent: El,
    rows: Vec<(K, Row)>,
    key: Rc<dyn Fn(&T) -> K>,
    build: Rc<dyn Fn(&T) -> El>,
    transitions: ListTransitions,
    /// The first change builds what is there already: it plays no enter transitions.
    first: bool,
}

impl<T: Clone + 'static, K: Eq + Hash + Clone + 'static> Keyed<T, K> {
    fn make(&self, item: &T) -> Row {
        Row::build(&self.parent, || (self.build)(item))
    }

    /// Where `el` is among the parent's children (rows leaving with an exit transition are
    /// still among them).
    fn dom_index(&self, el: &El) -> Option<usize> {
        self.parent.child_elements().iter().position(|c| c.id() == el.id())
    }

    /// Puts row `i` right after row `i - 1` (or first) among the parent's children.
    fn place(&self, i: usize) {
        let Some((_, row)) = self.rows.get(i) else { return };
        let target = match i.checked_sub(1).and_then(|p| self.rows.get(p)) {
            Some((_, prev)) => self.dom_index(prev).map_or(0, |p| p + 1),
            None => 0,
        };
        let cur = self.dom_index(row);
        if cur == Some(target) {
            return;
        }
        // Inserting detaches it first: a row before its target moves the target down one.
        let at = match cur {
            Some(c) if c < target => target - 1,
            _ => target,
        };
        self.parent.insert_child(at, row);
    }

    fn insert(&mut self, index: usize, item: &T) {
        let index = index.min(self.rows.len());
        let el = self.make(item);
        self.rows.insert(index, ((self.key)(item), el));
        self.place(index);
        if let (Some(t), false, Some((_, row))) = (&self.transitions.enter, self.first, self.rows.get(index))
        {
            remedy_ui_engine::view::play_enter(row, t);
        }
    }

    /// Takes `row` away: at once, or after its exit transition, inert meanwhile.
    fn leave(&self, row: Row) {
        match &self.transitions.exit {
            Some(t) => {
                let el = row.el.clone();
                remedy_ui_engine::view::play_exit(&el, t, move || row.remove());
            }
            None => row.remove(),
        }
    }

    fn remove(&mut self, index: usize) {
        if index < self.rows.len() {
            let (_, row) = self.rows.remove(index);
            self.leave(row);
        }
    }

    /// Compares `list` with the rows by key: rows whose key stays are kept (and moved into
    /// place), new keys are built, gone keys removed.
    fn reconcile(&mut self, list: &[T]) {
        let keys: Vec<K> = list.iter().map(|t| (self.key)(t)).collect();
        let mut old: HashMap<K, Row> = HashMap::with_capacity(self.rows.len());
        for (k, row) in std::mem::take(&mut self.rows) {
            // Two rows with one key: only one can be matched again, the other goes now.
            if let Some(twin) = old.insert(k, row) {
                twin.remove();
            }
        }
        let mut rows = Vec::with_capacity(list.len());
        let mut fresh = Vec::new();
        for (i, (item, k)) in list.iter().zip(keys).enumerate() {
            let row = match old.remove(&k) {
                Some(row) => row,
                None => {
                    fresh.push(i);
                    self.make(item)
                }
            };
            rows.push((k, row));
        }
        for (_, row) in old {
            self.leave(row);
        }
        self.rows = rows;
        for i in 0..self.rows.len() {
            self.place(i);
        }
        if let (Some(t), false) = (&self.transitions.enter, self.first) {
            for i in fresh {
                if let Some((_, row)) = self.rows.get(i) {
                    remedy_ui_engine::view::play_enter(row, t);
                }
            }
        }
    }

    fn apply(&mut self, change: Change<T>) {
        // Rows that stay glide to their new places (FLIP), from where they are now.
        if let (Some(m), false) = (self.transitions.moves, self.first) {
            for (_, row) in &self.rows {
                row.flip(m);
            }
        }
        match change {
            Change::All(list) => self.reconcile(&list),
            Change::Ops(ops) => {
                for op in ops {
                    match op {
                        VecDiff::Replace(list) => self.reconcile(&list),
                        VecDiff::Insert { index, value } => self.insert(index, &value),
                        VecDiff::Push(value) => self.insert(self.rows.len(), &value),
                        VecDiff::Remove { index } => self.remove(index),
                        VecDiff::Pop => self.remove(self.rows.len().saturating_sub(1)),
                        VecDiff::Clear => {
                            for (_, row) in std::mem::take(&mut self.rows) {
                                self.leave(row);
                            }
                        }
                        VecDiff::Set { index, value } => {
                            // The item changed: its row is built again from it, in place.
                            if index < self.rows.len() {
                                let (_, row) = self.rows.remove(index);
                                row.remove();
                                let el = self.make(&value);
                                self.rows.insert(index, ((self.key)(&value), el));
                                self.place(index);
                            }
                        }
                        VecDiff::Move { from, to } if from < self.rows.len() => {
                            let row = self.rows.remove(from);
                            let to = to.min(self.rows.len());
                            self.rows.insert(to, row);
                            self.place(to);
                        }
                        // A move from past the end changes nothing; operations `Feed::read` does not know arrive as `Change::All`.
                        _ => {}
                    }
                }
            }
        }
        self.first = false;
    }
}

/// How a keyed list's rows arrive, leave and move ([`for_each_with`]). Under reduced
/// motion each is instant.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ListTransitions {
    /// Played by a row that arrives (not by the rows there when the list is first built).
    pub enter: Option<Transition>,
    /// Played by a row that leaves; it stays, inert, until the transition ends.
    pub exit: Option<Transition>,
    /// How rows that stay glide to new places when others arrive, leave or move (FLIP).
    pub moves: Option<Motion>,
}

/// The keys of a list's rows in order, and a signal that changes whenever they do: rows
/// read it to know their place in the list.
#[derive(Clone)]
struct Order<K> {
    keys: Rc<RefCell<Vec<K>>>,
    version: Signal<u64>,
}

impl<K: Eq + Clone + 'static> Order<K> {
    /// `k`'s place (from 1) and the list's length, subscribing to changes of the order.
    fn place(&self, k: &K) -> Option<(usize, usize)> {
        let _ = self.version.try_with(|_| ());
        let keys = self.keys.borrow();
        keys.iter().position(|x| x == k).map(|i| (i + 1, keys.len()))
    }
}

fn keyed<T, K>(
    parent: &El,
    items: ListItems<T>,
    key: impl Fn(&T) -> K + 'static,
    build: impl Fn(&T) -> El + 'static,
    order: Option<Order<K>>,
    transitions: ListTransitions,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    let feed = Feed::new(items);
    let rows = RefCell::new(Keyed {
        parent: parent.clone(),
        rows: Vec::new(),
        key: Rc::new(key),
        build: Rc::new(build),
        transitions,
        first: true,
    });
    parent.clone().bind(move || {
        let change = feed.read();
        untrack(|| {
            let mut rows = rows.borrow_mut();
            rows.apply(change);
            if let Some(o) = &order {
                let keys: Vec<K> = rows.rows.iter().map(|(k, _)| k.clone()).collect();
                let changed = *o.keys.borrow() != keys;
                if changed {
                    *o.keys.borrow_mut() = keys;
                    let _ = o.version.try_update(|v| *v += 1);
                }
            }
        });
    })
}

/// Keeps `parent`'s children in step with `items` (a `Signal<Vec<T>>` or a
/// [`SignalVec`]): one element per key, built once by `build`, moved when the order changes
/// and removed when its item goes. Items whose key stays keep their element (and its
/// state); a `SignalVec`'s operations are applied as they come, so a push mounts one row.
pub fn for_each<T, K>(
    parent: &El,
    items: impl Into<ListItems<T>>,
    key: impl Fn(&T) -> K + 'static,
    build: impl Fn(&T) -> El + 'static,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    keyed(parent, items.into(), key, build, None, ListTransitions::default())
}

impl ListTransitions {
    /// No transitions: rows appear, leave and move at once.
    pub fn new() -> Self {
        ListTransitions::default()
    }

    /// Rows that arrive play `t`.
    pub fn enter(mut self, t: Transition) -> Self {
        self.enter = Some(t);
        self
    }

    /// Rows that leave play `t`.
    pub fn exit(mut self, t: Transition) -> Self {
        self.exit = Some(t);
        self
    }

    /// Rows that stay glide to new places with `m`.
    pub fn moves(mut self, m: Motion) -> Self {
        self.moves = Some(m);
        self
    }
}

/// [`for_each`] with transitions: rows that arrive play `enter`, rows that leave play
/// `exit` (staying, inert, until it ends), and rows that stay glide to their new places
/// with `moves` (FLIP). All of it is instant under reduced motion.
pub fn for_each_with<T, K>(
    parent: &El,
    items: impl Into<ListItems<T>>,
    key: impl Fn(&T) -> K + 'static,
    build: impl Fn(&T) -> El + 'static,
    transitions: ListTransitions,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    keyed(parent, items.into(), key, build, None, transitions)
}

/// A selectable list. `selected` holds the chosen key; clicking a row or moving with the
/// arrow keys selects it, and pressing a selected row again or Enter calls `on_open`.
///
/// Each row tells screen readers its name (what it shows, merged into one phrase), its
/// place in the list ("3 of 12") and whether it is selected; assistive technology clicks a
/// row to select or open it as a pointer would. It is a list box of options to them
/// (ARIA `listbox` and `option`), one selected at a time.
pub fn list<T, K>(
    ui: &Ui,
    label: impl Into<String>,
    items: impl Into<ListItems<T>>,
    key: impl Fn(&T) -> K + Clone + 'static,
    build: impl Fn(&T) -> El + 'static,
    selected: Signal<Option<K>>,
    on_open: impl Fn(&K) + 'static,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    let items: ListItems<T> = items.into();
    let on_open: Rc<dyn Fn(&K)> = Rc::new(on_open);
    let gap = ui.theme().spacing.unit * 0.5;
    let container =
        ui.el("list").column().gap(gap).semantics(Semantics::new(Role::ListBox).label(label.into()));
    let order = Order { keys: Rc::default(), version: Signal::new(0) };
    let row_key = key.clone();
    let open = on_open.clone();
    let o2 = order.clone();
    let rows = keyed(
        &container,
        items,
        key.clone(),
        move |item| {
            let k = row_key(item);
            let (k_click, open) = (k.clone(), open.clone());
            let row = ui_row(build(item)).on_click(move || {
                if selected.get_untracked().ok().flatten().as_ref() == Some(&k_click) {
                    open(&k_click);
                } else {
                    selected.set(Some(k_click.clone()));
                }
            });
            let r2 = row.clone();
            let (o3, k2) = (o2.clone(), k.clone());
            row.bind(move || {
                let on = selected.get().as_ref() == Some(&k);
                r2.state(States::SELECTED, on);
            })
            .semantics(move || {
                let on = selected.get().as_ref() == Some(&k2);
                let mut s =
                    Semantics::new(Role::Option).selected(on).action(Action::Click).merge_descendants();
                if let Some((pos, n)) = o3.place(&k2) {
                    s = s.set_position(pos, n);
                }
                s
            })
        },
        Some(order),
        ListTransitions::default(),
    );
    rows.on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let keys: Vec<K> = items.snapshot().iter().map(&key).collect();
        let cur = selected.get_untracked().ok().flatten().and_then(|s| keys.iter().position(|k| *k == s));
        if e.key == Key::Enter {
            if let Some(k) = cur.and_then(|c| keys.get(c)) {
                on_open(k);
                return true;
            }
            return false;
        }
        if keys.is_empty() {
            return false;
        }
        let next = match e.key {
            Key::ArrowDown => cur.map_or(0, |c| (c + 1).min(keys.len() - 1)),
            Key::ArrowUp => cur.map_or(0, |c| c.saturating_sub(1)),
            Key::Home => 0,
            Key::End => keys.len() - 1,
            _ => return false,
        };
        selected.set(keys.get(next).cloned());
        true
    })
}

/// Builds the row for item `i`.
type RowBuilder<T> = Rc<dyn Fn(usize, &T) -> El>;

fn ui_row(content: El) -> El {
    let ui = content.ui().clone();
    ui.el("list_item").row().align(remedy_ui_core::layout::Align::Center).fill_width().child(content)
}

/// The rows a virtual list shows, by item index.
type Shown = Rc<RefCell<Vec<(usize, Row)>>>;

/// A vertically scrolling list that builds only the rows near the viewport, for any number
/// of items. Rows may be any height: `estimate` places rows not yet measured, and each row's
/// real height replaces its estimate once shown, keeping the scroll position anchored.
///
/// With a [`SignalVec`], changes are applied as they come: an item added outside the rows
/// shown builds nothing, one added among them builds its row alone, and the rows around it
/// keep their elements.
#[track_caller]
pub fn virtual_list<T: Clone + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: impl Into<ListItems<T>>,
    estimate: f32,
    build: impl Fn(usize, &T) -> El + 'static,
) -> El {
    let items: ListItems<T> = items.into();
    let scroller = crate::base::scroll(ui).semantics(Semantics::new(Role::List).label(label.into()));
    let top = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let content = ui.el("virtual_rows").column().fill_width().layout(|l| l.shrink = 0.0);
    let bottom = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let scroller = scroller.child(top.clone()).child(content.clone()).child(bottom.clone());
    let vl = Rc::new(RefCell::new(VirtualList::new(0, estimate, false)));
    let shown: Shown = Rc::default();
    let build: RowBuilder<T> = Rc::new(build);
    // The items as the list knows them: what rows are built from.
    let model: Rc<RefCell<Vec<T>>> = Rc::default();
    let refresh = {
        let (vl, shown, scroller, top, content, bottom, model, build) =
            (vl.clone(), shown.clone(), scroller.clone(), top, content, bottom, model.clone(), build.clone());
        Rc::new(move || {
            let list = model.borrow();
            let mut v = vl.borrow_mut();
            // Measured heights of rows on screen replace their estimates.
            for (i, el) in shown.borrow().iter() {
                if let Some(b) = el.bounds()
                    && b.height() > 0.0
                {
                    v.set_height(*i, b.height());
                }
            }
            if let Some(b) = scroller.bounds() {
                v.set_viewport(b.height());
            }
            let range = v.visible_range(estimate * 4.0);
            let range = range.start.min(list.len())..range.end.min(list.len());
            let mut s = shown.borrow_mut();
            // Rows that left the range go; rows already built stay; missing ones are built.
            s.retain(|(i, el)| {
                let keep = range.contains(i);
                if !keep {
                    el.remove();
                }
                keep
            });
            for (n, i) in range.clone().enumerate() {
                match s.get(n) {
                    Some((j, _)) if *j == i => {}
                    _ => {
                        if let Some(item) = list.get(i) {
                            let el = Row::build(&content, || build(i, item));
                            content.insert_child(n, &el);
                            s.insert(n, (i, el));
                        }
                    }
                }
            }
            let above = v.offset_of(range.start);
            let below = (v.total_height() - v.offset_of(range.end)).max(0.0);
            let _ = top.clone().layout(|l| l.height = Length::Px(above));
            let _ = bottom.clone().layout(|l| l.height = Length::Px(below));
        })
    };
    let (vl2, r2) = (vl.clone(), refresh.clone());
    let scroller = scroller.on_scroll(move |o| {
        vl2.borrow_mut().scroll_to(o.y);
        r2();
    });
    // Once laid out (and whenever its size changes) it knows how many rows fit.
    let r4 = refresh.clone();
    let scroller = scroller.on_resize(move |_| r4());
    let feed = Feed::new(items);
    let (vl3, r3) = (vl, refresh);
    scroller.bind(move || {
        let change = feed.read();
        untrack(|| {
            let mut m = model.borrow_mut();
            let mut v = vl3.borrow_mut();
            let mut s = shown.borrow_mut();
            let rebuild_all =
                |m: &mut Vec<T>, v: &mut VirtualList, s: &mut Vec<(usize, Row)>, list: Vec<T>| {
                    for (_, el) in s.drain(..) {
                        el.remove();
                    }
                    let have = v.len();
                    v.remove(0..have);
                    v.push(list.len());
                    *m = list;
                };
            // Shifts the rows shown at or after `at` by `by`.
            let shift = |s: &mut Vec<(usize, Row)>, at: usize, by: isize| {
                for (i, _) in s.iter_mut() {
                    if *i >= at {
                        *i = i.saturating_add_signed(by);
                    }
                }
            };
            let drop_row = |s: &mut Vec<(usize, Row)>, at: usize| {
                if let Some(p) = s.iter().position(|(i, _)| *i == at) {
                    let (_, el) = s.remove(p);
                    el.remove();
                }
            };
            match change {
                Change::All(list) => rebuild_all(&mut m, &mut v, &mut s, list),
                Change::Ops(ops) => {
                    for op in ops {
                        match op {
                            VecDiff::Replace(list) => rebuild_all(&mut m, &mut v, &mut s, list),
                            VecDiff::Push(value) => {
                                m.push(value);
                                v.push(1);
                            }
                            VecDiff::Insert { index, value } => {
                                let index = index.min(m.len());
                                m.insert(index, value);
                                v.insert(index, 1);
                                shift(&mut s, index, 1);
                            }
                            VecDiff::Remove { index } => {
                                if index < m.len() {
                                    m.remove(index);
                                    v.remove(index..index + 1);
                                    drop_row(&mut s, index);
                                    shift(&mut s, index + 1, -1);
                                }
                            }
                            VecDiff::Pop => {
                                if let Some(last) = m.len().checked_sub(1) {
                                    m.pop();
                                    v.remove(last..last + 1);
                                    drop_row(&mut s, last);
                                }
                            }
                            VecDiff::Clear => rebuild_all(&mut m, &mut v, &mut s, Vec::new()),
                            VecDiff::Set { index, value } => {
                                if let Some(slot) = m.get_mut(index) {
                                    *slot = value;
                                    drop_row(&mut s, index);
                                }
                            }
                            VecDiff::Move { from, to } if from < m.len() => {
                                let item = m.remove(from);
                                let to = to.min(m.len());
                                m.insert(to, item);
                                v.remove(from..from + 1);
                                v.insert(to, 1);
                                drop_row(&mut s, from);
                                shift(&mut s, from + 1, -1);
                                shift(&mut s, to, 1);
                            }
                            // A move from past the end changes nothing; operations `Feed::read` does not know arrive as `Change::All`.
                            _ => {}
                        }
                    }
                }
            }
            // Rows stay in index order for `refresh`.
            s.sort_by_key(|(i, _)| *i);
        });
        r3();
    })
}
