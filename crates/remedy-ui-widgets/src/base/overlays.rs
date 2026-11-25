//! Overlays: tooltips, popovers, modals, dialogs, trusted prompts and lock screens, and the
//! placement that keeps overlays (menus too) inside the window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::input::{Key, PointerPhase};
use remedy_ui_core::layout::{AUTO, Align, Justify, Length, Position};
use remedy_ui_core::{Insets, Point, Rect, Size};
use remedy_ui_engine::view::{El, Ui};

/// How long the pointer rests on an element before its tooltip shows.
const TOOLTIP_DELAY: Duration = Duration::from_millis(600);
/// The widest a tooltip grows before its text wraps.
const TOOLTIP_WIDTH: f32 = 360.0;
/// How long a trusted prompt's buttons stay disabled after it appears (against
/// click-jacking: a click aimed at what was there before cannot land on them).
const TRUSTED_DELAY: Duration = Duration::from_millis(700);

/// An overlay pinned at a window point by its top-left corner, sized by its content. Its
/// content reads in the window's direction.
pub fn place_at(el: El, p: Point) -> El {
    el.layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: p.y, start: p.x, end: AUTO, bottom: AUTO };
        l.inset_physical = true;
    })
}

/// Space kept between an overlay and the window's edges.
const EDGE: f32 = 4.0;

