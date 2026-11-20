//! Blur, shadows and frosted glass on ordinary elements, from their style: a Prism aurora
//! behind a pane of frosted glass (`backdrop_blur`), cards with box shadows (`shadow`), a badge
//! whose drop shadow follows the shape of its text (`drop_shadow`), and a blurred placeholder
//! (`blur`). The same picture on the GPU and the CPU renderer.
//!
//! ```text
//! cargo run --release -p remedy-ui --example glass_and_shadows
//! cargo run --release -p remedy-ui --example glass_and_shadows -- --cpu
//! cargo run --release -p remedy-ui --example glass_and_shadows -- --snapshot glass.png
//! ```
//!
//! With `--snapshot <file>` it waits two seconds, saves a picture of the window (read back from
//! the GPU when the GPU draws it) and quits; `--cpu` keeps the window on the CPU renderer.

use std::time::Duration;

use remedy_ui::core::Color;
use remedy_ui::effects::{Instance, effect_view, library};
use remedy_ui::style::{Shadow, Style};
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// A light card with a soft box shadow under it.
fn card(ui: &Ui, title: &str, body: &str) -> El {
    let shadow = Shadow::new(0.0, 8.0, 24.0, Color::rgba8(0, 0, 0, 110)).spread(-2.0);
    w::column(ui)
        .gap(6.0)
        .padding(16.0)
        .width(220.0)
        .style(Style::new().background(Color::rgb8(250, 248, 242)).radius(12.0).shadow(shadow))
        .child(
            w::text(ui, title.to_owned())
                .style(Style::new().foreground(Color::rgb8(30, 32, 40)).weight(650.0)),
        )
        .child(w::text(ui, body.to_owned()).style(Style::new().foreground(Color::rgb8(70, 74, 86))))
}

fn content(ui: &Ui) -> El {
    // Frosted glass: whatever is behind the pane (here, the aurora) shows through, blurred.
    let glass = w::column(ui)
        .gap(10.0)
        .padding(20.0)
        .style(
            Style::new()
                .backdrop_blur(14.0)
                .background(Color::rgba8(255, 255, 255, 38))
                .border(Color::rgba8(255, 255, 255, 90))
                .border_width(1.0)
                .radius(18.0),
        )
        .child(
            w::text(ui, "Frosted glass".to_owned())
                .style(Style::new().foreground(Color::WHITE).weight(700.0)),
        )
        .child(
            w::text(ui, "backdrop_blur(14): the aurora behind, blurred".to_owned())
                .style(Style::new().foreground(Color::rgba8(255, 255, 255, 220))),
        );
    // A drop shadow follows the shape of what the element draws, text included.
    let badge = w::text(ui, "drop_shadow follows the letters".to_owned()).style(
        Style::new().foreground(Color::rgb8(255, 214, 102)).weight(700.0).drop_shadow(Shadow::new(
            2.0,
            3.0,
            6.0,
            Color::rgba8(0, 0, 0, 200),
        )),
    );
    // A placeholder blurred while its content loads.
    let blurred = card(ui, "Loading…", "blur(3): content under a filter").style(Style::new().blur(3.0));
    w::column(ui).gap(28.0).padding(36.0).child(glass).child(badge).child(
        w::row(ui)
            .gap(28.0)
            .child(card(ui, "Box shadow", "shadow(0, 8, 24): a soft shadow under the card"))
            .child(blurred),
    )
}

fn app(app: &App, snapshot: Option<String>) {
    let win = app.open(WindowSpec::new("Glass and shadows").size(760.0, 520.0), |ui, _| {
        let background = match library("aurora") {
            Some(e) => effect_view(ui, Instance::new(e)).0,
            None => ui.el("background"),
        };
        let back = background.style(Style::new().background(Color::rgb8(12, 16, 32))).child(content(ui));
        ui.root().child(back.grow(1.0));
    });
    if let Some(path) = snapshot {
        // The aurora's still frame: snapshots from either renderer show the same moment.
        app.set_reduce_motion(true);
        let a = app.clone();
        app.after(Duration::from_secs(2), move || {
            let a2 = a.clone();
            let drawn_by = win.renderer();
            win.snapshot(move |image| {
                if let Some(i) = image {
                    let _ = save(&path, &i);
                    println!("saved {path} (drawn by {drawn_by:?})");
                }
                a2.quit();
            });
        });
    }
}

/// Writes premultiplied RGBA as a PNG, unpremultiplied.
fn save(path: &str, image: &remedy_ui::paint::Image) -> std::io::Result<()> {
    let px: Vec<u8> = image
        .pixels
        .chunks_exact(4)
        .flat_map(|p| {
            let [r, g, b, a] = <[u8; 4]>::try_from(p).unwrap_or([0; 4]);
            let un = |c: u8| {
                (u32::from(c) * 255 + u32::from(a) / 2)
                    .checked_div(u32::from(a))
                    .map_or(0, |v| v.min(255) as u8)
            };
            [un(r), un(g), un(b), a]
        })
        .collect();
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, image.width, image.height);
    enc.set_color(png::ColorType::Rgba);
    let mut wr = enc.write_header().map_err(std::io::Error::other)?;
    wr.write_image_data(&px).map_err(std::io::Error::other)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let snapshot = args
        .iter()
        .position(|a| a == "--snapshot")
        .map(|i| args.get(i + 1).cloned().unwrap_or("glass.png".into()));
    let mut config = AppConfig::new("Glass and shadows");
    if args.iter().any(|a| a == "--cpu") {
        config = config.cpu_only();
    } else {
        // Show the GPU path from the first frame.
        config = config.gpu_first();
    }
    if let Err(e) = remedy_ui::run(config, move |a| app(a, snapshot.clone())) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
