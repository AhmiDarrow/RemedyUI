//! Tabs, navigation items, the nav rail, the bottom bar and breadcrumbs.

use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::{States, StylePatch};
use remedy_ui_view::{Align, El, Ui};
use remedy_ui_widgets_core::{self as w, Icon, MenuItem};

/// One tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tab {
    /// A stable id.
    pub id: String,
    /// What it says.
    pub title: String,
    /// Whether it can be closed (a close button, middle click, and Delete when focused).
    pub closable: bool,
}

impl Tab {
    /// A tab that cannot be closed.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Tab {
        Tab { id: id.into(), title: title.into(), closable: false }
    }

    /// A tab with a close button.
    pub fn closable(mut self) -> Tab {
        self.closable = true;
        self
    }
}

/// Moves the selection among `n` items for an arrow key: `forward` keys and `back` keys,
/// Home and End. Returns the new index.
fn step(key: &Key, cur: usize, n: usize, back: &[Key], forward: &[Key]) -> Option<usize> {
    if n == 0 {
        return None;
    }
    if forward.contains(key) {
        Some((cur + 1) % n)
    } else if back.contains(key) {
        Some((cur + n - 1) % n)
    } else if *key == Key::Home {
        Some(0)
    } else if *key == Key::End {
        Some(n - 1)
    } else {
        None
    }
}

/// A tab strip. `selected` holds the chosen tab's id; the app shows its content. Arrow keys,
/// Home and End move between tabs; closable tabs close by their button, a middle click, or
/// Delete, which calls `on_close`.
#[track_caller]
pub fn tabs(
    ui: &Ui,
    label: impl Into<String>,
    items: Signal<Vec<Tab>>,
    selected: Signal<Option<String>>,
    on_close: impl Fn(&str) + 'static,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let on_close: Rc<dyn Fn(&str)> = Rc::new(on_close);
    let strip = ui
        .el("tab_strip")
        .row()
        .gap(unit / 2.0)
        .fill_width()
        .semantics(Semantics::new(Role::TabList).label(label));
    let (s2, u) = (strip.clone(), ui.clone());
    strip.bind(move || {
        let list = items.get();
        let sel = selected.get();
        untrack(|| {
            // Rebuilding drops the focused tab: give focus to the selected one after.
            let refocus = s2.child_elements().iter().any(El::is_focused);
            s2.clear();
            let ids: Vec<String> = list.iter().map(|t| t.id.clone()).collect();
            let mut focus_to: Option<El> = None;
            for t in &list {
                let on = sel.as_deref() == Some(t.id.as_str());
                let id = t.id.clone();
                let close = on_close.clone();
                let (ids2, id2) = (ids.clone(), id.clone());
                let mut tab = w::min_target(u.el("tab"))
                    .row()
                    .align(Align::Center)
                    .gap(unit)
                    .child(u.el("tab_label").text(remedy_ui_view::RichText::plain(t.title.clone())).nowrap())
                    .on_click({
                        let id = id.clone();
                        move || selected.set(Some(id.clone()))
                    })
                    .on_key(move |e| {
                        if !e.pressed {
                            return false;
                        }
                        let cur = ids2.iter().position(|i| *i == id2).unwrap_or(0);
                        if let Some(n) = step(&e.key, cur, ids2.len(), &[Key::ArrowLeft], &[Key::ArrowRight])
                        {
                            selected.set(ids2.get(n).cloned());
                            return true;
                        }
                        false
                    })
                    .semantics(
                        Semantics::new(Role::Tab).label(t.title.clone()).selected(on).action(Action::Click),
                    );
                tab.state(States::SELECTED, on);
                if t.closable {
                    let (c1, c2, c3) = (close.clone(), id.clone(), id.clone());
                    let close_btn =
                        w::icon_button(&u, Icon::Close, format!("Close {}", t.title), move || c1(&c2))
                            .variant("quiet")
                            .width(w::MIN_TARGET)
                            .height(w::MIN_TARGET);
                    let c4 = close.clone();
                    tab = tab
                        .child(close_btn)
                        .on_pointer(move |e| {
                            if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::MIDDLE) {
                                c4(&c3);
                                return true;
                            }
                            false
                        })
                        .keyboard_alternative("Delete closes the focused tab");
                    let (c5, c6) = (close.clone(), id.clone());
                    tab = tab.on_key(move |e| {
                        if e.pressed && e.key == Key::Delete {
                            c5(&c6);
                            return true;
                        }
                        false
                    });
                }
                // Only the selected tab is a Tab stop; arrows move between tabs.
                let tab = tab.focusable(on);
                if on {
                    focus_to = Some(tab.clone());
                }
                let _ = s2.clone().child(tab);
            }
            if refocus && let Some(t) = focus_to {
                t.focus();
            }
        });
    })
}

