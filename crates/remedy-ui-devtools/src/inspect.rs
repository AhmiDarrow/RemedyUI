//! The element inspector: pick an element on screen, see what it is and where it came from,
//! check the window for accessibility problems, and edit theme colors live.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, Insets, Rect};
use remedy_ui_input::PointerPhase;
use remedy_ui_layout::Position;
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui_view::{El, ElementId, ElementInfo, Ui};
use remedy_ui_widgets_core as w;

/// Builds a style sheet for a theme (the app's own, so live edits restyle it correctly).
pub type SheetFn = Rc<dyn Fn(&Theme) -> StyleSheet>;

/// What is wrong with an element.
#[derive(Clone, Debug, PartialEq)]
pub enum ProblemKind {
    /// A control assistive technology cannot name.
    Unnamed,
    /// Text whose contrast with what is behind it is below WCAG AA.
    LowContrast {
        /// The ratio found.
        ratio: f32,
        /// The ratio needed (4.5, or 3 for large text).
        needed: f32,
    },
    /// Pointer handling with no keyboard way to do the same (WCAG 2.1.1): not focusable and
    /// no [`El::keyboard_alternative`](remedy_ui_view::El::keyboard_alternative).
    PointerOnly,
    /// Focusable but with no size: keyboard focus lands on nothing visible.
    InvisibleFocus,
    /// Focusable, but nothing about it changes when keyboard focus reaches it (WCAG 2.4.7
    /// Focus Visible): its style sheet has no focus rule.
    NoFocusIndicator,
    /// A control smaller than 24×24 (WCAG 2.2 target size).
    SmallTarget {
        /// Its size.
        width: f32,
        /// Its size.
        height: f32,
    },
}

/// One accessibility problem.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// The element.
    pub element: ElementId,
    /// What is wrong.
    pub kind: ProblemKind,
    /// Where the element was built.
    pub at: Option<&'static std::panic::Location<'static>>,
}

impl Problem {
    /// A sentence for people.
    pub fn message(&self) -> String {
        let at = self.at.map(|l| format!(" ({}:{})", l.file(), l.line())).unwrap_or_default();
        match &self.kind {
            ProblemKind::Unnamed => format!("Control with no accessible name{at}"),
            ProblemKind::PointerOnly => {
                format!("Works only with a pointer: make it focusable or name its keyboard alternative{at}")
            }
            ProblemKind::InvisibleFocus => format!("Takes keyboard focus but has no size{at}"),
            ProblemKind::NoFocusIndicator => format!("Shows no change when keyboard focus reaches it{at}"),
            ProblemKind::LowContrast { ratio, needed } => {
                format!("Text contrast {ratio:.1}:1, needs {needed:.1}:1{at}")
            }
            ProblemKind::SmallTarget { width, height } => {
                format!("Target {width:.0}×{height:.0}, needs 24×24{at}")
            }
        }
    }
}

/// True when someone looking would see the change from `a` to `b`.
fn visibly_differs(a: &remedy_ui_style::ResolvedStyle, b: &remedy_ui_style::ResolvedStyle) -> bool {
    let border = |s: &remedy_ui_style::ResolvedStyle| {
        if s.border_width > 0.0 && s.border.a > 0.0 { Some((s.border, s.border_width)) } else { None }
    };
    b.shows_focus_ring()
        || a.background != b.background
        || a.foreground != b.foreground
        || border(a) != border(b)
        || (a.opacity - b.opacity).abs() > 0.05
}

/// Every element of a window, depth first (content, then overlays, then the trusted layer),
/// with its depth.
pub fn walk(ui: &Ui) -> Vec<(usize, El)> {
    let mut out = Vec::new();
    for layer in ui.layers() {
        let mut stack = vec![(0usize, layer)];
        while let Some((depth, el)) = stack.pop() {
            let kids = el.child_elements();
            for k in kids.into_iter().rev() {
                stack.push((depth + 1, k));
            }
            out.push((depth, el));
        }
    }
    out
}

