//! The theme editor: every token the person can change, edited live with RemedyUI's own
//! widgets, contrast warnings, a preview, reset per token and for the whole theme, and
//! sharing (import and export).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::base as w;
use crate::forms::{NumberOptions, color_picker, number_input};
use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::host::FileDialog;
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::style::{Density, StylePatch};
use remedy_ui_core::{Color, Insets};
use remedy_ui_engine::view::{Align, El, FieldKind, TextField, Ui};
use remedy_ui_platform::services_i18n::Decimal;

use crate::customize::theme::{
    COLOR_ROLES, ImportPolicy, Refusal, RefusalKind, Token, TokenValue, contrast_warnings,
};
use crate::customize::{Customizations, CustomizeError, tr, tr_now, trf};

/// A button reading "Reset" whose accessible name (`name`, kept current as the window's
/// locale changes) says what it resets; dimmed while there is nothing to reset.
pub(crate) fn reset_button(
    ui: &Ui,
    name: impl Fn() -> String + 'static,
    active: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> El {
    let u = ui.clone();
    w::button(ui, move || tr(&u, "reset"), on_press)
        .variant("quiet")
        .semantics(move || Semantics::new(Role::Button).label(name()).action(Action::Click))
        .disabled(move || !active())
}

/// "Reset { $name }" in the window's locale, for [`reset_button`].
pub(crate) fn reset_named(ui: &Ui, name: impl Into<String>) -> impl Fn() -> String + 'static {
    let (u, name) = (ui.clone(), name.into());
    move || trf(&u, "reset-named", &[("name", name.as_str().into())])
}

/// Message `id` in the window's locale, as a property that follows it.
pub(crate) fn words(ui: &Ui, id: &'static str) -> impl Fn() -> String + 'static {
    let u = ui.clone();
    move || tr(&u, id)
}

/// A titled group whose title (`title`) follows the window's locale: the heading names the
/// group for assistive technology.
#[track_caller]
pub(crate) fn section(ui: &Ui, title: impl Fn() -> String + 'static) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    let title = Rc::new(title);
    let t2 = title.clone();
    ui.el("section")
        .column()
        .gap(gap)
        .semantics(move || Semantics::new(Role::Group).label(t2()))
        .child(w::heading(ui, 3, move || title()))
}

/// A one-line text input whose label follows the window's locale.
#[track_caller]
pub(crate) fn input(ui: &Ui, label: impl Fn() -> String + 'static) -> TextField {
    let field = w::text_input(ui, untrack(&label));
    follow_label(&field, label);
    field
}

/// Keeps a field's label current as the window's locale changes.
pub(crate) fn follow_label(field: &TextField, label: impl Fn() -> String + 'static) {
    let f = field.clone();
    let _ = field.el().bind(move || {
        let l = label();
        untrack(|| {
            let _ = f.clone().label(l);
        });
    });
}

/// Keeps a field's placeholder current as the window's locale changes.
pub(crate) fn follow_placeholder(field: &TextField, placeholder: impl Fn() -> String + 'static) {
    let f = field.clone();
    let _ = field.el().bind(move || {
        let p = placeholder();
        untrack(|| {
            let _ = f.clone().placeholder(p);
        });
    });
}

/// A search box (a one-line input with a search icon; Escape clears it) whose label and
/// placeholder follow the window's locale.
#[track_caller]
pub(crate) fn search(
    ui: &Ui,
    label: impl Fn() -> String + 'static,
    on_change: impl Fn(String) + 'static,
) -> El {
    let label = Rc::new(label);
    let field = ui.text_field("text_input", FieldKind::Single);
    let (l1, l2) = (label.clone(), label);
    follow_label(&field, move || l1());
    follow_placeholder(&field, move || l2());
    let _ = field.el().variant("bare");
    let f2 = field.clone();
    let field = field.on_change(move || on_change(f2.value()));
    let f3 = field.clone();
    let gap = ui.theme().spacing.unit * 1.5;
    ui.el("search")
        .row()
        .align(Align::Center)
        .gap(gap)
        .child(w::icon(ui, w::Icon::Search, ui.theme().type_scale.body + 2.0))
        .child(field.el().grow(1.0))
        .on_key(move |e| {
            if e.pressed && e.key == remedy_ui_core::input::Key::Escape && !f3.is_empty() {
                f3.set_value("");
                return true;
            }
            false
        })
}

/// A status line that assistive technology announces as `name`.
pub(crate) fn status_line(ui: &Ui, name: &'static str, status: Signal<String>) -> El {
    let u = ui.clone();
    w::text(ui, "")
        .variant("muted")
        .text(move || status.get())
        .semantics(move || Semantics::new(Role::Status).label(tr(&u, name)).value(status.get()))
}

/// A token's value in the person's theme now (tracked).
fn current(c: &Customizations, token: &Token) -> Option<TokenValue> {
    c.theme().and_then(|t| token.value_in(&t))
}

fn color_of(c: &Customizations, token: &Token) -> Color {
    match current(c, token) {
        Some(TokenValue::Color(col)) => col,
        _ => Color::TRANSPARENT,
    }
}

/// Shows what went wrong with a change in the editor's status line.
fn report(ui: &Ui, status: Signal<String>, r: Result<(), CustomizeError>) {
    if let Err(e) = r {
        status.set(tr_now(ui, "could-not-change", &[("error", e.localized(ui).into())]));
    }
}

/// A token's name in the window's locale, as a property that follows it.
fn token_label(ui: &Ui, token: &Token) -> impl Fn() -> String + 'static {
    let (u, t) = (ui.clone(), token.clone());
    move || t.localized_label(&u)
}

