//! The navigation widgets on a real view with the theme sheet.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::input::{Buttons, Chord, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::{Point, Size};
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_text::FontCollection;
use remedy_ui_widgets::base::{self as w, Icon};
use remedy_ui_widgets::nav::*;

fn ui() -> Ui {
    let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    let _ = ui.take_requests();
}

fn key(ui: &Ui, k: Key, m: Modifiers) -> bool {
    let r = ui.key(&KeyEvent::press(k).with_modifiers(m).with_text(None));
    frame(ui);
    r
}

fn press(ui: &Ui, el: &El, at: Duration) {
    frame(ui);
    let b = el.bounds().unwrap_or_default();
    let c = Point::new((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0);
    let ev = |phase, buttons| {
        PointerEvent::new(phase, c)
            .with_buttons(buttons)
            .with_time(at)
            .with_modifiers(remedy_ui_core::input::Modifiers::NONE)
    };
    ui.pointer(&ev(PointerPhase::Down, Buttons::PRIMARY));
    ui.pointer(&ev(PointerPhase::Up, Buttons::default()));
    frame(ui);
}

/// The `i`th child.
fn nth(el: &El, i: usize) -> El {
    el.child_elements().get(i).cloned().unwrap_or_else(|| unreachable!("child {i}"))
}

const CTRL: Modifiers = Modifiers { ctrl: true, shift: false, alt: false, meta: false };

fn audit_clean(ui: &Ui) {
    frame(ui);
    let found: Vec<String> = remedy_ui_widgets::devtools::audit(ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                ui.element(p.element)
                    .info()
                    .map(|i| remedy_ui_widgets::devtools::describe(&i))
                    .unwrap_or_default()
            )
        })
        .collect();
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn commands_run_from_shortcuts_toolbars_and_the_palette() {
    let ui = ui();
    let cmds = Commands::new();
    let saved = Rc::new(Cell::new(0));
    let s2 = saved.clone();
    cmds.add(
        "file.save",
        "Save",
        vec![Chord::new(CTRL, Key::Character("s".into()))],
        Some(Icon::Check),
        move || s2.set(s2.get() + 1),
    );
    let shared = Rc::new(Cell::new(0));
    let sh2 = shared.clone();
    cmds.add("file.share", "Share folder", vec![], None, move || sh2.set(sh2.get() + 1));
    cmds.add(
        "view.settings",
        "Open settings",
        vec![Chord::new(CTRL, Key::Character(",".into()))],
        None,
        || {},
    );
    cmds.attach(&ui);
    let slot = Signal::new(vec!["file.save".to_owned(), "file.share".to_owned()]);
    let bar = toolbar(&ui, "Main", &cmds, slot);
    let editor = toolbar_editor(&ui, &cmds, slot);
    ui.root().child(bar.clone()).child(editor);
    frame(&ui);
    key(&ui, Key::Character("s".into()), CTRL);
    assert_eq!(saved.get(), 1, "the shortcut ran it");
    cmds.set_enabled("file.save", false);
    key(&ui, Key::Character("s".into()), CTRL);
    assert_eq!(saved.get(), 1, "disabled commands do not run");
    cmds.set_enabled("file.save", true);
    // Rebinding to a chord another command owns is refused.
    assert!(cmds.rebind("file.share", vec![Chord::new(CTRL, Key::Character(",".into()))]).is_err());
    assert!(cmds.rebind("file.share", vec![Chord::new(CTRL, Key::Character("h".into()))]).is_ok());
    key(&ui, Key::Character("h".into()), CTRL);
    assert_eq!(shared.get(), 1);
    // The toolbar follows its slot, in order.
    frame(&ui);
    assert_eq!(bar.child_elements().len(), 2);
    slot.set(vec!["file.share".to_owned()]);
    frame(&ui);
    assert_eq!(bar.child_elements().len(), 1);
    press(&ui, &nth(&bar, 0), Duration::from_secs(1));
    assert_eq!(shared.get(), 2, "a toolbar button runs its command");
    // Fuzzy matching prefers word starts and runs.
    assert!(fuzzy_score("ss", "Save settings") > fuzzy_score("ss", "Unassigned"));
    assert_eq!(fuzzy_score("xyz", "Save"), None);
    let palette = CommandPalette::new(&ui, &cmds);
    palette.toggle();
    assert!(palette.is_open());
    ui.paste("share");
    frame(&ui);
    key(&ui, Key::Enter, Modifiers::NONE);
    assert_eq!(shared.get(), 3, "Enter ran the best match");
    assert!(!palette.is_open());
    audit_clean(&ui);
}