fn text_in(el: &El) -> Option<String> {
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if let Some(i) = e.info()
            && !i.hidden
        {
            if let Some(t) = i.text.filter(|t| !t.trim().is_empty()) {
                return Some(t);
            }
            stack.extend(e.child_elements().into_iter().rev());
        }
    }
    None
}

/// False when the element or an ancestor is hidden.
fn shown_chain(el: &El) -> bool {
    let mut cur = el.parent();
    while let Some(e) = cur {
        if e.info().is_some_and(|i| i.hidden) {
            return false;
        }
        cur = e.parent();
    }
    true
}

/// The color behind `el` and how opaque `el`'s own drawing is: each ancestor's background
/// laid over the window's, from the window down, through every ancestor's opacity (opacity
/// applies to a whole subtree, as it is drawn).
fn backdrop(el: &El, theme_bg: Color) -> (Color, f32) {
    let mut chain = Vec::new();
    let mut cur = el.parent();
    while let Some(e) = cur {
        chain.push(e.clone());
        cur = e.parent();
    }
    let over = |top: Color, a: f32, under: Color| {
        Color::new(
            top.r * a + under.r * (1.0 - a),
            top.g * a + under.g * (1.0 - a),
            top.b * a + under.b * (1.0 - a),
            1.0,
        )
    };
    let mut seen = theme_bg;
    let mut opacity = 1.0f32;
    for e in chain.iter().rev() {
        let Some(i) = e.info() else { continue };
        opacity *= i.style.opacity;
        seen = over(i.style.background, (i.style.background.a * opacity).clamp(0.0, 1.0), seen);
    }
    let own = el.info().map_or(1.0, |i| i.style.opacity);
    // The element's own background is behind its text too.
    if let Some(i) = el.info() {
        seen = over(i.style.background, (i.style.background.a * opacity * own).clamp(0.0, 1.0), seen);
    }
    (seen, (opacity * own).clamp(0.0, 1.0))
}

/// True when `el` sits inside a disabled control (WCAG exempts inactive controls from
/// contrast and naming rules).
fn inside_disabled(el: &El) -> bool {
    let mut cur = el.parent();
    while let Some(e) = cur {
        if e.info().is_some_and(|i| i.states.contains(States::DISABLED)) {
            return true;
        }
        cur = e.parent();
    }
    false
}

/// Checks a window (after a frame, so boxes are known) for unnamed controls, low-contrast
/// text and small targets. Hidden and disabled elements are skipped, as WCAG allows.
pub fn audit(ui: &Ui) -> Vec<Problem> {
    let bg = ui.theme().colors.bg;
    let mut out = Vec::new();
    for (_, el) in walk(ui) {
        let Some(i) = el.info() else { continue };
        let sized = i.bounds.is_some_and(|b| b.width() > 0.0 && b.height() > 0.0);
        let hidden_sem = i.semantics.as_ref().is_some_and(Semantics::is_hidden);
        if i.hidden
            || hidden_sem
            || i.states.contains(States::DISABLED)
            || !shown_chain(&el)
            || inside_disabled(&el)
        {
            continue;
        }
        if !sized {
            if i.focusable && i.bounds.is_some() {
                out.push(Problem { element: i.id, kind: ProblemKind::InvisibleFocus, at: i.created_at });
            }
            continue;
        }
        if i.pointer && !i.focusable && i.keyboard_alternative.is_none() {
            out.push(Problem { element: i.id, kind: ProblemKind::PointerOnly, at: i.created_at });
        }
        // Keyboard focus must show: a focus ring, or a focused style that differs visibly
        // from the unfocused one. Dialogs and windows take focus as containers and show it by
        // being on top; tables, trees and lists (WAI-ARIA composite widgets) show it on the
        // item the keyboard is on.
        let container = matches!(
            i.semantics.as_ref().map(Semantics::role),
            Some(Role::Dialog | Role::Window | Role::Table | Role::Tree | Role::List | Role::Grid)
        );
        if i.focusable && !container {
            let rest = i.states.with(States::FOCUS | States::FOCUS_VISIBLE, false);
            if let (Some(a), Some(b)) =
                (el.style_in(rest), el.style_in(rest | States::FOCUS | States::FOCUS_VISIBLE))
                && !visibly_differs(&a, &b)
            {
                out.push(Problem { element: i.id, kind: ProblemKind::NoFocusIndicator, at: i.created_at });
            }
        }
        let interactive = i.clickable
            || (i.focusable
                && !matches!(
                    i.semantics.as_ref().map(Semantics::role),
                    Some(Role::Group | Role::Grid | Role::Dialog)
                ));
        if interactive {
            let named = i.semantics.as_ref().and_then(|s| s.name()).is_some_and(|n| !n.trim().is_empty())
                || text_in(&el).is_some();
            if !named {
                out.push(Problem { element: i.id, kind: ProblemKind::Unnamed, at: i.created_at });
            }
            if let Some(b) = i.bounds
                && i.clickable
                && (b.width() < 23.5 || b.height() < 23.5)
            {
                out.push(Problem {
                    element: i.id,
                    kind: ProblemKind::SmallTarget { width: b.width(), height: b.height() },
                    at: i.created_at,
                });
            }
        }
        if i.text.as_deref().is_some_and(|t| !t.trim().is_empty()) {
            let fg = i.style.foreground;
            let (behind, opacity) = backdrop(&el, bg);
            // Partly transparent text blends with what is behind it.
            let a = (fg.a * opacity).clamp(0.0, 1.0);
            let seen = Color::new(
                fg.r * a + behind.r * (1.0 - a),
                fg.g * a + behind.g * (1.0 - a),
                fg.b * a + behind.b * (1.0 - a),
                1.0,
            );
            let ratio = seen.contrast(behind);
            let large = i.style.text_size >= 24.0 || (i.style.text_size >= 18.66 && i.style.weight >= 700.0);
            let needed = if large { 3.0 } else { 4.5 };
            if ratio + 0.005 < needed {
                out.push(Problem {
                    element: i.id,
                    kind: ProblemKind::LowContrast { ratio, needed },
                    at: i.created_at,
                });
            }
        }
    }
    out
}

