//! Menus and the menu bar, driven by keyboard and pointer on a real view with the theme
//! sheet, and checked through the accessibility tree as a screen reader meets them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_a11y::{ReaderNode, Role};
use remedy_ui_core::{Point, Rect, Size};
use remedy_ui_input::{Buttons, Chord, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_reactive::Signal;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, FieldKind, Request, Ui};
use remedy_ui_widgets_core::{self as w, MenuItem};

struct T {
    ui: Ui,
    size: Size,
    timers: Vec<u64>,
}

impl T {
    fn new() -> T {
        T::sized(640.0, 420.0)
    }

    fn sized(width: f32, height: f32) -> T {
        let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        let ui = Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts);
        T { ui, size: Size::new(width, height), timers: Vec::new() }
    }

    fn frame(&mut self) {
        // Twice: overlays measure themselves on the first and take their place on the next.
        for _ in 0..2 {
            let _ = self.ui.frame(self.size, 1.0);
            for r in self.ui.take_requests() {
                match r {
                    Request::Timer { id, .. } => self.timers.push(id),
                    Request::CancelTimer(id) => self.timers.retain(|t| *t != id),
                    _ => {}
                }
            }
        }
    }

    /// Fires every pending timer, as the runner does when they come due.
    fn fire(&mut self) {
        self.frame();
        for id in std::mem::take(&mut self.timers) {
            self.ui.fire_timer(id);
        }
        self.frame();
    }

    fn send(&mut self, e: KeyEvent) -> bool {
        let r = self.ui.key(&e);
        self.frame();
        r
    }

    fn key(&mut self, k: Key) -> bool {
        self.send(KeyEvent::press(k).with_text(None))
    }

    fn type_char(&mut self, c: char) -> bool {
        self.send(KeyEvent::press(Key::char(c)))
    }

    fn alt(&mut self, c: char) -> bool {
        let alt = Key::Named("Alt".into());
        let _ = self.ui.key(&KeyEvent::press(alt.clone()).with_modifiers(Modifiers::ALT));
        let r = self.ui.key(&KeyEvent::press(Key::char(c)).with_modifiers(Modifiers::ALT));
        let _ = self.ui.key(&KeyEvent::release(alt));
        self.frame();
        r
    }

    fn alt_tap(&mut self) -> bool {
        let alt = Key::Named("Alt".into());
        let _ = self.ui.key(&KeyEvent::press(alt.clone()).with_modifiers(Modifiers::ALT));
        let r = self.ui.key(&KeyEvent::release(alt));
        self.frame();
        r
    }

    fn pointer(&mut self, phase: PointerPhase, at: Point, buttons: Buttons) {
        let e = PointerEvent::new(phase, at).with_buttons(buttons).with_modifiers(Modifiers::NONE);
        let _ = self.ui.pointer(&e);
        self.frame();
    }

    fn click_at(&mut self, at: Point) {
        self.pointer(PointerPhase::Down, at, Buttons::PRIMARY);
        self.pointer(PointerPhase::Up, at, Buttons::default());
    }

    fn reader(&self) -> Vec<ReaderNode> {
        self.ui.reader_view()
    }

    fn node(&self, role: Role, name: &str) -> Option<ReaderNode> {
        self.reader().into_iter().find(|n| n.role == role && n.name.as_deref() == Some(name))
    }

    fn el(&self, role: Role, name: &str) -> El {
        self.node(role, name)
            .and_then(|n| self.ui.element_by_node_id(n.id))
            .unwrap_or_else(|| unreachable!("no {role:?} {name:?}: {:#?}", self.names()))
    }

    fn bounds(&self, role: Role, name: &str) -> Rect {
        self.el(role, name).bounds().unwrap_or_default()
    }

    fn focused(&self) -> Option<String> {
        let f = self.ui.tree().focused()?;
        self.reader()
            .into_iter()
            .find(|n| self.ui.element_by_node_id(n.id).is_some_and(|e| e.id() == f))?
            .name
    }

    fn menus(&self) -> Vec<Rect> {
        self.reader().into_iter().filter(|n| n.role == Role::Menu).filter_map(|n| n.bounds).collect()
    }

    fn names(&self) -> Vec<String> {
        self.reader().into_iter().filter_map(|n| n.name.map(|s| format!("{:?} {s}", n.role))).collect()
    }
}