#[test]
fn tabs_move_with_arrows_keep_one_tab_stop_and_close() {
    let ui = ui();
    let items = Signal::new(vec![
        Tab::new("a", "Pictures").closable(),
        Tab::new("b", "Documents").closable(),
        Tab::new("c", "Music"),
    ]);
    let selected = Signal::new(Some("a".to_owned()));
    let strip = tabs(&ui, "Shares", items, selected, move |id| items.update(|t| t.retain(|x| x.id != id)));
    ui.root().child(strip.clone());
    frame(&ui);
    let focusables =
        || strip.child_elements().iter().filter(|t| t.info().is_some_and(|i| i.focusable)).count();
    assert_eq!(focusables(), 1, "one tab stop");
    nth(&strip, 0).focus();
    key(&ui, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(selected.get().as_deref(), Some("b"));
    assert!(nth(&strip, 1).is_focused(), "focus followed the selection");
    key(&ui, Key::End, Modifiers::NONE);
    assert_eq!(selected.get().as_deref(), Some("c"));
    key(&ui, Key::Home, Modifiers::NONE);
    key(&ui, Key::Delete, Modifiers::NONE);
    assert_eq!(items.get().len(), 2, "Delete closed the focused closable tab");
    audit_clean(&ui);
}

#[test]
fn rails_breadcrumbs_and_stacks_navigate() {
    let ui = ui();
    let place = Signal::new("files".to_owned());
    let collapsed = Signal::new(false);
    let rail = nav_rail(
        &ui,
        "Places",
        vec![NavEntry::new("files", Icon::Note, "Files"), NavEntry::new("chat", Icon::Menu, "Chat")],
        place,
        collapsed,
    );
    let path = Signal::new(vec![
        "Home".to_owned(),
        "Pictures".to_owned(),
        "2026".to_owned(),
        "Trips".to_owned(),
        "Rome".to_owned(),
    ]);
    let went = Rc::new(Cell::new(usize::MAX));
    let g2 = went.clone();
    let crumbs = breadcrumbs(&ui, "Path", path, move |i| g2.set(i));
    let first = w::column(&ui).child(w::text_input(&ui, "Filter").el());
    let stack = NavStack::new(&ui, "Files", first.clone());
    ui.root().child(rail.clone()).child(crumbs.clone()).child(stack.el());
    frame(&ui);
    nth(&rail, 0).focus();
    key(&ui, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(place.get(), "chat");
    frame(&ui);
    let current = |i: usize| nth(&rail, i).info().and_then(|i| i.semantics).and_then(|s| s.current_state());
    assert_eq!(
        (current(0), current(1)),
        (None, Some(remedy_ui_core::a11y::Current::Page)),
        "the chosen place is the current page"
    );
    collapsed.set(true);
    frame(&ui);
    // Five segments fold to Home › … › Trips › Rome.
    let shown: Vec<String> = crumbs
        .child_elements()
        .iter()
        .filter_map(|c| c.info().and_then(|i| i.semantics.and_then(|s| s.name().map(str::to_owned))))
        .collect();
    assert!(
        shown.iter().any(|s| s == "Home")
            && shown.iter().any(|s| s == "More locations")
            && !shown.iter().any(|s| s == "Pictures"),
        "the folded segments' menu shows … and says what it holds: {shown:?}"
    );
    assert!(texts_under(&crumbs).iter().any(|t| t == "…"), "{:?}", texts_under(&crumbs));
    press(&ui, &nth(&crumbs, 0), Duration::from_secs(1));
    assert_eq!(went.get(), 0, "a crumb goes back there");
    // The stack keeps the page below as it was.
    let field = remedy_ui_widgets::base::text_input(&ui, "Name");
    stack.push("Details", w::column(&ui).child(field.el()));
    assert_eq!((stack.depth(), stack.title()), (2, "Details".to_owned()));
    key(&ui, Key::ArrowLeft, Modifiers { alt: true, ..Modifiers::NONE });
    assert_eq!((stack.depth(), stack.title()), (1, "Files".to_owned()));
    assert!(!stack.pop(), "the first page stays");
    audit_clean(&ui);
}

#[test]
fn drawers_trap_focus_close_on_escape_and_return_focus() {
    let ui = ui();
    let open = Signal::new(false);
    let opener = w::button(&ui, "Menu", move || open.set(true));
    let holder =
        drawer(&ui, "Menu", Edge::Start, open, w::column(&ui).child(w::button(&ui, "Settings", || {})));
    ui.root().child(opener.clone()).child(holder);
    frame(&ui);
    opener.focus();
    open.set(true);
    frame(&ui);
    assert_eq!(ui.layers()[1].child_elements().len(), 1, "the drawer is in the overlay layer");
    assert!(!opener.is_focused(), "focus moved into the drawer");
    key(&ui, Key::Escape, Modifiers::NONE);
    assert!(!open.get());
    assert!(ui.layers()[1].child_elements().is_empty());
    assert!(opener.is_focused(), "focus came back");
}

#[test]
fn drawers_close_on_a_press_outside_and_offer_no_dead_controls() {
    let ui = ui();
    let open = Signal::new(true);
    let inner = w::text(&ui, "Rooms");
    let holder = drawer(&ui, "Rooms", Edge::Start, open, w::column(&ui).child(inner.clone()));
    let away = w::button(&ui, "Elsewhere", || {}).layout(|l| {
        l.position = remedy_ui_core::layout::Position::Absolute;
        l.inset = remedy_ui_core::Insets {
            top: 10.0,
            start: 700.0,
            end: remedy_ui_core::layout::AUTO,
            bottom: remedy_ui_core::layout::AUTO,
        };
    });
    ui.root().child(holder).child(away.clone());
    frame(&ui);
    audit_clean(&ui);
    press(&ui, &inner, Duration::from_secs(1));
    assert!(open.get(), "a press inside the panel keeps it open");
    press(&ui, &away, Duration::from_secs(3));
    assert!(!open.get(), "a press on the scrim closes it");
}

#[test]
fn title_bars_move_on_press_and_maximize_on_double_click() {
    let ui = ui();
    let (moves, maxes) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (m2, x2) = (moves.clone(), maxes.clone());
    let maximized = Signal::new(false);
    let bar = title_bar(
        &ui,
        Signal::new("Hearth".to_owned()),
        None,
        TitleBarActions {
            begin_move: Box::new(move || m2.set(m2.get() + 1)),
            minimize: Box::new(|| {}),
            toggle_maximize: Box::new(move || {
                x2.set(x2.get() + 1);
                maximized.update(|m| *m = !*m);
            }),
            is_maximized: Box::new(move || maximized.get()),
            close: Box::new(|| {}),
        },
    );
    ui.root().child(bar.clone());
    ui.set_double_click_time(Duration::from_millis(400));
    frame(&ui);
    let title = nth(&bar, 0).clone();
    press(&ui, &title, Duration::from_millis(1000));
    assert_eq!((moves.get(), maxes.get()), (1, 0));
    press(&ui, &title, Duration::from_millis(1200));
    assert_eq!((moves.get(), maxes.get()), (1, 1), "a double click maximizes");
    press(&ui, &title, Duration::from_millis(3000));
    assert_eq!(moves.get(), 2, "a later press moves again");
    frame(&ui);
    let tree = format!("{:?}", {
        ui.connect_accessibility();
        frame(&ui);
        ui.take_accessibility_update()
    });
    assert!(tree.contains("Restore"), "the maximize button says Restore while maximized");
    audit_clean(&ui);
}

#[test]
fn pages_pushed_from_a_binding_keep_their_bindings_while_on_the_stack() {
    let ui = ui();
    let stack = NavStack::new(&ui, "Inbox", w::text(&ui, "Inbox"));
    ui.root().child(stack.el());
    frame(&ui);
    // An app pushes the page for the route it follows; the page reads its own signal.
    let route = Signal::new(0u32);
    let unread = Signal::new(3u32);
    let pushed = Rc::new(Cell::new(false));
    let (s2, u2, p2) = (stack.clone(), ui.clone(), pushed.clone());
    let follower = ui.el("route").hidden(true).bind(move || {
        let r = route.get();
        remedy_ui_core::reactive::untrack(|| {
            if r > 0 && !p2.replace(true) {
                s2.push("Thread", w::text(&u2, move || format!("{} unread", unread.get())));
            }
        });
    });
    ui.root().child(follower);
    route.set(1);
    frame(&ui);
    // The route binding runs again (another route change) while the page stays on the stack.
    route.set(2);
    unread.set(5);
    frame(&ui);
    let texts: Vec<String> = texts_under(&stack.el());
    assert!(texts.iter().any(|t| t == "5 unread"), "the page still follows its signal: {texts:?}");
}

/// Every text shown under `el`.
fn texts_under(el: &El) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if let Some(t) = e.info().and_then(|i| i.text) {
            out.push(t);
        }
        stack.extend(e.child_elements());
    }
    out
}