/// A one-line description of an element for the tree view.
pub fn describe(i: &ElementInfo) -> String {
    let mut s = i.widget.to_owned();
    if !i.variant.is_empty() {
        s.push('.');
        s.push_str(i.variant);
    }
    if let Some(name) = i.semantics.as_ref().and_then(|x| x.name()) {
        s.push_str(&format!(" \u{201C}{}\u{201D}", short(name)));
    } else if let Some(t) = &i.text {
        s.push_str(&format!(" \u{201C}{}\u{201D}", short(t)));
    }
    s
}

fn short(s: &str) -> String {
    let mut out: String = s.chars().take(32).collect();
    if s.chars().count() > 32 {
        out.push('…');
    }
    out
}

/// The color roles a theme has, by name.
pub const COLOR_ROLES: [&str; 10] =
    ["bg", "panel", "card", "text", "muted", "accent", "on_accent", "danger", "warn", "ok"];

fn role_color(t: &Theme, role: &str) -> Option<Color> {
    let c = &t.colors;
    Some(match role {
        "bg" => c.bg,
        "panel" => c.panel,
        "card" => c.card,
        "text" => c.text,
        "muted" => c.muted,
        "accent" => c.accent,
        "on_accent" => c.on_accent,
        "danger" => c.danger,
        "warn" => c.warn,
        "ok" => c.ok,
        _ => return None,
    })
}

fn set_role(t: &mut Theme, role: &str, v: Color) -> bool {
    let c = &mut t.colors;
    let slot = match role {
        "bg" => &mut c.bg,
        "panel" => &mut c.panel,
        "card" => &mut c.card,
        "text" => &mut c.text,
        "muted" => &mut c.muted,
        "accent" => &mut c.accent,
        "on_accent" => &mut c.on_accent,
        "danger" => &mut c.danger,
        "warn" => &mut c.warn,
        "ok" => &mut c.ok,
        _ => return false,
    };
    *slot = v;
    true
}

/// An inspector attached to one window's [`Ui`].
#[derive(Clone)]
pub struct Inspector(Rc<Inner>);

struct Inner {
    target: Ui,
    sheet: SheetFn,
    selected: Signal<Option<ElementId>>,
    picking: Signal<bool>,
    revision: Signal<u64>,
    highlight: RefCell<Option<El>>,
}

