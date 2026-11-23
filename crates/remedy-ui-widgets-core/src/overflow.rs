//! A one-line bar that moves children that do not fit into a "More" menu.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_layout::{Align, Length};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_view::{El, Ui};

use crate::menus::MenuItem;
use crate::{MIN_TARGET, button, menu, tooltip};

/// Logical pixels kept for shrinkable [`OverflowRow::keep`] children (an address
/// field) so the More menu does not claim the last of the row.
const FLEX_RESERVE: f32 = 48.0;

enum OverflowKind {
    /// The More menu activates the hidden child (a click).
    Activate,
    /// The More menu shows a submenu instead of activating the child (a list of
    /// bookmarks whose popover would otherwise open from a hidden anchor).
    Submenu(Rc<dyn Fn() -> Vec<MenuItem>>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Moves into the More menu when the row is too narrow.
    Auto,
    /// Stays in the row and keeps its width (Back, Home).
    Keep,
    /// Stays in the row and shrinks (the address field).
    Flex,
}

struct Item {
    el: El,
    label: Rc<dyn Fn() -> String>,
    slot: Slot,
    min: f32,
    kind: OverflowKind,
}

/// A one-line bar. Children that do not fit, starting from the end, move into a
/// "More" menu. Mark a child with [`OverflowRow::keep`] so it stays in the row
/// (it may still shrink). The bar clips, so nothing paints past its box.
pub struct OverflowRow {
    ui: Ui,
    gap: f32,
    items: Vec<Item>,
}

/// Starts a one-line overflowing bar. See [`OverflowRow`].
#[track_caller]
pub fn overflow_row(ui: &Ui) -> OverflowRow {
    OverflowRow { ui: ui.clone(), gap: ui.theme().spacing.unit, items: Vec::new() }
}

impl OverflowRow {
    /// Space between children, in logical pixels.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// A child that stays in the row (the address field, a title). It may shrink.
    pub fn keep(mut self, el: El) -> Self {
        self.items.push(Item {
            el,
            label: Rc::new(String::new),
            slot: Slot::Keep,
            min: MIN_TARGET,
            kind: OverflowKind::Activate,
        });
        self
    }

    /// A child that stays in the row and shrinks to make room (the address field).
    pub fn flex(mut self, el: El) -> Self {
        self.items.push(Item {
            el,
            label: Rc::new(String::new),
            slot: Slot::Flex,
            min: 0.0,
            kind: OverflowKind::Activate,
        });
        self
    }

    /// A child that moves into the More menu when the row is too narrow. `label`
    /// is the menu entry. The hidden control is activated with a click.
    pub fn child(self, label: impl Into<String>, el: El) -> Self {
        let label = label.into();
        self.child_fn(move || label.clone(), el)
    }

    /// Like [`OverflowRow::child`], with a label built each time the More menu opens.
    pub fn child_fn(self, label: impl Fn() -> String + 'static, el: El) -> Self {
        self.push_auto(label, el, MIN_TARGET)
    }

    /// Like [`OverflowRow::child`], but an unmeasured or collapsed child takes no
    /// space. Use for slots that `show()` hides until a condition is true, so they
    /// do not reserve [`MIN_TARGET`] and force the More menu early.
    pub fn child_maybe(self, label: impl Into<String>, el: El) -> Self {
        let label = label.into();
        self.child_maybe_fn(move || label.clone(), el)
    }

    /// Like [`OverflowRow::child_maybe`], with a label built each time the More menu opens.
    pub fn child_maybe_fn(self, label: impl Fn() -> String + 'static, el: El) -> Self {
        self.push_auto(label, el, 0.0)
    }

    fn push_auto(mut self, label: impl Fn() -> String + 'static, el: El, min: f32) -> Self {
        self.items.push(Item {
            el,
            label: Rc::new(label),
            slot: Slot::Auto,
            min,
            kind: OverflowKind::Activate,
        });
        self
    }

    /// Like [`OverflowRow::child`], but when this child overflows, the More menu
    /// shows `items` as a submenu instead of activating `el`.
    pub fn child_submenu(
        mut self,
        label: impl Into<String>,
        el: El,
        items: impl Fn() -> Vec<MenuItem> + 'static,
    ) -> Self {
        let label = label.into();
        self.items.push(Item {
            el,
            label: Rc::new(move || label.clone()),
            slot: Slot::Auto,
            min: MIN_TARGET,
            kind: OverflowKind::Submenu(Rc::new(items)),
        });
        self
    }