/// A destination in a nav rail or bottom bar.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NavEntry {
    /// A stable id.
    pub id: String,
    /// Its icon.
    pub icon: Icon,
    /// What it says.
    pub label: String,
}

impl NavEntry {
    /// An entry.
    pub fn new(id: impl Into<String>, icon: Icon, label: impl Into<String>) -> NavEntry {
        NavEntry { id: id.into(), icon, label: label.into() }
    }
}

/// [`nav_item`] whose label follows `label`.
#[deprecated(since = "0.1.0", note = "`nav_item` takes a closure or signal as its label")]
#[track_caller]
pub fn nav_item_dyn(
    ui: &Ui,
    icon: Icon,
    label: impl Fn() -> String + 'static,
    active: impl Fn() -> bool + 'static,
    badge: Option<Signal<u32>>,
    on_press: impl Fn() + 'static,
) -> El {
    nav_item(ui, icon, label, active, badge, on_press)
}

/// One navigation item: icon, label and an optional count badge. `active` says whether it is
/// the current place (tracked). The label may be a signal or closure (a message in the
/// window's language): it and what assistive technology reads follow it.
#[track_caller]
pub fn nav_item(
    ui: &Ui,
    icon: Icon,
    label: impl remedy_ui_view::IntoProp<String>,
    active: impl Fn() -> bool + 'static,
    badge: Option<Signal<u32>>,
    on_press: impl Fn() + 'static,
) -> El {
    let label = label.into_prop();
    let unit = ui.theme().spacing.unit;
    let l1 = label.clone();
    let item = w::min_target(ui.el("nav_item"))
        .row()
        .align(Align::Center)
        .gap(unit * 2.0)
        .padding(Insets::symmetric(unit * 1.5, unit * 2.5))
        .child(w::icon(ui, icon, ui.theme().type_scale.body + 4.0))
        .child(ui.el("nav_label").text(l1.map(remedy_ui_view::RichText::plain)).nowrap().grow(1.0))
        .on_click(on_press);
    let item = match badge {
        Some(count) => {
            let b = w::badge(ui, "").data_text();
            let b2 = b.clone();
            item.child(b.bind(move || {
                let n = count.get();
                let _ = b2.clone().hidden(n == 0).text(remedy_ui_view::RichText::plain(if n > 99 {
                    "99+".to_owned()
                } else {
                    n.to_string()
                }));
            }))
        }
        None => item,
    };
    let i2 = item.clone();
    item.bind(move || {
        let on = active();
        i2.state(States::SELECTED, on);
        let mut s = Semantics::new(Role::Tab).label(label.get()).selected(on).action(Action::Click);
        if let Some(c) = badge.map(|b| b.get()).filter(|c| *c > 0) {
            s = s.description(format!("{c} new"));
        }
        let _ = i2.clone().semantics(s);
    })
}

