//! The toolbar and text editors, the notices, and the settings page that brings every editor
//! together.

use crate::base as w;
use crate::nav::{Tab, fuzzy_score, tabs, toolbar, toolbar_editor};
use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::reactive::{Memo, Signal, untrack};
use remedy_ui_engine::view::{Align, El, Ui};
use remedy_ui_platform::services_i18n::MessageEntry;

use crate::customize::theme_editor::{
    follow_label, reset_button, reset_named, search, section, status_line, words,
};
use crate::customize::{
    Area, Customizations, CustomizeError, shortcut_editor, theme_editor, tr, tr_now, trf,
};

/// "`id`: what went wrong" in the window's locale now, for a status line.
fn failed(ui: &Ui, id: &str, e: &CustomizeError) -> String {
    tr_now(ui, id, &[("error", e.localized(ui).into())])
}

/// What could not be loaded or saved, each with its one-click reset and a dismiss button.
/// Shows nothing while everything loaded.
#[track_caller]
pub fn notices_view(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let list = w::column(ui).gap(unit * 2.0).fill_width();
    let (l2, u, c2) = (list.clone(), ui.clone(), c.clone());
    list.bind(move || {
        let notices = c2.notices();
        // What each notice says, in the window's locale (tracked: the list follows it).
        let shown: Vec<(String, String, String)> = notices
            .iter()
            .map(|n| {
                let file = n.path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let args = [
                    ("area", n.area.localized_name(&u).into()),
                    ("file", file.as_str().into()),
                    ("message", n.localized_message(&u).into()),
                ];
                let message = trf(&u, "notice-row", &args);
                let reset = trf(&u, "reset-named", &[("name", file.as_str().into())]);
                let dismiss = trf(&u, "notice-dismiss-named", &[("file", file.as_str().into())]);
                (message, reset, dismiss)
            })
            .collect();
        let (reset_text, dismiss_text) = (tr(&u, "reset"), tr(&u, "notice-dismiss"));
        untrack(|| {
            l2.clear();
            for (n, (message, reset_name, dismiss_name)) in notices.into_iter().zip(shown) {
                let (c3, u3, n3, c4, n4) = (c2.clone(), u.clone(), n.clone(), c2.clone(), n.clone());
                let reset = w::button(&u, reset_text.clone(), move || {
                    let _ = c3.reset_notice(&u3, &n3);
                })
                .variant("primary")
                .semantics(Semantics::new(Role::Button).label(reset_name).action(Action::Click));
                let dismiss = w::button(&u, dismiss_text.clone(), move || c4.dismiss(&n4))
                    .variant("quiet")
                    .semantics(Semantics::new(Role::Button).label(dismiss_name).action(Action::Click));
                let row = w::wrap(&u)
                    .gap(unit * 2.0)
                    .align(Align::Center)
                    .fill_width()
                    .semantics(Semantics::new(Role::Alert).label(message.clone()))
                    .child(w::text(&u, message).variant("error").grow(1.0))
                    .child(reset)
                    .child(dismiss);
                let _ =
                    l2.clone().child(w::card(&u).padding(remedy_ui_core::Insets::all(unit * 2.0)).child(row));
            }
        });
    })
}

/// Every toolbar and menu the app declared: its live toolbar, the editor to choose and order
/// its commands (`crate::nav::toolbar_editor`), and its reset; then a reset for
/// all. Edits are saved and undoable.
#[track_caller]
pub fn toolbars_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let Some(cmds) = c.toolbar_commands() else {
        return w::empty_state(ui, words(ui, "toolbars-off"), words(ui, "toolbars-off-hint"));
    };
    let status = Signal::new(String::new());
    let group = words(ui, "toolbar-editor");
    let mut col = w::column(ui)
        .gap(unit * 4.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "area-toolbars")));
    let slots = c.toolbar_slots();
    if slots.is_empty() {
        col = col.child(w::text(ui, words(ui, "no-toolbars")).variant("muted"));
    }
    for (id, label) in slots {
        let Some(slot) = c.toolbar(&id) else { continue };
        let (c1, id1, c2, u2, id2) = (c.clone(), id.clone(), c.clone(), ui.clone(), id.clone());
        let reset = reset_button(
            ui,
            reset_named(ui, &label),
            move || c1.is_toolbar_customized(&id1),
            move || {
                if let Err(e) = c2.reset_toolbar(&u2, &id2) {
                    status.set(failed(&u2, "could-not-reset", &e));
                }
            },
        );
        col = col.child(
            w::section(ui, &label)
                .child(w::card(ui).padding(remedy_ui_core::Insets::all(unit)).child(toolbar(
                    ui,
                    label.clone(),
                    &cmds,
                    slot,
                )))
                .child(toolbar_editor(ui, &cmds, slot))
                .child(reset),
        );
    }
    let (c3, u3, c4) = (c.clone(), ui.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        words(ui, "reset-all-toolbars"),
        move || c4.toolbar_slots().iter().any(|(id, _)| c4.is_toolbar_customized(id)),
        move || {
            if let Err(e) = c3.reset_toolbars(&u3) {
                status.set(failed(&u3, "could-not-reset", &e));
            }
        },
    );
    col.child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line(
        ui,
        "toolbar-status",
        status,
    )))
}