/// A row: the token's name, its control, and its reset.
fn row(ui: &Ui, c: &Customizations, token: &Token, control: El, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let (c1, c2, t1, t2, u2, u3, t3) =
        (c.clone(), c.clone(), token.clone(), token.clone(), ui.clone(), ui.clone(), token.clone());
    w::row(ui)
        .align(Align::Center)
        .gap(unit * 2.0)
        .fill_width()
        .child(w::text(ui, token_label(ui, token)).grow(1.0))
        .child(control)
        .child(reset_button(
            ui,
            move || trf(&u3, "reset-named", &[("name", t3.localized_label(&u3).into())]),
            move || c1.is_customized(&t1),
            move || {
                report(&u2, status, c2.reset_token(&u2, &t2));
            },
        ))
}

fn color_row(ui: &Ui, c: &Customizations, token: &Token, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let open = Signal::new(false);
    let (c1, t1) = (c.clone(), token.clone());
    let swatch = ui.el("swatch").width(unit * 6.0).height(unit * 6.0).style(move || {
        let mut v_ = StylePatch::default();
        v_.background = Some(color_of(&c1, &t1));
        v_.radius = Some(c1.theme().map_or(4.0, |t| t.radii.small));
        v_.border = c1.theme().map(|t| t.derive().line);
        v_.border_width = c1.theme().map(|t| t.stroke.max(1.0));
        v_
    });
    let (c2, t2) = (c.clone(), token.clone());
    let hex = w::text(ui, "").variant("muted").data_text().text(move || {
        let col = color_of(&c2, &t2);
        let h = col.to_hex();
        if col.a >= 0.999 { h.chars().take(7).collect() } else { h }
    });
    let u3 = ui.clone();
    let toggle = w::button(ui, words(ui, "edit"), move || {
        open.update(|o| *o = !*o);
        // Whatever was dragged in the picker was one change; the next starts another.
        u3.end_merge();
    })
    .variant("quiet");
    let t2b = toggle.clone();
    let (u2b, tk2) = (ui.clone(), token.clone());
    let toggle = toggle.bind(move || {
        let o = open.get();
        let name = trf(&u2b, "edit-named", &[("name", tk2.localized_label(&u2b).into())]);
        let _ = t2b.clone().semantics(
            Semantics::new(Role::Button).label(name).action(Action::Click).action(if o {
                Action::Collapse
            } else {
                Action::Expand
            }),
        );
    });
    let head = w::row(ui)
        .align(Align::Center)
        .gap(unit * 2.0)
        .fill_width()
        .child(swatch)
        .child(w::text(ui, token_label(ui, token)).grow(1.0))
        .child(hex)
        .child(toggle);
    let (c4, t4, u4) = (c.clone(), token.clone(), ui.clone());
    let reset_name = {
        let (u, t) = (ui.clone(), token.clone());
        move || trf(&u, "reset-named", &[("name", t.localized_label(&u).into())])
    };
    let head = head.child(reset_button(ui, reset_name, move || c4.is_customized(&t4), {
        let (c5, t5, u5) = (c.clone(), token.clone(), ui.clone());
        move || report(&u5, status, c5.reset_token(&u5, &t5))
    }));
    let body = w::column(ui).fill_width();
    let b2 = body.clone();
    let (c6, t6) = (c.clone(), token.clone());
    let body = body.bind(move || {
        let o = open.get();
        untrack(|| {
            b2.clear();
            if !o {
                return;
            }
            // Text roles are read against the background, text on the accent against it.
            let against = c6.theme().map(|t| match t6.path().get(1).map(String::as_str) {
                Some("on-accent") => t.colors.accent,
                Some("text" | "muted" | "danger" | "accent" | "line") => t.colors.bg,
                _ => t.colors.text,
            });
            let value = Signal::new(color_of(&c6, &t6));
            let picker = color_picker(&u4, t6.localized_label(&u4), value, against);
            let (c7, t7, c8, t8, u8) = (c6.clone(), t6.clone(), c6.clone(), t6.clone(), u4.clone());
            // Outside changes (undo, reset) show in the picker…
            let picker = picker.bind(move || {
                let now = color_of(&c7, &t7);
                if value.get_untracked().ok() != Some(now) {
                    value.set(now);
                }
            });
            // …and the picker's changes apply live.
            let picker = picker.bind(move || {
                let v = value.get();
                if untrack(|| color_of(&c8, &t8)) != v {
                    untrack(|| report(&u8, status, c8.set_token(&u8, &t8, TokenValue::Color(v))));
                }
            });
            let _ = b2.clone().child(picker);
        });
    });
    w::column(ui).gap(unit).fill_width().child(head).child(body)
}