/// The `i`th of `v`, which the test expects to be there.
fn nth(v: &[Rect], i: usize) -> Rect {
    v.get(i).copied().unwrap_or_else(|| unreachable!("menu {i} of {v:?}"))
}

fn inside(r: Rect, s: Size) -> bool {
    r.min.x >= -0.5 && r.min.y >= -0.5 && r.max.x <= s.width + 0.5 && r.max.y <= s.height + 0.5
}

fn ctrl(c: char) -> Chord {
    Chord::new(Modifiers::CTRL, Key::char(c))
}

/// A View menu as a PDF reader has one.
fn view_menu(log: &Rc<RefCell<Vec<String>>>, book: Signal<bool>, fit: Signal<&'static str>) -> Vec<MenuItem> {
    let say = |s: &'static str| {
        let l = log.clone();
        move || l.borrow_mut().push(s.to_owned())
    };
    vec![
        MenuItem::new("Zoom In", say("zoom in"))
            .chord(Chord::new(Modifiers::CTRL, Key::char('+')))
            .mnemonic('i'),
        MenuItem::new("Zoom Out", say("zoom out")).chord(ctrl('-')).mnemonic('o'),
        MenuItem::separator(),
        MenuItem::radio_signal("Fit Width", fit, "width").mnemonic('w'),
        MenuItem::radio_signal("Fit Page", fit, "page").mnemonic('p'),
        MenuItem::separator(),
        MenuItem::check_signal("Book Mode", book).chord(ctrl('b')).mnemonic('b'),
        MenuItem::new("Presentation", say("present")).disabled(true),
        MenuItem::submenu(
            "Page Appearance",
            vec![
                MenuItem::new("Light", say("light")).mnemonic('l'),
                MenuItem::new("Sepia", say("sepia")).mnemonic('s'),
                MenuItem::submenu("More", vec![MenuItem::new("Night", say("night"))]),
            ],
        )
        .mnemonic('a'),
    ]
}

