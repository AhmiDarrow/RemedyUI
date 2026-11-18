# Agents: automation with consent

An agent is an assistant that reads a window and operates it: an assistant inside the app,
or one in another process on the same machine. RemedyUI gives agents the same view of a
window a screen reader has, and the app's own commands. Agents never send synthetic mouse
or keyboard input.

The person stays in control, and the engine enforces it rather than leaving it to each app:

- **Consent.** Nothing is visible or operable until the person grants an agent access,
  with a scope and, if they like, an expiry. Each grant has a secret token the agent must
  present with every request.
- **Visible.** While any agent holds access, the window shows an indicator naming it, with
  a Stop button.
- **Out of reach.** Secrets are never read, and hidden content, lock screens and the
  trusted layer are never shown to an agent nor operated by one.
- **Undoable and named.** Every change an agent makes goes into the window's undo history
  under its name, and destructive commands ask the person first.

All of this lives in [`crate::agents`] (feature `agents`).

## Granting access

[`Window::attach_agents`](crate::Window::attach_agents) gives agents access to one window,
with the app's [`crate::nav::Commands`] if it has them, and returns its
[`Agents`](crate::agents::Agents). Nothing is granted yet.
[`Agents::request_access`](crate::agents::Agents::request_access) asks the person, in a
trusted prompt on the trusted layer that nothing the app or an agent draws can cover or press:
"Allow assistant?", saying what it may do and for how long. On Allow the access is granted and
the callback gets the token to hand the agent; on Deny it gets `None`. The scope is one of:

| [`crate::agents::Scope`] | The agent may |
|---|---|
| `Read` | read what the window shows and what can be done |
| `Act` | read and act |
| `ActWithConfirmation` | read, and act once the person confirms each action |

A grant shows the indicator ("assistant can see this window", with Stop) in the trusted
layer, where the agent cannot reach it. An app with its own consent screen (a settings page
listing assistants) calls [`Agents::grant`](crate::agents::Agents::grant) after the person
chose there; RemedyUI grants nothing on its own.

```
use remedy_ui::a11y::Role;
use remedy_ui::agents::{Node, Refused, Scope};
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::RefCell;
use std::rc::Rc;
# fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
#     nodes.iter().find_map(|n| {
#         if n.name.as_deref() == Some(name) { Some(n) } else { find(&n.children, name) }
#     })
# }

let noise = Signal::new(false);
let mut app = TestApp::window(move |ui, _| {
    let password = w::secret_input(ui, "Password");
    let settings = w::column(ui).child(w::switch(ui, "Noise suppression", noise)).child(password.el());
    ui.root().child(settings);
});
let Some(window) = app.runner().app().windows().into_iter().next() else { unreachable!("open") };
let Some(agents) = window.attach_agents(None) else { unreachable!("the window is open") };
assert_eq!(agents.snapshot("a guess"), Err(Refused::NoAccess), "nothing without a grant");

// The assistant asks; the person answers in the trusted prompt.
let given: Rc<RefCell<Option<String>>> = Rc::default();
let g = given.clone();
agents.request_access("assistant", Scope::Read, None, move |token| *g.borrow_mut() = token);
app.frames(60); // a second later: the prompt's buttons wake after a moment
assert!(app.press(Role::Button, "Allow"));
let Some(token) = given.borrow().clone() else { unreachable!("the person allowed it") };

let snapshot = agents.snapshot(&token)?;
let Some(switch) = find(&snapshot.tree, "Noise suppression") else {
    unreachable!("the switch is within reach")
};
assert_eq!(switch.checked, Some(false));
assert!(switch.actions.iter().any(|a| a == "click"));

// A password field shows its role and label, never its text, and cannot be written.
let Some(secret) = find(&snapshot.tree, "Password") else { unreachable!("listed") };
assert_eq!(secret.value, None);
assert!(!secret.actions.iter().any(|a| a == "set_value"));

// The person sees who has access.
let Some(ui) = app.ui() else { unreachable!("open") };
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("assistant can see this window")));
assert_eq!(agents.granted(), vec![("assistant".to_owned(), Scope::Read)]);
# Ok::<(), Refused>(())
```

[`Window::agents`](crate::Window::agents) gives the same `Agents` back later.
[`Agents::revoke`](crate::agents::Agents::revoke) ends an agent's access, as the Stop
button does. A grant made with an expiry (`Some(duration)`) ends by itself.

## What an agent sees

A [`crate::agents::Snapshot`] has three parts:

- `tree`: the window's elements as [`crate::agents::Node`]s, each with an id, a role
  (`button`, `switch`, `text_input`...), a name, a value, its states and the actions it
  offers (`click`, `focus`, `set_value`, `increment`...). Plain containers without a name
  pass their children up, so the tree holds what matters.
- `commands`: the app's commands as [`crate::agents::CommandNode`]s, with whether each can
  run now and whether it is destructive.
- `undo`: what Undo would take back, if anything.

Hidden elements, lock screens and the trusted layer are left out. An element the app hides
with [`El::hidden`](crate::El::hidden) is out of an agent's reach as it is out of a screen
reader's. Good semantics are what make a window operable by agents: see the
[accessibility guide](crate::guide::accessibility).

## Acting, and taking it back

[`Agents::act`](crate::agents::Agents::act) does an [`crate::agents::AgentAction`] on an
element: click, focus, increment, decrement, expand, collapse, scroll into view, or set a
field's text. It goes through the same paths a screen reader uses. Each change is recorded
in the window's undo history, named for the agent, so the person can take it back with
Ctrl+Z or [`Ui::undo_change`](crate::Ui::undo_change).

```
# use remedy_ui::testing::TestApp;
use remedy_ui::agents::{AgentAction, Agents, Answer, Node, Refused, Scope};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
# fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
#     nodes.iter().find_map(|n| {
#         if n.name.as_deref() == Some(name) { Some(n) } else { find(&n.children, name) }
#     })
# }
# let mut app = TestApp::window(|_, _| {});
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let noise = Signal::new(false);
let name = w::text_input(&ui, "Display name");
name.set_value("Sam");
ui.root().child(
    w::column(&ui).child(w::switch(&ui, "Noise suppression", noise)).child(name.el()),
);

let agents = Agents::new(&ui, None);
let token = agents.grant("assistant", Scope::Act, None);
# app.frames(1);
let snapshot = agents.snapshot(&token)?;
let (Some(switch), Some(field)) =
    (find(&snapshot.tree, "Noise suppression"), find(&snapshot.tree, "Display name"))
else {
    unreachable!("both are within reach")
};

assert_eq!(agents.act(&token, switch.id, AgentAction::Click, |_| {}), Answer::Done);
assert!(noise.get());
assert_eq!(agents.act(&token, field.id, AgentAction::SetValue("Alex".into()), |_| {}), Answer::Done);
assert_eq!(name.value(), "Alex");

// Each change is in the history under the agent's name.
let Some(last) = ui.next_undo() else { unreachable!("two changes were made") };
assert_eq!(last.undo_label(), "Undo Set Display name to Alex, by assistant");

// The person takes both back, latest first.
assert!(ui.undo_change());
assert_eq!(name.value(), "Sam");
assert!(ui.undo_change());
assert!(!noise.get());
# Ok::<(), Refused>(())
```

A button's own effect is the app's to record: the app's handler runs while the agent acts
([`Ui::acting_as`](crate::Ui::acting_as)), so anything the handler records with
[`Ui::record_change`](crate::Ui::record_change) carries the agent's name too.

## Commands, and asking the person

[`Agents::command`](crate::agents::Agents::command) runs one of the app's commands. A
command the app marks destructive with
[`Commands::set_destructive`](crate::nav::Commands::set_destructive) (it deletes, leaves or
sends), and every action under an `ActWithConfirmation` grant, asks the person first. The
question is a [`crate::widgets::trusted_prompt`] in the trusted layer: the agent cannot see
or press it, and its buttons only take effect after a short delay, so a stray click cannot
answer it. The call returns [`Answer::Asking`](crate::agents::Answer::Asking), and the
`done` callback hears the outcome.

```
# use remedy_ui::testing::TestApp;
use remedy_ui::agents::{Agents, Answer, Refused, Scope};
use remedy_ui::nav::Commands;
use remedy_ui::view::A11yAction;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
# let mut app = TestApp::window(|_, _| {});
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let deleted = Rc::new(Cell::new(0));
let d = deleted.clone();
let commands = Commands::new();
commands.add("chat.delete", "Delete chat", vec![], None, move || d.set(d.get() + 1));
commands.add("chat.save", "Save draft", vec![], None, || {});
commands.set_destructive("chat.delete", true);

let agents = Agents::new(&ui, Some(&commands));
let token = agents.grant("assistant", Scope::Act, None);
assert_eq!(agents.command(&token, "chat.save", |_| {}), Answer::Done, "plain commands run at once");

let outcome: Rc<RefCell<Option<Result<(), Refused>>>> = Rc::default();
let o = outcome.clone();
let answer = agents.command(&token, "chat.delete", move |r| *o.borrow_mut() = Some(r));
assert_eq!(answer, Answer::Asking);
assert_eq!(deleted.get(), 0, "nothing happens before the person answers");

// The person answers once the prompt's buttons are live, a moment later.
app.frames(60);
let reader = ui.reader_view();
let Some(allow) = reader.iter().find(|n| n.name.as_deref() == Some("Allow")) else {
    unreachable!("the prompt asks the person")
};
assert!(ui.accessibility_action(allow.id, A11yAction::Click));
assert_eq!(deleted.get(), 1);
assert_eq!(*outcome.borrow(), Some(Ok(())));
```

