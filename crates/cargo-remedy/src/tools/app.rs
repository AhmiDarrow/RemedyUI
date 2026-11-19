//! `cargo remedy app` (a running app, through its agents listener, under the person's grant)
//! and `cargo remedy devtools` (a running app's remote devtools).
//!
//! Each command connects, does one thing and closes. When the app asks the person first, the
//! command waits for their answer (`--wait`, 60 seconds by default).

use remedy_tools::devtools::{Devtools, DevtoolsConnectInput, ElementInput, SignalsInput};
use remedy_tools::live::{ActInput, CommandInput, ConnectInput, ConnectionInput, Live, Outcome, Status};

use super::{Args, err};

fn endpoint(a: &Args, env: &str) -> Result<(String, String), String> {
    let address = a
        .get("--address")
        .or_else(|| std::env::var(format!("{env}_ADDRESS")).ok())
        .ok_or(format!("give the app's address with --address (or {env}_ADDRESS)"))?;
    let token = a
        .get("--token")
        .or_else(|| std::env::var(format!("{env}_TOKEN")).ok())
        .ok_or(format!("give the token with --token (or {env}_TOKEN)"))?;
    Ok((address, token))
}

fn report(a: &Args, o: &Outcome) -> Result<(), String> {
    let mut text = o.message.clone();
    for e in &o.earlier {
        text.push_str(&format!("\nearlier: {e}"));
    }
    a.print(&text, o)?;
    match o.status {
        Status::Done => Ok(()),
        Status::Asking => Err("the person has not answered yet".into()),
        _ => Err(o.message.clone()),
    }
}

/// `app connect|snapshot|act|command|undo`.
pub fn app(a: &Args) -> Result<(), String> {
    let (address, token) = endpoint(a, "REMEDY_APP")?;
    let mut live = Live::new();
    let mut input = ConnectInput::new(address, token);
    input.answer_timeout_seconds = a.number("--timeout")?;
    let connected = live.connect(&input).map_err(err)?;
    let wait = Some(a.number::<u64>("--wait")?.unwrap_or(60));
    let sub = a
        .pos(0)
        .ok_or("usage: cargo remedy app connect|snapshot|act|command|undo --address <a> --token <t>")?;
    match sub {
        "connect" => a.print(
            &format!("connected to {}\n\n{}", connected.address, connected.snapshot.text()),
            &connected,
        ),
        "snapshot" => a.print(&connected.snapshot.text(), &connected.snapshot),
        "act" => {
            let node = a
                .pos(1)
                .and_then(|n| n.trim_start_matches('#').parse::<u64>().ok())
                .ok_or("usage: cargo remedy app act <node id> <action> [value]")?;
            let action = a.pos(2).ok_or("name the action: click, focus, increment, decrement, expand, collapse, scroll_into_view or set_value")?;
            let mut input = ActInput::new(node, action);
            input.value = a.pos(3).map(str::to_owned);
            input.wait_seconds = wait;
            let o = live.act(&input).map_err(err)?;
            report(a, &o)
        }
        "command" => {
            let id = a.pos(1).ok_or("usage: cargo remedy app command <id>")?;
            let mut input = CommandInput::new(id);
            input.wait_seconds = wait;
            let o = live.command(&input).map_err(err)?;
            report(a, &o)
        }
        "undo" => {
            let o = live.undo(&ConnectionInput::default()).map_err(err)?;
            report(a, &o)
        }
        other => Err(format!("no `app {other}`: connect, snapshot, act, command or undo")),
    }
}

/// `devtools tree|element|semantics|signals|updates|timings`.
pub fn devtools(a: &Args) -> Result<(), String> {
    let (address, token) = endpoint(a, "REMEDY_DEVTOOLS")?;
    let mut input = DevtoolsConnectInput::new(address, token);
    input.answer_timeout_seconds = a.number("--timeout")?;
    let mut d = Devtools::connect(&input).map_err(err)?;
    let id = |i: usize| a.pos(i).and_then(|n| n.trim_start_matches('#').parse::<u64>().ok());
    let out = match a.pos(0) {
        Some("tree") => d.tree(),
        Some("element") => {
            d.element(&ElementInput::new(id(1).ok_or("usage: cargo remedy devtools element <id>")?))
        }
        Some("semantics") => d.semantics(),
        Some("signals") => d.signals(&SignalsInput::new(id(1))),
        Some("updates") => d.updates(),
        Some("timings") => d.timings(),
        _ => {
            return Err(
                "usage: cargo remedy devtools tree|element <id>|semantics|signals [id]|updates|timings"
                    .into(),
            );
        }
    }
    .map_err(err)?;
    a.print(&out.text, &out.answer)
}