impl Inspector {
    /// Attaches to `target`. `sheet` builds the app's style sheet for edited themes.
    pub fn attach(target: &Ui, sheet: impl Fn(&Theme) -> StyleSheet + 'static) -> Inspector {
        Inspector(Rc::new(Inner {
            target: target.clone(),
            sheet: Rc::new(sheet),
            selected: Signal::new(None),
            picking: Signal::new(false),
            revision: Signal::new(0),
            highlight: RefCell::new(None),
        }))
    }

    /// The window being inspected.
    pub fn target(&self) -> &Ui {
        &self.0.target
    }

    /// The selected element (tracked).
    pub fn selected(&self) -> Option<ElementId> {
        self.0.selected.get()
    }

    /// True while picking (tracked).
    pub fn is_picking(&self) -> bool {
        self.0.picking.get()
    }

    /// Re-reads the target (after it changed).
    pub fn refresh(&self) {
        self.0.revision.update(|r| *r += 1);
        let sel = self.0.selected.get_untracked().ok().flatten();
        self.outline(sel);
    }

    /// Selects an element and outlines it in the target window.
    pub fn select(&self, id: Option<ElementId>) {
        self.0.selected.set(id);
        self.outline(id);
    }

    fn outline(&self, id: Option<ElementId>) {
        let t = &self.0.target;
        let bounds = id.and_then(|id| t.element(id).bounds());
        let mut slot = self.0.highlight.borrow_mut();
        match (bounds, slot.as_ref()) {
            (Some(b), Some(h)) => place(h, b),
            (Some(b), None) => {
                let accent = t.theme().colors.accent;
                let h = t
                    .el("inspector_highlight")
                    .pass_through()
                    .style(StylePatch {
                        background: Some(accent.with_alpha(0.18)),
                        border: Some(accent),
                        border_width: Some(2.0),
                        ..StylePatch::default()
                    })
                    .semantics(Semantics::new(Role::Group).hidden(true));
                place(&h, b);
                *slot = Some(t.overlay(h));
            }
            (None, Some(h)) => {
                h.remove();
                *slot = None;
            }
            (None, None) => {}
        }
    }

    /// Starts or stops picking: while on, the target window's pointer input only moves the
    /// outline, and a press selects what is under it.
    pub fn pick(&self, on: bool) {
        self.0.picking.set(on);
        if !on {
            self.0.target.set_pointer_interceptor(None);
            return;
        }
        let me: Weak<Inner> = Rc::downgrade(&self.0);
        self.0.target.set_pointer_interceptor(Some(Rc::new(move |e| {
            let Some(inner) = me.upgrade() else { return false };
            let insp = Inspector(inner);
            let hit = insp.0.target.element_at(e.position).map(|el| el.id());
            match e.phase {
                PointerPhase::Down => {
                    insp.select(hit);
                    insp.pick(false);
                    insp.0.revision.update(|r| *r += 1);
                }
                PointerPhase::Cancel => insp.pick(false),
                _ => insp.outline(hit),
            }
            true
        })));
    }

    /// Changes a theme color in the target window, restyling it at once.
    pub fn set_color(&self, role: &str, c: Color) {
        let mut t = self.0.target.theme();
        if set_role(&mut t, role, c) {
            let sheet = (self.0.sheet)(&t);
            self.0.target.set_theme(t, sheet);
            self.refresh();
        }
    }

    /// Stops picking and removes the outline.
    pub fn detach(&self) {
        self.pick(false);
        self.select(None);
    }
}

fn place(el: &El, b: Rect) {
    let _ = el.clone().layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets {
            top: b.min.y,
            start: b.min.x,
            end: remedy_ui_layout::AUTO,
            bottom: remedy_ui_layout::AUTO,
        };
        l.width = remedy_ui_layout::Length::Px(b.width());
        l.height = remedy_ui_layout::Length::Px(b.height());
    });
}

fn line(ui: &Ui, k: &str, v: String) -> El {
    let unit = ui.theme().spacing.unit;
    w::row(ui)
        .gap(unit * 2.0)
        .fill_width()
        .child(w::label(ui, k.to_owned(), "muted").width(110.0))
        .child(w::label(ui, v, "").data_text().grow(1.0))
}