/// Where an overlay of `size` goes for `anchor` (the rectangle it belongs to) in a window of
/// `window`: below the anchor at its start, slid back from the end edge, flipped above when
/// it does not fit below, and never past the window's start or top (nor its bottom, for an
/// anchor that has scrolled out below it).
pub fn fit_near(anchor: Rect, size: Size, gap: f32, window: Size) -> Point {
    let x = anchor.min.x.min(window.width - size.width - EDGE).max(EDGE);
    let below = anchor.max.y + gap;
    let above = anchor.min.y - gap - size.height;
    let y = if below + size.height + EDGE <= window.height || above < EDGE {
        below.min(window.height - size.height - EDGE)
    } else {
        above.min(window.height - size.height - EDGE)
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
    let first = Point::new(anchor.min.x, anchor.max.y + gap);
    place_fitted(ui, el, first, max_width, false, move |size, window| fit_near(anchor, size, gap, window))
}

/// Where a submenu of `size` goes beside `anchor` (the item that opened it) in a window of
/// `window`: past the anchor's end edge (its right, or its left in a right-to-left layout)
/// with its top at the anchor's, flipped to the other side when it does not fit there, slid
/// up from the bottom edge, and never past the window's start or top.
pub fn fit_beside(anchor: Rect, size: Size, gap: f32, window: Size, rtl: bool) -> Point {
    let after = anchor.max.x + gap;
    let before = anchor.min.x - gap - size.width;
    let fits_after = after + size.width + EDGE <= window.width;
    let fits_before = before >= EDGE;
    let x = match (rtl, fits_after, fits_before) {
        (false, true, _) | (true, true, false) => after,
        (false, false, true) | (true, _, true) => before,
        // Room on neither side: as far from the start as it fits.
        _ if rtl => EDGE,
        _ => window.width - size.width - EDGE,
    };
    let x = x.min(window.width - size.width - EDGE).max(EDGE);
    let y = anchor.min.y.min(window.height - size.height - EDGE).max(EDGE);
    Point::new(x.max(0.0), y.max(0.0))
}

/// [`fit_near`] for an overlay that lines up with its anchor's end edge in a right-to-left
/// layout (`rtl`): a menu under a menu bar's title starts at the title's right edge there.
pub fn fit_near_dir(anchor: Rect, size: Size, gap: f32, window: Size, rtl: bool) -> Point {
    if !rtl {
        return fit_near(anchor, size, gap, window);
    }
    // Mirror the window, place as in a left-to-right one, and mirror back.
    let mirrored =
        Rect::new(window.width - anchor.max.x, anchor.min.y, window.width - anchor.min.x, anchor.max.y);
    let p = fit_near(mirrored, size, gap, window);
    Point::new((window.width - p.x - size.width).max(0.0), p.y)
}

/// Pins a submenu (or any overlay that belongs beside something) next to `anchor` and keeps it
/// in the window ([`fit_beside`]), measured before it shows, placed again when it or the
/// window changes size, and no taller than the window (make it scroll for long content).
pub fn place_beside(ui: &Ui, el: El, anchor: Rect, gap: f32) -> El {
    let rtl = ui.root().is_rtl();
    let first = Point::new(anchor.max.x + gap, anchor.min.y);
    place_fitted(ui, el, first, f32::INFINITY, true, move |size, window| {
        fit_beside(anchor, size, gap, window, rtl)
    })
}

/// Places `el` at the point `fit` gives for its measured size and the window's, hidden until
/// measured, placed again when it or the window changes size; no wider than `max_width` nor
/// the window, and with `fit_height`, no taller than the window.
pub(crate) fn place_fitted(
    ui: &Ui,
    el: El,
    first: Point,
    max_width: f32,
    fit_height: bool,
    fit: impl Fn(Size, Size) -> Point + 'static,
) -> El {
    // Measured (hidden) from the window's start edge, where it has the whole width: at its
    // anchor it would have only what is left of the window there, and text in it would wrap
    // to that. Placed, it always has at least the width it was measured at.
    // Measured no wider (or taller) than it may be placed: measured unwrapped, it would show
    // for a frame where that size fits instead of where it belongs.
    let window = remedy_ui_core::reactive::untrack(|| ui.window_size());
    let el = place_at(el, Point::new(0.0, first.y))
        .layout(move |l| {
            let widest =
                if window.width > 0.0 { max_width.min(window.width - EDGE * 2.0) } else { max_width };
            if widest.is_finite() {
                l.max_width = Length::Px(widest.max(0.0));
            }
            if fit_height && window.height > 0.0 {
                l.max_height = Length::Px((window.height - EDGE * 2.0).max(0.0));
            }
        })
        .style(remedy_ui_core::style::StylePatch::default().opacity(0.0));
    // Its own size once measured; placed again when it or the window changes size.
    let measured: Rc<Cell<Option<Size>>> = Rc::default();
    let place = {
        let (e2, measured) = (el.clone(), measured.clone());
        Rc::new(move |window: Size| {
            let Some(size) = measured.get() else { return };
            if window.width <= 0.0 || window.height <= 0.0 {
                return;
            }
            let p = fit(size, window);
            let widest = max_width.min(window.width - EDGE * 2.0).max(0.0);
            let tallest = (window.height - EDGE * 2.0).max(0.0);
            let _ = e2
                .clone()
                .layout(move |l| {
                    l.inset = Insets { top: p.y, start: p.x, end: AUTO, bottom: AUTO };
                    l.inset_physical = true;
                    l.max_width = Length::Px(widest);
                    if fit_height {
                        l.max_height = Length::Px(tallest);
                    }
                })
                .style(remedy_ui_core::style::StylePatch::default().opacity(1.0));
        })
    };
    let (ui2, f2, ui3, f3) = (ui.clone(), place.clone(), ui.clone(), place);
    el.on_resize(move |size| {
        measured.set(Some(size));
        f2(remedy_ui_core::reactive::untrack(|| ui2.window_size()));
    })
    .bind(move || {
        let window = ui3.window_size();
        remedy_ui_core::reactive::untrack(|| f3(window));
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

/// Shows `target`'s whole text in a tooltip: for text that [`El::max_lines`] or
/// [`El::truncate`] may cut short.
pub fn full_text_tooltip(target: &El) {
    let me = target.clone();
    tooltip(target, move || me.text_content().unwrap_or_default());
}

/// Shows `text` near `target` after the pointer rests on it, until it leaves. A signal or
/// closure gives the text as it is when the tip shows.
pub fn tooltip(target: &El, text: impl remedy_ui_engine::view::IntoProp<String>) {
    let text = text.into_prop();
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
                // Nothing to say (a label that is empty now): no empty bubble.
                let said = text.get();
                if said.is_empty() {
                    return;
                }
                // Long tips wrap: at most a readable width, never past the window.
                let said = remedy_ui_engine::view::RichText::plain(said);
                let tip = ui2.el("tooltip").text(said).pass_through();
                let tip = ui2.overlay(place_near_within(&ui2, tip, b, 4.0, TOOLTIP_WIDTH));
                *shown.borrow_mut() = Some(tip);
            });
            *pending.borrow_mut() = Some(id);
        }
    });
}

