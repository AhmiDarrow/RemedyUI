//! Agents in RemedyUI apps: an AI assistant (the RemedyOS mind, the RemedyAI desktop agent, an
//! assistant inside the app) reads a window and operates it through the same semantics tree a
//! screen reader uses and the app's own commands, never through synthetic mouse or keyboard
//! input. The person stays in control:
//!
//! - **Consent.** Nothing is visible or operable until the person grants an agent access to the
//!   window, with a scope (`Read`, `Act`, `ActWithConfirmation`) and, if they like, an expiry.
//!   Each grant has a secret token the agent must present with every request.
//! - **Visible.** While any agent holds access the window shows an indicator naming it, with a
//!   button that ends its access.
//! - **Out of reach.** Secrets are never read (password fields have no value), and hidden
//!   content, lock screens and the trusted layer (prompts only the person may answer) are never
//!   shown to an agent nor operated by one.
//! - **Undoable and named.** Every change an agent makes goes into the window's undo history
//!   named for it ("Undo turn on Noise suppression, by assistant"), and commands the app marks
//!   destructive ask the person first.
//!
//! Tests and harnesses that need real input (a key press where the keyboard's goes, a click
//! at a point) use the facade's `Window::send_key`, `press_key`, `send_pointer` and
//! `click_at` instead, which hand events to a live window as the platform would, and
//! `Window::snapshot_png` for its picture; agents never get those.
//!
//! [`Agents::handle`] answers requests in a small JSON protocol, for a transport (a local
//! socket the app opens only while access is granted, or the RemedyOS semantics wire).

/// The agent prompts' own words (asking for access, the indicator, the undo history's names
/// for what agents did) in every language the engine ships, from `i18n/*.ftl`. Apps reword
/// them, or add a language, with [`remedy_ui_engine::view::strings::add_messages`] under the name
/// `automation`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("automation");

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_core::a11y::{Action, Role};
use remedy_ui_engine::view::{El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_nav::Commands;
use serde::{Deserialize, Serialize};

mod transport;
pub use transport::Listener;

/// What an agent may do in a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Scope {
    /// Read the window: what it shows and what can be done.
    Read,
    /// Read and act.
    Act,
    /// Read, and act once the person confirms each action.
    ActWithConfirmation,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "refused", content = "detail")]
#[non_exhaustive]
pub enum Refused {
    /// No grant matches the token (never granted, revoked or expired).
    NoAccess,
    /// The grant reads only.
    ReadOnly,
    /// No such element, or it is out of an agent's reach (hidden, secret, trusted).
    NoSuchElement,
    /// The element does not offer that action.
    NotOffered,
    /// No such command, or it cannot run now.
    NoSuchCommand,
    /// The person said no.
    Declined,
    /// The request could not be read.
    BadRequest(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NoAccess => f.write_str("this agent has no access to the window"),
            Refused::ReadOnly => f.write_str("this agent may only read the window"),
            Refused::NoSuchElement => f.write_str("no such element within reach"),
            Refused::NotOffered => f.write_str("the element does not offer that action"),
            Refused::NoSuchCommand => f.write_str("no such command, or it cannot run now"),
            Refused::Declined => f.write_str("the person declined"),
            Refused::BadRequest(why) => write!(f, "the request could not be read: {why}"),
        }
    }
}

impl std::error::Error for Refused {}

/// One element as an agent sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Node {
    /// Its id, to act on it.
    pub id: u64,
    /// Its role (`button`, `switch`, `text_input`...).
    pub role: String,
    /// Its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its value (a field's text, a slider's number). Never a secret's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Checked (switches, checkboxes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Selected (list items, tabs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    /// Expanded (menus, tree items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// Disabled.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub disabled: bool,
    /// What it offers (`click`, `focus`, `set_value`, `increment`...).
    pub actions: Vec<String>,
    /// What is inside it.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<Node>,
}

