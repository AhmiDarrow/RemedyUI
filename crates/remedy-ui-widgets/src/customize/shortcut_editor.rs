//! The shortcut editor: every command with its shortcuts, searchable; shortcuts recorded from
//! the keyboard, clashes shown before anything changes and resolved by the person; reset per
//! command and for all.

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::input::{Chord, Clash};
use remedy_ui_core::reactive::{Memo, Signal, untrack};
use remedy_ui_engine::view::strings::StrArg;
use remedy_ui_engine::view::{Align, El, Ui};
use crate::base::{self as w, Icon};
use crate::forms::key_capture;
use crate::nav::{Commands, fuzzy_score};

use crate::customize::theme_editor::{reset_button, search, status_line, words};
use crate::customize::{Customizations, CustomizeError, clauses_in, list_in, locale_of, tr_in, tr_now, trf};

/// A shortcut the person recorded that other commands already answer to.
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    id: String,
    label: String,
    chord: Chord,
    clashes: Vec<Clash>,
}

fn label_of(cmds: &Commands, id: &str) -> String {
    untrack(|| cmds.info(id)).map_or_else(|| id.to_owned(), |i| i.label)
}

/// What a change that could not be made says, in the window's locale.
fn explain(ui: &Ui, cmds: &Commands, e: &CustomizeError) -> String {
    let locale = locale_of(ui);
    match e {
        CustomizeError::Clashes(list) => {
            let parts: Vec<String> = list
                .iter()
                .map(|c| {
                    let who: Vec<String> = c.commands.iter().map(|id| label_of(cmds, id)).collect();
                    let chord = ("chord", c.chord.label(ui.platform()).into());
                    let commands = ("commands", list_in(&locale, &who).into());
                    tr_in(&locale, "shortcut-already-runs", &[chord, commands])
                })
                .collect();
            clauses_in(&locale, &parts)
        }
        other => other.message_in(&locale),
    }
}

fn command_row(
    ui: &Ui,
    c: &Customizations,
    cmds: &Commands,
    id: &str,
    pending: Signal<Option<Pending>>,
    status: Signal<String>,
) -> El {
    let unit = ui.theme().spacing.unit;
    let label = label_of(cmds, id);
    // The command's shortcuts, as removable chips.
    let chips = w::wrap(ui).gap(unit).align(Align::Center);
    let (ch2, u, c2, cmds2, id2, l2) =
        (chips.clone(), ui.clone(), c.clone(), cmds.clone(), id.to_owned(), label.clone());
    let chips = chips.bind(move || {
        let chords = cmds2.info(&id2).map(|i| i.chords).unwrap_or_default();
        untrack(|| {
            ch2.clear();
            if chords.is_empty() {
                let _ = ch2.clone().child(w::text(&u, words(&u, "no-shortcut")).variant("muted"));
            }
            for chord in chords {
                let shown = chord.label(u.platform());
                let (c3, u3, id3, cmds3) = (c2.clone(), u.clone(), id2.clone(), cmds2.clone());
                let name = {
                    let (u, shown, label) = (u.clone(), shown.clone(), l2.clone());
                    move || {
                        let args = [("chord", shown.as_str().into()), ("command", label.as_str().into())];
                        trf(&u, "shortcut-remove-named", &args)
                    }
                };
                let remove = w::icon_button(&u, Icon::Close, name, move || {
                    if let Err(e) = c3.remove_shortcut(&u3, &id3, &chord) {
                        status.set(explain(&u3, &cmds3, &e));
                    }
                })
                .variant("quiet");
                let chip = w::row(&u)
                    .align(Align::Center)
                    .gap(unit / 2.0)
                    .child(w::text(&u, shown).variant("code").data_text())
                    .child(remove);
                let _ = ch2.clone().child(chip);
            }
        });
    });
    // Recording a new one.
    let recorded: Signal<Option<Chord>> = Signal::new(None);
    let capture =
        key_capture(ui, tr_now(ui, "shortcut-add-named", &[("command", label.as_str().into())]), recorded);
    let (c4, u4, id4, l4, cmds4) = (c.clone(), ui.clone(), id.to_owned(), label.clone(), cmds.clone());
    let capture = capture.width(unit * 40.0).bind(move || {
        let Some(chord) = recorded.get() else { return };
        untrack(|| {
            recorded.set(None);
            match c4.add_shortcut(&u4, &id4, chord.clone(), false) {
                Ok(()) => {
                    let chord = ("chord", chord.label(u4.platform()).into());
                    status.set(tr_now(&u4, "shortcut-now-runs", &[chord, ("command", l4.as_str().into())]));
                }
                Err(CustomizeError::Clashes(clashes)) => {
                    pending.set(Some(Pending { id: id4.clone(), label: l4.clone(), chord, clashes }))
                }
                Err(e) => status.set(explain(&u4, &cmds4, &e)),
            }
        });
    });
    let (c5, id5, c6, u6, id6, cmds6) =
        (c.clone(), id.to_owned(), c.clone(), ui.clone(), id.to_owned(), cmds.clone());
    let reset_name = {
        let (u, label) = (ui.clone(), label.clone());
        move || trf(&u, "reset-shortcuts-named", &[("command", label.as_str().into())])
    };
    let reset = reset_button(
        ui,
        reset_name,
        move || c5.is_rebound(&id5),
        move || {
            if let Err(e) = c6.reset_shortcut(&u6, &id6) {
                status.set(explain(&u6, &cmds6, &e));
            }
        },
    );
    let name = w::column(ui)
        .grow(1.0)
        .layout(|l| l.min_width = remedy_ui_core::layout::Length::Px(160.0))
        .child(w::text(ui, label.clone()))
        .child(w::text(ui, id.to_owned()).variant("muted").data_text());
    w::wrap(ui)
        .gap(unit * 2.0)
        .align(Align::Center)
        .fill_width()
        .semantics(Semantics::new(Role::Group).label(label))
        .child(name)
        .child(chips)
        .child(capture)
        .child(reset)
}

