//! Toolbars whose buttons the user chooses and orders, and the command palette.

use std::cell::RefCell;
use std::rc::Rc;

use crate::base::{self as w, Icon};
use remedy_ui_core::Insets;
use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::{Key, Platform};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_engine::view::{Align, El, FieldKind, Ui};

use crate::nav::commands::{CommandInfo, Commands};

/// A command's tooltip in the window's language: its label, and its shortcut when it has one.
fn tip(ui: &Ui, c: &CommandInfo, platform: Platform) -> String {
    match c.chords.first() {
        Some(ch) => crate::nav::STRINGS.format(
            ui,
            "command-tooltip",
            &[("command", c.label.as_str().into()), ("shortcut", ch.label(platform).into())],
        ),
        None => c.label.clone(),
    }
}

/// A toolbar showing the commands whose ids are in `slot`, in that order. The app keeps
/// `slot` (and saves it); [`toolbar_editor`] lets the user change it. Icon commands show as
/// icon buttons, others as text buttons; disabled ones are dimmed; tooltips show shortcuts.
#[track_caller]
pub fn toolbar(ui: &Ui, label: impl Into<String>, commands: &Commands, slot: Signal<Vec<String>>) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let bar = ui
        .el("toolbar")
        .row()
        .align(Align::Center)
        .gap(unit)
        .semantics(Semantics::new(Role::Toolbar).label(label));
    let (b2, u, cmds) = (bar.clone(), ui.clone(), commands.clone());
    bar.bind(move || {
        let ids = slot.get();
        let infos: Vec<CommandInfo> = ids.iter().filter_map(|id| cmds.info(id)).collect();
        untrack(|| {
            b2.clear();
            for c in infos {
                let (cmds2, id) = (cmds.clone(), c.id.clone());
                let run = move || {
                    cmds2.run(&id);
                };
                let b = match c.icon {
                    Some(icon) => w::icon_button(&u, icon, c.label.clone(), run).variant("quiet"),
                    None => w::button(&u, c.label.clone(), run).variant("quiet"),
                };
                let (u1, c1) = (u.clone(), c.clone());
                w::tooltip(&b, move || tip(&u1, &c1, u1.platform()));
                let _ = b2.clone().child(b.disabled(!c.enabled));
            }
        });
    })
}

/// Lets the user choose and order a toolbar's commands: a switch per command, and buttons to
/// move shown ones up and down. Changes go straight into `slot`.
#[track_caller]
pub fn toolbar_editor(ui: &Ui, commands: &Commands, slot: Signal<Vec<String>>) -> El {
    let unit = ui.theme().spacing.unit;
    let list = w::column(ui).gap(unit).fill_width();
    let (l2, u, cmds) = (list.clone(), ui.clone(), commands.clone());
    list.bind(move || {
        let shown = slot.get();
        let all = cmds.all();
        untrack(|| {
            l2.clear();
            // Shown commands first, in their order, then the rest.
            let mut ordered: Vec<CommandInfo> =
                shown.iter().filter_map(|id| all.iter().find(|c| c.id == *id).cloned()).collect();
            ordered.extend(all.iter().filter(|c| !shown.contains(&c.id)).cloned());
            for c in ordered {
                let on = Signal::new(shown.contains(&c.id));
                let id = c.id.clone();
                let sw = w::switch(&u, c.label.clone(), on);
                let (id1, id2, id3) = (id.clone(), id.clone(), id.clone());
                let first = std::cell::Cell::new(true);
                let sw = sw.bind(move || {
                    let v = on.get();
                    if first.replace(false) {
                        return;
                    }
                    let id = id1.clone();
                    untrack(|| {
                        slot.update(|s| {
                            if v && !s.contains(&id) {
                                s.push(id.clone());
                            } else if !v {
                                s.retain(|x| *x != id);
                            }
                        })
                    });
                });
                let shift = move |by: isize, id: String| {
                    slot.update(|s| {
                        if let Some(i) = s.iter().position(|x| *x == id) {
                            let j = (i as isize + by).clamp(0, s.len() as isize - 1) as usize;
                            s.swap(i, j);
                        }
                    })
                };
                let row = w::row(&u)
                    .align(Align::Center)
                    .gap(unit)
                    .fill_width()
                    .child(sw.grow(1.0))
                    .child(
                        w::icon_button(
                            &u,
                            Icon::ChevronUp,
                            move_label(&u, "toolbar-move-earlier", &c.label),
                            move || shift(-1, id2.clone()),
                        )
                        .disabled(!shown.contains(&c.id)),
                    )
                    .child(
                        w::icon_button(
                            &u,
                            Icon::ChevronDown,
                            move_label(&u, "toolbar-move-later", &c.label),
                            move || shift(1, id3.clone()),
                        )
                        .disabled(!shown.contains(&c.id)),
                    );
                let _ = l2.clone().child(row);
            }
        });
    })
}

