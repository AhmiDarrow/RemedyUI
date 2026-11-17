//! Lists: keyed children, selectable lists and virtualized lists.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_input::Key;
use remedy_ui_layout::{Length, VirtualList};
use remedy_ui_reactive::Signal;
use remedy_ui_style::States;
use remedy_ui_view::{El, Ui};

/// Keeps `parent`'s children in step with `items`: one element per key, built once by
/// `build`, moved when the order changes and removed when its item goes. Items whose key
/// stays keep their element (and its state).
pub fn for_each<T, K>(
    parent: &El,
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    build: impl Fn(&T) -> El + 'static,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    let built: Rc<RefCell<HashMap<K, El>>> = Rc::default();
    let p = parent.clone();
    parent.clone().bind(move || {
        let list = items.get();
        let keys: Vec<K> = list.iter().map(&key).collect();
        let mut map = built.borrow_mut();
        let gone: Vec<K> = map.keys().filter(|k| !keys.contains(k)).cloned().collect();
        for k in gone {
            if let Some(el) = map.remove(&k) {
                el.remove();
            }
        }
        for (i, (item, k)) in list.iter().zip(&keys).enumerate() {
            let el = match map.get(k) {
                Some(el) => el.clone(),
                None => {
                    let el = remedy_ui_reactive::untrack(|| build(item));
                    map.insert(k.clone(), el.clone());
                    el
                }
            };
            let at = p.child_elements().iter().position(|c| c.id() == el.id());
            if at != Some(i) {
                p.insert_child(i, &el);
            }
        }
    })
}

/// A selectable list. `selected` holds the chosen key; clicking a row or moving with the
/// arrow keys selects it, and pressing a selected row again or Enter calls `on_open`.
pub fn list<T, K>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + Clone + 'static,
    build: impl Fn(&T) -> El + 'static,
    selected: Signal<Option<K>>,
    on_open: impl Fn(&K) + 'static,
) -> El
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
{
    let on_open: Rc<dyn Fn(&K)> = Rc::new(on_open);
    let gap = ui.theme().spacing.unit * 0.5;
    let container = ui.el("list").column().gap(gap).semantics(Semantics::new(Role::List).label(label.into()));
    let row_key = key.clone();
    let open = on_open.clone();
    let rows = for_each(&container, items, key.clone(), move |item| {
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
        row.bind(move || {
            let on = selected.get().as_ref() == Some(&k);
            r2.state(States::SELECTED, on);
            let _ = r2.clone().semantics(Semantics::new(Role::ListItem).selected(on).action(Action::Click));
        })
    });
    rows.on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let list = items.get_untracked().unwrap_or_default();
        let keys: Vec<K> = list.iter().map(&key).collect();
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
    ui.el("list_item").row().align(remedy_ui_layout::Align::Center).fill_width().child(content)
}

/// A vertically scrolling list that builds only the rows near the viewport, for any number
/// of items. Rows may be any height: `estimate` places rows not yet measured, and each row's
/// real height replaces its estimate once shown, keeping the scroll position anchored.
pub fn virtual_list<T: Clone + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<T>>,
    estimate: f32,
    build: impl Fn(usize, &T) -> El + 'static,
) -> El {
    let scroller = crate::scroll(ui).semantics(Semantics::new(Role::List).label(label.into()));
    let top = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let content = ui.el("virtual_rows").column().fill_width().layout(|l| l.shrink = 0.0);
    let bottom = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let scroller = scroller.child(top.clone()).child(content.clone()).child(bottom.clone());
    let vl = Rc::new(RefCell::new(VirtualList::new(0, estimate, false)));
    let shown: Rc<RefCell<Vec<(usize, El)>>> = Rc::default();
    let build: RowBuilder<T> = Rc::new(build);
    let refresh = {
        let (vl, shown, scroller, top, content, bottom) =
            (vl.clone(), shown.clone(), scroller.clone(), top, content, bottom);
        Rc::new(move || {
            let list = items.get_untracked().unwrap_or_default();
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
            let current: Vec<usize> = shown.borrow().iter().map(|(i, _)| *i).collect();
            if current != range.clone().collect::<Vec<_>>() {
                for (_, el) in shown.borrow_mut().drain(..) {
                    el.remove();
                }
                let mut s = shown.borrow_mut();
                for i in range.clone() {
                    if let Some(item) = list.get(i) {
                        let el = build(i, item);
                        let _ = content.clone().child(el.clone());
                        s.push((i, el));
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
    let (vl3, r3) = (vl, refresh);
    scroller.bind(move || {
        let n = items.get().len();
        {
            let mut v = vl3.borrow_mut();
            let have = v.len();
            if n > have {
                v.push(n - have);
            } else if n < have {
                v.remove(n..have);
            }
        }
        r3();
    })
}
