//! `cargo remedy rfx check|render|library`.

use cargo_remedy::devkit::Workspace;
use cargo_remedy::devkit::effects::{EffectInput, RenderInput, check, check_text, library, render};

use super::{Args, err};

const USAGE: &str = "usage: cargo remedy rfx check <file.rfx | built-in name> [--untrusted]\n       \
                     cargo remedy rfx render <file.rfx | built-in name> --png <out.png> [--size <w>x<h>] [--scale <s>] [--time <t>] [--set name=v[,v..]]... [--background #rrggbb|none]\n       \
                     cargo remedy rfx library";

fn effect(a: &Args) -> Result<EffectInput, String> {
    let what = a.pos(1).ok_or(USAGE)?;
    let input = if what.ends_with(".rfx") || std::path::Path::new(what).is_file() {
        EffectInput::from_path(what)
    } else {
        EffectInput::built_in(what)
    };
    Ok(input.untrusted(a.on("--untrusted")))
}

/// The `rfx` commands. A check that finds a problem fails, so scripts can rely on the exit
/// code.
pub fn rfx(a: &Args) -> Result<(), String> {
    let ws = Workspace::anywhere();
    match a.pos(0) {
        Some("check") => {
            let out = check(&effect(a)?, &ws).map_err(err)?;
            if out.ok {
                a.print(&check_text(&out), &out)
            } else {
                if a.on("--json") {
                    a.print("", &out)?;
                } else {
                    eprint!("{}", check_text(&out));
                }
                Err(format!("{} does not compile", a.pos(1).unwrap_or("the effect")))
            }
        }
        Some("render") => {
            let png = a.get("--png").ok_or("render needs --png <out.png>")?;
            let mut input = RenderInput::new(effect(a)?);
            if let Some(size) = a.get("--size") {
                let (w, h) = size.split_once('x').ok_or("--size is <width>x<height>")?;
                input.width = w.trim().parse().map_err(|_| format!("`{w}` is not a width"))?;
                input.height = h.trim().parse().map_err(|_| format!("`{h}` is not a height"))?;
            }
            if let Some(s) = a.number::<f32>("--scale")? {
                input.scale = s;
            }
            input.time = a.number::<f32>("--time")?;
            input.background = a.get("--background");
            for set in a.all("--set") {
                let (name, vals) = set.split_once('=').ok_or("--set is name=value[,value..]")?;
                let v: Result<Vec<f32>, _> = vals.split(',').map(|x| x.trim().parse::<f32>()).collect();
                input
                    .values
                    .insert(name.trim().to_owned(), v.map_err(|_| format!("`{vals}` is not numbers"))?);
            }
            let out = render(&input, &ws, Some(&png)).map_err(err)?;
            let mut text = format!(
                "wrote {} ({} x {}, drawn at {} x {}, at t = {} s, {:.2} ms on the CPU)\n",
                out.png_file.clone().unwrap_or(png),
                out.width,
                out.height,
                out.effect_width,
                out.effect_height,
                out.time,
                out.cpu_ms
            );
            if !out.ignored.is_empty() {
                text.push_str(&format!(
                    "ignored: {} (no such parameter, or another type)\n",
                    out.ignored.join(", ")
                ));
            }
            a.print(&text, &out)
        }
        Some("library") => {
            let lib = library();
            let text: String =
                lib.iter().map(|e| format!("{:<18} cost {:>4}  {}\n", e.name, e.cost, e.summary)).collect();
            a.print(&text, &lib)
        }
        _ => Err(USAGE.to_owned()),
    }
}
