//! Overlays: tooltips, menus, popovers, modals, dialogs, trusted prompts and lock screens.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Insets, Point, Rect};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_layout::{AUTO, Align, Justify, Length, Position};
use remedy_ui_view::{El, Ui};

/// How long the pointer rests on an element before its tooltip shows.
const TOOLTIP_DELAY: Duration = Duration::from_millis(600);
/// The widest a tooltip grows before its text wraps.
const TOOLTIP_WIDTH: f32 = 360.0;
/// How long a trusted prompt's buttons stay disabled after it appears (against
/// click-jacking: a click aimed at what was there before cannot land on them).
const TRUSTED_DELAY: Duration = Duration::from_millis(700);

/// An overlay pinned at a window point by its top-left corner, sized by its content.
pub fn place_at(el: El, p: Point) -> El {
    el.layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: p.y, start: p.x, end: AUTO, bottom: AUTO };
    })
}

/// Space kept between an overlay and the window's edges.
const EDGE: f32 = 4.0;

/// Where an overlay of `size` goes for `anchor` (the rectangle it belongs to) in a window of
/// `window`: below the anchor at its start, slid back from the end edge, flipped above when
/// it does not fit below, and never past the window's start or top.
pub fn fit_near(anchor: Rect, size: remedy_ui_core::Size, gap: f32, window: remedy_ui_core::Size) -> Point {
    let x = anchor.min.x.min(window.width - size.width - EDGE).max(EDGE);
    let below = anchor.max.y + gap;
    let above = anchor.min.y - gap - size.height;
    let y = if below + size.height + EDGE <= window.height || above < EDGE {
        below.min(window.height - size.height - EDGE)
    } else {
        above
    };
    Point::new(x.max(0.0), y.max(0.0))
}

/// Pins an overlay near `anchor` and keeps it in the window ([`fit_near`]). It is measured
/// before it shows, so it never flashes where it does not fit, and placed again if its size
/// changes.
pub fn place_near(ui: &Ui, el: El, anchor: Rect, gap: f32) -> El {
    place_near_within(ui, el, anchor, gap, f32::INFINITY)
}

/// [`place_near`], no wider than `max_width` (and never wider than the window): text in it
/// wraps rather than running past the edge.
pub fn place_near_within(ui: &Ui, el: El, anchor: Rect, gap: f32, max_width: f32) -> El {
    let el = place_at(el, Point::new(anchor.min.x, anchor.max.y + gap))
        .style(remedy_ui_style::StylePatch { opacity: Some(0.0), ..Default::default() });
    // Its own size once measured; placed again when it or the window changes size.
    let measured: Rc<std::cell::Cell<Option<remedy_ui_core::Size>>> = Rc::default();
    let fit = {
        let (e2, measured) = (el.clone(), measured.clone());
        Rc::new(move |window: remedy_ui_core::Size| {
            let Some(size) = measured.get() else { return };
            if window.width <= 0.0 || window.height <= 0.0 {
                return;
            }
            let p = fit_near(anchor, size, gap, window);
            let widest = max_width.min(window.width - EDGE * 2.0).max(0.0);
            let _ = e2
                .clone()
                .layout(move |l| {
                    l.inset = Insets { top: p.y, start: p.x, end: AUTO, bottom: AUTO };
                    l.max_width = Length::Px(widest);
                })
                .style(remedy_ui_style::StylePatch { opacity: Some(1.0), ..Default::default() });
        })
    };
    let (ui2, f2, ui3, f3) = (ui.clone(), fit.clone(), ui.clone(), fit);
    el.on_resize(move |size| {
        measured.set(Some(size));
        f2(remedy_ui_reactive::untrack(|| ui2.window_size()));
    })
    .bind(move || {
        let window = ui3.window_size();
        remedy_ui_reactive::untrack(|| f3(window));
    })
}

/// A full-window layer: absolute, filling the window (scrims, drawers, sheets).
pub fn fill_window(el: El) -> El {
    el.layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets::ZERO;
        l.width = Length::Percent(1.0);
        l.height = Length::Percent(1.0);
    })
}

