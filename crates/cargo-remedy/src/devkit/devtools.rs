//! Reading a running app's remote devtools (`RemoteServer`, `Window::serve_devtools`): its
//! element tree, an element's details, the screen reader's view with problems, the signal
//! graph, why elements updated, and frame timings.
//!
//! Debug builds serve these on a loopback port with a token; the developer gives the address
//! and token. The protocol is JSON lines: the token on its own line, answered by
//! `{"answer":"welcome","version":1}` (or `refused`), then one request per line
//! (`{"request":"tree"}`, `element`, `semantics`, `signals`, `updates`, `frames`), each
//! answered by one line.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::devkit::lines::LineClient;
use crate::devkit::{Result, ToolError};

/// Where the devtools server listens, and its token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct DevtoolsConnectInput {
    /// The server's loopback address (`127.0.0.1:52101`).
    pub address: String,
    /// The server's token.
    pub token: String,
    /// How long to wait for each answer, in seconds (30 when absent).
    #[serde(default)]
    pub answer_timeout_seconds: Option<u64>,
}

impl DevtoolsConnectInput {
    /// A connection to `address` with `token`.
    pub fn new(address: impl Into<String>, token: impl Into<String>) -> Self {
        DevtoolsConnectInput { address: address.into(), token: token.into(), answer_timeout_seconds: None }
    }
}

/// An element to ask about.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ElementInput {
    /// The element's id, from the tree.
    pub id: u64,
}

impl ElementInput {
    /// Element `id`.
    pub fn new(id: u64) -> Self {
        ElementInput { id }
    }
}

/// Whose signals to read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct SignalsInput {
    /// An element's id: its bindings, what they read and what else depends on that. Absent:
    /// every node of the window thread's reactive graph.
    #[serde(default)]
    pub id: Option<u64>,
}

impl SignalsInput {
    /// Element `id`'s bindings, or with `None` the whole graph.
    pub fn new(id: Option<u64>) -> Self {
        SignalsInput { id }
    }
}

/// An answer, as JSON and as text for people.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct DevtoolsOutput {
    /// The server's answer.
    pub answer: Value,
    /// The answer as readable text.
    pub text: String,
}