When the person says no, `done` hears [`Refused::Declined`](crate::agents::Refused::Declined).

## The JSON protocol

Agents in other processes speak a small JSON protocol: one JSON object per request, one per
answer. [`Agents::handle`](crate::agents::Agents::handle) answers a request; an answer that
waits for the person (`{"answer":"asking"}`) is followed by the outcome through `later`.

| Request | Fields |
|---|---|
| `{"request":"snapshot"}` | `token` |
| `{"request":"act"}` | `token`, `node` (an id from a snapshot), `action` (`{"do":"click"}`, `{"do":"set_value","value":"Alex"}`...) |
| `{"request":"command"}` | `token`, `id` |
| `{"request":"undo"}` | `token` |

Answers are `snapshot`, `done`, `asking`, or `refused` with the reason
([`crate::agents::Refused`]: `no_access`, `read_only`, `no_such_element`, `not_offered`,
`no_such_command`, `declined`, `bad_request`).

```
# use remedy_ui::testing::TestApp;
use remedy_ui::agents::{Agents, Node, Scope};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
# fn find<'a>(nodes: &'a [Node], name: &str) -> Option<&'a Node> {
#     nodes.iter().find_map(|n| {
#         if n.name.as_deref() == Some(name) { Some(n) } else { find(&n.children, name) }
#     })
# }
# let mut app = TestApp::window(|_, _| {});
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let noise = Signal::new(false);
ui.root().child(w::switch(&ui, "Noise suppression", noise));
let agents = Agents::new(&ui, None);
let token = agents.grant("assistant", Scope::Act, None);
# app.frames(1);

let refused = agents.handle(r#"{"request":"snapshot","token":"wrong"}"#, |_| {});
assert!(refused.contains("no_access"), "{refused}");

let text = agents.handle(&format!(r#"{{"request":"snapshot","token":"{token}"}}"#), |_| {});
assert!(text.starts_with(r#"{"answer":"snapshot""#), "{text}");
assert!(text.contains(r#""name":"Noise suppression""#));

// The same snapshot as Rust values (answers and their parts are serde types).
let Ok(snapshot) = agents.snapshot(&token) else { unreachable!("granted") };
let Some(switch) = find(&snapshot.tree, "Noise suppression") else { unreachable!("listed") };

let act = format!(
    r#"{{"request":"act","token":"{token}","node":{},"action":{{"do":"click"}}}}"#,
    switch.id
);
assert_eq!(agents.handle(&act, |_| {}), r#"{"answer":"done"}"#);
assert!(noise.get());
assert_eq!(agents.handle(&format!(r#"{{"request":"undo","token":"{token}"}}"#), |_| {}), r#"{"answer":"done"}"#);
assert!(!noise.get());
```

## Agents in other processes

[`Agents::listen`](crate::agents::Agents::listen) opens a loopback port the system picks
and returns a [`crate::agents::Listener`]; give its address to the agent along with its
token. A connection opens by sending the token on its own line. Anything else is turned
away on the connection's own thread, before a byte reaches the window. After that, each line
is one request and each answer comes back as one line. Requests are answered on the
window's thread, posted through the reactive runtime's waker, so the app's loop wakes for
them like any other work. Dropping the listener stops listening and closes its
connections.

```
# use remedy_ui::testing::TestApp;
use remedy_ui::agents::{Agents, Scope};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
# let mut app = TestApp::window(|_, _| {});
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let agents = Agents::new(&ui, None);
let _token = agents.grant("assistant", Scope::Read, None);
let listener = agents.listen()?;
assert!(listener.addr().ip().is_loopback(), "only this machine can connect");

// A connection without a live token is refused before it reaches the window.
let mut stranger = TcpStream::connect(listener.addr())?;
writeln!(stranger, "0123456789abcdef0123456789abcdef")?;
let mut line = String::new();
BufReader::new(stranger).read_line(&mut line)?;
assert!(line.contains("no_access"), "{line}");
drop(listener);
# Ok::<(), std::io::Error>(())
```