/// One command as an agent sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CommandNode {
    /// Its id.
    pub id: String,
    /// What it is called.
    pub label: String,
    /// It can run now.
    pub enabled: bool,
    /// Running it asks the person first.
    pub destructive: bool,
}

/// What an agent sees of a window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Snapshot {
    /// The window's elements as a tree.
    pub tree: Vec<Node>,
    /// The app's commands.
    pub commands: Vec<CommandNode>,
    /// The change Undo would take back, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
}

/// An action an agent asks for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "do", content = "value")]
#[non_exhaustive]
pub enum AgentAction {
    /// Press it.
    Click,
    /// Move focus to it.
    Focus,
    /// Step it up (a slider, a stepper).
    Increment,
    /// Step it down.
    Decrement,
    /// Open it.
    Expand,
    /// Close it.
    Collapse,
    /// Bring it into view.
    ScrollIntoView,
    /// Replace a field's text.
    SetValue(String),
}

impl AgentAction {
    fn a11y(&self) -> Action {
        match self {
            AgentAction::Click => Action::Click,
            AgentAction::Focus => Action::Focus,
            AgentAction::Increment => Action::Increment,
            AgentAction::Decrement => Action::Decrement,
            AgentAction::Expand => Action::Expand,
            AgentAction::Collapse => Action::Collapse,
            AgentAction::ScrollIntoView => Action::ScrollIntoView,
            AgentAction::SetValue(_) => Action::SetValue,
        }
    }
}

fn role_name(r: Role) -> String {
    let s = format!("{r:?}");
    // CamelCase to snake_case: "TextInput" -> "text_input", "Heading(2)" -> "heading(2)".
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A grant the person made.
#[derive(Clone, Debug)]
struct Grant {
    agent: String,
    scope: Scope,
    token: String,
    until: Option<Instant>,
}

fn token() -> String {
    remedy_ui_core::token::random_token()
}

struct State {
    grants: Vec<Grant>,
    indicator: Option<El>,
}

/// Agents' access to one window. Cheap to clone.
#[derive(Clone)]
pub struct Agents {
    ui: Ui,
    commands: Option<Commands>,
    state: Rc<RefCell<State>>,
    /// The tokens of the grants in force, for transports to turn strangers away before they
    /// reach the window's thread.
    tokens: Arc<Mutex<HashSet<String>>>,
}

impl std::fmt::Debug for Agents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Agents({} granted)", self.state.borrow().grants.len())
    }
}

/// What a request asked for, in the JSON protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "request")]
#[non_exhaustive]
pub enum Request {
    /// What the window shows and offers.
    Snapshot {
        /// The grant's token.
        token: String,
    },
    /// An action on an element.
    Act {
        /// The grant's token.
        token: String,
        /// The element's id from a snapshot.
        node: u64,
        /// What to do.
        action: AgentAction,
    },
    /// Run one of the app's commands.
    Command {
        /// The grant's token.
        token: String,
        /// The command's id.
        id: String,
    },
    /// Undo the latest change in the window.
    Undo {
        /// The grant's token.
        token: String,
    },
}

/// The answer to a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "answer")]
#[non_exhaustive]
pub enum Answer {
    /// The snapshot asked for.
    Snapshot(Snapshot),
    /// Done.
    Done,
    /// Waiting for the person to confirm; the outcome follows as its own answer.
    Asking,
    /// Refused, and why.
    Refused(Refused),
}

impl Agents {
    /// Agents' access to `ui`'s window, with the app's `commands` if it has them.
    pub fn new(ui: &Ui, commands: Option<&Commands>) -> Self {
        Agents {
            ui: ui.clone(),
            commands: commands.cloned(),
            state: Rc::new(RefCell::new(State { grants: Vec::new(), indicator: None })),
            tokens: Arc::default(),
        }
    }

