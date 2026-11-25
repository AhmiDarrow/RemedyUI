//! Agents reading and operating a real view, under the person's consent.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_widgets::automation::*;
use remedy_ui_core::Size;
use remedy_ui_core::reactive::Signal;
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Request as ViewRequest, Ui};
use remedy_ui_widgets::base as w;
use remedy_ui_widgets::nav::Commands;

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

struct T {
    ui: Ui,
    timers: Vec<u64>,
}

impl T {
    fn new() -> T {
        let theme = remedy_ui_widgets::themes::ThemeChoice::default().resolve(None, false);
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            let _ = fonts.add_static(face.data);
        }
        T { ui: Ui::new(theme.clone(), remedy_ui_widgets::themes::sheet(&theme), fonts), timers: Vec::new() }
    }

    fn frame(&mut self) {
        let _ = self.ui.frame(Size::new(800.0, 600.0), 1.0);
        for r in self.ui.take_requests() {
            match r {
                ViewRequest::Timer { id, .. } => self.timers.push(id),
                ViewRequest::CancelTimer(id) => self.timers.retain(|t| *t != id),
                _ => {}
            }
        }
    }

    fn fire(&mut self) {
        self.frame();
        for id in std::mem::take(&mut self.timers) {
            self.ui.fire_timer(id);
        }
        self.frame();
    }
}

fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
    for n in nodes {
        if n.name.as_deref() == Some(name) {
            return Some(n);
        }
        if let Some(f) = find(&n.children, name) {
            return Some(f);
        }
    }
    None
}

fn has_text(nodes: &[Node], text: &str) -> bool {
    nodes.iter().any(|n| {
        n.name.as_deref().is_some_and(|s| s.contains(text))
            || n.value.as_deref().is_some_and(|s| s.contains(text))
            || has_text(&n.children, text)
    })
}

/// An element in the trusted layer named `name` (the person's own controls).
fn trusted(ui: &Ui, name: &str) -> Option<El> {
    let [_, _, layer] = ui.layers();
    let mut stack = vec![layer];
    while let Some(e) = stack.pop() {
        if e.composed_semantics().is_some_and(|s| s.name() == Some(name)) {
            return Some(e);
        }
        stack.extend(e.child_elements());
    }
    None
}

fn settings(t: &mut T) -> (Signal<bool>, remedy_ui_engine::view::TextField, remedy_ui_engine::view::TextField) {
    let noise = Signal::new(false);
    let name = w::text_input(&t.ui, "Display name");
    name.set_value("Sam");
    let pw = w::secret_input(&t.ui, "Password");
    let _ = t.ui.root().child(
        w::column(&t.ui)
            .child(w::switch(&t.ui, "Noise suppression", noise))
            .child(name.el())
            .child(pw.el())
            .child(w::text(&t.ui, "Welcome back")),
    );
    t.frame();
    (noise, name, pw)
}

#[test]
fn nothing_is_visible_without_a_grant_and_reading_cannot_act() {
    let mut t = T::new();
    let (noise, _name, _pw) = settings(&mut t);
    let agents = Agents::new(&t.ui, None);
    assert_eq!(agents.snapshot("guess"), Err(Refused::NoAccess));
    assert!(trusted(&t.ui, "assistant can see this window").is_none(), "no indicator without a grant");

    let token = agents.grant("assistant", Scope::Read, None);
    t.frame();
    assert!(trusted(&t.ui, "assistant can see this window").is_some(), "the indicator shows while connected");
    let snap = agents.snapshot(&token).must("granted");
    let switch = find(&snap.tree, "Noise suppression").must("the switch is visible");
    assert_eq!(switch.checked, Some(false));
    assert!(switch.actions.iter().any(|a| a == "click"));
    assert_eq!(find(&snap.tree, "Display name").and_then(|n| n.value.as_deref()), Some("Sam"));
    let pw = find(&snap.tree, "Password").must("a password field shows its role and label");
    assert_eq!(pw.value, None, "but never its text");
    assert!(!pw.actions.iter().any(|a| a == "set_value"));
    assert!(!has_text(&snap.tree, "hunter2"));
    assert!(has_text(&snap.tree, "Welcome back"));
    assert!(!has_text(&snap.tree, "can see this window"), "the trusted layer is out of reach");

    let id = switch.id;
    assert!(matches!(agents.act(&token, id, AgentAction::Click, |_| {}), Answer::Refused(Refused::ReadOnly)));
    assert!(!noise.get());

    // Stop in the indicator ends access.
    let stop = trusted(&t.ui, "Stop").must("a stop button");
    assert!(t.ui.accessibility_action(stop.node_id(), remedy_ui_core::a11y::Action::Click));
    t.frame();
    assert_eq!(agents.snapshot(&token), Err(Refused::NoAccess));
    assert!(agents.granted().is_empty());
    assert!(trusted(&t.ui, "assistant can see this window").is_none(), "and the indicator goes");
}