/// The clash waiting for the person's decision.
fn clash_panel(
    ui: &Ui,
    c: &Customizations,
    cmds: &Commands,
    pending: Signal<Option<Pending>>,
    status: Signal<String>,
) -> El {
    let unit = ui.theme().spacing.unit;
    let panel = w::column(ui).gap(unit).fill_width();
    let (p2, u, c2, cmds2) = (panel.clone(), ui.clone(), c.clone(), cmds.clone());
    panel.bind(move || {
        let p = pending.get();
        // The words in the window's locale (tracked: the panel follows it).
        let locale = u.locale();
        untrack(|| {
            p2.clear();
            let Some(p) = p else { return };
            let shown = p.chord.label(u.platform());
            let others: Vec<String> =
                p.clashes.iter().flat_map(|cl| cl.commands.iter()).map(|id| label_of(&cmds2, id)).collect();
            let others = list_in(&locale, &others);
            let chord: (&str, StrArg) = ("chord", shown.as_str().into());
            let commands = ("commands", StrArg::from(others.as_str()));
            let message = tr_in(&locale, "shortcut-clash", &[chord.clone(), commands]);
            let take_label = tr_in(&locale, "shortcut-take", &[chord, ("command", p.label.as_str().into())]);
            let keep_label = tr_in(&locale, "shortcut-keep", &[("commands", others.as_str().into())]);
            let (c3, u3, p3, cmds3) = (c2.clone(), u.clone(), p.clone(), cmds2.clone());
            let take = w::button(&u, take_label, move || {
                pending.set(None);
                match c3.add_shortcut(&u3, &p3.id, p3.chord.clone(), true) {
                    Ok(()) => {
                        let chord = ("chord", p3.chord.label(u3.platform()).into());
                        let command = ("command", p3.label.as_str().into());
                        status.set(tr_now(&u3, "shortcut-now-runs", &[chord, command]));
                    }
                    Err(e) => status.set(explain(&u3, &cmds3, &e)),
                }
            })
            .variant("primary");
            let keep = w::button(&u, keep_label, move || pending.set(None));
            let box_ = w::column(&u)
                .gap(unit)
                .fill_width()
                .semantics(Semantics::new(Role::Alert).label(message.clone()))
                .child(w::text(&u, message).variant("strong"))
                .child(w::wrap(&u).gap(unit).child(take).child(keep));
            let _ = p2.clone().child(box_);
        });
    })
}

/// The shortcut editor: search the commands, record shortcuts from the keyboard (focus the
/// field and press the keys; Escape leaves it, Backspace clears), remove them, and reset
/// each command or all of them. A shortcut another command answers to is not applied until
/// the person chooses: give it to this command (the other loses it) or keep it where it was.
/// Every change is undoable.
#[track_caller]
pub fn shortcut_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let Some(cmds) = c.commands() else {
        return w::empty_state(ui, words(ui, "shortcuts-off"), words(ui, "shortcuts-off-hint"));
    };
    let query = Signal::new(String::new());
    let pending: Signal<Option<Pending>> = Signal::new(None);
    let status = Signal::new(String::new());
    let search = search(ui, words(ui, "search-commands"), move |q| query.set(q));
    let list_name = words(ui, "commands");
    let list = w::column(ui)
        .gap(unit * 2.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::List).label(list_name()));
    // The commands listed: rows are rebuilt only when these change, so a row keeps its
    // focus and its own bindings while shortcuts change.
    let cmds1 = cmds.clone();
    let listed = Memo::new(move || {
        let q = query.get();
        let mut found: Vec<(i32, usize, String)> = cmds1
            .all()
            .into_iter()
            .enumerate()
            .filter_map(|(i, info)| {
                let score = fuzzy_score(&q, &info.label).or_else(|| fuzzy_score(&q, &info.id))?;
                Some((score, i, info.id))
            })
            .collect();
        if !q.trim().is_empty() {
            found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        found.into_iter().map(|(_, _, id)| id).collect::<Vec<String>>()
    });
    let (l2, u, c2, cmds2) = (list.clone(), ui.clone(), c.clone(), cmds.clone());
    let list = list.bind(move || {
        let ids = listed.get();
        untrack(|| {
            l2.clear();
            if ids.is_empty() {
                let _ = l2.clone().child(w::text(&u, words(&u, "no-matching-command")).variant("muted"));
            }
            for id in &ids {
                let _ = l2.clone().child(command_row(&u, &c2, &cmds2, id, pending, status));
            }
        });
    });
    let (c3, u3, cmds3, c4) = (c.clone(), ui.clone(), cmds.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        words(ui, "reset-all-shortcuts"),
        move || !c4.commands().is_some_and(|k| k.bindings().is_empty()),
        move || {
            if let Err(e) = c3.reset_shortcuts(&u3) {
                status.set(explain(&u3, &cmds3, &e));
            }
        },
    );
    let status_line = status_line(ui, "shortcut-status", status);
    let group = words(ui, "shortcut-editor");
    w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(move || Semantics::new(Role::Group).label(group()))
        .child(w::heading(ui, 2, words(ui, "area-shortcuts")))
        .child(search)
        .child(clash_panel(ui, c, &cmds, pending, status))
        .child(list)
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line))
}