fn text_row(ui: &Ui, c: &Customizations, source: &str, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let Some(catalog) = c.text_catalog() else { return w::column(ui) };
    let field = w::text_input(ui, source.to_owned());
    let (f2, c2, u2, s2) = (field.clone(), c.clone(), ui.clone(), source.to_owned());
    let field = field.on_change(move || {
        if let Err(e) = c2.set_text(&u2, &untrack(|| u2.locale()), &s2, &f2.value()) {
            status.set(failed(&u2, "could-not-change-text", &e));
        }
    });
    let (el, f3, c3, u3, s3, cat3) =
        (field.el().fill_width(), field.clone(), c.clone(), ui.clone(), source.to_owned(), catalog);
    let el = el.bind(move || {
        let locale = u3.locale();
        let want = c3.text_override(&locale, &s3);
        let translation = cat3.translation_in(&locale, &s3);
        untrack(|| {
            let _ = f3.clone().placeholder(translation.clone());
            let shown = f3.value();
            let same = match &want {
                Some(w) => *w == shown,
                None => shown.trim().is_empty() || shown == translation,
            };
            if !same {
                f3.set_value(want.as_deref().unwrap_or(""));
            }
        });
    });
    let (c4, s4, u4, c5, u5, s5) =
        (c.clone(), source.to_owned(), ui.clone(), c.clone(), ui.clone(), source.to_owned());
    let reset_name = {
        let (u, s) = (ui.clone(), source.to_owned());
        move || trf(&u, "reset-text", &[("text", s.as_str().into())])
    };
    let reset = reset_button(
        ui,
        reset_name,
        move || c4.text_override(&u4.locale(), &s4).is_some(),
        move || {
            if let Err(e) = c5.reset_text(&u5, &untrack(|| u5.locale()), &s5) {
                status.set(failed(&u5, "could-not-reset-text", &e));
            }
        },
    );
    w::column(ui)
        .gap(unit)
        .fill_width()
        .child(w::text(ui, source.to_owned()).variant("muted").data_text())
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).fill_width().child(el.grow(1.0)).child(reset))
}

/// The message `id` in the window's language: shown now, the app's own, and rewritten or not.
fn message_now(c: &Customizations, ui: &Ui, id: &str) -> Option<(MessageEntry, MessageEntry, bool)> {
    c.message_entries(&ui.locale()).into_iter().find(|(e, _, _)| e.id == id)
}

