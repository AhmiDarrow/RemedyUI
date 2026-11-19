//! Gestures, scrolling and commands.

use std::time::Duration;

use remedy_ui_core::Point;
use remedy_ui_input::*;

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn ev(phase: PointerPhase, x: f32, y: f32, t: u64, kind: PointerKind) -> PointerEvent {
    PointerEvent::new(phase, Point::new(x, y))
        .with_kind(kind)
        .with_buttons(Buttons::PRIMARY)
        .with_time(ms(t))
        .with_modifiers(remedy_ui_input::Modifiers::NONE)
}

#[test]
fn velocity_from_recent_samples() {
    let mut v = VelocityTracker::new();
    for i in 0..10u64 {
        v.add(Point::new(i as f32 * 10.0, 0.0), ms(i * 10)); // 10 px per 10 ms = 1000 px/s
    }
    let vel = v.velocity();
    assert!((vel.x - 1000.0).abs() < 1.0, "{vel:?}");
    assert!(vel.y.abs() < 1e-3);
    // Old samples fall out of the window.
    v.add(Point::new(90.0, 0.0), ms(1000));
    assert_eq!(v.velocity().x, 0.0);
}

#[test]
fn a_horizontal_swipe_beats_the_vertical_list_and_the_tap() {
    let mut arena = GestureArena::new(GestureSettings::default());
    arena.join(Recognizer::new(1, GestureKind::Tap)); // the row
    arena.join(Recognizer::new(2, GestureKind::HorizontalPan)); // swipe to reply
    arena.join(Recognizer::new(3, GestureKind::VerticalPan)); // the list
    assert!(arena.handle(&ev(PointerPhase::Down, 100.0, 100.0, 0, PointerKind::Touch)).is_none());
    assert!(
        arena.handle(&ev(PointerPhase::Move, 104.0, 101.0, 10, PointerKind::Touch)).is_none(),
        "inside the slop"
    );
    let w = arena.handle(&ev(PointerPhase::Move, 130.0, 104.0, 30, PointerKind::Touch)).map(|r| r.owner);
    assert_eq!(w, Some(2));
    assert!(
        arena.handle(&ev(PointerPhase::Up, 140.0, 104.0, 40, PointerKind::Touch)).is_none(),
        "decided once"
    );
}

#[test]
fn a_still_press_is_a_tap_or_a_long_press() {
    let mut tap = GestureArena::new(GestureSettings::default());
    tap.join(Recognizer::new(1, GestureKind::Tap));
    tap.join(Recognizer::new(2, GestureKind::LongPress));
    tap.handle(&ev(PointerPhase::Down, 0.0, 0.0, 0, PointerKind::Mouse));
    assert_eq!(
        tap.handle(&ev(PointerPhase::Up, 1.0, 0.0, 120, PointerKind::Mouse)).map(|r| r.owner),
        Some(1)
    );

    let mut long = GestureArena::new(GestureSettings::default());
    long.join(Recognizer::new(1, GestureKind::Tap));
    long.join(Recognizer::new(2, GestureKind::LongPress));
    long.handle(&ev(PointerPhase::Down, 0.0, 0.0, 0, PointerKind::Touch));
    assert!(long.tick(ms(400)).is_none());
    assert_eq!(long.tick(ms(500)).map(|r| r.owner), Some(2));
}

#[test]
fn flings_decay_and_stop_at_clamped_edges() {
    let mut a = ScrollAxis::new(10_000.0, ScrollPhysics::CLAMPED);
    a.fling(3000.0);
    let mut frames = 0;
    while a.step(1.0 / 120.0) {
        frames += 1;
        assert!(frames < 2000, "a fling ends");
    }
    assert!(a.offset > 500.0 && a.offset < 1500.0, "{}", a.offset);
    let mut edge = ScrollAxis::new(100.0, ScrollPhysics::CLAMPED);
    edge.fling(-5000.0);
    edge.step(0.1);
    assert_eq!(edge.offset, 0.0);
    edge.drag(-50.0);
    assert_eq!(edge.offset, 0.0, "no rubber band on clamped physics");
}

#[test]
fn bouncing_overscroll_resists_and_springs_back() {
    let mut a = ScrollAxis::new(100.0, ScrollPhysics::BOUNCING);
    a.drag(-100.0);
    assert!(a.offset < 0.0 && a.offset > -100.0, "resisted: {}", a.offset);
    let mut frames = 0;
    while a.step(1.0 / 60.0) {
        frames += 1;
        assert!(frames < 600);
    }
    assert_eq!(a.offset, 0.0);
}