/// Shows `text` near `target` after the pointer rests on it, until it leaves.
pub fn tooltip(target: &El, text: impl Into<String>) {
    let text = text.into();
    let ui = target.ui().clone();
    let shown: Rc<RefCell<Option<El>>> = Rc::default();
    let pending: Rc<RefCell<Option<u64>>> = Rc::default();
    // The tooltip goes with its target (a window rebuilt for another language, a closed panel).
    let (shown2, pending2, ui0) = (shown.clone(), pending.clone(), ui.clone());
    let _ = target.clone().on_removed(move || {
        if let Some(id) = pending2.borrow_mut().take() {
            ui0.cancel_timer(id);
        }
        if let Some(tip) = shown2.borrow_mut().take() {
            tip.remove();
        }
    });
    let t = target.clone();
    let _ = target.clone().on_hover(move |inside| {
        if let Some(id) = pending.borrow_mut().take() {
            ui.cancel_timer(id);
        }
        if let Some(tip) = shown.borrow_mut().take() {
            tip.remove();
        }
        if inside {
            let (ui2, t2, text, shown) = (ui.clone(), t.clone(), text.clone(), shown.clone());
            let id = ui.after(TOOLTIP_DELAY, move || {
                let Some(b) = t2.bounds() else { return };
                // Long tips wrap: at most a readable width, never past the window.
                let tip = ui2.el("tooltip").text(remedy_ui_view::RichText::plain(text)).pass_through();
                let tip = ui2.overlay(place_near_within(&ui2, tip, b, 4.0, TOOLTIP_WIDTH));
                *shown.borrow_mut() = Some(tip);
            });
            *pending.borrow_mut() = Some(id);
        }
    });
}

/// One menu entry.
#[derive(Clone)]
pub struct MenuItem {
    label: String,
    danger: bool,
    action: Rc<dyn Fn()>,
}

impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MenuItem({})", self.label)
    }
}

impl MenuItem {
    /// An entry that runs `action`.
    pub fn new(label: impl Into<String>, action: impl Fn() + 'static) -> Self {
        MenuItem { label: label.into(), danger: false, action: Rc::new(action) }
    }

    /// Marks a destructive entry (shown in the danger color).
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// A transparent full-window layer that closes `content` when pressed outside it, and on
/// Escape.
fn dismissable(ui: &Ui, content: El) -> El {
    let layer = fill_window(ui.el("dismiss_layer")).focus_scope();
    let l2 = layer.clone();
    let l3 = layer.clone();
    let inside = content.clone();
    let layer = layer
        .on_pointer(move |e| {
            // Presses inside the content (its buttons, its fields) are the content's own.
            let within = inside.bounds().is_some_and(|b| b.contains(e.position));
            if e.phase == PointerPhase::Down && !within {
                l2.remove();
                return true;
            }
            false
        })
        .on_key(move |e| {
            if e.pressed && e.key == Key::Escape {
                l3.remove();
                return true;
            }
            false
        })
        // A press outside closes it; from the keyboard, Escape does (focus stays inside).
        .keyboard_alternative("Escape closes it")
        .child(content);
    ui.overlay(layer)
}

fn menu_at(ui: &Ui, anchor: Rect, items: Vec<MenuItem>) -> El {
    let menu = place_near(ui, ui.el("menu").column(), anchor, 4.0).semantics(Semantics::new(Role::Menu));
    let n = items.len();
    let layer: Rc<RefCell<Option<El>>> = Rc::default();
    let menu = items.into_iter().enumerate().fold(menu, |m, (i, item)| {
        let (action, layer) = (item.action.clone(), layer.clone());
        let mut entry = ui
            .el("menu_item")
            .row()
            .child(ui.el("menu_label").text(remedy_ui_view::RichText::plain(item.label.clone())).nowrap())
            .on_click(move || {
                if let Some(l) = layer.borrow_mut().take() {
                    l.remove();
                }
                action();
            })
            .semantics(
                Semantics::new(Role::MenuItem)
                    .label(item.label.clone())
                    .set_position(i + 1, n)
                    .action(Action::Click),
            );
        if item.danger {
            entry = entry.variant("danger");
        }
        m.child(entry)
    });
    // Up and down move between entries.
    let m2 = menu.clone();
    let menu = menu.on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let entries = m2.child_elements();
        let cur = entries.iter().position(El::is_focused);
        let next = match e.key {
            Key::ArrowDown => cur.map_or(0, |c| (c + 1) % entries.len().max(1)),
            Key::ArrowUp => cur.map_or(0, |c| (c + entries.len().max(1) - 1) % entries.len().max(1)),
            _ => return false,
        };
        if let Some(el) = entries.get(next) {
            el.focus();
        }
        true
    });
    let l = dismissable(ui, menu.clone());
    *layer.borrow_mut() = Some(l.clone());
    menu.focus_first();
    l
}

/// Opens a menu below `anchor`. Returns the menu's layer; it closes itself when an entry is
/// chosen, on a press outside, and on Escape.
#[track_caller]
pub fn menu(ui: &Ui, anchor: &El, items: Vec<MenuItem>) -> El {
    let b = anchor.bounds().unwrap_or(Rect::ZERO);
    menu_at(ui, b, items)
}

/// Opens a menu at the pointer on a secondary press (right click) on `target`, or with the
/// Menu key or Shift+F10 while it has focus.
#[track_caller]
pub fn context_menu(target: &El, items: impl Fn() -> Vec<MenuItem> + 'static) -> El {
    context_menu_at(target, move |_| items())
}