fn message_row(ui: &Ui, c: &Customizations, id: &str) -> El {
    let unit = ui.theme().spacing.unit;
    let status = Signal::new(String::new());
    let Some((now, app, _)) = untrack(|| message_now(c, ui, id)) else { return w::column(ui) };
    let value = app.value.as_ref().map(|_| {
        let f = w::text_area(ui, id.to_owned());
        f.set_value(now.value.as_deref().unwrap_or_default());
        f
    });
    let attrs: Vec<(String, remedy_ui_engine::view::TextField)> = app
        .attributes
        .iter()
        .map(|(name, _)| {
            let f = w::text_input(ui, format!("{id}.{name}"));
            let v =
                now.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_default();
            f.set_value(&v);
            (name.clone(), f)
        })
        .collect();
    // The message as the fields say it.
    let read = {
        let (value, attrs, id) = (value.clone(), attrs.clone(), id.to_owned());
        std::rc::Rc::new(move || MessageEntry {
            id: id.clone(),
            value: value.as_ref().map(|f| f.value()),
            attributes: attrs.iter().map(|(n, f)| (n.clone(), f.value())).collect(),
        })
    };
    let save = {
        let (c, u, read) = (c.clone(), ui.clone(), read.clone());
        std::rc::Rc::new(move || match c.set_message(&u, &untrack(|| u.locale()), &read()) {
            Ok(()) => status.set(String::new()),
            Err(e) => status.set(failed(&u, "not-saved", &e)),
        })
    };
    let mut col =
        w::column(ui).gap(unit).fill_width().semantics(Semantics::new(Role::Group).label(id.to_owned()));
    col = col.child(w::text(ui, id.to_owned()).variant("code").data_text());
    let fields: Vec<remedy_ui_engine::view::TextField> =
        value.iter().cloned().chain(attrs.iter().map(|(_, f)| f.clone())).collect();
    for f in &fields {
        let s2 = save.clone();
        let f = f.clone().on_change(move || s2());
        col = col.child(f.el().fill_width());
    }
    // Outside changes (an undo, a reset) show in the fields not being typed in.
    let (c2, u2, id2, fields2) = (c.clone(), ui.clone(), id.to_owned(), fields.clone());
    let col = col.bind(move || {
        let Some((now, _, _)) = message_now(&c2, &u2, &id2) else { return };
        let wanted: Vec<String> =
            now.value.iter().cloned().chain(now.attributes.iter().map(|(_, v)| v.clone())).collect();
        untrack(|| {
            for (f, want) in fields2.iter().zip(wanted) {
                if !f.el().is_focused() && f.value() != want {
                    f.set_value(&want);
                }
            }
        });
    });
    let (c3, u3, id3, c4, u4, id4) =
        (c.clone(), ui.clone(), id.to_owned(), c.clone(), ui.clone(), id.to_owned());
    let reset = reset_button(
        ui,
        reset_named(ui, id),
        move || message_now(&c3, &u3, &id3).is_some_and(|(_, _, mine)| mine),
        move || {
            if let Err(e) = c4.reset_message(&u4, &untrack(|| u4.locale()), &id4) {
                status.set(failed(&u4, "could-not-reset", &e));
            }
        },
    );
    let (id5, u5) = (id.to_owned(), ui.clone());
    let check = w::text(ui, "").variant("error").text(move || status.get()).semantics(move || {
        let name = trf(&u5, "message-check", &[("id", id5.as_str().into())]);
        Semantics::new(Role::Status).label(name).value(status.get())
    });
    col.child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset).child(check))
}

/// The app's Fluent messages, searchable, each with its value and attributes to rewrite.
fn messages_section(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let query = Signal::new(String::new());
    let search = search(ui, words(ui, "search-messages"), move |q| query.set(q));
    let (c1, u1) = (c.clone(), ui.clone());
    let listed = Memo::new(move || {
        let q = query.get();
        c1.message_entries(&u1.locale())
            .into_iter()
            .filter(|(e, _, _)| {
                fuzzy_score(&q, &e.id).is_some()
                    || e.value.as_deref().is_some_and(|v| fuzzy_score(&q, v).is_some())
            })
            .map(|(e, _, _)| e.id)
            .collect::<Vec<String>>()
    });
    let list_name = words(ui, "messages");
    let list = w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::List).label(list_name()));
    let (l2, u, c2) = (list.clone(), ui.clone(), c.clone());
    let list = list.bind(move || {
        let ids = listed.get();
        untrack(|| {
            l2.clear();
            if ids.is_empty() {
                let _ = l2.clone().child(w::text(&u, words(&u, "no-matching-message")).variant("muted"));
            }
            for id in &ids {
                let _ = l2.clone().child(message_row(&u, &c2, id));
            }
        });
    });
    section(ui, words(ui, "messages"))
        .child(w::text(ui, words(ui, "messages-hint")).variant("muted"))
        .child(search)
        .child(list)
}