#[test]
fn agents_act_through_semantics_and_every_change_is_undoable_by_name() {
    let mut t = T::new();
    let (noise, name, pw) = settings(&mut t);
    let agents = Agents::new(&t.ui, None);
    let token = agents.grant("assistant", Scope::Act, None);
    t.frame();
    let snap = agents.snapshot(&token).must("granted");
    let switch = find(&snap.tree, "Noise suppression").must("switch").id;
    let field = find(&snap.tree, "Display name").must("field").id;
    let secret = find(&snap.tree, "Password").must("password").id;

    assert_eq!(agents.act(&token, switch, AgentAction::Click, |_| {}), Answer::Done);
    t.frame();
    assert!(noise.get(), "the switch turned on");
    assert_eq!(
        t.ui.next_undo().map(|h| h.undo_label()).as_deref(),
        Some("Undo Press Noise suppression, by assistant")
    );

    assert_eq!(agents.act(&token, field, AgentAction::SetValue("Alex".into()), |_| {}), Answer::Done);
    t.frame();
    assert_eq!(name.value(), "Alex");
    assert_eq!(
        agents.snapshot(&token).ok().and_then(|s| s.undo).as_deref(),
        Some("Undo Set Display name to Alex, by assistant")
    );

    // The person undoes both, latest first, and can redo.
    assert!(t.ui.undo_change());
    t.frame();
    assert_eq!(name.value(), "Sam");
    assert!(t.ui.undo_change());
    t.frame();
    assert!(!noise.get());
    assert!(t.ui.redo_change());
    t.frame();
    assert!(noise.get());

    // Secrets cannot be written, nor can anything that does not exist.
    assert_eq!(
        agents.act(&token, secret, AgentAction::SetValue("x".into()), |_| {}),
        Answer::Refused(Refused::NotOffered)
    );
    assert!(pw.take_secret().expose().is_empty(), "nothing was written into the password");
    assert_eq!(
        agents.act(&token, 9_999_999, AgentAction::Click, |_| {}),
        Answer::Refused(Refused::NoSuchElement)
    );
}