#[test]
fn menu_entries_tell_assistive_technology_what_they_are() {
    let mut t = T::new();
    let log = Rc::default();
    let (book, fit) = (Signal::new(true), Signal::new("width"));
    let anchor = w::button(&t.ui, "View", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let _ = w::menu(&t.ui, &anchor, view_menu(&log, book, fit));
    t.frame();
    let zoom = t.node(Role::MenuItem, "Zoom In").unwrap_or_else(|| unreachable!("{:#?}", t.names()));
    assert_eq!(zoom.shortcut.as_deref(), Some("Ctrl++"));
    assert_eq!(zoom.access_key.as_deref(), Some("I"));
    assert_eq!(zoom.position, Some((1, 7)), "separators are not counted in the set");
    assert!(zoom.announcement().contains("Ctrl++"), "{}", zoom.announcement());
    let seps = t.reader().into_iter().filter(|n| n.role == Role::Separator).count();
    assert_eq!(seps, 2);
    assert_eq!(t.node(Role::MenuItemRadio, "Fit Width").and_then(|n| n.checked), Some(true));
    assert_eq!(t.node(Role::MenuItemRadio, "Fit Page").and_then(|n| n.checked), Some(false));
    assert_eq!(t.node(Role::MenuItemCheckBox, "Book Mode").and_then(|n| n.checked), Some(true));
    let present = t.node(Role::MenuItem, "Presentation").unwrap_or_else(|| unreachable!());
    assert!(present.disabled && present.states.contains(&"disabled"));
    let appearance = t.node(Role::MenuItem, "Page Appearance").unwrap_or_else(|| unreachable!());
    assert_eq!(appearance.expanded, Some(false), "a submenu entry says it opens one");
}

#[test]
fn the_keyboard_walks_menus_and_submenus_and_gives_focus_back() {
    let mut t = T::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (book, fit) = (Signal::new(false), Signal::new("width"));
    let anchor = w::button(&t.ui, "View", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    anchor.focus();
    let layer = w::menu(&t.ui, &anchor, view_menu(&log, book, fit));
    t.frame();
    assert_eq!(t.focused().as_deref(), Some("Zoom In"), "the first entry takes focus");
    t.key(Key::ArrowDown);
    t.key(Key::ArrowDown);
    assert_eq!(t.focused().as_deref(), Some("Fit Width"), "Down passes over the separator");
    t.key(Key::End);
    assert_eq!(t.focused().as_deref(), Some("Page Appearance"));
    t.key(Key::ArrowUp);
    assert_eq!(t.focused().as_deref(), Some("Presentation"), "a disabled entry takes focus");
    t.key(Key::Enter);
    assert!(t.ui.tree().contains(layer.id()) && log.borrow().is_empty(), "but does nothing");
    t.key(Key::ArrowDown);
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("Light"), "Right opens the submenu and enters it");
    assert_eq!(t.node(Role::MenuItem, "Page Appearance").and_then(|n| n.expanded), Some(true));
    assert_eq!(t.menus().len(), 2);
    t.key(Key::ArrowLeft);
    assert_eq!(t.focused().as_deref(), Some("Page Appearance"), "Left closes it");
    assert_eq!(t.menus().len(), 1);
    t.key(Key::Enter);
    t.key(Key::ArrowDown);
    t.key(Key::ArrowDown);
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("Night"), "a third level opens");
    t.key(Key::Escape);
    assert_eq!(t.focused().as_deref(), Some("More"), "Escape closes one level");
    assert_eq!(t.menus().len(), 2);
    t.key(Key::Escape);
    t.key(Key::Escape);
    assert!(!t.ui.tree().contains(layer.id()), "and at the top, the menu");
    assert!(anchor.is_focused(), "focus goes back where it was");
}

