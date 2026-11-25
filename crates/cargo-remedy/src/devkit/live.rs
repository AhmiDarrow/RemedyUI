//! Operating a running app as an agent, through its agents listener (`remedy_ui::agents`,
//! `Agents::listen`), under the person's consent.
//!
//! The app opens the listener and the person grants access: they give the agent the
//! listener's address and a token, whose scope they chose (`read`, `act`, or
//! `act_with_confirmation`). Nothing here gets around that: every request carries the token,
//! the app answers on its own thread, a read-only grant cannot act, the person's own prompts
//! are out of reach, and every change lands in the window's undo history under the agent's
//! name. When the app asks the person first (destructive commands, confirming grants) the
//! answer is "asking" and the outcome follows when the person answers; these tools wait for
//! it for a while and otherwise report it on a later call.
//!
//! The protocol is JSON lines: the connection opens with the token on its own line, then each
//! request (`{"request":"snapshot","token":…}`, `act`, `command`, `undo`) is answered by one
//! line (`{"answer":"snapshot",…}`, `done`, `asking`, `refused`), an `asking` answer later
//! followed by the outcome on its own line.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::devkit::lines::LineClient;
use crate::devkit::{Result, ToolError};

/// Where the app listens, and the grant's token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConnectInput {
    /// The listener's loopback address, as the person gave it (`127.0.0.1:52100`).
    pub address: String,
    /// The grant's token, as the person gave it.
    pub token: String,
    /// A name for this connection, to use several apps at once (`app` when absent).
    #[serde(default)]
    pub connection: Option<String>,
    /// How long to wait for the app to answer a request, in seconds (30 when absent).
    #[serde(default)]
    pub answer_timeout_seconds: Option<u64>,
}

impl ConnectInput {
    /// A connection named `app` to `address` with `token`.
    pub fn new(address: impl Into<String>, token: impl Into<String>) -> Self {
        ConnectInput { address: address.into(), token: token.into(), ..Default::default() }
    }
}

/// Names a connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConnectionInput {
    /// The connection's name (`app` when absent).
    #[serde(default)]
    pub connection: Option<String>,
}

/// What to do to an element.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ActInput {
    /// The connection's name (`app` when absent).
    #[serde(default)]
    pub connection: Option<String>,
    /// The element's id from a snapshot.
    pub node: u64,
    /// The action, one the element offers: `click`, `focus`, `increment`, `decrement`,
    /// `expand`, `collapse`, `scroll_into_view` or `set_value`.
    pub action: String,
    /// The new text, for `set_value`.
    #[serde(default)]
    pub value: Option<String>,
    /// When the app asks the person first, how long to wait for their answer, in seconds
    /// (60 when absent; 0 returns at once).
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

impl ActInput {
    /// `action` on element `node` of the connection named `app`.
    pub fn new(node: u64, action: impl Into<String>) -> Self {
        ActInput { node, action: action.into(), ..Default::default() }
    }
}

/// A command to run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct CommandInput {
    /// The connection's name (`app` when absent).
    #[serde(default)]
    pub connection: Option<String>,
    /// The command's id from a snapshot (`chat.delete`).
    pub id: String,
    /// When the app asks the person first, how long to wait for their answer, in seconds
    /// (60 when absent; 0 returns at once).
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

impl CommandInput {
    /// Command `id` on the connection named `app`.
    pub fn new(id: impl Into<String>) -> Self {
        CommandInput { id: id.into(), ..Default::default() }
    }
}

/// Waiting for an answer the person has not given yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct WaitInput {
    /// The connection's name (`app` when absent).
    #[serde(default)]
    pub connection: Option<String>,
    /// How long to wait, in seconds (60 when absent).
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

/// One element as an agent sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Node {
    /// Its id, to act on it.
    pub id: u64,
    /// Its role (`button`, `switch`, `text_input`, …).
    pub role: String,
    /// Its name.
    #[serde(default)]
    pub name: Option<String>,
    /// Its value (never a secret's).
    #[serde(default)]
    pub value: Option<String>,
    /// Checked.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Selected.
    #[serde(default)]
    pub selected: Option<bool>,
    /// Expanded.
    #[serde(default)]
    pub expanded: Option<bool>,
    /// Disabled.
    #[serde(default)]
    pub disabled: bool,
    /// What it offers.
    #[serde(default)]
    pub actions: Vec<String>,
    /// What is inside it.
    #[serde(default)]
    pub children: Vec<Node>,
}

