//! Prism effects gallery: every built-in effect in a live tile, drawn by the GPU kernel or the
//! CPU renderer (whichever draws the window), under the ambient policy.
//!
//! ```text
//! cargo run --release -p remedy-ui --example effects_gallery
//! cargo run --release -p remedy-ui --example effects_gallery -- --snapshot gallery.png
//! ```
//!
//! With `--snapshot <file>` it waits two seconds, saves a picture of the window (read back
//! from the GPU when the GPU draws it, effects included) and quits; `--cpu` keeps the window
//! on the CPU renderer.

use std::time::Duration;

use remedy_ui::core::{Color, Rect};
use remedy_ui::effects::{Instance, LIBRARY, effect_view, library};
use remedy_ui::paint::Brush;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

const TILE_W: f32 = 236.0;
const TILE_H: f32 = 148.0;

/// A test picture for effects that take one (heat shimmer, CRT screen).
fn picture() -> remedy_ui::paint::Image {
    let (pw, ph) = (256u32, 160u32);
    let mut px = Vec::with_capacity((pw * ph * 4) as usize);
    for y in 0..ph {
        for x in 0..pw {
            let check = if (x / 20 + y / 20) % 2 == 0 { 230u8 } else { 70 };
            let (dx, dy) = (x as f32 - 128.0, y as f32 - 80.0);
            let disc = (dx * dx + dy * dy).sqrt() < 46.0;
            px.extend(if disc {
                [255, 196, 70, 255]
            } else {
                [check, (x * 255 / pw) as u8, (y * 255 / ph) as u8, 255]
            });
        }
    }
    remedy_ui::paint::Image::new(pw, ph, px.into())
}

fn tile(ui: &Ui, name: &str) -> El {
    let Some(effect) = library(name) else { return w::text(ui, format!("{name}: does not compile")) };
    let mut instance = Instance::new(effect.clone());
    for t in effect.textures.clone() {
        let _ = instance.set_texture(&t, Some(picture()));
    }
    let (fx, _view) = effect_view(ui, instance);
    // A night-blue ground under each effect, so light effects have something to add to.
    let ground = ui
        .el("ground")
        .width(TILE_W)
        .height(TILE_H)
        .paint(|cx| {
            cx.list.fill_rect(
                Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
                Brush::Solid(Color::rgb8(18, 22, 38)),
            )
        })
        .child(fx.width(TILE_W).height(TILE_H));
    w::column(ui).gap(4.0).child(w::text(ui, name.to_owned())).child(ground)
}

fn app(app: &App, snapshot: Option<String>) {
    let win = app.open(WindowSpec::new("Prism effects").size(1240.0, 960.0), |ui, _| {
        let mut grid = w::wrap(ui).gap(10.0).padding(12.0);
        for (name, _) in LIBRARY {
            grid = grid.child(tile(ui, name));
        }
        ui.root().child(w::scroll(ui).grow(1.0).child(grid));
    });
    if let Some(path) = snapshot {
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
        .map(|i| args.get(i + 1).cloned().unwrap_or("gallery.png".into()));
    let mut config = AppConfig::new("Prism effects");
    // `--cpu` keeps the window on the CPU renderer (effects then run on the CPU interpreter).
    if args.iter().any(|a| a == "--cpu") {
        config = config.cpu_only();
    }
    if let Err(e) = remedy_ui::run(config, move |a| app(a, snapshot.clone())) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