    /// Builds the bar. It fills the width it is given and clips any leftover
    /// overflow (a frame before the More menu catches up, or keep-children that
    /// cannot shrink enough).
    pub fn el(self) -> El {
        let OverflowRow { ui, gap, items } = self;
        let items = Rc::new(items);
        let hide = Signal::new(0usize);
        let cached = Rc::new(RefCell::new(vec![0.0f32; items.len()]));
        let more_w = Rc::new(RefCell::new(MIN_TARGET));
        let avail = Rc::new(Cell::new(0.0f32));
        let recompute = {
            let hide = hide.clone();
            let items = items.clone();
            let cached = cached.clone();
            let more_w = more_w.clone();
            let avail = avail.clone();
            Rc::new(move || {
                let n = hide_count(avail.get(), &items, &cached.borrow(), *more_w.borrow(), gap);
                if n != hide.get_untracked().unwrap_or(0) {
                    hide.set(n);
                }
            })
        };

        let wrap = ui.el("overflow_row").column().layout(|l| {
            l.width = Length::Percent(1.0);
            l.min_width = Length::Px(0.0);
        });
        let bar = ui.el("overflow_row_bar").row().align(Align::Center).gap(gap).clip().layout(|l| {
            l.width = Length::Percent(1.0);
            l.min_width = Length::Px(0.0);
        });

        for (i, item) in items.iter().enumerate() {
            let el = item.el.clone();
            if item.slot != Slot::Flex {
                let cached = cached.clone();
                let rec = recompute.clone();
                let auto = item.slot == Slot::Auto;
                let el = el
                    .layout(move |l| {
                        if auto {
                            l.shrink = 0.0;
                        }
                    })
                    .on_resize(move |s| {
                        if s.width > 1.0 {
                            let mut c = cached.borrow_mut();
                            if (c[i] - s.width).abs() > 0.5 {
                                c[i] = s.width;
                                drop(c);
                                rec();
                            }
                        }
                    });
                let _ = bar.clone().child(el);
            } else {
                let _ = bar.clone().child(el);
            }
        }

        let ui_m = ui.clone();
        let items_m = items.clone();
        let hide_m = hide.clone();
        let more = more_button(
            &ui,
            Rc::new(move || {
                let n_hide = hide_m.get_untracked().unwrap_or(0);
                let auto: Vec<usize> = items_m
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| it.slot == Slot::Auto)
                    .map(|(i, _)| i)
                    .collect();
                let hidden: HashSet<usize> = auto.iter().rev().take(n_hide).copied().collect();
                menu_items(&ui_m, &items_m, &hidden)
            }),
        )
        .hidden(true);
        let rec = recompute.clone();
        let more_w_b = more_w.clone();
        let more = more.on_resize(move |s| {
            if s.width > 1.0 {
                *more_w_b.borrow_mut() = s.width.max(MIN_TARGET);
                rec();
            }
        });
        let _ = bar.clone().child(more.clone());

        let items_b = items.clone();
        let more_b = more.clone();
        let bar = bar.bind(move || {
            let n_hide = hide.get();
            let auto: Vec<usize> =
                items_b.iter().enumerate().filter(|(_, it)| it.slot == Slot::Auto).map(|(i, _)| i).collect();
            let hidden: HashSet<usize> = auto.iter().rev().take(n_hide).copied().collect();
            untrack(|| {
                for (i, item) in items_b.iter().enumerate() {
                    if item.slot == Slot::Auto {
                        let _ = item.el.clone().hidden(hidden.contains(&i));
                    }
                }
                let _ = more_b.clone().hidden(n_hide == 0);
            });
        });

        let avail_r = avail.clone();
        let rec = recompute.clone();
        let bar = bar.on_resize(move |s| {
            avail_r.set(s.width);
            rec();
        });

        wrap.child(bar)
    }
}

fn menu_items(ui: &Ui, items: &[Item], hidden: &HashSet<usize>) -> Vec<MenuItem> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if !hidden.contains(&i) {
            continue;
        }
        let label = (item.label)();
        match &item.kind {
            OverflowKind::Submenu(items) => {
                let items = items.clone();
                out.push(MenuItem::submenu_with(label, move || items()));
            }
            OverflowKind::Activate => {
                let el = item.el.clone();
                let ui = ui.clone();
                out.push(MenuItem::new(label, move || {
                    let _ = ui.accessibility_action(el.node_id(), Action::Click);
                }));
            }
        }
    }
    out
}

fn hide_count(avail: f32, items: &[Item], cached: &[f32], more_w: f32, gap: f32) -> usize {
    // Wait for the first layout so a 0-width pass does not hide every child.
    if avail <= 1.0 {
        return 0;
    }
    let auto: Vec<usize> =
        items.iter().enumerate().filter(|(_, it)| it.slot == Slot::Auto).map(|(i, _)| i).collect();
    let reserved = if items.iter().any(|it| it.slot == Slot::Flex) { FLEX_RESERVE } else { 0.0 };
    for n_hide in 0..=auto.len() {
        let hidden: HashSet<usize> = auto.iter().rev().take(n_hide).copied().collect();
        let mut n = 0u32;
        let mut sum = 0.0;
        let mut add = |w: f32| {
            if n > 0 {
                sum += gap;
            }
            sum += w;
            n += 1;
        };
        for (i, item) in items.iter().enumerate() {
            if item.slot == Slot::Flex || hidden.contains(&i) {
                continue;
            }
            let w = if i < cached.len() && cached[i] > 1.0 { cached[i] } else { item.min };
            add(w);
        }
        if n_hide > 0 {
            add(more_w.max(MIN_TARGET));
        }
        if sum + reserved <= avail {
            return n_hide;
        }
    }
    auto.len()
}

fn more_button(ui: &Ui, items: Rc<dyn Fn() -> Vec<MenuItem>>) -> El {
    let anchor: Rc<RefCell<Option<El>>> = Rc::default();
    let label = crate::tr(ui, "menu-more");
    let ui2 = ui.clone();
    let a2 = anchor.clone();
    let items2 = items.clone();
    let btn = button(ui, "…", move || {
        if let Some(a) = a2.borrow().clone() {
            let _ = menu(&ui2, &a, items2());
        }
    })
    .variant("quiet")
    .semantics({
        let label = label.clone();
        move || Semantics::new(Role::Button).label(label.clone()).action(Action::Click)
    });
    tooltip(&btn, label);
    *anchor.borrow_mut() = Some(btn.clone());
    btn
}