/// One of the app's commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct CommandNode {
    /// Its id.
    pub id: String,
    /// Its label.
    pub label: String,
    /// It can run now.
    pub enabled: bool,
    /// Running it asks the person first.
    pub destructive: bool,
}

/// What the agent sees of the window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Snapshot {
    /// The window's elements.
    pub tree: Vec<Node>,
    /// The app's commands.
    #[serde(default)]
    pub commands: Vec<CommandNode>,
    /// What Undo would take back.
    #[serde(default)]
    pub undo: Option<String>,
}

impl Snapshot {
    /// The tree as indented lines (`switch "Noise suppression" [off] {click} #12`), then the
    /// commands and the undo entry.
    pub fn text(&self) -> String {
        fn walk(out: &mut String, nodes: &[Node], depth: usize) {
            for n in nodes {
                out.push_str(&"  ".repeat(depth));
                out.push_str(&n.role);
                if let Some(name) = &n.name {
                    out.push_str(&format!(" {name:?}"));
                }
                if let Some(v) = &n.value {
                    out.push_str(&format!(" = {v:?}"));
                }
                let mut states = Vec::new();
                if let Some(c) = n.checked {
                    states.push(if c { "on" } else { "off" });
                }
                if n.selected == Some(true) {
                    states.push("selected");
                }
                if let Some(e) = n.expanded {
                    states.push(if e { "expanded" } else { "collapsed" });
                }
                if n.disabled {
                    states.push("disabled");
                }
                if !states.is_empty() {
                    out.push_str(&format!(" [{}]", states.join(", ")));
                }
                if !n.actions.is_empty() {
                    out.push_str(&format!(" {{{}}}", n.actions.join(", ")));
                }
                out.push_str(&format!(" #{}\n", n.id));
                walk(out, &n.children, depth + 1);
            }
        }
        let mut out = String::new();
        walk(&mut out, &self.tree, 0);
        if !self.commands.is_empty() {
            out.push_str("commands:\n");
            for c in &self.commands {
                let mut flags = Vec::new();
                if !c.enabled {
                    flags.push("disabled");
                }
                if c.destructive {
                    flags.push("asks the person");
                }
                let flags = if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) };
                out.push_str(&format!("  {} {:?}{flags}\n", c.id, c.label));
            }
        }
        if let Some(u) = &self.undo {
            out.push_str(&format!("undo: {u}\n"));
        }
        out
    }
}

/// How a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Status {
    /// Done.
    Done,
    /// The app is asking the person, who has not answered yet.
    Asking,
    /// Refused (see `refused`).
    Refused,
}

/// The outcome of an action, a command or an undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Outcome {
    /// How it ended.
    pub status: Status,
    /// Why it was refused (`no_access`, `read_only`, `no_such_element`, `not_offered`,
    /// `no_such_command`, `declined`, `bad_request`).
    #[serde(default)]
    pub refused: Option<String>,
    /// What happened, in a sentence.
    pub message: String,
    /// The app asked the person before going ahead.
    pub asked_the_person: bool,
    /// Outcomes of earlier requests the person answered since the last call.
    #[serde(default)]
    pub earlier: Vec<String>,
}

/// A connection made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Connected {
    /// The connection's name.
    pub connection: String,
    /// The app's address.
    pub address: String,
    /// What the agent sees now.
    pub snapshot: Snapshot,
}

fn refusal_sentence(code: &str, detail: Option<&str>) -> String {
    match code {
        "no_access" => {
            "the app refused: this agent has no access (never granted, revoked, or expired)".into()
        }
        "read_only" => "the app refused: this grant may only read the window".into(),
        "no_such_element" => "the app refused: no such element within reach (take a new snapshot)".into(),
        "not_offered" => "the app refused: the element does not offer that action".into(),
        "no_such_command" => "the app refused: no such command, or it cannot run now".into(),
        "declined" => "the person declined".into(),
        "bad_request" => format!("the app could not read the request: {}", detail.unwrap_or("")),
        other => format!("the app refused: {other}"),
    }
}

