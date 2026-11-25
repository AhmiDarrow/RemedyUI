//! A pane docked on the start or end of a row, with a resizable divider and a
//! collapse strip.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::Insets;
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::layout::{Align, Justify, Length};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};

/// Which side of the row the docked pane sits on, in the container's start/end
/// direction (start is left in left-to-right text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DockSide {
    /// The start edge (left in left-to-right text).
    #[default]
    Start,
    /// The end edge (right in left-to-right text).
    End,
}

impl DockSide {
    /// The other edge.
    pub fn flip(self) -> Self {
        match self {
            DockSide::Start => DockSide::End,
            DockSide::End => DockSide::Start,
        }
    }
}

/// Width, side, and whether the docked pane is collapsed to a slim strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockState {
    /// Which edge the pane is on.
    pub side: DockSide,
    /// Pixel width when expanded; clamped to the [`DockSpec`] min and max.
    pub width: f32,
    /// When true, the pane is a slim strip with a chevron.
    pub collapsed: bool,
}

impl DockState {
    /// An expanded pane on the start edge, `width` pixels wide.
    pub fn new(width: f32) -> Self {
        Self { side: DockSide::Start, width, collapsed: false }
    }

    /// Toggles the collapsed flag.
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }
}

/// Limits and the accessible name for a [`dock`].
#[derive(Clone, Debug)]
pub struct DockSpec {
    /// Narrowest expanded width, in pixels.
    pub min_width: f32,
    /// Widest expanded width, in pixels.
    pub max_width: f32,
    /// Width of the chevron strip when collapsed, in pixels.
    pub strip_width: f32,
    /// Accessible name used for the chevron, strip, and resize handle.
    pub label: String,
}

impl Default for DockSpec {
    fn default() -> Self {
        Self { min_width: 200.0, max_width: 560.0, strip_width: 36.0, label: String::new() }
    }
}

impl DockSpec {
    /// A dock named `label` with the default widths.
    pub fn named(label: impl Into<String>) -> Self {
        Self { label: label.into(), ..Self::default() }
    }

    fn clamp_width(&self, width: f32) -> f32 {
        width.clamp(self.min_width, self.max_width)
    }
}

/// A docked pane and a main pane, with a draggable divider, a collapse
/// chevron, and drag-to-the-other-edge to redock.
///
/// `state` is the caller's to persist. Dragging the divider past the midpoint
/// of the row docks the pane on the other side and keeps its width. The
/// chevron (and Enter/Space on the strip) collapses the pane to a slim strip;
/// the strip's tooltip names the pane.
#[track_caller]
pub fn dock(ui: &Ui, state: Signal<DockState>, spec: DockSpec, docked: El, main: El) -> El {
    let label = if spec.label.is_empty() { crate::base::tr(ui, "resize-panes") } else { spec.label.clone() };
    let spec = DockSpec {
        min_width: spec.min_width.max(1.0),
        max_width: spec.max_width.max(spec.min_width),
        strip_width: spec.strip_width.max(16.0),
        label: label.clone(),
    };

    let st_clamp = state;
    let spec_clamp = spec.clone();
    st_clamp.update(|s| s.width = spec_clamp.clamp_width(s.width));

    let row = ui.el("dock").row().layout(|l| {
        l.width = Length::Percent(1.0);
        l.height = Length::Percent(1.0);
        l.grow = 1.0;
        l.shrink = 1.0;
        l.min_width = Length::Px(0.0);
        l.min_height = Length::Px(0.0);
    });

    let st_size = state;
    let spec_size = spec.clone();
    let row = row.on_resize(move |size| {
        let cur = st_size.get();
        if cur.collapsed {
            return;
        }
        let max = (size.width - 80.0).max(spec_size.min_width);
        let width = cur.width.clamp(spec_size.min_width, spec_size.max_width.min(max));
        if (width - cur.width).abs() > 0.5 {
            st_size.set(DockState { width, ..cur });
        }
    });

    let st_side = state;
    let row_side = row.clone();
    let row = row.bind(move || {
        let reverse = st_side.get().side == DockSide::End;
        let _ = row_side.clone().layout(move |l| l.reverse = reverse);
    });

    let pane = dock_pane(ui, state, &spec, &label, docked);
    let handle = resize_handle(ui, state, &spec, &label, row.clone());
    let handle_b = handle.clone();
    let st_h = state;
    let handle = handle.bind(move || {
        let _ = handle_b.clone().hidden(st_h.get().collapsed);
    });

    let main = main
        .layout(|l| {
            l.grow = 1.0;
            l.shrink = 1.0;
            l.basis = Length::Px(0.0);
            l.min_width = Length::Px(0.0);
            l.min_height = Length::Px(0.0);
            l.height = Length::Percent(1.0);
        })
        .clip();

    row.child(pane).child(handle).child(main).clip()
}