#[test]
fn check_and_radio_entries_follow_and_set_their_signals() {
    let mut t = T::new();
    let log = Rc::default();
    let (book, fit) = (Signal::new(false), Signal::new("width"));
    let anchor = w::button(&t.ui, "View", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let _ = w::menu(&t.ui, &anchor, view_menu(&log, book, fit));
    t.frame();
    t.type_char('b');
    assert!(book.get(), "its access key chose Book Mode");
    assert!(t.menus().is_empty(), "choosing closes the menu");
    let _ = w::menu(&t.ui, &anchor, view_menu(&log, book, fit));
    t.frame();
    assert_eq!(t.node(Role::MenuItemCheckBox, "Book Mode").and_then(|n| n.checked), Some(true));
    t.click_at(t.bounds(Role::MenuItemRadio, "Fit Page").center());
    assert_eq!(fit.get(), "page", "a click chose the radio entry");
    let _ = w::menu(&t.ui, &anchor, view_menu(&log, book, fit));
    t.frame();
    assert_eq!(t.node(Role::MenuItemRadio, "Fit Page").and_then(|n| n.checked), Some(true));
    assert_eq!(t.node(Role::MenuItemRadio, "Fit Width").and_then(|n| n.checked), Some(false));
}

#[test]
fn letters_choose_by_access_key_or_move_by_first_letter() {
    let mut t = T::new();
    let chosen: Rc<RefCell<Vec<&str>>> = Rc::default();
    let pick = |s: &'static str| {
        let c = chosen.clone();
        move || c.borrow_mut().push(s)
    };
    let items = || {
        vec![
            MenuItem::new("Paste", pick("paste")),
            MenuItem::new("Print…", pick("print")),
            MenuItem::new("Properties", pick("properties")),
            MenuItem::new("Save", pick("save")).mnemonic('s'),
        ]
    };
    let anchor = w::button(&t.ui, "Edit", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let _ = w::menu(&t.ui, &anchor, items());
    t.frame();
    t.type_char('p');
    assert_eq!(t.focused().as_deref(), Some("Print…"), "the next entry starting with P");
    t.type_char('p');
    assert_eq!(t.focused().as_deref(), Some("Properties"));
    t.type_char('p');
    assert_eq!(t.focused().as_deref(), Some("Paste"), "around again");
    assert!(chosen.borrow().is_empty(), "first letters only move");
    // Alt with the access key works in a menu too.
    t.send(KeyEvent::press(Key::char('s')).with_modifiers(Modifiers::ALT));
    assert_eq!(chosen.borrow().as_slice(), &["save"]);
}

#[test]
fn resting_on_a_submenu_entry_opens_it_and_moving_on_closes_it() {
    let mut t = T::new();
    let log = Rc::default();
    let anchor = w::button(&t.ui, "View", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let _ = w::menu(&t.ui, &anchor, view_menu(&log, Signal::new(false), Signal::new("width")));
    t.frame();
    let sub = t.bounds(Role::MenuItem, "Page Appearance").center();
    t.pointer(PointerPhase::Move, sub, Buttons::default());
    assert_eq!(t.menus().len(), 1, "not at once");
    t.fire();
    assert_eq!(t.menus().len(), 2, "after a moment");
    assert_eq!(t.focused().as_deref(), Some("Page Appearance"), "focus stays on the entry");
    // Into the submenu: the parent's pending close is called off.
    t.pointer(PointerPhase::Move, t.bounds(Role::MenuItem, "Sepia").center(), Buttons::default());
    t.fire();
    assert_eq!(t.menus().len(), 2);
    t.pointer(PointerPhase::Move, t.bounds(Role::MenuItem, "Zoom In").center(), Buttons::default());
    t.fire();
    assert_eq!(t.menus().len(), 1, "resting on a sibling closes the submenu");
}

#[test]
fn menus_near_the_edges_stay_inside_the_window() {
    let mut t = T::sized(360.0, 300.0);
    // A button in the bottom-right corner, its labels long (German).
    let anchor = w::button(&t.ui, "Ansicht", || {});
    t.ui.root().child(
        w::column(&t.ui)
            .fill_width()
            .height(300.0)
            .justify(remedy_ui_view::Justify::End)
            .align(remedy_ui_view::Align::End)
            .child(anchor.clone()),
    );
    t.frame();
    let items = vec![
        MenuItem::new("Seitenansicht vergrößern", || {}).chord(Chord::new(Modifiers::CTRL, Key::char('+'))),
        MenuItem::submenu(
            "Seitendarstellung",
            vec![
                MenuItem::new("Hell", || {}),
                MenuItem::new("Sepia (warmes Papier)", || {}),
                MenuItem::new("Nachtmodus mit gedämpften Farben", || {}),
            ],
        ),
    ];
    let _ = w::menu(&t.ui, &anchor, items);
    t.frame();
    let size = t.size;
    let first = t.menus();
    assert_eq!(first.len(), 1);
    assert!(inside(nth(&first, 0), size), "the menu {:?} fits in {size:?}", nth(&first, 0));
    let a = anchor.bounds().unwrap_or_default();
    assert!(nth(&first, 0).max.y <= a.min.y + 0.5, "no room below: it opens above the button");
    t.key(Key::End);
    t.key(Key::ArrowRight);
    let menus = t.menus();
    assert_eq!(menus.len(), 2);
    assert!(inside(nth(&menus, 1), size), "the submenu {:?} fits in {size:?}", nth(&menus, 1));
    assert!(nth(&menus, 1).min.x < nth(&first, 0).min.x, "no room on the right: it flips to the left");
}

#[test]
fn a_menu_taller_than_the_window_scrolls_inside_it() {
    let mut t = T::sized(300.0, 200.0);
    let anchor = w::button(&t.ui, "Fonts", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let items = (0..40).map(|i| MenuItem::new(format!("Font {i}"), || {})).collect();
    let _ = w::menu(&t.ui, &anchor, items);
    t.frame();
    let m = t.menus();
    assert!(inside(nth(&m, 0), t.size), "{:?}", nth(&m, 0));
}

#[test]
fn shortcuts_are_written_in_the_persons_language() {
    let mut t = T::new();
    t.ui.set_locale("de-DE");
    let anchor = w::button(&t.ui, "Bearbeiten", || {});
    t.ui.root().child(anchor.clone());
    t.frame();
    let redo = Chord::new(Modifiers { ctrl: true, shift: true, ..Modifiers::NONE }, Key::char('z'));
    let _ = w::menu(
        &t.ui,
        &anchor,
        vec![
            MenuItem::new("Wiederholen", || {}).chord(redo),
            MenuItem::new("Löschen", || {}).chord(Chord::new(Modifiers::NONE, Key::Delete)),
        ],
    );
    t.frame();
    assert_eq!(
        t.node(Role::MenuItem, "Wiederholen").and_then(|n| n.shortcut).as_deref(),
        Some("Strg+Umschalt+Z")
    );
    assert_eq!(t.node(Role::MenuItem, "Löschen").and_then(|n| n.shortcut).as_deref(), Some("Entf"));
}

/// A window with a text field and a File / Edit / View / Help bar.
fn bar_window(t: &T, log: &Rc<RefCell<Vec<String>>>) -> (El, El) {
    let say = |s: &'static str| {
        let l = log.clone();
        move || l.borrow_mut().push(s.to_owned())
    };
    let bar = w::menu_bar(
        &t.ui,
        vec![
            MenuItem::submenu(
                "File",
                vec![
                    MenuItem::new("Open…", say("open")).chord(ctrl('o')).mnemonic('o'),
                    MenuItem::new("Save", say("save")).chord(ctrl('s')).mnemonic('s'),
                    MenuItem::separator(),
                    MenuItem::new("Quit", say("quit")).chord(ctrl('q')).mnemonic('q'),
                ],
            )
            .mnemonic('f'),
            MenuItem::submenu(
                "Edit",
                vec![MenuItem::new("Find…", say("find")).chord(ctrl('f')).mnemonic('f')],
            )
            .mnemonic('e'),
            MenuItem::submenu(
                "View",
                vec![MenuItem::submenu("UI Scale", vec![MenuItem::new("100%", say("100"))]).mnemonic('u')],
            )
            .mnemonic('v'),
            MenuItem::submenu("Help", vec![MenuItem::new("About", say("about")).mnemonic('a')]).mnemonic('h'),
        ],
    );
    let field = t.ui.text_field("text_input", FieldKind::Single).label("Notes").el();
    t.ui.root().child(w::column(&t.ui).fill_width().child(bar.clone()).child(field.clone()));
    (bar, field)
}

#[test]
fn the_menu_bar_answers_f10_and_the_arrows() {
    let mut t = T::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (_bar, field) = bar_window(&t, &log);
    t.frame();
    assert!(t.reader().iter().any(|n| n.role == Role::MenuBar));
    field.focus();
    t.key(Key::F(10));
    assert_eq!(t.focused().as_deref(), Some("File"), "F10 reaches the bar from the field");
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("Edit"));
    t.key(Key::ArrowLeft);
    t.key(Key::ArrowLeft);
    assert_eq!(t.focused().as_deref(), Some("Help"), "around the end");
    t.key(Key::Home);
    t.key(Key::ArrowDown);
    assert_eq!(t.focused().as_deref(), Some("Open…"), "Down opens the menu at its first entry");
    assert_eq!(t.node(Role::MenuItem, "File").and_then(|n| n.expanded), Some(true));
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("Find…"), "Right opens the next menu");
    assert_eq!(t.node(Role::MenuItem, "File").and_then(|n| n.expanded), Some(false));
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("UI Scale"));
    t.key(Key::ArrowRight);
    assert_eq!(t.focused().as_deref(), Some("100%"), "on a submenu entry, Right opens it");
    assert_eq!(t.menus().len(), 2);
    t.key(Key::Escape);
    assert_eq!(t.focused().as_deref(), Some("UI Scale"));
    t.key(Key::Escape);
    assert_eq!(t.focused().as_deref(), Some("View"), "Escape closes the menu; its title keeps focus");
    assert!(t.menus().is_empty());
    t.key(Key::Escape);
    assert!(field.is_focused(), "Escape again gives focus back to the field");
    t.key(Key::F(10));
    t.key(Key::Enter);
    t.key(Key::ArrowUp);
    assert_eq!(t.focused().as_deref(), Some("Quit"), "Up goes round to the last entry");
    t.key(Key::Enter);
    assert_eq!(log.borrow().as_slice(), &["quit"]);
    assert!(field.is_focused(), "choosing gives focus back first");
    // F10 twice leaves the bar.
    t.key(Key::F(10));
    t.key(Key::F(10));
    assert!(field.is_focused());
}