fn outcome(answer: &Value, asked: bool) -> Outcome {
    match answer.get("answer").and_then(Value::as_str) {
        Some("done") => Outcome {
            status: Status::Done,
            refused: None,
            message: if asked { "the person allowed it; done".into() } else { "done".into() },
            asked_the_person: asked,
            earlier: Vec::new(),
        },
        Some("asking") => Outcome {
            status: Status::Asking,
            refused: None,
            message:
                "the app is asking the person; their answer has not come yet (wait for it with app_wait)"
                    .into(),
            asked_the_person: true,
            earlier: Vec::new(),
        },
        Some("refused") => {
            let code = answer.get("refused").and_then(Value::as_str).unwrap_or("refused").to_owned();
            let message = refusal_sentence(&code, answer.get("detail").and_then(Value::as_str));
            Outcome {
                status: Status::Refused,
                refused: Some(code),
                message,
                asked_the_person: asked,
                earlier: Vec::new(),
            }
        }
        _ => Outcome {
            status: Status::Refused,
            refused: Some("bad_answer".into()),
            message: format!("the app answered something unexpected: {answer}"),
            asked_the_person: asked,
            earlier: Vec::new(),
        },
    }
}

struct Session {
    client: LineClient,
    token: String,
    answer_timeout: Duration,
    /// Requests the person has yet to answer.
    pending: usize,
    /// Their outcomes, heard while doing something else.
    heard: Vec<String>,
    /// A request went unanswered: its late answer would be taken for the next request's.
    lost: bool,
}

/// The connections to running apps, by name.
#[derive(Default)]
pub struct Live {
    sessions: BTreeMap<String, Session>,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Live({:?})", self.sessions.keys().collect::<Vec<_>>())
    }
}

fn name(c: &Option<String>) -> String {
    c.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "app".into())
}

fn parse(line: &str) -> Result<Value> {
    serde_json::from_str(line)
        .map_err(|e| ToolError::new(format!("the app sent something that is not JSON ({e}): {line}")))
}

impl Session {
    fn read(&mut self) -> Result<Value> {
        match self.client.recv(self.answer_timeout)? {
            Some(l) => parse(&l),
            None => {
                self.lost = true;
                Err(ToolError::new(format!(
                    "the app did not answer within {} s (is its window's loop running?): connect again",
                    self.answer_timeout.as_secs()
                )))
            }
        }
    }

    /// Hears outcomes of earlier requests that arrived meanwhile, without waiting.
    fn drain(&mut self) -> Result<()> {
        while self.pending > 0 {
            let Some(line) = self.client.recv(Duration::ZERO)? else { break };
            let o = outcome(&parse(&line)?, true);
            self.pending -= 1;
            self.heard.push(o.message);
        }
        Ok(())
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        self.drain()?;
        let req = json!({"request": "snapshot", "token": self.token});
        self.client.send(&req.to_string())?;
        loop {
            let answer = self.read()?;
            match answer.get("answer").and_then(Value::as_str) {
                Some("snapshot") => {
                    return serde_json::from_value(answer)
                        .map_err(|e| ToolError::new(format!("the app's snapshot could not be read: {e}")));
                }
                Some("refused")
                    if self.pending == 0
                        || matches!(
                            answer.get("refused").and_then(Value::as_str),
                            Some("no_access" | "bad_request")
                        ) =>
                {
                    return Err(ToolError::new(outcome(&answer, false).message));
                }
                // An earlier request's outcome, arriving first.
                _ if self.pending > 0 => {
                    self.pending -= 1;
                    self.heard.push(outcome(&answer, true).message);
                }
                _ => return Err(ToolError::new(format!("the app answered something unexpected: {answer}"))),
            }
        }
    }

    /// Sends a request that changes something, and reads its outcome, waiting up to `wait` for
    /// the person when the app asks them.
    fn change(&mut self, req: Value, wait: Duration) -> Result<Outcome> {
        self.drain()?;
        if self.pending > 0 {
            return Err(ToolError::new(
                "the person has not answered an earlier request yet: wait for it (app_wait) before asking for more",
            ));
        }
        self.client.send(&req.to_string())?;
        let first = self.read()?;
        let mut result = outcome(&first, false);
        if result.status == Status::Asking {
            match self.client.recv(wait)? {
                Some(line) => result = outcome(&parse(&line)?, true),
                None => self.pending += 1,
            }
        }
        result.earlier = std::mem::take(&mut self.heard);
        Ok(result)
    }

