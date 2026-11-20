//! Sheets and drawers: panels that slide over the window from an edge, and a custom title
//! bar for frameless windows.

use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_input::{Key, PointerPhase};
use remedy_ui_layout::{Length, Position};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::StylePatch;
use remedy_ui_view::{Align, El, Justify, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

/// The window edge a panel comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Edge {
    /// The start edge (left in left-to-right windows).
    Start,
    /// The end edge (right in left-to-right windows).
    End,
    /// The bottom edge.
    Bottom,
}

/// A panel over the window from `edge`, with a scrim behind it, shown while `open` is true.
/// Escape, a click on the scrim, or setting `open` false closes it; focus stays inside
/// while it is open and returns where it was after.
#[track_caller]
pub fn drawer(ui: &Ui, label: impl Into<String>, edge: Edge, open: Signal<bool>, content: El) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let (u, l2) = (ui.clone(), label.clone());
    // Place the returned element anywhere in the window: it is invisible and keeps the
    // drawer's binding alive; the panel itself shows in the overlay layer.
    let holder = ui.el("drawer_holder").hidden(true);
    let content = Rc::new(content);
    let restore: Rc<std::cell::RefCell<Option<El>>> = Rc::default();
    let layer: Rc<std::cell::RefCell<Option<El>>> = Rc::default();
    holder.bind(move || {
        let show = open.get();
        untrack(|| {
            if show && layer.borrow().is_none() {
                *restore.borrow_mut() = u.tree().focused().map(|f| u.element(f));
                let panel = u
                    .el("drawer")
                    .column()
                    .gap(unit * 2.0)
                    .padding(Insets::all(unit * 4.0))
                    .semantics(Semantics::new(Role::Dialog).label(l2.clone()))
                    .child((*content).clone());
                let panel = match edge {
                    Edge::Bottom => panel.fill_width().layout(|l| l.max_height = Length::Percent(0.85)),
                    _ => panel.layout(|l| {
                        l.height = Length::Percent(1.0);
                        l.width = Length::Px(320.0);
                        l.max_width = Length::Percent(0.9);
                    }),
                };
                // A press on the scrim, outside the panel, closes it. Read from the pointer
                // rather than a click handler on the panel, so the panel is not a control
                // assistive technology would be offered with nothing to do.
                let inside = panel.clone();
                let scrim = w::fill_window(u.el("scrim"))
                    .focus_scope()
                    .on_pointer(move |e| {
                        let outside = !inside.bounds().is_some_and(|b| b.contains(e.position));
                        if e.phase == PointerPhase::Down && outside {
                            open.set(false);
                            return true;
                        }
                        false
                    })
                    .keyboard_alternative("Escape closes the panel")
                    .focusable(false)
                    .on_key(move |e| {
                        if e.pressed && e.key == Key::Escape {
                            open.set(false);
                            return true;
                        }
                        false
                    });
                let scrim = match edge {
                    Edge::Start => scrim.row().justify(Justify::Start),
                    Edge::End => scrim.row().justify(Justify::End),
                    Edge::Bottom => scrim.column().justify(Justify::End),
                }
                .child(panel.clone());
                u.overlay(scrim.clone());
                panel.focus_first();
                *layer.borrow_mut() = Some(scrim);
            } else if !show && let Some(l) = layer.borrow_mut().take() {
                l.remove();
                if let Some(r) = restore.borrow_mut().take() {
                    r.focus();
                }
            }
        });
    })
}

/// A bottom sheet: a [`drawer`] from the bottom edge with a title and a close button.
#[track_caller]
pub fn sheet(ui: &Ui, title: impl Into<String>, open: Signal<bool>, content: El) -> El {
    let title = title.into();
    let unit = ui.theme().spacing.unit;
    let head =
        w::row(ui).align(Align::Center).fill_width().child(w::heading(ui, 2, title.clone()).grow(1.0)).child(
            w::icon_button(ui, Icon::Close, ui.word("Close"), move || open.set(false)).variant("quiet"),
        );
    let body = w::column(ui).gap(unit * 3.0).fill_width().child(head).child(content);
    drawer(ui, title, Edge::Bottom, open, body)
}

/// What a custom title bar does with its buttons (the app wires them to its window).
pub struct TitleBarActions {
    /// Starts moving the window (a press on the bar).
    pub begin_move: Box<dyn Fn()>,
    /// Minimizes.
    pub minimize: Box<dyn Fn()>,
    /// Maximizes or restores (also on a double click of the bar).
    pub toggle_maximize: Box<dyn Fn()>,
    /// Whether the window is maximized (tracked: the button shows maximize or restore).
    pub is_maximized: Box<dyn Fn() -> bool>,
    /// Closes.
    pub close: Box<dyn Fn()>,
}

impl std::fmt::Debug for TitleBarActions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TitleBarActions")
    }
}

/// A title bar for a frameless window: drag it to move, double-click to maximize, and the
/// minimize, maximize/restore and close buttons. `extra` sits between the title and the
/// buttons (search, tabs, a menu). On Apple platforms the buttons go to the start edge.
#[track_caller]
pub fn title_bar(ui: &Ui, title: Signal<String>, extra: Option<El>, actions: TitleBarActions) -> El {
    let unit = ui.theme().spacing.unit;
    let a = Rc::new(actions);
    let t = w::text(ui, "").variant("strong").data_text().nowrap();
    let t2 = t.clone();
    let t = t.bind(move || {
        let _ = t2.clone().text(remedy_ui_view::RichText::plain(title.get()));
    });
    let (a1, a2, a3, a4) = (a.clone(), a.clone(), a.clone(), a.clone());
    let max = w::icon_button(ui, Icon::Maximize, ui.word("Maximize"), move || (a2.toggle_maximize)())
        .variant("quiet");
    let (m2, u, a5) = (max.clone(), ui.clone(), a.clone());
    let max = max.bind(move || {
        let on = (a5.is_maximized)();
        let label = if on { u.word("Restore") } else { u.word("Maximize") };
        untrack(|| {
            m2.clear();
            let _ = m2.clone().child(w::icon(
                &u,
                if on { Icon::Restore } else { Icon::Maximize },
                u.theme().type_scale.body + 3.0,
            ));
        });
        let _ = m2
            .clone()
            .semantics(Semantics::new(Role::Button).label(label).action(remedy_ui_a11y::Action::Click));
    });
    let buttons = w::row(ui)
        .gap(unit / 2.0)
        .child(w::icon_button(ui, Icon::Minus, ui.word("Minimize"), move || (a1.minimize)()).variant("quiet"))
        .child(max)
        .child(w::icon_button(ui, Icon::Close, ui.word("Close"), move || (a3.close)()).variant("quiet"));
    let apple = ui.platform() == remedy_ui_input::Platform::Apple;
    let mut bar = ui
        .el("title_bar")
        .row()
        .align(Align::Center)
        .gap(unit * 2.0)
        .padding(Insets::symmetric(unit, unit * 2.0))
        .fill_width()
        .style(StylePatch::default())
        .semantics(Semantics::new(Role::Group).label(ui.word("Title bar")));
    if apple {
        bar = bar.child(buttons.clone());
    }
    bar = bar.child(t.grow(1.0));
    if let Some(e) = extra {
        bar = bar.child(e);
    }
    if !apple {
        bar = bar.child(buttons);
    }
    let u2 = ui.clone();
    bar.drag_region(move || {
        // A double click maximizes instead of moving (the system's double-click time).
        if u2.click_count() >= 2 {
            (a4.toggle_maximize)();
        } else {
            (a.begin_move)();
        }
    })
    .layout(|l| l.position = Position::Relative)
}