/// Runs two frames at `size` (overlays measure on the first and take their place on the
/// next), firing timers that came due.
fn frames_at(ui: &Ui, size: Size) {
    for _ in 0..2 {
        let _ = ui.frame(size, 1.0);
        let _ = ui.take_requests();
    }
}

fn audit_clean_at(ui: &Ui, size: Size) {
    frames_at(ui, size);
    let found: Vec<String> = remedy_ui_widgets::devtools::audit(ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                ui.element(p.element)
                    .info()
                    .map(|i| remedy_ui_widgets::devtools::describe(&i))
                    .unwrap_or_default()
            )
        })
        .collect();
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn menus_made_from_commands_follow_the_commands() {
    let ui = ui();
    ui.set_locale("de-DE");
    let cmds = Commands::new();
    let saved = Rc::new(Cell::new(0));
    let s2 = saved.clone();
    cmds.add("file.save", "Speichern", vec![Chord::new(CTRL, Key::char('s'))], None, move || {
        s2.set(s2.get() + 1)
    });
    cmds.add(
        "file.delete",
        "Löschen",
        vec![Chord::new(Modifiers::NONE, Key::Delete)],
        Some(Icon::Trash),
        || {},
    );
    cmds.set_destructive("file.delete", true);
    let bar = w::menu_bar(
        &ui,
        vec![
            w::MenuItem::submenu_with("Datei", {
                let cmds = cmds.clone();
                move || {
                    vec![
                        cmds.menu_item("file.save").mnemonic('s'),
                        w::MenuItem::separator(),
                        cmds.menu_item("file.delete").mnemonic('l'),
                    ]
                }
            })
            .mnemonic('d'),
        ],
    );
    ui.root().child(bar);
    frames_at(&ui, Size::new(640.0, 400.0));
    let node = |name: &str| ui.reader_view().into_iter().find(|n| n.name.as_deref() == Some(name));
    assert!(ui.key(&KeyEvent::press(Key::F(10))));
    assert!(ui.key(&KeyEvent::press(Key::Enter)));
    frames_at(&ui, Size::new(640.0, 400.0));
    assert_eq!(node("Speichern").and_then(|n| n.shortcut).as_deref(), Some("Strg+S"));
    assert_eq!(node("Löschen").and_then(|n| n.shortcut).as_deref(), Some("Entf"));
    audit_clean_at(&ui, Size::new(640.0, 400.0));
    // Disabled, renamed and rebound commands show so in the menu the next time it opens.
    assert!(ui.key(&KeyEvent::press(Key::Escape)));
    assert!(ui.key(&KeyEvent::press(Key::Escape)));
    cmds.set_enabled("file.save", false);
    cmds.set_label("file.delete", "In den Papierkorb");
    let _ = cmds.rebind("file.delete", vec![Chord::new(CTRL, Key::Delete)]);
    assert!(ui.key(&KeyEvent::press(Key::F(10))));
    assert!(ui.key(&KeyEvent::press(Key::Enter)));
    frames_at(&ui, Size::new(640.0, 400.0));
    let save = node("Speichern").unwrap_or_else(|| unreachable!());
    assert!(save.disabled);
    assert_eq!(node("In den Papierkorb").and_then(|n| n.shortcut).as_deref(), Some("Strg+Entf"));
    assert!(ui.key(&KeyEvent::press(Key::Enter)));
    assert_eq!(saved.get(), 0, "a disabled command does not run from its menu");
}