    fn wait(&mut self, wait: Duration) -> Result<Outcome> {
        let mut earlier = std::mem::take(&mut self.heard);
        if self.pending == 0 {
            let message = if earlier.is_empty() {
                "nothing is waiting for the person".into()
            } else {
                earlier.remove(0)
            };
            return Ok(Outcome {
                status: Status::Done,
                refused: None,
                message,
                asked_the_person: false,
                earlier,
            });
        }
        match self.client.recv(wait)? {
            Some(line) => {
                self.pending -= 1;
                let mut o = outcome(&parse(&line)?, true);
                o.earlier = earlier;
                Ok(o)
            }
            None => {
                let mut o = outcome(&json!({"answer": "asking"}), true);
                o.earlier = earlier;
                Ok(o)
            }
        }
    }
}

const ACTIONS: &[&str] =
    &["click", "focus", "increment", "decrement", "expand", "collapse", "scroll_into_view", "set_value"];

impl Live {
    /// No connections.
    pub fn new() -> Live {
        Live::default()
    }

    fn session(&mut self, c: &Option<String>) -> Result<&mut Session> {
        let n = name(c);
        let known: Vec<String> = self.sessions.keys().cloned().collect();
        self.sessions.get_mut(&n).ok_or_else(|| {
            if known.is_empty() {
                ToolError::new("not connected to an app: connect with the address and token the person gave (app_connect)")
            } else {
                ToolError::new(format!("no connection `{n}`; connected: {}", known.join(", ")))
            }
        })
    }

    /// Runs `f` on a connection, and closes the connection when a request went unanswered.
    fn with<T>(&mut self, c: &Option<String>, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        let s = self.session(c)?;
        let result = f(s);
        if s.lost {
            self.sessions.remove(&name(c));
        }
        result
    }

    /// Connects and takes a first snapshot, which also proves the token.
    pub fn connect(&mut self, input: &ConnectInput) -> Result<Connected> {
        let token = input.token.trim().to_owned();
        if token.is_empty() || token.contains(['\n', '\r']) {
            return Err(ToolError::new("the token is one line, as the person gave it"));
        }
        let timeout = Duration::from_secs(input.answer_timeout_seconds.unwrap_or(30).max(1));
        let mut client = LineClient::connect(&input.address, timeout)?;
        client.send(&token)?;
        let address = client.addr().to_string();
        let mut session =
            Session { client, token, answer_timeout: timeout, pending: 0, heard: Vec::new(), lost: false };
        let snapshot = session.snapshot()?;
        let connection = name(&input.connection);
        self.sessions.insert(connection.clone(), session);
        Ok(Connected { connection, address, snapshot })
    }

    /// What the window shows and offers now.
    pub fn snapshot(&mut self, input: &ConnectionInput) -> Result<Snapshot> {
        self.with(&input.connection, Session::snapshot)
    }

    /// Acts on an element.
    pub fn act(&mut self, input: &ActInput) -> Result<Outcome> {
        let action = input.action.trim().to_lowercase().replace(['-', ' '], "_");
        if !ACTIONS.contains(&action.as_str()) {
            return Err(ToolError::new(format!(
                "no action `{}`; actions: {}",
                input.action,
                ACTIONS.join(", ")
            )));
        }
        let what = if action == "set_value" {
            let Some(v) = &input.value else { return Err(ToolError::new("set_value needs `value`")) };
            json!({"do": "set_value", "value": v})
        } else {
            json!({"do": action})
        };
        self.with(&input.connection, |s| {
            let req = json!({"request": "act", "token": s.token, "node": input.node, "action": what});
            s.change(req, Duration::from_secs(input.wait_seconds.unwrap_or(60)))
        })
    }

    /// Runs one of the app's commands.
    pub fn command(&mut self, input: &CommandInput) -> Result<Outcome> {
        self.with(&input.connection, |s| {
            let req = json!({"request": "command", "token": s.token, "id": input.id});
            s.change(req, Duration::from_secs(input.wait_seconds.unwrap_or(60)))
        })
    }