    /// The person grants `agent` access with `scope`, for `lasts` (or until revoked). Returns
    /// the token to hand the agent.
    pub fn grant(&self, agent: &str, scope: Scope, lasts: Option<Duration>) -> String {
        let t = token();
        {
            let mut s = self.state.borrow_mut();
            s.grants.retain(|g| g.agent != agent);
            s.grants.push(Grant {
                agent: agent.to_owned(),
                scope,
                token: t.clone(),
                until: lasts.map(|d| Instant::now() + d),
            });
        }
        self.show_indicator();
        t
    }

    /// Asks the person, in a trusted prompt nothing the app or an agent draws can cover or
    /// press, whether `agent` may have `scope` over this window for `lasts` (or until they
    /// end it). On Allow the access is granted and `answered` gets the token to hand the agent;
    /// on Deny it gets `None`. Returns the prompt's layer (it removes itself when answered).
    ///
    /// ```
    /// # fn ask(ui: &remedy_ui_engine::view::Ui) {
    /// use remedy_ui_automation::{Agents, Scope};
    ///
    /// let agents = Agents::new(ui, None);
    /// agents.request_access("Assistant", Scope::Read, None, |token| {
    ///     if let Some(token) = token {
    ///         // Hand `token` to the assistant with the listener's address.
    ///         let _ = token;
    ///     }
    /// });
    /// # }
    /// ```
    pub fn request_access(
        &self,
        agent: &str,
        scope: Scope,
        lasts: Option<Duration>,
        answered: impl FnOnce(Option<String>) + 'static,
    ) -> El {
        let ui = &self.ui;
        let how_long = match lasts {
            Some(d) if d.as_secs() >= 120 => {
                STRINGS.format(ui, "agent-for-minutes", &[("count", (d.as_secs() / 60).into())])
            }
            Some(d) => STRINGS.format(ui, "agent-for-seconds", &[("count", d.as_secs().max(1).into())]),
            None => STRINGS.get(ui, "agent-until-stopped"),
        };
        let asks = match scope {
            Scope::Read => "agent-asks-read",
            Scope::Act => "agent-asks-act",
            Scope::ActWithConfirmation => "agent-asks-confirm",
        };
        let message = STRINGS.format(ui, asks, &[("agent", agent.into()), ("how_long", how_long.into())]);
        let (me, name) = (self.clone(), agent.to_owned());
        let answered = RefCell::new(Some(answered));
        let title = STRINGS.format(ui, "agent-allow", &[("agent", agent.into())]);
        w::trusted_prompt(&self.ui, title, message, move |yes| {
            let Some(f) = answered.borrow_mut().take() else { return };
            f(yes.then(|| me.grant(&name, scope, lasts)));
        })
    }

    /// Ends `agent`'s access.
    pub fn revoke(&self, agent: &str) {
        self.state.borrow_mut().grants.retain(|g| g.agent != agent);
        self.show_indicator();
    }

    /// The agents with access now, and their scopes.
    pub fn granted(&self) -> Vec<(String, Scope)> {
        self.expire();
        self.state.borrow().grants.iter().map(|g| (g.agent.clone(), g.scope)).collect()
    }

    fn expire(&self) {
        let now = Instant::now();
        let before = self.state.borrow().grants.len();
        self.state.borrow_mut().grants.retain(|g| g.until.is_none_or(|u| u > now));
        if self.state.borrow().grants.len() != before {
            self.show_indicator();
        }
    }

    fn grant_for(&self, token: &str) -> Option<Grant> {
        self.expire();
        self.state.borrow().grants.iter().find(|g| g.token == token).cloned()
    }