/// Tooltips for a widget that paints many things in one element (the notes of a score, the
/// bars of a chart): the widget says which part the pointer is over as it moves, and the tip
/// of a part shows beside that part once the pointer has rested on it.
#[derive(Clone)]
pub struct PartTooltip {
    ui: Ui,
    /// The part pointed at, with the timer waiting to show its tip or the tip shown.
    at: Rc<RefCell<Option<Rect>>>,
    pending: Rc<Cell<Option<u64>>>,
    shown: Rc<RefCell<Option<El>>>,
}

impl PartTooltip {
    /// Tooltips for the parts of `target`. They go when the pointer leaves it and when it
    /// is removed. (This takes the element's hover handler.)
    pub fn new(target: &El) -> PartTooltip {
        let me = PartTooltip {
            ui: target.ui().clone(),
            at: Rc::default(),
            pending: Rc::default(),
            shown: Rc::default(),
        };
        let gone = me.clone();
        let _ = target.clone().on_removed(move || gone.hide());
        let left = me.clone();
        let _ = target.clone().on_hover(move |inside| {
            if !inside {
                left.hide();
            }
        });
        me
    }

    /// The pointer is over the part at `rect` (window coordinates) whose tip says `text`, or
    /// over no part (`None`, or an empty text). Call it on every pointer move: pointing at
    /// the same part again changes nothing.
    pub fn point(&self, part: Option<(Rect, String)>) {
        let part = part.filter(|(_, text)| !text.is_empty());
        if part.as_ref().map(|(r, _)| *r) == *self.at.borrow() {
            return;
        }
        self.hide();
        let Some((rect, text)) = part else { return };
        *self.at.borrow_mut() = Some(rect);
        let me = self.clone();
        let id = self.ui.after(TOOLTIP_DELAY, move || {
            me.pending.set(None);
            let tip = me.ui.el("tooltip").text(remedy_ui_engine::view::RichText::plain(text)).pass_through();
            let tip = me.ui.overlay(place_near_within(&me.ui, tip, rect, 4.0, TOOLTIP_WIDTH));
            *me.shown.borrow_mut() = Some(tip);
        });
        self.pending.set(Some(id));
    }

    /// Takes the tip away, and forgets the part pointed at.
    pub fn hide(&self) {
        *self.at.borrow_mut() = None;
        if let Some(id) = self.pending.take() {
            self.ui.cancel_timer(id);
        }
        if let Some(tip) = self.shown.borrow_mut().take() {
            tip.remove();
        }
    }
}

/// A transparent full-window layer that closes `content` when pressed outside it, and on
/// Escape, which gives focus back to what had it when it opened.
pub(crate) fn dismissable(ui: &Ui, content: El) -> El {
    let restore = ui.tree().focused().map(|f| ui.element(f));
    let u = ui.clone();
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
                if let Some(r) = restore.as_ref().filter(|r| u.tree().contains(r.id())) {
                    r.focus();
                }
                return true;
            }
            false
        })
        // A press outside closes it; from the keyboard, Escape does (focus stays inside).
        .keyboard_alternative({
            let u = ui.clone();
            move || crate::base::tr(&u, "escape-closes")
        })
        .child(content);
    ui.overlay(layer)
}

/// Shows `content` below `anchor` until a press outside or Escape (which gives focus back).
/// It stays inside the window as [`fit_near`] places it, and scrolls when it is taller than
/// the window.
#[track_caller]
pub fn popover(ui: &Ui, anchor: &El, content: El) -> El {
    let b = anchor.bounds().unwrap_or(Rect::ZERO);
    let first = Point::new(b.min.x, b.max.y + 4.0);
    let card = ui.el("menu").column().scroll();
    let card = place_fitted(ui, card, first, f32::INFINITY, true, move |size, window| {
        fit_near(b, size, 4.0, window)
    })
    .child(content);
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
        // Closed already: focus stays where it has gone since.
        if !self.is_open() {
            return;
        }
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
    open_modal(ui, title.into(), content, escape_closes, true)
}

/// [`modal`] for content that fits itself to the room the window gives it: a head and a foot
/// that stay while the part between them scrolls, or panes that scroll on their own. The
/// dialog does not scroll; `content` is as tall as the dialog may be at most and shrinks with
/// the window, so give the part that scrolls a [`scroll`](crate::base::scroll) of its own.
#[track_caller]
pub fn modal_fitted(ui: &Ui, title: impl Into<String>, content: El, escape_closes: bool) -> Modal {
    open_modal(ui, title.into(), content, escape_closes, false)
}