#[test]
fn open_menus_pass_the_audit_in_narrow_windows_and_long_languages() {
    for size in [Size::new(640.0, 400.0), Size::new(180.0, 220.0)] {
        let ui = ui();
        ui.set_locale("de-DE");
        let view = Signal::new(1usize);
        let bar = w::menu_bar(
            &ui,
            vec![
                w::MenuItem::submenu(
                    "Datei",
                    vec![
                        w::MenuItem::new("Öffnen…", || {})
                            .chord(Chord::new(CTRL, Key::char('o')))
                            .mnemonic('f'),
                        w::MenuItem::new("Speichern unter…", || {}).mnemonic('u'),
                    ],
                )
                .mnemonic('d'),
                w::MenuItem::submenu(
                    "Ansicht",
                    vec![
                        w::MenuItem::radio_signal("An Seitenbreite anpassen", view, 1),
                        w::MenuItem::radio_signal("Ganze Seite", view, 2),
                        w::MenuItem::separator(),
                        w::MenuItem::submenu(
                            "Sichtbarkeitsdesign",
                            vec![w::MenuItem::new("Hoher Kontrast (Schwarz auf Weiß)", || {})],
                        ),
                    ],
                )
                .mnemonic('a'),
                w::MenuItem::submenu("Hilfe", vec![w::MenuItem::new("Über RemedyPDF", || {})]).mnemonic('h'),
            ],
        );
        ui.root().child(bar);
        frames_at(&ui, size);
        frames_at(&ui, size);
        audit_clean_at(&ui, size);
        // Open View and its submenu from the keyboard, then check every edge again.
        assert!(ui.key(&KeyEvent::press(Key::char('a')).with_modifiers(Modifiers::ALT)));
        frames_at(&ui, size);
        for _ in 0..3 {
            let _ = ui.key(&KeyEvent::press(Key::ArrowUp));
        }
        let _ = ui.key(&KeyEvent::press(Key::End));
        let _ = ui.key(&KeyEvent::press(Key::ArrowRight));
        frames_at(&ui, size);
        let menus =
            ui.reader_view().into_iter().filter(|n| n.role == remedy_ui_core::a11y::Role::Menu).count();
        assert!(menus >= 2, "the submenu opened at {size:?}");
        audit_clean_at(&ui, size);
    }
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The accessibility tree as text, without the isolation marks placeholders get in messages.
fn spoken(ui: &Ui) -> String {
    frame(ui);
    format!("{:?}", ui.take_accessibility_update()).replace("\\u{2068}", "").replace("\\u{2069}", "")
}

#[test]
fn built_in_labels_follow_the_window_language() {
    let ui = ui();
    ui.connect_accessibility();
    let bar = title_bar(
        &ui,
        Signal::new("Hearth".to_owned()),
        None,
        TitleBarActions {
            begin_move: Box::new(|| {}),
            minimize: Box::new(|| {}),
            toggle_maximize: Box::new(|| {}),
            is_maximized: Box::new(|| false),
            close: Box::new(|| {}),
        },
    );
    let items = Signal::new(vec![Tab::new("a", "Pictures").closable()]);
    let strip = tabs(&ui, "Shares", items, Signal::new(Some("a".to_owned())), |_| {});
    let path = Signal::new(["Home", "Pictures", "2024", "Trips", "Rome"].map(str::to_owned).to_vec());
    let crumbs = breadcrumbs(&ui, "Path", path, |_| {});
    ui.root().child(bar).child(strip).child(crumbs);
    let tree = spoken(&ui);
    for s in ["Title bar", "Maximize", "Close Pictures", "More locations", "Rome, current"] {
        assert!(tree.contains(s), "{s}: {tree}");
    }
    ui.set_locale("de");
    let tree = spoken(&ui);
    for s in ["Titelleiste", "Maximieren", "Pictures schließen", "Weitere Orte", "Rome, aktuell"] {
        assert!(tree.contains(s), "{s}: {tree}");
    }
    assert!(!tree.contains("Title bar") && !tree.contains("More locations"), "{tree}");
    audit_clean(&ui);
}