fn number_row(
    ui: &Ui,
    c: &Customizations,
    token: &Token,
    precision: u8,
    unit_name: &'static str,
    status: Signal<String>,
) -> El {
    let read = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Px(v)) => f64::from(v),
            Some(TokenValue::Duration(d)) => d.as_secs_f64() * 1000.0,
            _ => 0.0,
        }
    };
    let to_dec = move |v: f64| Decimal::from_f64(v, precision).unwrap_or(Decimal::ZERO);
    let value = Signal::new(to_dec(read()));
    let options = {
        let mut v_ = NumberOptions::default();
        v_.min = Some(Decimal::ZERO);
        v_.max = None;
        v_.step = Decimal::ONE;
        v_.precision = precision;
        v_.unit = Some(untrack(|| tr(ui, unit_name)));
        v_
    };
    let field = number_input(ui, untrack(|| token.localized_label(ui)), value, options);
    let read2 = Rc::new(read);
    let r1 = read2.clone();
    let field = field.bind(move || {
        let now = to_dec(r1());
        if value.get_untracked().ok() != Some(now) {
            value.set(now);
        }
    });
    let (c2, t2, u2) = (c.clone(), token.clone(), ui.clone());
    let field = field.bind(move || {
        let v = value.get();
        if untrack(|| to_dec(read2())) == v {
            return;
        }
        let n = v.to_f64();
        let tv = match t2.kind() {
            crate::customize::TokenKind::Duration => {
                TokenValue::Duration(Duration::from_secs_f64(n.max(0.0) / 1000.0))
            }
            _ => TokenValue::Px(n as f32),
        };
        untrack(|| report(&u2, status, c2.set_token(&u2, &t2, tv)));
    });
    row(ui, c, token, field, status)
}