#[test]
fn alt_and_a_letter_opens_a_menu_from_a_text_field() {
    let mut t = T::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (_bar, field) = bar_window(&t, &log);
    t.frame();
    field.focus();
    assert!(t.alt('e'), "the bar took Alt+E before the field");
    assert_eq!(t.focused().as_deref(), Some("Find…"));
    t.type_char('f');
    assert_eq!(log.borrow().as_slice(), &["find"], "F chose Find… in the open menu");
    assert!(field.is_focused());
    assert!(!t.alt('x'), "a letter no menu has goes on to the app");
    // A lone Alt tap reaches the bar; another leaves it.
    assert!(t.alt_tap());
    assert_eq!(t.focused().as_deref(), Some("File"));
    assert!(t.alt_tap());
    assert!(field.is_focused());
    // Alt with something else between press and release is a chord, not a tap.
    let alt = Key::Named("Alt".into());
    let _ = t.ui.key(&KeyEvent::press(alt.clone()).with_modifiers(Modifiers::ALT));
    assert!(t.ui.alt_held(), "access keys show while Alt is down");
    let _ = t.ui.key(&KeyEvent::press(Key::Tab).with_modifiers(Modifiers::ALT));
    let _ = t.ui.key(&KeyEvent::release(alt));
    t.frame();
    assert!(!t.ui.alt_held());
    assert!(field.is_focused(), "Alt+Tab did not reach the bar");
}