fn dock_pane(ui: &Ui, state: Signal<DockState>, spec: &DockSpec, label: &str, body: El) -> El {
    let expanded = dock_chrome(ui, state, label);
    let collapsed_bar = strip_chrome(ui, state, label);
    let st_exp = state;
    let exp_b = expanded.clone();
    let expanded = expanded.bind(move || {
        let _ = exp_b.clone().hidden(st_exp.get().collapsed);
    });
    let st_col = state;
    let col_b = collapsed_bar.clone();
    let collapsed_bar = collapsed_bar.bind(move || {
        let _ = col_b.clone().hidden(!st_col.get().collapsed);
    });

    let body = body.layout(|l| {
        l.grow = 1.0;
        l.shrink = 1.0;
        l.basis = Length::Px(0.0);
        l.min_height = Length::Px(0.0);
        l.width = Length::Percent(1.0);
    });
    let body_b = body.clone();
    let st_body = state;
    let body = body.bind(move || {
        let _ = body_b.clone().hidden(st_body.get().collapsed);
    });

    let width = spec.clamp_width(state.get().width);
    let col = crate::base::column(ui)
        .layout(move |l| {
            l.width = Length::Px(width);
            l.min_width = Length::Px(spec.min_width);
            l.max_width = Length::Px(spec.max_width);
            l.height = Length::Percent(1.0);
            l.shrink = 0.0;
            l.grow = 0.0;
        })
        .clip()
        .child(expanded)
        .child(collapsed_bar)
        .child(body);

    let st = state;
    let spec_w = spec.clone();
    let col_b = col.clone();
    let col = col.bind(move || {
        let cur = st.get();
        let w = if cur.collapsed { spec_w.strip_width } else { spec_w.clamp_width(cur.width) };
        let min = if cur.collapsed { spec_w.strip_width } else { spec_w.min_width };
        let max = if cur.collapsed { spec_w.strip_width } else { spec_w.max_width };
        let _ = col_b.clone().layout(move |l| {
            l.width = Length::Px(w);
            l.min_width = Length::Px(min);
            l.max_width = Length::Px(max);
        });
    });
    let st_s = state;
    let label_s = label.to_string();
    col.semantics(move || {
        let name = if st_s.get().collapsed { format!("{label_s} (collapsed)") } else { label_s.clone() };
        Semantics::new(Role::Region).label(name)
    })
}

fn strip_chrome(ui: &Ui, state: Signal<DockState>, label: &str) -> El {
    let chevron = toggle_btn(ui, state, label);
    let grip = header_grip(ui, state, label).layout(|l| {
        l.width = Length::Percent(1.0);
        l.grow = 1.0;
        l.basis = Length::Px(0.0);
    });
    crate::base::column(ui)
        .align(Align::Center)
        .layout(|l| {
            l.width = Length::Percent(1.0);
            l.height = Length::Percent(1.0);
            l.grow = 1.0;
            l.shrink = 1.0;
            l.min_width = Length::Px(0.0);
        })
        .child(chevron)
        .child(grip)
}

fn dock_chrome(ui: &Ui, state: Signal<DockState>, label: &str) -> El {
    let chevron = toggle_btn(ui, state, label);
    let grip = header_grip(ui, state, label);
    crate::base::row(ui)
        .align(Align::Center)
        .justify(Justify::SpaceBetween)
        .padding(Insets { top: 4.0, end: 4.0, bottom: 4.0, start: 4.0 })
        .layout(|l| {
            l.width = Length::Percent(1.0);
            l.shrink = 0.0;
        })
        .child(grip)
        .child(chevron)
}

fn toggle_btn(ui: &Ui, state: Signal<DockState>, label: &str) -> El {
    let st = state;
    let mark = {
        let st = state;
        move || {
            let cur = st.get();
            // Point toward the edge when expanded (collapse), toward the
            // center when collapsed (expand). Flip for the end edge.
            let inward = match cur.side {
                DockSide::Start => !cur.collapsed,
                DockSide::End => cur.collapsed,
            };
            if inward { "‹".to_string() } else { "›".to_string() }
        }
    };
    let btn = crate::base::button(ui, mark, move || {
        st.update(|s| s.toggle());
    })
    .variant("quiet");
    let tip = {
        let st = state;
        let label = label.to_string();
        let ui = ui.clone();
        move || {
            let key = if st.get().collapsed { "icon-expand" } else { "icon-collapse" };
            format!("{} {label}", crate::base::tr(&ui, key))
        }
    };
    crate::base::tooltip(&btn, tip);
    let st_s = state;
    let label_s = label.to_string();
    btn.semantics(move || {
        let name =
            if st_s.get().collapsed { format!("Expand {label_s}") } else { format!("Collapse {label_s}") };
        Semantics::new(Role::Button).label(name).action(Action::Click)
    })
}