#[track_caller]
fn open_modal(ui: &Ui, title: String, content: El, escape_closes: bool, scrolls: bool) -> Modal {
    let restore = ui.tree().focused().map(|f| ui.element(f));
    let body = ui.el("dialog").column().gap(ui.theme().spacing.unit * 3.0);
    // Taller than the window (a phone in landscape, large text): it scrolls, or its content
    // shrinks to fit when that lays itself out.
    let (body, content) = if scrolls {
        (body.scroll(), content)
    } else {
        (body.clip(), content.layout(|l| l.min_height = Length::Px(0.0)))
    };
    let body = body
        .layout(|l| {
            l.max_width = Length::Percent(0.92);
            l.min_width = Length::Px(260.0);
            l.max_height = Length::Percent(0.92);
        })
        .semantics(Semantics::new(Role::Dialog).label(title).modal(true))
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
    crate::base::feedback::raise_toasts(ui);
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
        .child(crate::base::heading(ui, 2, title.clone()))
        .child(crate::base::text(ui, message.into()));
    let slot: Rc<RefCell<Option<Modal>>> = Rc::default();
    let row = buttons.into_iter().fold(row, |r, DialogButton { label, variant, action }| {
        let s = slot.clone();
        r.child(
            crate::base::button(ui, label, move || {
                if let Some(m) = s.borrow().as_ref() {
                    m.close();
                }
                action();
            })
            .variant(variant),
        )
    });
    // Buttons that do not fit side by side (long labels, a narrow window) go onto more rows.
    let row = row.layout(|l| l.wrap = true);
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
    let (u1, u2) = (ui.clone(), ui.clone());
    let deny =
        crate::base::button(ui, move || crate::base::tr(&u1, "prompt-deny"), answer(false)).disabled(true);
    let allow = crate::base::button(ui, move || crate::base::tr(&u2, "prompt-allow"), answer(true))
        .variant("primary")
        .disabled(true);
    // A long message in a small window scrolls inside the prompt, as a modal's does: the
    // buttons are never left outside the window.
    let body = ui
        .el("dialog")
        .column()
        .gap(gap)
        .scroll()
        .layout(|l| {
            l.max_width = Length::Percent(0.92);
            l.max_height = Length::Percent(0.92);
        })
        .semantics(Semantics::new(Role::Dialog).label(title.clone()).modal(true))
        .child(crate::base::icon(ui, crate::base::Icon::Lock, ui.theme().type_scale.title))
        .child(crate::base::heading(ui, 2, title))
        .child(crate::base::text(ui, message.into()))
        .child(
            ui.el("dialog_actions")
                .row()
                .gap(gap)
                .justify(Justify::End)
                .layout(|l| l.wrap = true)
                .child(deny.clone())
                .child(allow.clone()),
        );
    let scrim = fill_window(ui.el("scrim"))
        .column()
        .align(Align::Center)
        .justify(Justify::Center)
        .focus_scope()
        .child(body.clone());
    // The delay starts when the prompt is first seen (after the frame that shows it), not
    // when it is built: built outside an event, in a minimized or covered window, or long
    // before the next frame, it would otherwise be shortened or over before anyone saw it.
    // Hidden again (the window minimized or covered), it disarms, and starts over when seen.
    let pending: Rc<Cell<Option<u64>>> = Rc::default();
    let u = ui.clone();
    let _ = body.on_visibility(move |seen| {
        if let Some(id) = pending.take() {
            u.cancel_timer(id);
        }
        let _ = deny.clone().disabled(true);
        let _ = allow.clone().disabled(true);
        if !seen {
            return;
        }
        let (deny, allow, p) = (deny.clone(), allow.clone(), pending.clone());
        pending.set(Some(u.after(TRUSTED_DELAY, move || {
            p.set(None);
            let _ = deny.clone().disabled(false);
            let _ = allow.clone().disabled(false);
            deny.focus();
        })));
    });
    let scrim = ui.trusted_overlay(scrim);
    *layer.borrow_mut() = Some(scrim.clone());
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
        // An anchor scrolled out below the window: the overlay stays inside all the same.
        let p = fit_near(Rect::new(100.0, 700.0, 140.0, 730.0), tip, 4.0, WIN);
        assert!(p.y + tip.height <= WIN.height, "{p:?}");
        // Near the start edge: not pushed past it.
        assert_eq!(fit_near(Rect::new(0.0, 0.0, 20.0, 20.0), tip, 4.0, WIN).x, 4.0);
    }
}