/// Every string the app shows, in the window's language, each with a field to rewrite it
/// (empty: the app's own text, shown as the placeholder) and its reset; a search; the folder
/// the message files live in, with a button to read hand edits; and a reset for all.
/// Strings come from the catalog given to [`Customizations::enable_text`]; Fluent messages
/// ([`Customizations::enable_messages`]) are rewritten in `.ftl` files in the same folder.
#[track_caller]
pub fn text_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    if !c.is_enabled(Area::Text) {
        return w::empty_state(ui, words(ui, "text-off"), words(ui, "text-off-hint"));
    }
    let status = Signal::new(String::new());
    let query = Signal::new(String::new());
    let folder = {
        let (u, dir) = (ui.clone(), c.text_dir().display().to_string());
        move || trf(&u, "text-folder", &[("folder", dir.as_str().into())])
    };
    let c0 = c.clone();
    let reload = w::button(ui, words(ui, "read-files-again"), move || c0.reload_text());
    let group = words(ui, "text-editor");
    let mut col = w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "area-text")))
        .child(w::text(ui, folder).variant("muted").data_text())
        .child(reload);
    if c.text_catalog().is_some() {
        let search = search(ui, words(ui, "search-text"), move |q| query.set(q));
        let list_name = words(ui, "strings");
        let list = w::column(ui)
            .gap(unit * 2.0)
            .fill_width()
            .semantics(move || Semantics::new(Role::List).label(list_name()));
        // The strings listed: rows are rebuilt only when these change, so a field keeps its
        // focus and its bindings while the person types.
        let c1 = c.clone();
        let listed = Memo::new(move || {
            let q = query.get();
            let sources = c1.text_catalog().map(|cat| cat.sources()).unwrap_or_default();
            sources.into_iter().filter(|s| fuzzy_score(&q, s).is_some()).collect::<Vec<String>>()
        });
        let (l2, u, c2) = (list.clone(), ui.clone(), c.clone());
        let list = list.bind(move || {
            let found = listed.get();
            untrack(|| {
                l2.clear();
                if found.is_empty() {
                    let _ = l2.clone().child(w::text(&u, words(&u, "no-matching-text")).variant("muted"));
                }
                for s in &found {
                    let _ = l2.clone().child(text_row(&u, &c2, s, status));
                }
            });
        });
        col = col.child(search).child(list);
    }
    if c.text_messages().is_some() {
        col = col.child(messages_section(ui, c));
    }
    let (c3, u3, c4) = (c.clone(), ui.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        words(ui, "reset-all-text"),
        move || !c4.files_of_text_is_empty(),
        move || {
            if let Err(e) = c3.reset_texts(&u3) {
                status.set(failed(&u3, "could-not-reset", &e));
            }
        },
    );
    col.child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line(
        ui,
        "text-status",
        status,
    )))
}

fn layout_section(ui: &Ui, c: &Customizations, file: &str) -> El {
    let unit = ui.theme().spacing.unit;
    let status = Signal::new(String::new());
    let source = c.layout_source(file).unwrap_or_default();
    let options = {
        let mut v_ = crate::doc_code::CodeOptions::default();
        v_.language = "rui".into();
        v_
    };
    let editor_name = {
        let (u, f) = (ui.clone(), file.to_owned());
        move || trf(&u, "layout-named", &[("file", f.as_str().into())])
    };
    let (editor, field) = crate::doc_code::code_editor(ui, untrack(&editor_name), &source, options);
    follow_label(&field, editor_name);
    // Checked as the person types: what is wrong, at which line and column.
    let (f2, c2, file2, u2) = (field.clone(), c.clone(), file.to_owned(), ui.clone());
    let check = move || match c2.check_layout(&file2, &f2.value()) {
        Ok(()) => status.set(tr_now(&u2, "layout-ok", &[])),
        Err(e) => status.set(e.localized(&u2)),
    };
    check();
    field.observe_text(check);
    // Outside changes (a reset, an undo) show in the editor while it is not being typed in.
    let (el, f3, c3, file3) = (editor.fill_width(), field.clone(), c.clone(), file.to_owned());
    let el2 = el.clone();
    let el = el.bind(move || {
        let Some(now) = c3.layout_source(&file3) else { return };
        untrack(|| {
            if !el2.is_focused() && f3.value() != now {
                f3.set_value(&now);
            }
        });
    });
    let (f4, c4, file4, u4) = (field.clone(), c.clone(), file.to_owned(), ui.clone());
    let (u7, file7) = (ui.clone(), file.to_owned());
    let save = w::button(ui, words(ui, "save"), move || match c4.save_layout(&file4, &f4.value()) {
        Ok(()) => status.set(tr_now(&u4, "layout-saved", &[])),
        Err(e) => status.set(failed(&u4, "not-saved", &e)),
    })
    .variant("primary")
    .semantics(move || {
        let name = trf(&u7, "save-layout-named", &[("file", file7.as_str().into())]);
        Semantics::new(Role::Button).label(name).action(Action::Click)
    });
    let (c5, file5, c6, u6, file6) = (c.clone(), file.to_owned(), c.clone(), ui.clone(), file.to_owned());
    let reset_name = {
        let (u, f) = (ui.clone(), file.to_owned());
        move || trf(&u, "reset-layout-named", &[("file", f.as_str().into())])
    };
    let reset = reset_button(
        ui,
        reset_name,
        move || c5.is_layout_customized(&file5),
        move || {
            if let Err(e) = c6.reset_layout(&u6, &file6) {
                status.set(failed(&u6, "could-not-reset", &e));
            }
        },
    );
    let status_el = status_line(ui, "layout-check", status);
    let file_name = file.to_owned();
    section(ui, move || file_name.clone())
        .child(el)
        .child(status_el)
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(save).child(reset))
}