fn families_row(ui: &Ui, c: &Customizations, token: &Token, status: Signal<String>) -> El {
    let field = input(ui, token_label(ui, token));
    let read = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Families(f)) => f.join(", "),
            _ => String::new(),
        }
    };
    let (f2, c2, t2, u2) = (field.clone(), c.clone(), token.clone(), ui.clone());
    let field = field.on_change(move || {
        let list: Vec<String> =
            f2.value().split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            report(&u2, status, c2.set_token(&u2, &t2, TokenValue::Families(list)));
        }
    });
    let (el, f3) = (field.el().width(ui.theme().spacing.unit * 60.0), field.clone());
    let el2 = el.clone();
    let el = el.bind(move || {
        let want = read();
        if !el2.is_focused() && f3.value() != want {
            untrack(|| f3.set_value(&want));
        }
    });
    row(ui, c, token, el, status)
}

fn name_row(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let token = Token::name();
    let field = input(ui, token_label(ui, &token));
    let (f2, c2, t2, u2) = (field.clone(), c.clone(), token.clone(), ui.clone());
    let field = field.on_change(move || {
        report(&u2, status, c2.set_token(&u2, &t2, TokenValue::Name(f2.value())));
    });
    let (el, f3, c3, t3) =
        (field.el().width(ui.theme().spacing.unit * 60.0), field.clone(), c.clone(), token.clone());
    let el2 = el.clone();
    let el = el.bind(move || {
        let want = match current(&c3, &t3) {
            Some(TokenValue::Name(n)) => n,
            _ => String::new(),
        };
        if !el2.is_focused() && f3.value() != want {
            untrack(|| f3.set_value(&want));
        }
    });
    row(ui, c, &token, el, status)
}

fn density_row(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let token = Token::density();
    let all = [Density::Compact, Density::Comfortable, Density::Spacious];
    let index = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Density(d)) => all.iter().position(|x| *x == d).unwrap_or(1),
            _ => 1,
        }
    };
    let chosen = Signal::new(index());
    let ids = ["density-compact", "density-comfortable", "density-spacious"];
    let names = untrack(|| ids.map(|id| tr(ui, id)));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let seg = w::segmented(ui, untrack(|| token.localized_label(ui)), &names, chosen);
    let index = Rc::new(index);
    let i1 = index.clone();
    let seg = seg.bind(move || {
        let now = i1();
        if chosen.get_untracked().ok() != Some(now) {
            chosen.set(now);
        }
    });
    let (c2, t2, u2) = (c.clone(), token.clone(), ui.clone());
    let seg = seg.bind(move || {
        let i = chosen.get();
        if untrack(|| index()) != i
            && let Some(d) = all.get(i)
        {
            untrack(|| report(&u2, status, c2.set_token(&u2, &t2, TokenValue::Density(*d))));
        }
    });
    row(ui, c, &token, seg, status)
}

/// The pairs below WCAG contrast, live.
fn contrast_panel(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let list = w::column(ui).gap(unit).fill_width();
    let (l2, u, c2) = (list.clone(), ui.clone(), c.clone());
    let list = list.bind(move || {
        let warnings = c2.theme().map(|t| contrast_warnings(&t)).unwrap_or_default();
        // What the list says, in the window's locale (tracked).
        let lines: Vec<String> = if warnings.is_empty() {
            vec![tr(&u, "contrast-all-pass")]
        } else {
            warnings.iter().map(|wn| wn.localized(&u)).collect()
        };
        untrack(|| {
            l2.clear();
            let variant = if warnings.is_empty() { "muted" } else { "error" };
            for line in lines {
                let _ = l2.clone().child(w::text(&u, line).variant(variant));
            }
        });
    });
    let (c3, u3) = (c.clone(), ui.clone());
    section(ui, words(ui, "contrast")).child(list).semantics(move || {
        let n = c3.theme().map(|t| contrast_warnings(&t).len()).unwrap_or(0);
        let s = Semantics::new(Role::Status).label(tr(&u3, "contrast"));
        if n == 0 {
            s.value(tr(&u3, "contrast-status-all-pass"))
        } else {
            s.value(trf(&u3, "contrast-status-failing", &[("count", n.into())]))
        }
    })
}