fn key(k: Key, m: Modifiers) -> KeyEvent {
    KeyEvent::press(k).with_modifiers(m).with_text(None)
}

#[test]
fn commands_match_by_scope_and_respect_ime_and_enabled() {
    let p = Platform::Standard;
    let mut t = CommandTable::new();
    t.register(
        Scope::App,
        Command {
            id: "app.find".into(),
            label: "Find".into(),
            default_chords: vec![Chord::primary(p, Key::char('f'))],
            enabled: true,
        },
    );
    t.register(
        Scope::Element,
        Command {
            id: "editor.find".into(),
            label: "Find in text".into(),
            default_chords: vec![Chord::primary(p, Key::char('f'))],
            enabled: true,
        },
    );
    let ctrl_f = key(Key::char('F'), Modifiers::primary(p));
    let active = [Scope::App, Scope::Element];
    assert_eq!(t.match_key(&ctrl_f, &active).map(|c| c.id.as_str()), Some("editor.find"), "inner scope wins");
    assert_eq!(t.match_key(&ctrl_f, &[Scope::App]).map(|c| c.id.as_str()), Some("app.find"));
    t.set_enabled("editor.find", false);
    assert_eq!(
        t.match_key(&ctrl_f, &active).map(|c| c.id.as_str()),
        Some("app.find"),
        "a disabled command lets the outer one answer"
    );
    let composing = ctrl_f.clone().with_composing(true);
    assert!(t.match_key(&composing, &active).is_none(), "IME composition never fires shortcuts");
    assert!(t.match_key(&key(Key::char('f'), Modifiers::NONE), &active).is_none());
}

#[test]
fn rebinding_checks_conflicts_and_can_reset() {
    let p = Platform::Standard;
    let mut t = CommandTable::new();
    t.register(
        Scope::Window,
        Command {
            id: "reply".into(),
            label: "Reply".into(),
            default_chords: vec![Chord::primary(p, Key::char('r'))],
            enabled: true,
        },
    );
    t.register(
        Scope::Window,
        Command {
            id: "react".into(),
            label: "React".into(),
            default_chords: vec![Chord::primary(p, Key::char('e'))],
            enabled: true,
        },
    );
    let clash = t.rebind("react", vec![Chord::primary(p, Key::char('r'))]);
    assert_eq!(clash.map_err(|c| c.existing), Err("reply".to_owned()));
    assert_eq!(t.rebind("react", vec![Chord::new(Modifiers::NONE, Key::F(9))]), Ok(()));
    assert_eq!(
        t.match_key(&key(Key::F(9), Modifiers::NONE), &[Scope::Window]).map(|c| c.id.as_str()),
        Some("react")
    );
    t.reset("react");
    assert_eq!(t.chords("react"), vec![Chord::primary(p, Key::char('e'))]);
}

#[test]
fn chord_labels_follow_the_platform() {
    let undo_redo = Chord::new(Modifiers::primary(Platform::Standard).with_shift(), Key::char('z'));
    assert_eq!(undo_redo.label(Platform::Standard), "Ctrl+Shift+Z");
    let mac = Chord::new(Modifiers::primary(Platform::Apple).with_shift(), Key::char('z'));
    assert_eq!(mac.label(Platform::Apple), "⇧⌘Z");
}

#[test]
fn chords_have_a_portable_text_form_that_reads_back() {
    let cases = [
        (Chord::new(Modifiers::primary(Platform::Standard).with_shift(), Key::char('z')), "Ctrl+Shift+Z"),
        (Chord::new(Modifiers { alt: true, ..Modifiers::NONE }, Key::F(4)), "Alt+F4"),
        (Chord::new(Modifiers { meta: true, ..Modifiers::NONE }, Key::ArrowLeft), "Meta+Left"),
        (Chord::new(Modifiers::primary(Platform::Standard), Key::Character("+".into())), "Ctrl+Plus"),
        (Chord::new(Modifiers::NONE, Key::Named("MediaPlayPause".into())), "MediaPlayPause"),
        (Chord::new(Modifiers::NONE, Key::Escape), "Escape"),
    ];
    for (chord, text) in cases {
        assert_eq!(chord.to_string(), text);
        assert_eq!(text.parse::<Chord>(), Ok(chord.clone()), "{text}");
    }
    // Other spellings people write.
    let ctrl_plus = Chord::new(Modifiers::primary(Platform::Standard), Key::Character("+".into()));
    assert_eq!("ctrl++".parse::<Chord>(), Ok(ctrl_plus));
    assert_eq!(
        "Cmd + Option + k".parse::<Chord>(),
        Ok(Chord::new(Modifiers { meta: true, alt: true, ..Modifiers::NONE }, Key::char('k')))
    );
    assert_eq!("esc".parse::<Chord>().map(|c| c.key), Ok(Key::Escape));
    for bad in ["", "Ctrl+", "+K", "Hyper+K", "Ctrl+Ctrl+K", "Ctrl++K+"] {
        assert!(bad.parse::<Chord>().is_err(), "{bad:?} is not a chord");
    }
}

