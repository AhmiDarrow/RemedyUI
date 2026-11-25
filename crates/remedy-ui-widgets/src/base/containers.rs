//! Containers.

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_core::layout::{Axis, Display, Length, Position};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, IntoProp, Ui};

/// Children in a column.
#[track_caller]
pub fn column(ui: &Ui) -> El {
    ui.el("column").column()
}

/// Children in a row.
#[track_caller]
pub fn row(ui: &Ui) -> El {
    ui.el("row").row()
}

/// Children in rows that wrap onto the next line.
#[track_caller]
pub fn wrap(ui: &Ui) -> El {
    ui.el("wrap").row().layout(|l| l.wrap = true)
}

/// Children on top of each other, each filling the stack (later ones above).
#[track_caller]
pub fn stack(ui: &Ui, children: impl IntoIterator<Item = El>) -> El {
    let s = ui.el("stack").layout(|l| l.position = Position::Relative);
    children.into_iter().fold(s, |s, c| {
        s.child(c.layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets::ZERO;
        }))
    })
}

/// Takes the free space in a row or column.
#[track_caller]
pub fn spacer(ui: &Ui) -> El {
    ui.el("spacer").grow(1.0)
}

/// A thin line between groups: horizontal in a column, vertical in a row (`vertical`).
#[track_caller]
pub fn divider(ui: &Ui, vertical: bool) -> El {
    let stroke = ui.theme().stroke;
    ui.el("divider").layout(move |l| {
        if vertical {
            l.width = Length::Px(stroke);
            l.align_self = Some(remedy_ui_core::layout::Align::Stretch);
        } else {
            l.height = Length::Px(stroke);
            l.width = Length::Percent(1.0);
        }
        l.shrink = 0.0;
    })
}

/// A raised surface holding related content.
#[track_caller]
pub fn card(ui: &Ui) -> El {
    ui.el("card").column()
}

/// A titled group: the heading names the group for assistive technology.
#[track_caller]
pub fn section(ui: &Ui, title: &str) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    ui.el("section")
        .column()
        .gap(gap)
        .semantics(Semantics::new(Role::Group).label(title))
        .child(crate::base::heading(ui, 3, title))
}

/// A part that opens and closes under its title (a page's `<details>`, an expander): the
/// head shows a marker and `title` and flips `open` when pressed; `body` shows only while
/// open. Style the head as `disclosure_head` (hover, focus) and the part as `disclosure`.
#[track_caller]
pub fn disclosure(ui: &Ui, title: impl IntoProp<String>, body: El, open: Signal<bool>) -> El {
    let title = title.into_prop();
    let unit = ui.theme().spacing.unit;
    let size = ui.theme().type_scale.small + 2.0;
    let (closed_mark, open_mark) =
        (crate::base::icon(ui, crate::base::Icon::ChevronRight, size), crate::base::icon(ui, crate::base::Icon::ChevronDown, size));
    let head = crate::base::min_target(ui.el("disclosure_head"))
        .row()
        .align(remedy_ui_core::layout::Align::Center)
        .gap(unit * 1.5)
        .child(closed_mark.clone())
        .child(open_mark.clone())
        .child(ui.el("label").text(title.clone().map(remedy_ui_engine::view::RichText::plain)))
        .on_click(move || open.update(|o| *o = !*o))
        .semantics(move || {
            Semantics::new(Role::Button)
                .label(title.get())
                .expanded(open.get())
                .action(remedy_ui_core::a11y::Action::Click)
        });
    let b2 = body.clone();
    let body = body.bind(move || {
        let on = open.get();
        let _ = closed_mark.clone().hidden(on);
        let _ = open_mark.clone().hidden(!on);
        let _ = b2.clone().hidden(!on);
    });
    ui.el("disclosure").column().gap(unit * 2.0).child(head).child(body)
}

/// A vertically scrolling column; scrolling runs on the compositor.
#[track_caller]
pub fn scroll(ui: &Ui) -> El {
    ui.el("scroll").scroll().column().layout(|l| {
        l.grow = 1.0;
        l.min_height = Length::Px(0.0);
    })
}

/// Two panes with a draggable divider. `fraction` (0..1) is the first pane's share of the
/// split's width (`Axis::Row`) or height (`Axis::Column`); dragging and arrow keys move it.
#[track_caller]
pub fn split(ui: &Ui, axis: Axis, first: El, second: El, fraction: Signal<f32>) -> El {
    let horizontal = axis == Axis::Row;
    let container = ui.el("split").layout(move |l| {
        l.display = Display::Flex;
        l.axis = axis;
        l.grow = 1.0;
    });
    // The divider decides the panes' sizes, not their content: each pane may be narrower
    // (or shorter) than what is in it, which then scrolls or clips inside it.
    let no_min = move |l: &mut remedy_ui_core::layout::LayoutStyle| {
        if horizontal {
            l.min_width = Length::Px(0.0);
        } else {
            l.min_height = Length::Px(0.0);
        }
    };
    let a = first.clone().layout(move |l| {
        l.shrink = 1.0;
        no_min(l);
    });
    let b = second.layout(move |l| {
        l.grow = 1.0;
        l.basis = Length::Px(0.0);
        no_min(l);
    });
    let a_bind = a.clone();
    let a = a.bind(move || {
        let f = fraction.get().clamp(0.05, 0.95);
        let _ = a_bind.clone().layout(|l| {
            l.basis = Length::Percent(f);
            l.grow = 0.0;
        });
    });
    let c2 = container.clone();
    let key_el = container.clone();
    let u = ui.clone();
    let handle = ui
        .el("split_handle")
        .layout(move |l| {
            if horizontal {
                l.width = Length::Px(6.0);
            } else {
                l.height = Length::Px(6.0);
            }
            l.shrink = 0.0;
        })
        .focusable(true)
        .semantics(move || {
            Semantics::new(Role::Slider).label(crate::base::tr(&u, "resize-panes")).numeric(
                f64::from(fraction.get()),
                0.0,
                1.0,
            )
        })
        .on_pointer(move |e| {
            if e.buttons.contains(remedy_ui_core::input::Buttons::PRIMARY)
                && e.phase == remedy_ui_core::input::PointerPhase::Move
            {
                if let Some(b) = c2.bounds() {
                    // The first pane is at the start: the right, right to left.
                    let f = if horizontal && c2.is_rtl() {
                        (b.max.x - e.position.x) / b.width()
                    } else if horizontal {
                        (e.position.x - b.min.x) / b.width()
                    } else {
                        (e.position.y - b.min.y) / b.height()
                    };
                    fraction.set(f.clamp(0.05, 0.95));
                }
                return true;
            }
            false
        })
        .on_key(move |e| {
            // The arrows move the divider the way they point.
            let flip = if horizontal && key_el.is_rtl() { -1.0 } else { 1.0 };
            let step = match e.key {
                remedy_ui_core::input::Key::ArrowUp => -0.02,
                remedy_ui_core::input::Key::ArrowDown => 0.02,
                remedy_ui_core::input::Key::ArrowLeft => -0.02 * flip,
                remedy_ui_core::input::Key::ArrowRight => 0.02 * flip,
                _ => return false,
            };
            fraction.update(|f| *f = (*f + step).clamp(0.05, 0.95));
            true
        });
    container.child(a).child(handle).child(b)
}