fn header_grip(ui: &Ui, state: Signal<DockState>, label: &str) -> El {
    let st = state;
    let start_x = Signal::new(0.0f32);
    let dragging = Signal::new(false);
    let grip = ui
        .el("dock-grip")
        .layout(|l| {
            l.width = Length::Px(28.0);
            l.height = Length::Px(16.0);
            l.shrink = 0.0;
        })
        .semantics(Semantics::new(Role::Button).label(format!("Move {label}")))
        .on_pointer(move |e| {
            if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::PRIMARY) {
                start_x.set(e.position.x);
                dragging.set(true);
                return true;
            }
            if dragging.get() && e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY) {
                return true;
            }
            if dragging.get() && matches!(e.phase, PointerPhase::Up | PointerPhase::Cancel) {
                dragging.set(false);
                let dx = e.position.x - start_x.get();
                if dx.abs() > 64.0 {
                    st.update(|s| s.side = s.side.flip());
                }
                return true;
            }
            false
        });
    crate::base::tooltip(&grip, format!("Drag to dock {label} on the other side"));
    grip
}

fn resize_handle(ui: &Ui, state: Signal<DockState>, spec: &DockSpec, label: &str, row: El) -> El {
    let spec = spec.clone();
    let dragging = Signal::new(false);
    let u = ui.clone();
    let handle = ui
        .el("dock-handle")
        .layout(|l| {
            l.width = Length::Px(8.0);
            l.height = Length::Percent(1.0);
            l.shrink = 0.0;
            l.min_width = Length::Px(8.0);
        })
        .focusable(true)
        .semantics({
            let st = state;
            let label = label.to_string();
            let spec = spec.clone();
            move || {
                Semantics::new(Role::Slider)
                    .label(format!("{} {label}", crate::base::tr(&u, "resize-panes")))
                    .numeric(f64::from(st.get().width), f64::from(spec.min_width), f64::from(spec.max_width))
            }
        })
        .on_pointer({
            let st = state;
            let spec = spec.clone();
            let row = row.clone();
            move |e| {
                if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::PRIMARY) {
                    dragging.set(true);
                    apply_resize(&st, &spec, &row, e.position.x);
                    return true;
                }
                if dragging.get() && e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY) {
                    apply_resize(&st, &spec, &row, e.position.x);
                    return true;
                }
                if dragging.get() && matches!(e.phase, PointerPhase::Up | PointerPhase::Cancel) {
                    dragging.set(false);
                    if let Some(b) = row.bounds() {
                        let mid = (b.min.x + b.max.x) * 0.5;
                        let cur = st.get();
                        let on_start =
                            if row.is_rtl() { e.position.x > mid + 24.0 } else { e.position.x < mid - 24.0 };
                        let on_end =
                            if row.is_rtl() { e.position.x < mid - 24.0 } else { e.position.x > mid + 24.0 };
                        let want = if on_start {
                            DockSide::Start
                        } else if on_end {
                            DockSide::End
                        } else {
                            cur.side
                        };
                        if want != cur.side {
                            st.set(DockState { side: want, ..cur });
                        }
                    }
                    return true;
                }
                false
            }
        })
        .on_key({
            let st = state;
            move |e| {
                if !e.pressed {
                    return false;
                }
                let rtl = row.is_rtl();
                let step = match e.key {
                    Key::ArrowLeft => -16.0,
                    Key::ArrowRight => 16.0,
                    _ => return false,
                };
                let cur = st.get();
                let sign = match (cur.side, rtl) {
                    (DockSide::Start, false) | (DockSide::End, true) => 1.0,
                    (DockSide::Start, true) | (DockSide::End, false) => -1.0,
                };
                let width = spec.clamp_width(cur.width + sign * step);
                st.set(DockState { width, ..cur });
                true
            }
        });
    crate::base::tooltip(&handle, crate::base::tr(ui, "resize-panes"));
    handle
}

fn apply_resize(st: &Signal<DockState>, spec: &DockSpec, row: &El, x: f32) {
    let Some(b) = row.bounds() else {
        return;
    };
    let cur = st.get();
    let rtl = row.is_rtl();
    let from_start = if rtl { b.max.x - x } else { x - b.min.x };
    let from_end = if rtl { x - b.min.x } else { b.max.x - x };
    let raw = match cur.side {
        DockSide::Start => from_start,
        DockSide::End => from_end,
    };
    let width = spec.clamp_width(raw);
    if (width - cur.width).abs() > 0.5 {
        st.set(DockState { width, ..cur });
    }
}