/// Sample widgets in the live theme.
fn preview(ui: &Ui) -> El {
    let unit = ui.theme().spacing.unit;
    let on = Signal::new(true);
    let progress = Signal::new(Some(0.6f32));
    let field = input(ui, words(ui, "preview-field"));
    field.set_value(&untrack(|| tr(ui, "preview-field-text")));
    let group = words(ui, "preview-group");
    w::card(ui)
        .gap(unit * 2.0)
        .padding(Insets::all(unit * 3.0))
        .layout(|l| l.min_width = remedy_ui_core::layout::Length::Px(240.0))
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "preview")))
        .child(w::text(ui, words(ui, "preview-body")))
        .child(w::text(ui, words(ui, "preview-muted")).variant("muted"))
        .child(
            w::wrap(ui)
                .gap(unit)
                .child(w::button(ui, words(ui, "preview-primary"), || {}).variant("primary"))
                .child(w::button(ui, words(ui, "preview-secondary"), || {}))
                .child(w::button(ui, words(ui, "preview-danger"), || {}).variant("danger")),
        )
        .child(w::switch(ui, words(ui, "preview-switch"), on))
        .child(field.el())
        .child(w::progress(ui, words(ui, "preview-progress"), progress))
        .child(w::link(ui, words(ui, "preview-link"), || {}))
        .child(w::text(ui, words(ui, "preview-error")).variant("error"))
}

/// A shared theme waiting for the person to decide about what it carries beyond tokens:
/// its file (when it came from one, so its files can be found), its text, and what was
/// refused.
type Pending = Option<(Option<PathBuf>, String, Vec<Refusal>)>;