fn entries(
    ui: &Ui,
    container: El,
    items: Vec<NavEntry>,
    selected: Signal<String>,
    collapsed: Option<Signal<bool>>,
    vertical: bool,
) -> El {
    let ids: Vec<String> = items.iter().map(|e| e.id.clone()).collect();
    let mut c = container;
    for e in items {
        let id = e.id.clone();
        let id2 = id.clone();
        let item = nav_item(
            ui,
            e.icon,
            e.label.clone(),
            move || selected.get() == id,
            None,
            move || selected.set(id2.clone()),
        );
        if let Some(col) = collapsed {
            // Collapsed: icons only, with the label as a tooltip and the accessible name.
            let label_el = item.child_elements().into_iter().nth(1);
            if let Some(l) = label_el {
                let l2 = l.clone();
                let _ = l.bind(move || {
                    let _ = l2.clone().hidden(col.get());
                });
            }
            w::tooltip(&item, e.label.clone());
        }
        let (ids2, id3) = (ids.clone(), e.id.clone());
        let (back, forward) =
            if vertical { ([Key::ArrowUp], [Key::ArrowDown]) } else { ([Key::ArrowLeft], [Key::ArrowRight]) };
        let item = item.on_key(move |k| {
            if !k.pressed {
                return false;
            }
            let cur = ids2.iter().position(|i| *i == id3).unwrap_or(0);
            match step(&k.key, cur, ids2.len(), &back, &forward) {
                Some(n) => {
                    if let Some(next) = ids2.get(n) {
                        selected.set(next.clone());
                    }
                    true
                }
                None => false,
            }
        });
        c = c.child(item);
    }
    c
}

/// A vertical navigation rail. `collapsed` shows icons only (labels become tooltips).
#[track_caller]
pub fn nav_rail(
    ui: &Ui,
    label: impl Into<String>,
    items: Vec<NavEntry>,
    selected: Signal<String>,
    collapsed: Signal<bool>,
) -> El {
    let unit = ui.theme().spacing.unit;
    let rail = ui
        .el("nav_rail")
        .column()
        .gap(unit)
        .padding(Insets::all(unit * 2.0))
        .semantics(Semantics::new(Role::TabList).label(label));
    entries(ui, rail, items, selected, Some(collapsed), true)
}

/// A bottom navigation bar, for narrow windows and phones.
#[track_caller]
pub fn bottom_bar(ui: &Ui, label: impl Into<String>, items: Vec<NavEntry>, selected: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let bar = ui
        .el("bottom_bar")
        .row()
        .fill_width()
        .gap(unit)
        .padding(Insets::all(unit))
        .semantics(Semantics::new(Role::TabList).label(label));
    entries(ui, bar, items, selected, None, false)
}

/// Breadcrumbs for a path: each segment but the last is a link to go back there
/// (`on_select` gets its index). Long paths keep the first and the last two segments and fold
/// the rest into a "…" menu.
#[track_caller]
pub fn breadcrumbs(
    ui: &Ui,
    label: impl Into<String>,
    path: Signal<Vec<String>>,
    on_select: impl Fn(usize) + 'static,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let on_select: Rc<dyn Fn(usize)> = Rc::new(on_select);
    let bar = ui
        .el("breadcrumbs")
        .row()
        .align(Align::Center)
        .gap(unit)
        .semantics(Semantics::new(Role::Group).label(label));
    let (b2, u) = (bar.clone(), ui.clone());
    bar.bind(move || {
        let segs = path.get();
        untrack(|| {
            b2.clear();
            let n = segs.len();
            let folded: Vec<usize> = if n > 4 { (1..n - 2).collect() } else { Vec::new() };
            let mut shown_any = false;
            for (i, seg) in segs.iter().enumerate() {
                if folded.contains(&i) {
                    if i == 1 {
                        let items: Vec<MenuItem> = folded
                            .iter()
                            .filter_map(|j| segs.get(*j).map(|s| (*j, s.clone())))
                            .map(|(j, s)| {
                                let f = on_select.clone();
                                MenuItem::new(s, move || f(j))
                            })
                            .collect();
                        let _ =
                            b2.clone().child(sep(&u)).child(w::menu_button(&u, "…", items).variant("quiet"));
                    }
                    continue;
                }
                if shown_any {
                    let _ = b2.clone().child(sep(&u));
                }
                shown_any = true;
                let last = i + 1 == n;
                let el = if last {
                    w::text(&u, seg.clone())
                        .variant("strong")
                        .data_text()
                        .semantics(Semantics::new(Role::Label).label(format!("{seg}, current")))
                } else {
                    let f = on_select.clone();
                    w::link(&u, seg.clone(), move || f(i)).data_text()
                };
                let _ = b2.clone().child(el);
            }
        });
    })
}

fn sep(ui: &Ui) -> El {
    w::icon(ui, Icon::ChevronRight, ui.theme().type_scale.small + 2.0)
        .style(StylePatch::default().opacity(0.6))
}