#[test]
fn the_pointer_opens_menus_and_slides_between_them() {
    let mut t = T::new();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (_bar, field) = bar_window(&t, &log);
    t.frame();
    field.focus();
    let file = t.bounds(Role::MenuItem, "File").center();
    t.pointer(PointerPhase::Down, file, Buttons::PRIMARY);
    assert_eq!(t.menus().len(), 1, "a press on a title opens its menu");
    assert!(t.node(Role::MenuItem, "Open…").is_some());
    t.pointer(PointerPhase::Up, file, Buttons::default());
    assert_eq!(t.menus().len(), 1, "and the release keeps it open");
    t.pointer(PointerPhase::Move, t.bounds(Role::MenuItem, "Help").center(), Buttons::default());
    assert!(t.node(Role::MenuItem, "About").is_some(), "moving onto Help opens Help's menu");
    assert!(t.node(Role::MenuItem, "Open…").is_none());
    t.click_at(t.bounds(Role::MenuItem, "Help").center());
    assert!(t.menus().is_empty(), "a press on the open title closes it");
    assert!(field.is_focused(), "and the field kept focus throughout");
    // Press, drag into the menu, release on an entry: chosen.
    let file = t.bounds(Role::MenuItem, "File").center();
    t.pointer(PointerPhase::Down, file, Buttons::PRIMARY);
    let save = t.bounds(Role::MenuItem, "Save").center();
    t.pointer(PointerPhase::Move, save, Buttons::PRIMARY);
    t.pointer(PointerPhase::Up, save, Buttons::default());
    assert_eq!(log.borrow().as_slice(), &["save"]);
    // A press outside closes the menu.
    t.click_at(file);
    t.click_at(Point::new(600.0, 400.0));
    assert!(t.menus().is_empty());
}