/// A connection to an app's devtools server.
#[derive(Debug)]
pub struct Devtools {
    client: LineClient,
    timeout: Duration,
    version: u64,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn items<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn ms(ns: u64) -> String {
    format!("{:.2} ms", ns as f64 / 1e6)
}

fn node_line(v: &Value) -> String {
    format!("{} {} (#{}, {})", s(v, "kind"), s(v, "label"), n(v, "key"), s(v, "created_at"))
}

/// An answer as text.
pub fn text(answer: &Value) -> String {
    let mut out = String::new();
    match s(answer, "answer") {
        "tree" => {
            let elements = items(answer, "elements");
            for e in elements {
                let bounds = e
                    .get("bounds")
                    .and_then(Value::as_array)
                    .map(|b| {
                        let f: Vec<String> =
                            b.iter().map(|x| format!("{:.0}", x.as_f64().unwrap_or(0.0))).collect();
                        format!(" @{}", f.join(","))
                    })
                    .unwrap_or_default();
                let hidden =
                    if e.get("hidden").and_then(Value::as_bool) == Some(true) { " (hidden)" } else { "" };
                // No element is deeper than there are elements, whatever the app says.
                let depth = usize::try_from(n(e, "depth")).unwrap_or(0).min(elements.len());
                out.push_str(&format!(
                    "{}{} #{}{bounds}{hidden}\n",
                    "  ".repeat(depth),
                    s(e, "label"),
                    n(e, "id")
                ));
            }
        }
        "element" => {
            out.push_str(&format!("element #{}\n", n(answer, "id")));
            for row in items(answer, "details") {
                if let Some([k, v]) = row.as_array().map(Vec::as_slice) {
                    out.push_str(&format!("  {}: {}\n", k.as_str().unwrap_or(""), v.as_str().unwrap_or("")));
                }
            }
        }
        "semantics" => {
            let nodes = items(answer, "nodes");
            for node in nodes {
                let depth = usize::try_from(n(node, "depth")).unwrap_or(0).min(nodes.len());
                out.push_str(&format!(
                    "{}{} #{}\n",
                    "  ".repeat(depth),
                    s(node, "announcement"),
                    n(node, "id")
                ));
                for p in items(node, "problems") {
                    out.push_str(&format!("{}  ! {}\n", "  ".repeat(depth), p.as_str().unwrap_or("")));
                }
            }
            let problems = items(answer, "problems");
            out.push_str(&format!("{} problem(s)\n", problems.len()));
            for p in problems {
                out.push_str(&format!("  #{}: {}\n", n(p, "element"), s(p, "message")));
            }
        }
        "bindings" => {
            for b in items(answer, "bindings") {
                let node = b.get("node").cloned().unwrap_or_default();
                out.push_str(&format!("{}\n", node_line(&node)));
                for r in items(b, "reads") {
                    let rn = r.get("node").cloned().unwrap_or_default();
                    out.push_str(&format!("  reads {}\n", node_line(&rn)));
                    for d in items(r, "dependents") {
                        out.push_str(&format!("    also read by {}\n", node_line(d)));
                    }
                }
            }
        }
        "graph" => {
            for node in items(answer, "nodes") {
                let el = node
                    .get("element")
                    .and_then(Value::as_u64)
                    .map(|e| format!(" element #{e}"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{}{el}: {} source(s), {} subscriber(s)\n",
                    node_line(node),
                    items(node, "sources").len(),
                    items(node, "subscribers").len()
                ));
            }
        }
        "updates" => {
            for u in items(answer, "entries") {
                out.push_str(&format!("{} {}\n", ms(n(u, "at_ns")), s(u, "sentence")));
            }
        }
        "frames" => {
            let frames = items(answer, "frames");
            if answer.get("recording").and_then(Value::as_bool) == Some(false) {
                out.push_str(
                    "frame timings are not recorded (the app installed its own tracing subscriber)\n",
                );
            }
            let mut totals: Vec<u64> = frames.iter().map(|f| n(f, "frame_ns")).collect();
            totals.sort_unstable();
            if let Some(&worst) = totals.last() {
                let p50 = totals.get(totals.len() / 2).copied().unwrap_or(0);
                let p95 =
                    totals.get((totals.len() * 95).div_ceil(100).saturating_sub(1)).copied().unwrap_or(0);
                out.push_str(&format!(
                    "{} frames: p50 {}, p95 {}, worst {}\n",
                    totals.len(),
                    ms(p50),
                    ms(p95),
                    ms(worst)
                ));
            }
            if let Some(last) = frames.last() {
                out.push_str(&format!("last frame {}:", ms(n(last, "frame_ns"))));
                for p in items(last, "phases") {
                    if let Some([name, t]) = p.as_array().map(Vec::as_slice) {
                        out.push_str(&format!(
                            " {} {}",
                            name.as_str().unwrap_or(""),
                            ms(t.as_u64().unwrap_or(0))
                        ));
                    }
                }
                out.push_str(&format!(", other {}\n", ms(n(last, "other_ns"))));
            }
        }
        "error" => out.push_str(&format!("error: {}\n", s(answer, "message"))),
        other => out.push_str(&format!("{other}: {answer}\n")),
    }
    out
}

impl Devtools {
    /// Connects and presents the token.
    pub fn connect(input: &DevtoolsConnectInput) -> Result<Devtools> {
        let token = input.token.trim();
        if token.is_empty() || token.contains(['\n', '\r']) {
            return Err(ToolError::new("the token is one line, as the app printed it"));
        }
        let timeout = Duration::from_secs(input.answer_timeout_seconds.unwrap_or(30).max(1));
        let mut client = LineClient::connect(&input.address, timeout)?;
        client.send(token)?;
        let Some(line) = client.recv(timeout)? else {
            return Err(ToolError::new("the devtools server did not greet within the time"));
        };
        let greeting: Value = serde_json::from_str(&line)
            .map_err(|e| ToolError::new(format!("unexpected greeting ({e}): {line}")))?;
        match s(&greeting, "answer") {
            "welcome" => Ok(Devtools { client, timeout, version: n(&greeting, "version") }),
            "refused" => Err(ToolError::new("the devtools server refused the token")),
            _ => Err(ToolError::new(format!("unexpected greeting: {line}"))),
        }
    }

    /// The protocol version the server speaks.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The address connected to.
    pub fn address(&self) -> String {
        self.client.addr().to_string()
    }