/// Like [`context_menu`], for menus that depend on what is under the pointer: `items` gets the
/// window point of the press, or `None` when the menu was opened from the keyboard.
#[track_caller]
pub fn context_menu_at(target: &El, items: impl Fn(Option<Point>) -> Vec<MenuItem> + 'static) -> El {
    let ui = target.ui().clone();
    let items = Rc::new(items);
    let (ui2, items2, t2) = (ui.clone(), items.clone(), target.clone());
    target
        .clone()
        .on_pointer(move |e| {
            if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::SECONDARY) {
                let _ = menu_at(&ui, Rect::from_points(e.position, e.position), items(Some(e.position)));
                return true;
            }
            false
        })
        .on_key(move |e| {
            let menu_key = matches!(&e.key, Key::Named(n) if n == "ContextMenu")
                || (e.key == Key::F(10) && e.modifiers.shift);
            if e.pressed && menu_key {
                let b = t2.bounds().unwrap_or(Rect::ZERO);
                let _ = menu_at(&ui2, Rect::new(b.min.x, b.max.y, b.min.x, b.max.y), items2(None));
                return true;
            }
            false
        })
        // The same menu opens from the keyboard wherever focus is inside the target.
        .keyboard_alternative("Shift+F10 or the Menu key opens the same menu")
}

/// Shows `content` below `anchor` until a press outside or Escape.
#[track_caller]
pub fn popover(ui: &Ui, anchor: &El, content: El) -> El {
    let b = anchor.bounds().unwrap_or(Rect::ZERO);
    let card = place_near(ui, ui.el("menu").column(), b, 4.0).child(content);
    let l = dismissable(ui, card.clone());
    card.focus_first();
    l
}

/// An open modal: dim the window, take focus, keep Tab inside, close on Escape (when
/// allowed). Close it with [`Modal::close`].
#[derive(Clone, Debug)]
pub struct Modal {
    layer: El,
    body: El,
    restore: Option<El>,
}

impl Modal {
    /// The modal's content column.
    pub fn body(&self) -> El {
        self.body.clone()
    }

    /// Closes the modal and gives focus back to what had it.
    pub fn close(&self) {
        self.layer.remove();
        if let Some(r) = &self.restore {
            r.focus();
        }
    }

    /// True while open.
    pub fn is_open(&self) -> bool {
        self.layer.ui().tree().contains(self.layer.id())
    }
}

/// Opens `content` as a modal over the window. `escape_closes` lets Escape close it.
#[track_caller]
pub fn modal(ui: &Ui, title: impl Into<String>, content: El, escape_closes: bool) -> Modal {
    let restore = ui.tree().focused().map(|f| ui.element(f));
    let body = ui
        .el("dialog")
        .column()
        .gap(ui.theme().spacing.unit * 3.0)
        .layout(|l| {
            l.max_width = Length::Percent(0.92);
            l.min_width = Length::Px(260.0);
        })
        .semantics(Semantics::new(Role::Dialog).label(title.into()))
        .child(content);
    let layer = fill_window(ui.el("scrim"))
        .column()
        .align(Align::Center)
        .justify(Justify::Center)
        .focus_scope()
        .child(body.clone());
    let m = Modal { layer: layer.clone(), body: body.clone(), restore };
    if escape_closes {
        let m2 = m.clone();
        let _ = layer.clone().on_key(move |e| {
            if e.pressed && e.key == Key::Escape {
                m2.close();
                return true;
            }
            false
        });
    }
    ui.overlay(layer);
    body.focus_first();
    m
}

/// A dialog button: its label, its style variant (`primary`, `quiet`, `danger_solid`, …)
/// and what it does.
#[derive(Clone)]
pub struct DialogButton {
    label: String,
    variant: &'static str,
    action: Rc<dyn Fn()>,
}

impl std::fmt::Debug for DialogButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DialogButton({}, {})", self.label, self.variant)
    }
}

impl DialogButton {
    /// A button labeled `label` in style `variant` that runs `action`.
    pub fn new(label: impl Into<String>, variant: &'static str, action: impl Fn() + 'static) -> Self {
        DialogButton { label: label.into(), variant, action: Rc::new(action) }
    }
}

/// A modal dialog: a title, a message, and buttons in a row (the first is the default).
/// Each button closes the dialog and then runs its action.
#[track_caller]
pub fn dialog(
    ui: &Ui,
    title: impl Into<String>,
    message: impl Into<String>,
    buttons: Vec<DialogButton>,
) -> Modal {
    let title = title.into();
    let gap = ui.theme().spacing.unit * 2.0;
    let row = ui.el("dialog_actions").row().gap(gap).justify(Justify::End);
    let content = ui
        .el("dialog_content")
        .column()
        .gap(gap)
        .child(crate::heading(ui, 2, title.clone()))
        .child(crate::text(ui, message.into()));
    let slot: Rc<RefCell<Option<Modal>>> = Rc::default();
    let row = buttons.into_iter().fold(row, |r, DialogButton { label, variant, action }| {
        let s = slot.clone();
        r.child(
            crate::button(ui, label, move || {
                if let Some(m) = s.borrow().as_ref() {
                    m.close();
                }
                action();
            })
            .variant(variant),
        )
    });
    let m = modal(ui, title, content.child(row), true);
    *slot.borrow_mut() = Some(m.clone());
    m
}

