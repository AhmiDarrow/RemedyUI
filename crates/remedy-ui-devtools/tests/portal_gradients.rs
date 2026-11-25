//! Portal's gradients (its page glow, the poster art's glow, the cast screen, the hero and
//! info scrims, the full-screen player's scrims, the seek track's hard stops), written as
//! Portal's styles.css writes them, drawn through the style, view and CPU renderer, and
//! compared pixel by pixel with Chrome drawing the same CSS
//! (`tests/portal_gradients/<case>.png`, from `cases.txt` by `make_references.py`).
//! `REMEDY_SHOTS=<dir>` writes RemedyUI's pictures too.

use remedy_ui_core::Color;
use remedy_ui_devtools::{Preview, Snapshot};
use remedy_ui_core::style::{ColorRef, ColorRole, Gradient, Style};
use remedy_ui_engine::view::Ui;

/// A case's background color: `var(--name)` of Portal's, or a hex color.
fn color(css: &str) -> ColorRef {
    match css.strip_prefix("var(--").and_then(|v| v.strip_suffix(')')) {
        Some("bg") => ColorRole::Bg.into(),
        Some("well") => ColorRole::Custom("well").into(),
        Some(other) => unreachable!("no case uses --{other}"),
        None => Color::from_hex(css).unwrap_or_else(|_| unreachable!("{css}")).into(),
    }
}

/// How far RemedyUI's picture is from Chrome's: the largest channel difference, and the mean.
fn distance(a: &Snapshot, b: &Snapshot) -> (u8, f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (mut max, mut sum) = (0u8, 0u64);
    for (x, y) in a.rgba.iter().zip(&b.rgba) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += u64::from(d);
    }
    (max, sum as f64 / a.rgba.len() as f64)
}

#[test]
fn portals_gradients_match_chrome() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/portal_gradients");
    let cases = std::fs::read_to_string(dir.join("cases.txt")).unwrap_or_else(|e| unreachable!("{e}"));
    let shots = std::env::var_os("REMEDY_SHOTS").map(std::path::PathBuf::from);
    let mut report = Vec::new();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let [name, w, h, bg, layers] = line.split('|').collect::<Vec<_>>()[..] else {
            unreachable!("{line}")
        };
        let (w, h): (f32, f32) = (w.parse().unwrap_or_default(), h.parse().unwrap_or_default());
        let layers = Gradient::parse_layers(layers).unwrap_or_else(|e| unreachable!("{name}: {e}"));
        let bg = color(bg);
        let preview = Preview::new(name, move |ui: &Ui| {
            ui.el("box")
                .width(w)
                .height(h)
                .style(Style::new().background(bg).background_layers(layers.clone()))
        })
        .themes(&["portal:portal"])
        .sizes(&[(w, h)]);
        let variation =
            preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("one theme, one size"));
        let ours = preview.render(&variation);
        let bytes = std::fs::read(dir.join(format!("{name}.png"))).unwrap_or_else(|e| unreachable!("{e}"));
        let chrome = Snapshot::from_png(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
        if let (Some(d), Ok(png)) = (&shots, ours.to_png()) {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(d.join(format!("portal-gradient-{name}.png")), png);
        }
        let (max, mean) = distance(&ours, &chrome);
        report.push(format!("{name}: max {max}, mean {mean:.3}"));
        // Chrome dithers its gradients by a level or two.
        assert!(max <= 5 && mean < 0.5, "{name}: max {max}, mean {mean:.3}");
    }
    eprintln!("{}", report.join("\n"));
}