#[test]
fn destructive_commands_ask_the_person_who_alone_can_answer() {
    let mut t = T::new();
    let _ = settings(&mut t);
    let commands = Commands::new();
    let deleted = Rc::new(Cell::new(0));
    let saved = Rc::new(Cell::new(0));
    let (d2, s2) = (deleted.clone(), saved.clone());
    commands.add("chat.delete", "Delete chat", Vec::new(), None, move || d2.set(d2.get() + 1));
    commands.add("chat.save", "Save draft", Vec::new(), None, move || s2.set(s2.get() + 1));
    commands.set_destructive("chat.delete", true);
    let agents = Agents::new(&t.ui, Some(&commands));
    let token = agents.grant("assistant", Scope::Act, None);
    t.frame();

    let snap = agents.snapshot(&token).must("granted");
    let del = snap.commands.iter().find(|c| c.id == "chat.delete").must("listed");
    assert!(del.destructive);
    assert_eq!(agents.command(&token, "chat.save", |_| {}), Answer::Done);
    assert_eq!(saved.get(), 1, "a plain command runs at once");
    assert_eq!(agents.command(&token, "nope", |_| {}), Answer::Refused(Refused::NoSuchCommand));

    let outcome: Rc<RefCell<Vec<Result<(), Refused>>>> = Rc::default();
    let o2 = outcome.clone();
    assert_eq!(agents.command(&token, "chat.delete", move |r| o2.borrow_mut().push(r)), Answer::Asking);
    t.frame();
    assert_eq!(deleted.get(), 0, "nothing happens before the person answers");
    let allow = trusted(&t.ui, "Allow").must("the prompt is in the trusted layer");
    // The agent cannot press it: the trusted layer is out of its reach.
    assert_eq!(
        agents.act(&token, allow.node_id(), AgentAction::Click, |_| {}),
        Answer::Refused(Refused::NoSuchElement)
    );
    t.fire();
    let deny = trusted(&t.ui, "Deny").must("deny");
    assert!(t.ui.accessibility_action(deny.node_id(), remedy_ui_core::a11y::Action::Click));
    t.frame();
    assert_eq!(deleted.get(), 0);
    assert_eq!(outcome.borrow().as_slice(), &[Err(Refused::Declined)]);

    let o3 = outcome.clone();
    assert_eq!(agents.command(&token, "chat.delete", move |r| o3.borrow_mut().push(r)), Answer::Asking);
    t.fire();
    let allow = trusted(&t.ui, "Allow").must("asked again");
    assert!(t.ui.accessibility_action(allow.node_id(), remedy_ui_core::a11y::Action::Click));
    t.frame();
    assert_eq!(deleted.get(), 1, "it runs once allowed");
    assert_eq!(outcome.borrow().last(), Some(&Ok(())));
}

#[test]
fn confirming_grants_ask_before_every_action() {
    let mut t = T::new();
    let (noise, _, _) = settings(&mut t);
    let agents = Agents::new(&t.ui, None);
    let token = agents.grant("helper", Scope::ActWithConfirmation, None);
    t.frame();
    let id = find(&agents.snapshot(&token).must("granted").tree, "Noise suppression").must("switch").id;
    let done = Rc::new(Cell::new(false));
    let d2 = done.clone();
    assert_eq!(agents.act(&token, id, AgentAction::Click, move |r| d2.set(r.is_ok())), Answer::Asking);
    t.frame();
    assert!(!noise.get());
    t.fire();
    let allow = trusted(&t.ui, "Allow").must("asked");
    assert!(t.ui.accessibility_action(allow.node_id(), remedy_ui_core::a11y::Action::Click));
    t.frame();
    assert!(noise.get() && done.get());
    assert_eq!(t.ui.next_undo().and_then(|h| h.by).as_deref(), Some("helper"));
}

#[test]
fn locked_and_hidden_content_is_out_of_reach_and_grants_expire() {
    let mut t = T::new();
    let _ = settings(&mut t);
    let hidden = w::text(&t.ui, "Draft nobody sees").hidden(true);
    let _ = t.ui.root().child(hidden);
    t.frame();
    let agents = Agents::new(&t.ui, None);
    let token = agents.grant("assistant", Scope::Act, None);
    t.frame();
    let snap = agents.snapshot(&token).must("granted");
    assert!(!has_text(&snap.tree, "Draft nobody sees"));
    let switch = find(&snap.tree, "Noise suppression").must("switch").id;

    let unlock = w::secret_input(&t.ui, "Master password");
    let lock = w::lock_screen(&t.ui, unlock.el());
    t.frame();
    let snap = agents.snapshot(&token).must("still granted");
    assert!(!has_text(&snap.tree, "Welcome back"), "locked content is gone");
    assert!(!has_text(&snap.tree, "Master password"), "and the unlock form is the person's");
    assert_eq!(
        agents.act(&token, switch, AgentAction::Click, |_| {}),
        Answer::Refused(Refused::NoSuchElement)
    );
    lock.remove();
    t.frame();
    assert!(has_text(&agents.snapshot(&token).must("granted").tree, "Welcome back"));

    let brief = agents.grant("visitor", Scope::Read, Some(Duration::ZERO));
    assert_eq!(agents.snapshot(&brief), Err(Refused::NoAccess), "an expired grant is no grant");
    assert_eq!(agents.granted(), vec![("assistant".to_owned(), Scope::Act)]);
}