/// Import and export.
fn sharing(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let path = input(ui, words(ui, "theme-file"));
    follow_placeholder(&path, words(ui, "theme-file-placeholder"));
    let pending: Signal<Pending> = Signal::new(None);
    let import = {
        let (c, u) = (c.clone(), ui.clone());
        Rc::new(move |from: Option<PathBuf>, text: String, policy: ImportPolicy| {
            let done = match &from {
                Some(p) => c.import_theme_file(&u, p, policy),
                None => c.import_theme(&u, &text, policy),
            };
            match done {
                Ok(r) if r.dropped.is_empty() => {
                    pending.set(None);
                    status.set(tr_now(&u, "imported", &[("name", r.name.into())]));
                }
                Ok(r) => {
                    pending.set(None);
                    let args = [("name", r.name.into()), ("count", r.dropped.len().into())];
                    status.set(tr_now(&u, "imported-leaving-out", &args));
                }
                Err(CustomizeError::Refused(list)) => pending.set(Some((from, text, list))),
                Err(e) => status.set(tr_now(&u, "could-not-import", &[("error", e.localized(&u).into())])),
            }
        })
    };
    let read_and_import = {
        let (f, u) = (import.clone(), ui.clone());
        Rc::new(move |p: PathBuf| match std::fs::read_to_string(&p) {
            Ok(text) => f(Some(p), text, ImportPolicy::default()),
            Err(e) => {
                let args = [("path", p.display().to_string().into()), ("detail", e.to_string().into())];
                status.set(tr_now(&u, "could-not-read", &args));
            }
        })
    };
    let export_to = {
        let (c, u) = (c.clone(), ui.clone());
        Rc::new(move |p: PathBuf| match c.export_theme(&p) {
            Ok(()) => status.set(tr_now(&u, "exported", &[("path", p.display().to_string().into())])),
            Err(e) => status.set(tr_now(&u, "could-not-export", &[("error", e.localized(&u).into())])),
        })
    };
    let (p1, r1, u1) = (path.clone(), read_and_import.clone(), ui.clone());
    let import_btn = w::button(ui, words(ui, "import-from-path"), move || {
        let p = p1.value();
        if p.trim().is_empty() {
            status.set(tr_now(&u1, "import-path-first", &[]));
        } else {
            r1(p.trim().into());
        }
    });
    let (p2, e2, u2) = (path.clone(), export_to.clone(), ui.clone());
    let export_btn = w::button(ui, words(ui, "export-to-path"), move || {
        let p = p2.value();
        if p.trim().is_empty() {
            status.set(tr_now(&u2, "export-path-first", &[]));
        } else {
            e2(p.trim().into());
        }
    });
    let mut buttons = w::wrap(ui).gap(unit).child(import_btn).child(export_btn);
    if c.has_file_picker() {
        let (c1, r2, u3) = (c.clone(), read_and_import.clone(), ui.clone());
        let choose = w::button(ui, words(ui, "choose-theme"), move || {
            let r3 = r2.clone();
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = tr_now(&u3, "dialog-import-theme", &[]);
                v_.filters = vec![(tr_now(&u3, "filter-design-tokens", &[]), vec!["json".into()])];
                v_
            };
            c1.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    r3(p);
                }
            });
        });
        let (c2, e3, u4) = (c.clone(), export_to.clone(), ui.clone());
        let save = w::button(ui, words(ui, "export-to-file"), move || {
            let e4 = e3.clone();
            let name = c2.theme().map(|t| t.name).filter(|n| !n.trim().is_empty());
            let name = name.unwrap_or_else(|| tr_now(&u4, "export-default-name", &[]));
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = tr_now(&u4, "dialog-export-theme", &[]);
                v_.save = true;
                v_.file_name = Some(format!("{name}.json"));
                v_.filters = vec![(tr_now(&u4, "filter-design-tokens", &[]), vec!["json".into()])];
                v_
            };
            c2.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    e4(p);
                }
            });
        });
        buttons = buttons.child(choose).child(save);
    }
    // The decision about refused parts.
    let decide = w::column(ui).gap(unit).fill_width();
    let (d2, u) = (decide.clone(), ui.clone());
    let decide = decide.bind(move || {
        let p = pending.get();
        untrack(|| {
            d2.clear();
            let Some((from, text, list)) = p else { return };
            let has = |k: RefusalKind| list.iter().any(|r| r.kind == k);
            let allow_shaders = Signal::new(false);
            let allow_links = Signal::new(false);
            let allow_files = Signal::new(false);
            let alert = words(&u, "theme-carries-more");
            let mut box_ = w::column(&u)
                .gap(unit)
                .fill_width()
                .semantics(move || Semantics::new(Role::Alert).label(alert()))
                .child(w::text(&u, words(&u, "theme-carries-more-detail")).variant("strong"));
            for r in &list {
                let (r, u2) = (r.clone(), u.clone());
                box_ = box_.child(w::text(&u, move || r.localized(&u2)).variant("muted").data_text());
            }
            if has(RefusalKind::Shader) {
                box_ = box_.child(w::switch(&u, words(&u, "allow-shaders"), allow_shaders));
            }
            if has(RefusalKind::ExternalUrl) {
                box_ = box_.child(w::switch(&u, words(&u, "allow-links"), allow_links));
            }
            if has(RefusalKind::Asset) {
                box_ = box_.child(w::switch(&u, words(&u, "allow-files"), allow_files));
            }
            let (f1, f2, t1, t2, from1, from2) =
                (import.clone(), import.clone(), text.clone(), text, from.clone(), from);
            let row = w::wrap(&u)
                .gap(unit)
                .child(
                    w::button(&u, words(&u, "import"), move || {
                        f1(
                            from1.clone(),
                            t1.clone(),
                            ImportPolicy {
                                allow_shaders: allow_shaders.get_untracked().unwrap_or(false),
                                allow_external_urls: allow_links.get_untracked().unwrap_or(false),
                                allow_assets: allow_files.get_untracked().unwrap_or(false),
                                drop_refused: true,
                            },
                        )
                    })
                    .variant("primary"),
                )
                .child(w::button(&u, words(&u, "import-without"), move || {
                    f2(
                        from2.clone(),
                        t2.clone(),
                        ImportPolicy { drop_refused: true, ..ImportPolicy::default() },
                    )
                }))
                .child(w::button(&u, words(&u, "cancel"), move || pending.set(None)));
            let _ = d2.clone().child(box_.child(row));
        });
    });
    section(ui, words(ui, "section-share"))
        .child(w::text(ui, words(ui, "share-hint")))
        .child(path.el().fill_width())
        .child(buttons)
        .child(decide)
}