    fn ask(&mut self, req: Value) -> Result<DevtoolsOutput> {
        self.client.send(&req.to_string())?;
        let Some(line) = self.client.recv(self.timeout)? else {
            return Err(ToolError::new("the app did not answer in time (is its window's loop running?)"));
        };
        let answer: Value = serde_json::from_str(&line)
            .map_err(|e| ToolError::new(format!("the app sent something that is not JSON ({e})")))?;
        if s(&answer, "answer") == "error" {
            return Err(ToolError::new(s(&answer, "message").to_owned()));
        }
        let text = text(&answer);
        Ok(DevtoolsOutput { answer, text })
    }

    /// Every element of the window, depth first.
    pub fn tree(&mut self) -> Result<DevtoolsOutput> {
        self.ask(json!({"request": "tree"}))
    }

    /// One element's details (widget, source line, box, style, states, semantics).
    pub fn element(&mut self, input: &ElementInput) -> Result<DevtoolsOutput> {
        self.ask(json!({"request": "element", "id": input.id}))
    }

    /// The window as a screen reader meets it, and its accessibility problems.
    pub fn semantics(&mut self) -> Result<DevtoolsOutput> {
        self.ask(json!({"request": "semantics"}))
    }

    /// An element's bindings, or the whole reactive graph.
    pub fn signals(&mut self, input: &SignalsInput) -> Result<DevtoolsOutput> {
        match input.id {
            Some(id) => self.ask(json!({"request": "signals", "id": id})),
            None => self.ask(json!({"request": "signals"})),
        }
    }

    /// Why elements updated, recently.
    pub fn updates(&mut self) -> Result<DevtoolsOutput> {
        self.ask(json!({"request": "updates"}))
    }

    /// Recent frames' timings by phase.
    pub fn timings(&mut self) -> Result<DevtoolsOutput> {
        self.ask(json!({"request": "frames"}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read_as_text() {
        let tree = json!({"answer": "tree", "elements": [
            {"id": 1, "parent": null, "depth": 0, "widget": "column", "label": "column", "hidden": false, "bounds": [0.0, 0.0, 400.0, 300.0]},
            {"id": 2, "parent": 1, "depth": 1, "widget": "button", "label": "button \"Save\"", "hidden": false, "bounds": null}
        ]});
        assert_eq!(text(&tree), "column #1 @0,0,400,300\n  button \"Save\" #2\n");
        let frames = json!({"answer": "frames", "recording": true, "frames": [
            {"start_ns": 0, "frame_ns": 2_000_000, "phases": [["layout", 500_000]], "other_ns": 0},
            {"start_ns": 1, "frame_ns": 4_000_000, "phases": [["layout", 1_000_000]], "other_ns": 100_000}
        ]});
        let t = text(&frames);
        assert!(t.contains("2 frames: p50 4.00 ms, p95 4.00 ms, worst 4.00 ms"), "{t}");
        assert!(t.contains("last frame 4.00 ms: layout 1.00 ms, other 0.10 ms"), "{t}");
        let deep =
            json!({"answer": "tree", "elements": [{"id": 1, "depth": 4_000_000_000_u32, "label": "x"}]});
        assert_eq!(text(&deep), "  x #1\n", "a depth the tree cannot have is held to its size");
        let sem = json!({"answer": "semantics", "nodes": [{"id": 5, "depth": 0, "announcement": "button", "problems": ["Control with no accessible name"]}], "problems": [{"element": 5, "message": "Control with no accessible name"}]});
        assert!(
            text(&sem).contains("! Control with no accessible name") && text(&sem).contains("1 problem(s)")
        );
    }

    #[test]
    fn a_refused_token_is_reported() {
        let Ok(server) = std::net::TcpListener::bind("127.0.0.1:0") else { unreachable!("a port") };
        let Ok(addr) = server.local_addr() else { unreachable!("bound") };
        let t = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let Ok((mut s, _)) = server.accept() else { return };
            let mut line = String::new();
            let Ok(clone) = s.try_clone() else { return };
            let _ = std::io::BufReader::new(clone).read_line(&mut line);
            let _ = writeln!(s, "{{\"answer\":\"refused\"}}");
        });
        let r = Devtools::connect(&DevtoolsConnectInput {
            address: addr.to_string(),
            token: "nope".into(),
            answer_timeout_seconds: Some(10),
        });
        assert!(r.is_err_and(|e| e.0.contains("refused")));
        let _ = t.join();
    }
}