#[test]
fn the_json_protocol_answers_requests() {
    let mut t = T::new();
    let (noise, _, _) = settings(&mut t);
    let agents = Agents::new(&t.ui, None);
    let token = agents.grant("assistant", Scope::Act, None);
    t.frame();
    let later = |_: String| {};

    let bad = agents.handle("{nope", later);
    assert!(bad.contains("bad_request"), "{bad}");
    let none = agents.handle(r#"{"request":"snapshot","token":"wrong"}"#, later);
    assert!(none.contains("no_access"), "{none}");

    let snap = agents.handle(&format!(r#"{{"request":"snapshot","token":"{token}"}}"#), later);
    let answer: Answer = serde_json::from_str(&snap).must("an answer");
    let Answer::Snapshot(s) = answer else { unreachable!("{snap}") };
    let id = find(&s.tree, "Noise suppression").must("switch").id;

    let act = agents.handle(
        &format!(r#"{{"request":"act","token":"{token}","node":{id},"action":{{"do":"click"}}}}"#),
        later,
    );
    assert_eq!(serde_json::from_str::<Answer>(&act).ok(), Some(Answer::Done), "{act}");
    t.frame();
    assert!(noise.get());
    let undo = agents.handle(&format!(r#"{{"request":"undo","token":"{token}"}}"#), later);
    assert_eq!(serde_json::from_str::<Answer>(&undo).ok(), Some(Answer::Done), "{undo}");
    t.frame();
    assert!(!noise.get());
}

#[test]
fn agents_in_other_processes_connect_with_their_token_and_strangers_are_turned_away() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let mut t = T::new();
    let (noise, _, _) = settings(&mut t);
    let agents = Agents::new(&t.ui, None);
    let token = agents.grant("assistant", Scope::Act, None);
    t.frame();
    let listener = agents.listen().must("listening");
    let addr = listener.addr();
    assert!(addr.ip().is_loopback(), "only this machine can connect");

    // Runs the window's posted work until `done` says so.
    let pump = |done: &dyn Fn() -> bool| {
        let start = std::time::Instant::now();
        while !done() {
            let _ = remedy_ui_core::reactive::run_posted();
            assert!(start.elapsed() < Duration::from_secs(20), "the window answered in time");
            std::thread::yield_now();
        }
    };

    let stranger = std::thread::spawn(move || {
        let mut s = TcpStream::connect(addr).must("connect");
        writeln!(s, "0123456789abcdef0123456789abcdef").must("write");
        let mut line = String::new();
        let _ = BufReader::new(s).read_line(&mut line);
        line
    });
    let refused = stranger.join().must("stranger");
    assert!(refused.contains("no_access"), "{refused}");

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tok = token.clone();
    let client = std::thread::spawn(move || {
        let mut s = TcpStream::connect(addr).must("connect");
        let mut r = BufReader::new(s.try_clone().must("clone"));
        writeln!(s, "{tok}").must("hello");
        writeln!(s, r#"{{"request":"snapshot","token":"{tok}"}}"#).must("ask");
        let mut line = String::new();
        r.read_line(&mut line).must("answer");
        let _ = tx.send(line);
        s
    });
    let got: RefCell<Option<String>> = RefCell::new(None);
    pump(&|| {
        if let Ok(l) = rx.try_recv() {
            *got.borrow_mut() = Some(l);
        }
        got.borrow().is_some()
    });
    let snap = got.borrow().clone().unwrap_or_default();
    let Ok(Answer::Snapshot(s)) = serde_json::from_str::<Answer>(&snap) else { unreachable!("{snap}") };
    let id = find(&s.tree, "Noise suppression").must("switch").id;
    let mut stream = client.join().must("client");
    writeln!(stream, r#"{{"request":"act","token":"{token}","node":{id},"action":{{"do":"click"}}}}"#)
        .must("act");
    pump(&|| noise.get());
    t.frame();
    assert_eq!(t.ui.next_undo().and_then(|h| h.by).as_deref(), Some("assistant"));
    drop(listener);
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_widgets::automation::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}
