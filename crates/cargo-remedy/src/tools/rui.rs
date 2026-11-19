//! `cargo remedy rui check|compile|elements`.

use remedy_tools::Workspace;
use remedy_tools::markup::{ElementSpec, MarkupInput, built_in, check, compile};

use super::{Args, err};

fn input(a: &Args) -> Result<MarkupInput, String> {
    let file =
        a.pos(1).ok_or("usage: cargo remedy rui check|compile <file.rui> [--elements <specs.json>]")?;
    let elements: Vec<ElementSpec> = match a.get("--elements") {
        Some(p) => {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
            serde_json::from_str(&text).map_err(|e| format!("{p}: {e}"))?
        }
        None => Vec::new(),
    };
    let mut input = MarkupInput::from_path(file);
    input.elements = elements;
    Ok(input)
}

/// The `rui` commands. A check that finds a problem fails, so scripts can rely on the exit
/// code.
pub fn rui(a: &Args) -> Result<(), String> {
    let ws = Workspace::anywhere();
    match a.pos(0) {
        Some("check") => {
            let out = check(&input(a)?, &ws).map_err(err)?;
            if a.on("--json") {
                a.print("", &out)?;
            } else if out.error.is_none() {
                let mut t = String::from("ok\n");
                for s in &out.signals {
                    t.push_str(&format!("signal ${}: {} (Signal<{}>)\n", s.name, s.kind, s.rust_type));
                }
                for c in &out.commands {
                    t.push_str(&format!("command !{c}\n"));
                }
                print!("{t}");
            }
            match out.error {
                Some(e) => Err(e.to_string()),
                None => Ok(()),
            }
        }
        Some("compile") => {
            let out = compile(&input(a)?, &ws).map_err(err)?;
            if let Some(e) = &out.error {
                if a.on("--json") {
                    a.print("", &out)?;
                }
                return Err(e.to_string());
            }
            let rust = out.rust.clone().unwrap_or_default();
            match a.get("--out") {
                Some(path) => {
                    std::fs::write(&path, &rust).map_err(|e| format!("{path}: {e}"))?;
                    a.print(&format!("wrote {path}\n"), &out)
                }
                None => a.print(&rust, &out),
            }
        }
        Some("elements") => {
            let els = built_in();
            let mut text = String::new();
            for e in &els {
                let ps: Vec<String> = e
                    .params
                    .iter()
                    .map(|p| format!("{}: {}{}", p.name, p.kind, if p.required { "" } else { "?" }))
                    .collect();
                text.push_str(&format!(
                    "{}({}){}\n",
                    e.name,
                    ps.join(", "),
                    if e.children { " { children }" } else { "" }
                ));
            }
            a.print(&text, &els)
        }
        _ => Err("usage: cargo remedy rui check|compile <file.rui> | rui elements".into()),
    }
}