/// A prompt asking the user to allow or deny something (a permission, an agent action). It
/// shows above anything the app can draw, keeps focus, and its buttons stay disabled for a
/// moment after it appears so a press aimed at what was there before cannot land on them.
#[track_caller]
pub fn trusted_prompt(
    ui: &Ui,
    title: impl Into<String>,
    message: impl Into<String>,
    on_answer: impl Fn(bool) + 'static,
) -> El {
    let title = title.into();
    let on_answer = Rc::new(on_answer);
    let gap = ui.theme().spacing.unit * 2.0;
    let layer: Rc<RefCell<Option<El>>> = Rc::default();
    let answer = |yes: bool| {
        let (a, l) = (on_answer.clone(), layer.clone());
        move || {
            if let Some(el) = l.borrow_mut().take() {
                el.remove();
            }
            a(yes);
        }
    };
    let deny = crate::button(ui, "Deny", answer(false)).disabled(true);
    let allow = crate::button(ui, "Allow", answer(true)).variant("primary").disabled(true);
    let body = ui
        .el("dialog")
        .column()
        .gap(gap)
        .semantics(Semantics::new(Role::Dialog).label(title.clone()))
        .child(crate::icon(ui, crate::Icon::Lock, ui.theme().type_scale.title))
        .child(crate::heading(ui, 2, title))
        .child(crate::text(ui, message.into()))
        .child(
            ui.el("dialog_actions")
                .row()
                .gap(gap)
                .justify(Justify::End)
                .child(deny.clone())
                .child(allow.clone()),
        );
    let scrim = fill_window(ui.el("scrim"))
        .column()
        .align(Align::Center)
        .justify(Justify::Center)
        .focus_scope()
        .child(body.clone());
    let scrim = ui.trusted_overlay(scrim);
    *layer.borrow_mut() = Some(scrim.clone());
    ui.after(TRUSTED_DELAY, move || {
        let _ = deny.clone().disabled(false);
        let _ = allow.clone().disabled(false);
        deny.focus();
    });
    scrim
}

/// Covers the window with `content` (an unlock form): takes all input, hides everything
/// below it from the frame and from assistive technology. Remove the returned layer to
/// unlock.
#[track_caller]
pub fn lock_screen(ui: &Ui, content: El) -> El {
    let root = ui.root();
    let _ = root.clone().hidden(true);
    let layer = fill_window(ui.el("lock_screen"))
        .column()
        .align(Align::Center)
        .justify(Justify::Center)
        .focus_scope()
        .child(content);
    let layer = ui.overlay(layer);
    // When the lock goes away, the content comes back.
    let _ = layer.clone().on_removed(move || {
        let _ = root.clone().hidden(false);
    });
    layer.focus_first();
    layer
}

#[cfg(test)]
mod tests {
    use remedy_ui_core::{Point, Rect, Size};

    use super::fit_near;

    const WIN: Size = Size { width: 800.0, height: 600.0 };

    #[test]
    fn overlays_stay_inside_the_window() {
        let tip = Size::new(120.0, 30.0);
        // Room everywhere: below the anchor, at its start.
        assert_eq!(fit_near(Rect::new(100.0, 100.0, 140.0, 130.0), tip, 4.0, WIN), Point::new(100.0, 134.0));
        // A gear in the top-right corner: slid back so the end edge stays inside.
        let p = fit_near(Rect::new(770.0, 4.0, 796.0, 30.0), tip, 4.0, WIN);
        assert_eq!(p, Point::new(800.0 - 120.0 - 4.0, 34.0));
        // At the bottom: flipped above.
        assert_eq!(fit_near(Rect::new(100.0, 570.0, 140.0, 596.0), tip, 4.0, WIN), Point::new(100.0, 536.0));
        // Too tall for either side: kept inside from the bottom.
        let tall = Size::new(100.0, 590.0);
        let p = fit_near(Rect::new(10.0, 300.0, 30.0, 320.0), tall, 4.0, WIN);
        assert!(p.y + tall.height <= WIN.height && p.y >= 0.0, "{p:?}");
        // Near the start edge: not pushed past it.
        assert_eq!(fit_near(Rect::new(0.0, 0.0, 20.0, 20.0), tip, 4.0, WIN).x, 4.0);
    }
}
