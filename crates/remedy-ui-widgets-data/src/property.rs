//! Property grids: named values in collapsible groups, each with its own editor, and a
//! filter over the names.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_input::Key;
use remedy_ui_layout::Length;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{Align, El, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

/// One named value and the control that edits it.
#[derive(Debug)]
pub struct Property {
    /// The name, shown in the first column and matched by the filter.
    pub name: String,
    /// What it does, shown under the name and matched by the filter too.
    pub description: Option<String>,
    /// The control that shows and edits the value (any widget).
    pub editor: El,
}

impl Property {
    /// A property `name` edited by `editor`.
    pub fn new(name: impl Into<String>, editor: El) -> Self {
        Property { name: name.into(), description: None, editor }
    }

    /// With a description.
    pub fn describe(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
}

/// Properties under a heading.
#[derive(Debug)]
pub struct PropertyGroup {
    /// The heading.
    pub title: String,
    /// The properties, in order.
    pub properties: Vec<Property>,
    /// Starts collapsed.
    pub collapsed: bool,
}

impl PropertyGroup {
    /// A group `title` of `properties`, open.
    pub fn new(title: impl Into<String>, properties: Vec<Property>) -> Self {
        PropertyGroup { title: title.into(), properties, collapsed: false }
    }

    /// Starts collapsed.
    pub fn collapsed(mut self) -> Self {
        self.collapsed = true;
        self
    }
}

/// True when `filter` (trimmed, any case) is empty or found in the name or description.
fn matches(name: &str, description: Option<&str>, filter: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty()
        || name.to_lowercase().contains(&f)
        || description.is_some_and(|d| d.to_lowercase().contains(&f))
}

/// A property grid of `groups`, with a filter box over the names and descriptions. While a
/// filter is typed, every group with a match shows open.
#[track_caller]
pub fn property_grid(ui: &Ui, label: impl Into<String>, groups: Vec<PropertyGroup>) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let filter = Signal::new(String::new());
    let search = w::search(ui, remedy_ui_view::words::fill(&ui.word("Filter {label}"), &[("label", &label)]), move |s| filter.set(s));
    let mut body = ui.el("property_body").column().gap(unit).fill_width();
    let mut all_rows: Vec<(String, Option<String>)> = Vec::new();
    for g in groups {
        let open = Signal::new(!g.collapsed);
        let title = g.title.clone();
        let size = ui.theme().type_scale.small + 2.0;
        let (down, right) = (w::icon(ui, Icon::ChevronDown, size), w::icon(ui, Icon::ChevronRight, size));
        let (d2, r2) = (down.clone(), right.clone());
        let heading = w::min_target(ui.el("property_group"))
            .row()
            .align(Align::Center)
            .gap(unit)
            .fill_width()
            .child(down)
            .child(right)
            .child(w::text(ui, g.title.clone()).variant("strong").nowrap())
            .on_click(move || open.update(|o| *o = !*o))
            .on_key(move |e| {
                if e.pressed && matches!(e.key, Key::ArrowLeft | Key::ArrowRight) {
                    open.set(e.key == Key::ArrowRight);
                    return true;
                }
                false
            });
        let h2 = heading.clone();
        let heading = heading.bind(move || {
            let filtering = !filter.get().trim().is_empty();
            let shown = open.get() || filtering;
            let _ = d2.clone().hidden(!shown);
            let _ = r2.clone().hidden(shown);
            let _ = h2.clone().semantics(
                Semantics::new(Role::Button)
                    .label(title.clone())
                    .expanded(shown)
                    .action(Action::Click)
                    .action(if shown { Action::Collapse } else { Action::Expand }),
            );
        });
        let mut rows = ui.el("property_rows").column().fill_width();
        let mut group_props: Vec<(String, Option<String>)> = Vec::new();
        for p in g.properties {
            let mut name = ui.el("property_name").column().layout(|l| {
                l.grow = 2.0;
                l.basis = Length::Px(0.0);
            });
            name = name.child(w::text(ui, p.name.clone()).nowrap());
            if let Some(d) = &p.description {
                name = name.child(w::text(ui, d.clone()).variant("muted"));
            }
            let editor = ui.el("property_editor").row().align(Align::Center).layout(|l| {
                l.grow = 3.0;
                l.basis = Length::Px(0.0);
            });
            let row = w::min_target(ui.el("property_row"))
                .row()
                .align(Align::Center)
                .gap(unit * 2.0)
                .fill_width()
                .child(name)
                .child(editor.child(p.editor))
                .semantics(Semantics::new(Role::Group).label(p.name.clone()));
            group_props.push((p.name.clone(), p.description.clone()));
            all_rows.push((p.name.clone(), p.description.clone()));
            let (r2, n, d) = (row.clone(), p.name, p.description);
            let row = row.bind(move || {
                let f = filter.get();
                let _ = r2.clone().hidden(!matches(&n, d.as_deref(), &f));
            });
            rows = rows.child(row);
        }
        let r2 = rows.clone();
        let rows = rows.bind(move || {
            let shown = open.get() || !filter.get().trim().is_empty();
            let _ = r2.clone().hidden(!shown);
        });
        let section = ui.el("property_section").column().fill_width().child(heading).child(rows);
        let s2 = section.clone();
        let section = section.bind(move || {
            let f = filter.get();
            let any = group_props.iter().any(|(n, d)| matches(n, d.as_deref(), &f));
            let _ = s2.clone().hidden(!any);
        });
        body = body.child(section);
    }
    let none = w::text(ui, ui.word("No properties match")).variant("muted");
    let n2 = none.clone();
    let none = none.bind(move || {
        let f = filter.get();
        let any = all_rows.iter().any(|(n, d)| matches(n, d.as_deref(), &f));
        let _ = n2.clone().hidden(any);
    });
    ui.el("property_grid")
        .column()
        .gap(unit * 2.0)
        .fill_width()
        .child(search)
        .child(body.child(none))
        .semantics(Semantics::new(Role::Group).label(label))
}