#[test]
fn a_narrow_window_moves_the_menus_that_do_not_fit_into_more() {
    let mut t = T::sized(150.0, 300.0);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (bar, field) = bar_window(&t, &log);
    t.frame();
    t.frame();
    let more = t.node(Role::MenuItem, "More").unwrap_or_else(|| unreachable!("{:#?}", t.names()));
    assert!(t.node(Role::MenuItem, "Help").is_none(), "Help no longer fits");
    let size = t.size;
    for n in t.reader().into_iter().filter(|n| n.role == Role::MenuItem) {
        let b = n.bounds.unwrap_or_default();
        assert!(inside(b, size), "{:?} at {b:?} is cut off", n.name);
    }
    let titles = t.reader().iter().filter(|n| n.role == Role::MenuItem).count();
    assert_eq!(more.position, Some((titles, titles)), "More is the last title of the set");
    // Alt+H still opens Help, inside More.
    field.focus();
    t.alt('h');
    assert_eq!(t.focused().as_deref(), Some("About"));
    assert_eq!(t.menus().len(), 2);
    for m in t.menus() {
        assert!(inside(m, size), "{m:?}");
    }
    t.key(Key::Enter);
    assert_eq!(log.borrow().as_slice(), &["about"]);
    // Wide again: every title is back.
    t.size = Size::new(640.0, 300.0);
    t.frame();
    t.frame();
    assert!(t.node(Role::MenuItem, "Help").is_some() && t.node(Role::MenuItem, "More").is_none());
    drop(bar);
}

#[test]
fn a_right_to_left_menu_bar_is_mirrored() {
    let mut t = T::new();
    t.ui.set_locale("ar");
    let _ = t.ui.root().layout(|l| l.direction = remedy_ui_layout::Direction::Rtl);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let (_bar, field) = bar_window(&t, &log);
    t.frame();
    let (file, edit) = (t.bounds(Role::MenuItem, "File"), t.bounds(Role::MenuItem, "Edit"));
    assert!(file.min.x > edit.min.x, "File is at the right: {file:?} {edit:?}");
    field.focus();
    t.key(Key::F(10));
    t.key(Key::ArrowLeft);
    assert_eq!(t.focused().as_deref(), Some("Edit"), "Left moves on in a right-to-left bar");
    t.key(Key::ArrowDown);
    let m = t.menus();
    assert!((nth(&m, 0).max.x - edit.max.x).abs() < 6.0, "the menu lines up with the title's right edge");
}

#[test]
fn a_menu_button_says_whether_its_menu_is_open() {
    let mut t = T::new();
    let chosen = Rc::new(Cell::new(false));
    let c2 = chosen.clone();
    let b = w::menu_button(&t.ui, "More", vec![MenuItem::new("Rename", move || c2.set(true))]);
    t.ui.root().child(b.clone());
    t.frame();
    b.focus();
    assert_eq!(t.node(Role::Button, "More").and_then(|n| n.expanded), Some(false));
    t.key(Key::ArrowDown);
    assert_eq!(t.node(Role::Button, "More").and_then(|n| n.expanded), Some(true));
    assert_eq!(t.focused().as_deref(), Some("Rename"));
    t.key(Key::Enter);
    assert!(chosen.get() && b.is_focused());
    assert_eq!(t.node(Role::Button, "More").and_then(|n| n.expanded), Some(false));
}
