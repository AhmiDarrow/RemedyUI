//! A small app around a Lua widget: loads `pulse.lua` with a theme and an app element, then
//! drives it the way a window's frame loop does (render, then `advance` when it asks for a
//! frame) and prints what it shows. A theme change halfway through shows tokens following.
//!
//! `cargo run -p remedy-ui-engine --features script-lua --example frames`

use std::time::Duration;

use remedy_ui_engine::script::{Limits, Node, ScriptWidget};
use remedy_ui_engine::script_lua::{Grants, LuaEngine};
use serde_json::{Map, json};

fn show(n: &Node, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match n {
        Node::Column { children, .. } | Node::Row { children, .. } => {
            for c in children {
                show(c, indent, out);
            }
        }
        Node::Text { text, .. } => out.push_str(&format!("{pad}{text}\n")),
        Node::Element { kind, props, .. } => {
            out.push_str(&format!("{pad}<{kind} {}>\n", serde_json::Value::Object(props.clone())))
        }
        other => out.push_str(&format!("{pad}{other:?}\n")),
    }
}

fn main() {
    let grants = Grants::none().element("dot").theme(json!({ "color": { "accent": "#5865f2" } }));
    let engine = LuaEngine::with_grants(grants);
    let mut widget = match engine.load_source(include_str!("pulse.lua"), Limits::default()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("the widget did not load: {e}");
            return;
        }
    };
    let mut clock = Duration::ZERO;
    let mut frames = 0;
    while clock < Duration::from_millis(2500) {
        match widget.render(&Map::new(), &json!({})) {
            Ok(node) => {
                let mut out = String::new();
                show(&node, 2, &mut out);
                println!("at {:>5} ms:\n{out}", clock.as_millis());
            }
            Err(e) => {
                eprintln!("stopped: {e}");
                return;
            }
        }
        if clock >= Duration::from_millis(1200) && frames == 0 {
            frames = 1;
            if let Err(e) = widget.set_theme(&json!({ "color": { "accent": "#e8590c" } })) {
                eprintln!("theme: {e}");
            }
        }
        // The frame loop: as soon as the widget asks, with nothing in between.
        let Some(next) = widget.next_frame() else { break };
        let step = next.max(Duration::from_millis(100));
        clock += step;
        if let Err(e) = widget.advance(step) {
            eprintln!("stopped: {e}");
            return;
        }
    }
}
