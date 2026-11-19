//! `cargo remedy preview` and `cargo remedy audit`.

use remedy_tools::Workspace;
use remedy_tools::preview::{ListInput, PreviewInput, list, render};

use super::{Args, err};

/// Renders a preview (`audit`: reports its accessibility problems, failing when there are
/// any).
pub fn preview(a: &Args, audit: bool) -> Result<(), String> {
    let function = a.pos(0).ok_or("usage: cargo remedy preview <function> [--png <file>] (a `fn(&Ui) -> El` of the app's library, such as home)")?;
    let (width, height) = match a.get("--size") {
        Some(s) => {
            let (w, h) =
                s.split_once(['x', 'X']).ok_or(format!("--size takes <width>x<height>, not `{s}`"))?;
            let n = |v: &str| {
                v.trim().parse::<f32>().map_err(|_| format!("--size takes <width>x<height>, not `{s}`"))
            };
            (Some(n(w)?), Some(n(h)?))
        }
        None => (None, None),
    };
    let mut input = PreviewInput::new(a.get("--app").unwrap_or_else(|| ".".into()), function);
    input.width = width;
    input.height = height;
    input.scale = a.number("--scale")?;
    input.theme = a.get("--theme");
    input.locale = a.get("--locale");
    input.text_scale = a.number("--text-scale")?;
    input.timeout_seconds = a.number("--timeout")?;
    let png = if audit { None } else { a.get("--png") };
    let out = render(&input, &Workspace::anywhere(), png.as_deref()).map_err(err)?;
    let mut text = String::new();
    if !audit {
        text.push_str(&format!(
            "{}x{} px, theme {} ({}), locale {}{}\n",
            out.width,
            out.height,
            out.theme,
            if out.dark { "dark" } else { "light" },
            out.locale,
            if out.rtl { ", right to left" } else { "" }
        ));
        match &out.png_file {
            Some(f) => text.push_str(&format!("wrote {f}\n")),
            None => text.push_str("(add --png <file> to save the image)\n"),
        }
        text.push_str("\nscreen reader view:\n");
        text.push_str(&out.reader_text());
    }
    text.push_str(&format!("{} accessibility problem(s)\n", out.problems.len()));
    for p in &out.problems {
        text.push_str(&format!("  #{}: {}\n", p.element, p.message));
    }
    a.print(&text, &out)?;
    if audit && !out.problems.is_empty() {
        return Err(format!("{} accessibility problem(s)", out.problems.len()));
    }
    Ok(())
}

/// Lists the app's `#[preview]` functions.
pub fn previews(a: &Args) -> Result<(), String> {
    let found = list(&ListInput::new(a.get("--app").unwrap_or_else(|| ".".into())), &Workspace::anywhere())
        .map_err(err)?;
    let mut text = String::new();
    for d in &found {
        let reach =
            if d.public { "" } else { " (private: make it pub to render it with `cargo remedy preview`)" };
        text.push_str(&format!("{}{reach}\n  {} at {}:{}\n", d.function, d.attribute, d.file, d.line));
    }
    if found.is_empty() {
        text.push_str("no #[preview] functions\n");
    }
    a.print(&text, &found)
}