#[test]
fn conflicts_and_clashes_are_reported_without_changing_anything() {
    let p = Platform::Standard;
    let mut t = CommandTable::new();
    let cmd = |id: &str, ch: char| Command {
        id: id.into(),
        label: id.into(),
        default_chords: vec![Chord::primary(p, Key::char(ch))],
        enabled: true,
    };
    t.register(Scope::App, cmd("save", 's'));
    t.register(Scope::App, cmd("send", 'e'));
    t.register(Scope::Window, cmd("search", 's'));
    let ctrl_s = Chord::primary(p, Key::char('s'));
    let found = t.conflicts("send", std::slice::from_ref(&ctrl_s));
    let found: Vec<_> = found.into_iter().map(|c| (c.scope, c.chord, c.commands)).collect();
    assert_eq!(found, vec![(Scope::App, ctrl_s.clone(), vec!["save".to_owned()])]);
    assert_eq!(t.chords("send"), vec![Chord::primary(p, Key::char('e'))], "nothing changed");
    assert!(t.clashes().is_empty(), "the window-scoped search does not clash with save");
    assert_eq!(t.default_chords("send"), vec![Chord::primary(p, Key::char('e'))]);
    assert_eq!(t.user_chords("send"), None);

    // A whole set at once: send takes Ctrl+S while save keeps it, so they clash.
    let mut user = std::collections::HashMap::new();
    user.insert("send".to_string(), vec![ctrl_s.clone()]);
    user.insert("not-registered-yet".to_string(), vec![Chord::new(Modifiers::NONE, Key::F(2))]);
    t.set_user_bindings(user);
    assert_eq!(t.user_chords("send"), Some(&[ctrl_s.clone()][..]));
    let clashes: Vec<_> = t.clashes().into_iter().map(|c| (c.scope, c.chord, c.commands)).collect();
    assert_eq!(clashes, vec![(Scope::App, ctrl_s, vec!["save".to_owned(), "send".to_owned()])]);
    // A binding set before its command exists applies once it is registered.
    t.register(Scope::App, cmd("not-registered-yet", 'n'));
    assert_eq!(t.chords("not-registered-yet"), vec![Chord::new(Modifiers::NONE, Key::F(2))]);
    assert_eq!(t.user_bindings().len(), 2);
}

#[test]
fn chord_labels_speak_the_persons_language() {
    let c = |s: &str| s.parse::<Chord>().unwrap_or_else(|e| unreachable!("{e}"));
    let redo = c("Ctrl+Shift+Z");
    assert_eq!(redo.label_in(Platform::Standard, "de-DE"), "Strg+Umschalt+Z");
    assert_eq!(redo.label_in(Platform::Standard, "fr-CA"), "Ctrl+Maj+Z");
    assert_eq!(redo.label_in(Platform::Standard, "es"), "Ctrl+Mayús+Z");
    assert_eq!(
        redo.label_in(Platform::Standard, "ja-JP"),
        "Ctrl+Shift+Z",
        "English where a language has no names"
    );
    assert_eq!(redo.label_in(Platform::Apple, "de-DE"), "⌃⇧Z");
    assert_eq!(c("Delete").label_in(Platform::Standard, "de"), "Entf");
    assert_eq!(c("Ctrl+PageDown").label_in(Platform::Standard, "en"), "Ctrl+PgDn");
    assert_eq!(c("Alt+Left").label_in(Platform::Standard, "it"), "Alt+Sinistra");
    assert_eq!(c("Meta+Left").label_in(Platform::Apple, "en"), "⌘←");
    assert_eq!(c("F11").label_in(Platform::Standard, "de"), "F11");
    assert_eq!(c("Ctrl+Plus").label_in(Platform::Standard, "de"), "Strg++");
}