    /// Undoes the latest change in the window.
    pub fn undo(&mut self, input: &ConnectionInput) -> Result<Outcome> {
        self.with(&input.connection, |s| {
            let req = json!({"request": "undo", "token": s.token});
            s.change(req, Duration::ZERO)
        })
    }

    /// Waits for the person's answer to an earlier request.
    pub fn wait(&mut self, input: &WaitInput) -> Result<Outcome> {
        self.session(&input.connection)?.wait(Duration::from_secs(input.wait_seconds.unwrap_or(60)))
    }

    /// Closes a connection.
    pub fn disconnect(&mut self, input: &ConnectionInput) -> Result<String> {
        let n = name(&input.connection);
        match self.sessions.remove(&n) {
            Some(_) => Ok(format!("disconnected from `{n}`")),
            None => Err(ToolError::new(format!("no connection `{n}`"))),
        }
    }

    /// The open connections and their addresses.
    pub fn connections(&self) -> Vec<(String, String)> {
        self.sessions.iter().map(|(n, s)| (n.clone(), s.client.addr().to_string())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_read_as_indented_text() {
        let s: Snapshot = serde_json::from_value(json!({
            "tree": [{"id": 3, "role": "group", "name": "Voice", "actions": [], "children": [
                {"id": 4, "role": "switch", "name": "Noise", "checked": false, "actions": ["click"]}
            ]}],
            "commands": [{"id": "chat.delete", "label": "Delete chat", "enabled": true, "destructive": true}],
            "undo": "Undo Press Noise, by assistant"
        }))
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            s.text(),
            "group \"Voice\" #3\n  switch \"Noise\" [off] {click} #4\ncommands:\n  chat.delete \"Delete chat\" [asks the person]\nundo: Undo Press Noise, by assistant\n"
        );
    }

    #[test]
    fn answers_become_outcomes() {
        assert_eq!(outcome(&json!({"answer": "done"}), false).status, Status::Done);
        let r = outcome(&json!({"answer": "refused", "refused": "read_only"}), false);
        assert_eq!((r.status, r.refused.as_deref()), (Status::Refused, Some("read_only")));
        let d = outcome(&json!({"answer": "refused", "refused": "declined"}), true);
        assert!(d.message.contains("declined") && d.asked_the_person);
        let bad = outcome(&json!({"answer": "refused", "refused": "bad_request", "detail": "eof"}), false);
        assert!(bad.message.contains("eof"));
    }

    #[test]
    fn an_unanswered_request_closes_the_connection() {
        let Ok(server) = std::net::TcpListener::bind("127.0.0.1:0") else { unreachable!("a loopback port") };
        let Ok(addr) = server.local_addr() else { unreachable!("bound") };
        // An app that accepts and never answers; it lets go when the test's end closes the channel.
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let silent = std::thread::spawn(move || {
            let held = server.accept();
            let _ = wait.recv_timeout(Duration::from_secs(5));
            drop(held);
        });
        let Ok(client) = LineClient::connect(&addr.to_string(), Duration::from_secs(5)) else {
            unreachable!("connects")
        };
        let mut live = Live::new();
        live.sessions.insert(
            "app".into(),
            Session {
                client,
                token: "t".into(),
                answer_timeout: Duration::from_millis(50),
                pending: 0,
                heard: Vec::new(),
                lost: false,
            },
        );
        assert!(live.undo(&ConnectionInput::default()).is_err());
        assert!(live.connections().is_empty(), "a late answer must not meet the next request");
        drop(done);
        let _ = silent.join();
    }

    #[test]
    fn requests_need_a_connection() {
        let mut live = Live::new();
        assert!(live.snapshot(&ConnectionInput::default()).is_err());
        assert!(live.act(&ActInput { action: "jump".into(), ..Default::default() }).is_err());
        assert!(
            live.connect(&ConnectInput {
                address: "127.0.0.1:1".into(),
                token: String::new(),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            live.connect(&ConnectInput {
                address: "8.8.8.8:80".into(),
                token: "t".into(),
                ..Default::default()
            })
            .is_err()
        );
    }
}