/// The files the theme brings: a background image and fonts.
fn files_section(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    // The background.
    let (c0, u0) = (c.clone(), ui.clone());
    let shown = w::text(ui, "").variant("muted").data_text().text(move || {
        match c0.theme().and_then(|t| t.assets.background) {
            Some(r) if r.starts_with("data:") => tr(&u0, "background-inline"),
            Some(r) => trf(&u0, "background-file", &[("file", r.into())]),
            None => tr(&u0, "no-background"),
        }
    });
    let image_path = input(ui, words(ui, "background-file-field"));
    follow_placeholder(&image_path, words(ui, "background-placeholder"));
    let (c1, u1, p1) = (c.clone(), ui.clone(), image_path.clone());
    let use_image = w::button(ui, words(ui, "use-image"), move || {
        let p = p1.value();
        if p.trim().is_empty() {
            status.set(tr_now(&u1, "image-path-first", &[]));
        } else {
            report(&u1, status, c1.set_background(&u1, Some(Path::new(p.trim()))));
        }
    });
    let (c2, u2, c3) = (c.clone(), ui.clone(), c.clone());
    let remove = reset_button(
        ui,
        words(ui, "reset-background"),
        move || c3.theme().is_some_and(|t| t.assets.background.is_some()),
        move || report(&u2, status, c2.set_background(&u2, None)),
    );
    let mut image_row =
        w::wrap(ui).gap(unit).align(Align::Center).child(image_path.el().grow(1.0)).child(use_image);
    if c.has_file_picker() {
        let (c4, u4) = (c.clone(), ui.clone());
        image_row = image_row.child(w::button(ui, words(ui, "choose-image"), move || {
            let (c5, u5) = (c4.clone(), u4.clone());
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = tr_now(&u4, "dialog-choose-image", &[]);
                v_.filters = vec![(
                    tr_now(&u4, "filter-images", &[]),
                    ["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"].map(String::from).to_vec(),
                )];
                v_
            };
            c4.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    report(&u5, status, c5.set_background(&u5, Some(&p)));
                }
            });
        }));
    }
    // The fonts.
    let list = w::column(ui).gap(unit).fill_width();
    let (l2, u, c6) = (list.clone(), ui.clone(), c.clone());
    let list = list.bind(move || {
        let fonts: Vec<String> =
            c6.theme().map(|t| t.assets.fonts.keys().cloned().collect()).unwrap_or_default();
        untrack(|| {
            l2.clear();
            if fonts.is_empty() {
                let _ = l2.clone().child(w::text(&u, words(&u, "no-fonts")).variant("muted"));
            }
            for family in fonts {
                let (c7, u7, f7, u8, f8) = (c6.clone(), u.clone(), family.clone(), u.clone(), family.clone());
                let on_press = move || report(&u7, status, c7.remove_theme_font(&u7, &f7));
                let remove =
                    w::button(&u, words(&u, "remove"), on_press).variant("quiet").semantics(move || {
                        let name = trf(&u8, "remove-font-named", &[("family", f8.as_str().into())]);
                        Semantics::new(Role::Button).label(name).action(Action::Click)
                    });
                let row = w::row(&u)
                    .gap(unit * 2.0)
                    .align(Align::Center)
                    .child(w::text(&u, family).grow(1.0))
                    .child(remove);
                let _ = l2.clone().child(row);
            }
        });
    });
    let font_path = input(ui, words(ui, "font-file"));
    follow_placeholder(&font_path, words(ui, "font-file-placeholder"));
    let font_name = input(ui, words(ui, "font-name"));
    follow_placeholder(&font_name, words(ui, "font-name-placeholder"));
    let (c8, u8, fp, fname) = (c.clone(), ui.clone(), font_path.clone(), font_name.clone());
    let add = w::button(ui, words(ui, "add-font"), move || {
        let (p, n) = (fp.value(), fname.value());
        if p.trim().is_empty() || n.trim().is_empty() {
            status.set(tr_now(&u8, "font-first", &[]));
        } else {
            report(&u8, status, c8.add_theme_font(&u8, n.trim(), Path::new(p.trim()), true));
        }
    });
    section(ui, words(ui, "section-files"))
        .child(w::text(ui, words(ui, "files-hint")))
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(shown.grow(1.0)).child(remove))
        .child(image_row)
        .child(list)
        .child(
            w::wrap(ui)
                .gap(unit)
                .align(Align::Center)
                .child(font_path.el().grow(1.0))
                .child(font_name.el().grow(1.0))
                .child(add),
        )
}