    /// The indicator in the trusted layer: who has access, and a button to end it. Shown while
    /// any agent has access.
    fn show_indicator(&self) {
        let old = self.state.borrow_mut().indicator.take();
        if let Some(old) = old {
            old.remove();
        }
        let grants = self.state.borrow().grants.clone();
        *self.tokens.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
            grants.iter().map(|g| g.token.clone()).collect();
        if grants.is_empty() {
            return;
        }
        let ui = &self.ui;
        let unit = ui.theme().spacing.unit;
        let names: Vec<String> = grants.iter().map(|g| g.agent.clone()).collect();
        let text = STRINGS.format(ui, "agents-can-see", &[("agents", names.join(", ").into())]);
        let me = self.clone();
        let stop = w::button(ui, STRINGS.get(ui, "agents-stop"), move || {
            for n in &names {
                me.revoke(n);
            }
        });
        let bar = ui
            .el("agent_indicator")
            .row()
            .gap(unit * 2.0)
            .padding(remedy_ui_core::Insets::all(unit * 2.0))
            .child(w::text(ui, text.clone()).variant("strong"))
            .child(stop)
            .semantics(remedy_ui_core::a11y::Semantics::new(Role::Status).label(text));
        let [_, _, trusted] = ui.layers();
        let _ = trusted.child(bar.clone());
        self.state.borrow_mut().indicator = Some(bar);
    }

    /// The elements an agent may see: the content and overlay layers, without hidden
    /// content, lock screens or the trusted layer.
    fn reachable(&self) -> Vec<El> {
        let [content, overlays, _trusted] = self.ui.layers();
        vec![content, overlays]
    }

    fn node_of(el: &El) -> Option<Vec<Node>> {
        let info = el.info()?;
        if info.hidden || info.widget == "lock_screen" {
            return Some(Vec::new());
        }
        let children: Vec<Node> = el.child_elements().iter().filter_map(Self::node_of).flatten().collect();
        let sem = el.composed_semantics()?;
        // Plain containers pass their children up; hidden ones leave the tree.
        if sem.is_hidden() {
            return Some(Vec::new());
        }
        if sem.role() == Role::Group && sem.name().is_none() && sem.actions().is_empty() {
            return Some(children);
        }
        let secret = sem.role() == Role::PasswordInput;
        Some(vec![Node {
            id: el.node_id(),
            role: role_name(sem.role()),
            name: sem.name().map(str::to_owned),
            value: if secret { None } else { sem.value_text().map(str::to_owned) },
            checked: sem.checked_state(),
            selected: sem.selected_state(),
            expanded: sem.expanded_state(),
            disabled: info.states.contains(remedy_ui_core::style::States::DISABLED),
            actions: sem
                .actions()
                .iter()
                .filter(|a| !(secret && **a == Action::SetValue))
                .map(|a| a.name().to_owned())
                .collect(),
            children,
        }])
    }

    /// The element with `id` within an agent's reach.
    fn find(&self, id: u64) -> Option<El> {
        let mut stack = self.reachable();
        while let Some(e) = stack.pop() {
            let info = e.info()?;
            if info.hidden || info.widget == "lock_screen" {
                continue;
            }
            if e.node_id() == id {
                return Some(e);
            }
            stack.extend(e.child_elements());
        }
        None
    }