/// The name of a button that moves the command labeled `label` (message `id`), following the
/// window's language.
fn move_label(ui: &Ui, id: &'static str, label: &str) -> impl Fn() -> String + 'static {
    let (ui, label) = (ui.clone(), label.to_owned());
    move || crate::nav::STRINGS.format(&ui, id, &[("command", label.as_str().into())])
}

/// How well `query` matches `text`: the query's characters in order, case-insensitively,
/// scoring runs, word starts and the match's start; `None` when they do not all appear.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0i32;
    let mut qi = 0usize;
    let mut last: Option<usize> = None;
    for (i, c) in t.iter().enumerate() {
        let Some(want) = q.get(qi) else { break };
        if c == want {
            let word_start = i == 0 || t.get(i - 1).is_some_and(|p| !p.is_alphanumeric());
            score += 1;
            if word_start {
                score += 8;
            }
            if last.is_some_and(|l| l + 1 == i) {
                score += 5;
            }
            if qi == 0 {
                score -= i.min(20) as i32 / 2;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

/// The command palette: a search box over every enabled command; arrows choose, Enter runs,
/// Escape closes. Open it from a command (Ctrl+Shift+P or Ctrl+K are the usual chords).
#[derive(Clone)]
pub struct CommandPalette {
    commands: Commands,
    ui: Ui,
    open: Rc<RefCell<Option<w::Modal>>>,
}

impl std::fmt::Debug for CommandPalette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CommandPalette(open {})", self.is_open())
    }
}

impl CommandPalette {
    /// A palette over `commands` in this window.
    pub fn new(ui: &Ui, commands: &Commands) -> CommandPalette {
        CommandPalette { commands: commands.clone(), ui: ui.clone(), open: Rc::default() }
    }

    /// True while shown.
    pub fn is_open(&self) -> bool {
        self.open.borrow().as_ref().is_some_and(w::Modal::is_open)
    }

    /// Closes it.
    pub fn close(&self) {
        if let Some(m) = self.open.borrow_mut().take() {
            m.close();
        }
    }

    /// Shows it (or closes it when shown: the same chord toggles).
    pub fn toggle(&self) {
        if self.is_open() {
            self.close();
            return;
        }
        let ui = &self.ui;
        let unit = ui.theme().spacing.unit;
        let query = Signal::new(String::new());
        let chosen = Signal::new(0usize);
        // The palette is built when it opens, in the window's language then.
        let field = ui
            .text_field("text_input", FieldKind::Single)
            .label(crate::nav::tr(ui, "palette-search"))
            .placeholder(crate::nav::tr(ui, "palette-placeholder"));
        let f2 = field.clone();
        let field = field.on_change(move || {
            query.set(f2.value());
            chosen.set(0);
        });
        let cmds = self.commands.clone();
        let matches = move || -> Vec<CommandInfo> {
            let q = query.get();
            let mut scored: Vec<(i32, usize, CommandInfo)> = cmds
                .all()
                .into_iter()
                .enumerate()
                .filter(|(_, c)| c.enabled)
                .filter_map(|(i, c)| fuzzy_score(&q, &c.label).map(|s| (s, i, c)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            scored.into_iter().map(|(_, _, c)| c).collect()
        };
        let matches = Rc::new(matches);
        let list = w::column(ui)
            .gap(unit / 2.0)
            .fill_width()
            .semantics(Semantics::new(Role::List).label(crate::nav::tr(ui, "palette-commands")));
        let (l2, u, m2, me) = (list.clone(), ui.clone(), matches.clone(), self.clone());
        let list = list.bind(move || {
            let found = m2();
            let at = chosen.get();
            untrack(|| {
                l2.clear();
                if found.is_empty() {
                    let _ = l2
                        .clone()
                        .child(w::text(&u, crate::nav::tr_fn(&u, "palette-no-match")).variant("muted"));
                }
                for (i, c) in found.iter().enumerate() {
                    let (me2, id) = (me.clone(), c.id.clone());
                    let shortcut = c.chords.first().map(|ch| ch.label(u.platform())).unwrap_or_default();
                    let row = w::min_target(u.el("list_item"))
                        .row()
                        .align(Align::Center)
                        .gap(unit * 2.0)
                        .padding(Insets::symmetric(unit, unit * 2.0))
                        .fill_width()
                        .child(w::text(&u, c.label.clone()).grow(1.0))
                        .child(w::text(&u, shortcut.clone()).variant("muted").data_text())
                        .on_click(move || {
                            me2.close();
                            me2.commands.run(&id);
                        })
                        .focusable(false)
                        .semantics(
                            Semantics::new(Role::ListItem)
                                .label(c.label.clone())
                                .selected(i == at)
                                .action(Action::Click),
                        );
                    row.state(remedy_ui_core::style::States::SELECTED, i == at);
                    let _ = l2.clone().child(row);
                }
            });
        });
        // Enter submits the field: run the chosen command.
        let (m4, me4) = (matches.clone(), self.clone());
        let field = field.on_submit(move || {
            let pick = m4().get(chosen.get_untracked().unwrap_or(0)).map(|c| c.id.clone());
            me4.close();
            if let Some(id) = pick {
                me4.commands.run(&id);
            }
        });
        let m3 = matches.clone();
        let field_el = field.el().fill_width().on_key(move |e| {
            if !e.pressed {
                return false;
            }
            let n = m3().len();
            match e.key {
                Key::ArrowDown if n > 0 => chosen.update(|c| *c = (*c + 1) % n),
                Key::ArrowUp if n > 0 => chosen.update(|c| *c = (*c + n - 1) % n),
                _ => return false,
            }
            true
        });
        let content = w::column(ui).gap(unit * 2.0).fill_width().child(field_el.clone()).child(
            w::scroll(ui).child(list).layout(|l| {
                l.max_height = remedy_ui_core::layout::Length::Px(360.0);
            }),
        );
        let m = w::modal(ui, crate::nav::tr(ui, "palette-title"), content, true);
        *self.open.borrow_mut() = Some(m);
        field_el.focus();
    }
}