fn details(ui: &Ui, i: &ElementInfo) -> El {
    let unit = ui.theme().spacing.unit;
    let s = &i.style;
    let mut col = w::column(ui).gap(unit).fill_width();
    let rows: Vec<(&str, String)> = vec![
        (
            "Widget",
            if i.variant.is_empty() { i.widget.to_owned() } else { format!("{} ({})", i.widget, i.variant) },
        ),
        (
            "Built at",
            i.created_at.map_or_else(
                || "unknown".to_owned(),
                |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
            ),
        ),
        (
            "Box",
            i.bounds.map_or_else(
                || "not laid out".to_owned(),
                |b| format!("{:.1}, {:.1} · {:.1} × {:.1}", b.min.x, b.min.y, b.width(), b.height()),
            ),
        ),
        (
            "Padding",
            format!(
                "{:.1} {:.1} {:.1} {:.1}",
                s.padding.top, s.padding.end, s.padding.bottom, s.padding.start
            ),
        ),
        ("Background", s.background.to_hex()),
        ("Foreground", s.foreground.to_hex()),
        ("Border", format!("{} × {:.1}", s.border.to_hex(), s.border_width)),
        ("Radius", format!("{:.1}", s.radius)),
        (
            "Text",
            format!("{:.1} px · weight {:.0}{}", s.text_size, s.weight, if s.mono { " · mono" } else { "" }),
        ),
        ("Opacity", format!("{:.2}", s.opacity)),
        ("States", format!("{:?}", i.states)),
        (
            "Semantics",
            i.semantics.as_ref().map_or_else(
                || "none".to_owned(),
                |m| format!("{:?} \u{201C}{}\u{201D} {:?}", m.role(), m.name().unwrap_or(""), m.actions()),
            ),
        ),
        (
            "Behavior",
            [
                (i.focusable, "focusable"),
                (i.clickable, "clickable"),
                (i.field, "text field"),
                (i.painter, "custom paint"),
                (i.hidden, "hidden"),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(", "),
        ),
        ("Bindings", i.bindings.to_string()),
        (
            "Reads",
            if i.reads.is_empty() {
                "nothing reactive".to_owned()
            } else {
                i.reads.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
            },
        ),
    ];
    for (k, v) in rows {
        col = col.child(line(ui, k, v));
    }
    col
}

/// The inspector's panel, built into the inspector window's own [`Ui`]: picking, the
/// selected element's details, the element tree, the accessibility check and theme colors.
#[track_caller]
pub fn inspector_panel(ui: &Ui, insp: &Inspector) -> El {
    let unit = ui.theme().spacing.unit;
    let rev = insp.0.revision;
    let target = insp.0.target.clone();

    let i1 = insp.clone();
    let pick = w::button(ui, "Pick an element", move || {
        let on = !i1.0.picking.get_untracked().unwrap_or(false);
        i1.pick(on);
    })
    .variant("primary");
    let label = pick.child_elements().into_iter().next();
    let p2 = insp.0.picking;
    let pick = pick.bind(move || {
        let on = p2.get();
        if let Some(l) = &label {
            let _ = l.clone().text(remedy_ui_view::RichText::plain(if on {
                "Click an element in the window…"
            } else {
                "Pick an element"
            }));
        }
    });
    let i2 = insp.clone();
    let toolbar =
        w::wrap(ui).gap(unit * 2.0).child(pick).child(w::button(ui, "Refresh", move || i2.refresh())).child(
            w::button(ui, "Clear", {
                let i = insp.clone();
                move || i.select(None)
            })
            .variant("quiet"),
        );

    // Details of the selection.
    let detail = w::column(ui).fill_width();
    let (d2, u2, i3, t2) = (detail.clone(), ui.clone(), insp.clone(), target.clone());
    let detail = detail.bind(move || {
        let _ = rev.get();
        let sel = i3.0.selected.get();
        untrack(|| {
            d2.clear();
            let content = match sel.and_then(|id| t2.element(id).info()) {
                Some(info) => details(&u2, &info),
                None => {
                    w::label(&u2, "Nothing selected. Pick an element, or choose one in the tree.", "muted")
                }
            };
            let _ = d2.clone().child(content);
        });
    });

    // The tree.
    let tree = w::column(ui).fill_width();
    let (tr2, u3, i4, t3) = (tree.clone(), ui.clone(), insp.clone(), target.clone());
    let tree = tree.bind(move || {
        let _ = rev.get();
        let sel = i4.0.selected.get();
        untrack(|| {
            tr2.clear();
            let mut col = tr2.clone();
            for (depth, el) in walk(&t3) {
                let Some(info) = el.info() else { continue };
                let id = info.id;
                let i5 = i4.clone();
                let row = u3
                    .el("list_item")
                    .row()
                    .fill_width()
                    .padding(Insets {
                        top: unit / 2.0,
                        bottom: unit / 2.0,
                        start: unit + depth as f32 * unit * 3.0,
                        end: unit,
                    })
                    .child(
                        w::label(&u3, describe(&info), if info.hidden { "muted" } else { "" })
                            .data_text()
                            .nowrap(),
                    )
                    .on_click(move || i5.select(Some(id)))
                    .semantics(
                        Semantics::new(Role::ListItem).label(describe(&info)).selected(sel == Some(id)),
                    );
                row.state(States::SELECTED, sel == Some(id));
                col = col.child(row);
            }
        });
    });

    // Accessibility.
    let problems = w::column(ui).gap(unit).fill_width();
    let (pr2, u4, i6, t4) = (problems.clone(), ui.clone(), insp.clone(), target.clone());
    let problems = problems.bind(move || {
        let _ = rev.get();
        untrack(|| {
            pr2.clear();
            let found = audit(&t4);
            let mut col = pr2.clone();
            if found.is_empty() {
                col = col.child(w::label(&u4, "No problems found.", "muted"));
            }
            for p in found {
                let id = p.element;
                let i7 = i6.clone();
                col = col.child(w::link(&u4, p.message(), move || i7.select(Some(id))));
            }
            let _ = col;
        });
    });

    // Theme colors, editable.
    let mut tokens = w::column(ui).gap(unit * 2.0).fill_width();
    let theme = target.theme();
    for role in COLOR_ROLES {
        let Some(c) = role_color(&theme, role) else { continue };
        let value = Signal::new(c);
        let i8 = insp.clone();
        let first = std::cell::Cell::new(true);
        let swatch = ui.el("swatch").width(20.0).height(20.0).style_dyn(move || StylePatch {
            background: Some(value.get()),
            radius: Some(4.0),
            ..StylePatch::default()
        });
        let editor = w::column(ui);
        let (e2, u5) = (editor.clone(), ui.clone());
        let open = Signal::new(false);
        let editor = editor.bind(move || {
            let shown = open.get();
            untrack(|| {
                e2.clear();
                if shown {
                    let _ = e2.clone().child(remedy_ui_widgets_forms::color_picker(&u5, role, value, None));
                }
            });
        });
        // Every change restyles the target.
        let watcher = w::column(ui).bind(move || {
            let c = value.get();
            if first.replace(false) {
                return;
            }
            untrack(|| i8.set_color(role, c));
        });
        let head = w::row(ui)
            .gap(unit * 2.0)
            .align(remedy_ui_view::Align::Center)
            .child(swatch)
            .child(w::label(ui, role, "").grow(1.0))
            .child(w::label(ui, "", "muted").text_dyn(move || value.get().to_hex()))
            .child(w::button(ui, "Edit", move || open.update(|o| *o = !*o)).variant("quiet"));
        tokens = tokens.child(head).child(editor).child(watcher);
    }

    let body = w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .padding(Insets::all(unit * 4.0))
        .child(toolbar)
        .child(w::section(ui, "Selected").child(detail))
        .child(w::section(ui, "Accessibility check").child(problems))
        .child(w::section(ui, "Elements").child(tree))
        .child(w::section(ui, "Theme colors").child(tokens));
    w::scroll(ui).grow(1.0).fill_width().child(body)
}