    /// What the window shows and offers, for the agent holding `token`.
    pub fn snapshot(&self, token: &str) -> Result<Snapshot, Refused> {
        self.grant_for(token).ok_or(Refused::NoAccess)?;
        let tree = self.reachable().iter().filter_map(Self::node_of).flatten().collect();
        let commands = self
            .commands
            .as_ref()
            .map(|c| {
                c.all()
                    .into_iter()
                    .map(|i| CommandNode {
                        id: i.id,
                        label: i.label,
                        enabled: i.enabled,
                        destructive: i.destructive,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Snapshot { tree, commands, undo: self.ui.next_undo().map(|h| h.undo_label()) })
    }

    /// Does `action` on element `node` for the agent holding `token`. With a confirming
    /// grant the person is asked first and `done` hears the outcome; otherwise it happens now.
    pub fn act(
        &self,
        token: &str,
        node: u64,
        action: AgentAction,
        done: impl Fn(Result<(), Refused>) + 'static,
    ) -> Answer {
        let Some(grant) = self.grant_for(token) else { return Answer::Refused(Refused::NoAccess) };
        if grant.scope == Scope::Read {
            return Answer::Refused(Refused::ReadOnly);
        }
        let Some(el) = self.find(node) else { return Answer::Refused(Refused::NoSuchElement) };
        let sem = el.composed_semantics();
        let offered = sem.as_ref().is_some_and(|s| s.actions().contains(&action.a11y()))
            && sem.as_ref().is_none_or(|s| !(s.role() == Role::PasswordInput));
        if !offered {
            return Answer::Refused(Refused::NotOffered);
        }
        let name = sem.and_then(|s| s.name().map(str::to_owned)).unwrap_or_default();
        let what = describe(&self.ui, &action, &name);
        let me = self.clone();
        let agent = grant.agent.clone();
        let run = move || me.perform(&agent, &el, &action, &what);
        if grant.scope == Scope::ActWithConfirmation {
            self.confirm(&grant.agent, &describe_question(&self.ui, &grant.agent, &name), run, done);
            return Answer::Asking;
        }
        match run() {
            Ok(()) => Answer::Done,
            Err(r) => Answer::Refused(r),
        }
    }

    /// Does the action as the agent, recording it in the window's history.
    fn perform(&self, agent: &str, el: &El, action: &AgentAction, what: &str) -> Result<(), Refused> {
        let ui = self.ui.clone();
        let id = el.node_id();
        let before = el.composed_semantics().and_then(|s| s.value_text().map(str::to_owned));
        let ok = ui.acting_as(agent, || match action {
            AgentAction::SetValue(v) => ui.accessibility_set_value(id, v),
            other => ui.accessibility_action(id, other.a11y()),
        });
        if !ok {
            return Err(Refused::NotOffered);
        }
        // A value set is undone by setting the old one back; a switch or button by pressing
        // it again, which is how people take those back.
        let (u, r) = (ui.clone(), ui.clone());
        match action {
            AgentAction::SetValue(v) => {
                let (old, new) = (before.unwrap_or_default(), v.clone());
                ui.record_change(
                    what.to_owned(),
                    Some(agent.to_owned()),
                    move || {
                        u.accessibility_set_value(id, &old);
                    },
                    move || {
                        r.accessibility_set_value(id, &new);
                    },
                );
            }
            AgentAction::Click if el.composed_semantics().is_some_and(|s| s.checked_state().is_some()) => {
                ui.record_change(
                    what.to_owned(),
                    Some(agent.to_owned()),
                    move || {
                        u.accessibility_action(id, Action::Click);
                    },
                    move || {
                        r.accessibility_action(id, Action::Click);
                    },
                );
            }
            AgentAction::Increment | AgentAction::Decrement => {
                let (back, again) = match action {
                    AgentAction::Increment => (Action::Decrement, Action::Increment),
                    _ => (Action::Increment, Action::Decrement),
                };
                ui.record_change(
                    what.to_owned(),
                    Some(agent.to_owned()),
                    move || {
                        u.accessibility_action(id, back);
                    },
                    move || {
                        r.accessibility_action(id, again);
                    },
                );
            }
            // Focus, scrolling, opening and closing change nothing to take back; a button's
            // own effect is the app's to record (it runs while the agent acts, so its entries
            // carry the agent's name).
            _ => {}
        }
        Ok(())
    }

    /// Runs command `id` for the agent holding `token`. Destructive commands, and every command
    /// under a confirming grant, ask the person first.
    pub fn command(&self, token: &str, id: &str, done: impl Fn(Result<(), Refused>) + 'static) -> Answer {
        let Some(grant) = self.grant_for(token) else { return Answer::Refused(Refused::NoAccess) };
        if grant.scope == Scope::Read {
            return Answer::Refused(Refused::ReadOnly);
        }
        let Some(commands) = self.commands.clone() else { return Answer::Refused(Refused::NoSuchCommand) };
        let Some(info) = commands.info(id).filter(|i| i.enabled) else {
            return Answer::Refused(Refused::NoSuchCommand);
        };
        let (ui, agent, cid) = (self.ui.clone(), grant.agent.clone(), id.to_owned());
        let run = move || {
            if ui.acting_as(&agent, || commands.run(&cid)) { Ok(()) } else { Err(Refused::NoSuchCommand) }
        };
        if info.destructive || grant.scope == Scope::ActWithConfirmation {
            self.confirm(
                &grant.agent,
                &STRINGS.format(
                    &self.ui,
                    "agent-let-command",
                    &[("agent", (&grant.agent).into()), ("command", info.label.to_lowercase().into())],
                ),
                run,
                done,
            );
            return Answer::Asking;
        }
        match run() {
            Ok(()) => Answer::Done,
            Err(r) => Answer::Refused(r),
        }
    }

    /// Asks the person, in the trusted layer (which agents cannot reach), whether to let the
    /// agent go ahead.
    fn confirm(
        &self,
        agent: &str,
        question: &str,
        run: impl Fn() -> Result<(), Refused> + 'static,
        done: impl Fn(Result<(), Refused>) + 'static,
    ) {
        let _ = w::trusted_prompt(
            &self.ui,
            STRINGS.format(&self.ui, "agent-asks", &[("agent", agent.into())]),
            question,
            move |yes| {
                done(if yes { run() } else { Err(Refused::Declined) });
            },
        );
    }

    /// Answers one request in the JSON protocol: `request` is one JSON object, the answer one
    /// JSON object. An answer that waits for the person is followed by `later` with the outcome.
    pub fn handle(&self, request: &str, later: impl Fn(String) + 'static) -> String {
        let req: Request = match serde_json::from_str(request) {
            Ok(r) => r,
            Err(e) => return to_json(&Answer::Refused(Refused::BadRequest(e.to_string()))),
        };
        let later = Rc::new(later);
        let outcome = move |r: Result<(), Refused>| {
            later(to_json(&match r {
                Ok(()) => Answer::Done,
                Err(e) => Answer::Refused(e),
            }));
        };
        let answer = match req {
            Request::Snapshot { token } => match self.snapshot(&token) {
                Ok(s) => Answer::Snapshot(s),
                Err(r) => Answer::Refused(r),
            },
            Request::Act { token, node, action } => self.act(&token, node, action, outcome),
            Request::Command { token, id } => self.command(&token, &id, outcome),
            Request::Undo { token } => match self.grant_for(&token) {
                None => Answer::Refused(Refused::NoAccess),
                Some(g) if g.scope == Scope::Read => Answer::Refused(Refused::ReadOnly),
                Some(_) => {
                    if self.ui.undo_change() {
                        Answer::Done
                    } else {
                        Answer::Refused(Refused::NotOffered)
                    }
                }
            },
        };
        to_json(&answer)
    }
}

fn to_json(a: &Answer) -> String {
    serde_json::to_string(a).unwrap_or_else(|_| String::from("{\"answer\":\"refused\"}"))
}

/// What an action did, for the history: "Set Name to Alex", "Press Send".
fn describe(ui: &Ui, action: &AgentAction, name: &str) -> String {
    let id = match action {
        AgentAction::SetValue(_) => "did-set",
        AgentAction::Click => "did-press",
        AgentAction::Increment => "did-raise",
        AgentAction::Decrement => "did-lower",
        AgentAction::Focus => "did-focus",
        AgentAction::Expand => "did-expand",
        AgentAction::Collapse => "did-collapse",
        AgentAction::ScrollIntoView => "did-show",
    };
    let value = if let AgentAction::SetValue(v) = action { v.to_string() } else { String::new() };
    STRINGS.format(ui, id, &[("name", name.into()), ("value", value.into())])
}

fn describe_question(ui: &Ui, agent: &str, name: &str) -> String {
    STRINGS.format(ui, "agent-let-use", &[("agent", agent.into()), ("name", name.into())])
}