/// The theme editor: colors (with a picker and contrast readout each, and warnings for pairs
/// below WCAG contrast), fonts and text sizes, spacing, radii, line width, motion and
/// density, each with its reset, applied live to every attached window, beside a preview;
/// import and export; and a reset for the whole theme. Every change is undoable.
#[track_caller]
pub fn theme_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let Some(theme) = c.theme() else {
        return w::empty_state(ui, words(ui, "theme-off"), words(ui, "theme-off-hint"));
    };
    let status = Signal::new(String::new());
    let mut colors = section(ui, words(ui, "section-colors"));
    for (role, _) in COLOR_ROLES {
        colors = colors.child(color_row(ui, c, &Token::color(role), status));
    }
    for name in theme.colors.custom.keys() {
        colors = colors.child(color_row(ui, c, &Token::custom_color(name), status));
    }
    let px = |t: Token| number_row(ui, c, &t, 1, "unit-px", status);
    let ms = |t: Token| number_row(ui, c, &t, 0, "unit-ms", status);
    let text = section(ui, words(ui, "section-text"))
        .child(name_row(ui, c, status))
        .child(families_row(ui, c, &Token::font_family(), status))
        .child(families_row(ui, c, &Token::mono_family(), status))
        .child(px(Token::size("small")))
        .child(px(Token::size("body")))
        .child(px(Token::size("title")));
    let shape = section(ui, words(ui, "section-shape"))
        .child(px(Token::spacing_unit()))
        .child(px(Token::control_height()))
        .child(px(Token::radius("small")))
        .child(px(Token::radius("medium")))
        .child(px(Token::radius("large")))
        .child(px(Token::stroke()))
        .child(density_row(ui, c, status));
    let motion = section(ui, words(ui, "section-motion"))
        .child(ms(Token::motion("quick")))
        .child(ms(Token::motion("standard")))
        .child(ms(Token::motion("emphasized")));
    let (c2, u2, c3) = (c.clone(), ui.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        words(ui, "reset-theme"),
        move || !c3.customized_tokens().is_empty(),
        move || {
            report(&u2, status, c2.reset_theme(&u2));
        },
    );
    let status_line = status_line(ui, "theme-status", status);
    let controls = w::column(ui)
        .gap(unit * 4.0)
        .grow(1.0)
        .layout(|l| l.min_width = remedy_ui_core::layout::Length::Px(280.0))
        .child(colors)
        .child(contrast_panel(ui, c))
        .child(text)
        .child(shape)
        .child(motion)
        .child(files_section(ui, c, status))
        .child(sharing(ui, c, status))
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line));
    let group = words(ui, "theme-editor");
    w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "area-theme")))
        .child(w::wrap(ui).gap(unit * 4.0).fill_width().child(controls).child(preview(ui)))
}