/// Every screen the app lets the person rearrange: its `.rui` text in a code editor, checked
/// as the person types (what is wrong, at which line and column), saved when it loads, and
/// reset to the app's own; then a reset for all. Saves and resets are undoable.
#[track_caller]
pub fn layouts_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    if !c.is_enabled(Area::Layouts) {
        return w::empty_state(ui, words(ui, "layouts-off"), words(ui, "layouts-off-hint"));
    }
    let status = Signal::new(String::new());
    let folder = {
        let (u, dir) = (ui.clone(), c.layouts_dir().display().to_string());
        move || trf(&u, "layouts-folder", &[("folder", dir.as_str().into())])
    };
    let group = words(ui, "layout-editor");
    let mut col = w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "area-layouts")))
        .child(w::text(ui, folder).variant("muted").data_text());
    let files = c.layout_files();
    if files.is_empty() {
        col = col.child(w::text(ui, words(ui, "no-layouts")).variant("muted"));
    }
    for f in &files {
        col = col.child(layout_section(ui, c, f));
    }
    let (c3, u3, c4) = (c.clone(), ui.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        words(ui, "reset-all-layouts"),
        move || c4.layout_files().iter().any(|f| c4.is_layout_customized(f)),
        move || {
            if let Err(e) = c3.reset_layouts(&u3) {
                status.set(failed(&u3, "could-not-reset", &e));
            }
        },
    );
    col.child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line(
        ui,
        "layout-status",
        status,
    )))
}

impl Customizations {
    /// Whether the person has no message files (tracked).
    fn files_of_text_is_empty(&self) -> bool {
        self.0.revision.get();
        self.files_of(Area::Text).is_empty()
    }
}

/// The settings page: the notices, a tab for each kind the app turned on (theme, shortcuts,
/// toolbars and menus, text) with its editor, and a reset for everything. Its words are in
/// the window's language and follow it when it changes (the open editor is built again).
#[track_caller]
pub fn settings_page(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let areas = c.areas();
    let tab_items = {
        let (u, areas) = (ui.clone(), areas.clone());
        move || areas.iter().map(|a| Tab::new(format!("{a:?}"), a.localized_name(&u))).collect::<Vec<Tab>>()
    };
    let items = untrack(&tab_items);
    let first = items.first().map(|t| t.id.clone());
    let items = Signal::new(items);
    let selected = Signal::new(first);
    let strip = tabs(ui, untrack(|| tr(ui, "customize")), items, selected, |_| {});
    // The strip's name, as `tabs` gives it, in the window's language.
    let strip_name = words(ui, "customize");
    let strip = strip.semantics(move || Semantics::new(Role::TabList).label(strip_name()));
    // Tab titles in the window's language.
    let strip = strip.bind(move || {
        let now = tab_items();
        if items.get_untracked().ok().as_ref() != Some(&now) {
            untrack(|| items.set(now));
        }
    });
    let content = w::column(ui).fill_width().grow(1.0);
    let (ct2, u, c2) = (content.clone(), ui.clone(), c.clone());
    let content = content.bind(move || {
        let sel = selected.get();
        // Built again when the window's language changes, so every label follows it.
        let _ = u.locale();
        untrack(|| {
            ct2.clear();
            let editor = match sel.as_deref() {
                Some("Theme") => theme_editor(&u, &c2),
                Some("Shortcuts") => shortcut_editor(&u, &c2),
                Some("Toolbars") => toolbars_editor(&u, &c2),
                Some("Text") => text_editor(&u, &c2),
                Some("Layouts") => layouts_editor(&u, &c2),
                _ => {
                    let hint = words(&u, "nothing-to-customize-hint");
                    w::empty_state(&u, words(&u, "nothing-to-customize"), hint)
                }
            };
            let _ = ct2.clone().child(editor);
        });
    });
    let status = Signal::new(String::new());
    let (c3, u3) = (c.clone(), ui.clone());
    let everything_name = words(ui, "reset-all-customizations");
    let everything = w::button(ui, words(ui, "reset-everything"), move || match c3.reset_all(&u3) {
        Ok(()) => status.set(tr_now(&u3, "reset-everything-done", &[])),
        Err(e) => status.set(failed(&u3, "could-not-reset", &e)),
    })
    .variant("danger")
    .semantics(move || Semantics::new(Role::Button).label(everything_name()).action(Action::Click));
    let group = words(ui, "customize");
    w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 1, words(ui, "customize")))
        .child(notices_view(ui, c))
        .child(strip)
        .child(w::scroll(ui).child(content))
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(everything).child(status_line(
            ui,
            "customize-status",
            status,
        )))
}
