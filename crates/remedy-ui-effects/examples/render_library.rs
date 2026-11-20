//! Renders every built-in effect on the CPU into PNG files, at a few moments, to look at.
//!
//! ```text
//! cargo run --release -p remedy-ui-effects --example render_library -- <out dir> [width height]
//! ```

use std::sync::Arc;

use remedy_ui_effects::{CpuEffect, Effect, FrameInput, Instance, LIBRARY};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| "effects-out".into()));
    let w: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(480.0);
    let h: f32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(320.0);
    std::fs::create_dir_all(&dir)?;
    for (name, src) in LIBRARY {
        let effect = match Effect::compile(src) {
            Ok(e) => Arc::new(e),
            Err(e) => {
                eprintln!("{name}: {e}");
                continue;
            }
        };
        let instance = Instance::new(effect.clone());
        let mut cpu = CpuEffect::new();
        for t in [0.0f32, 1.5, 3.0] {
            let started = std::time::Instant::now();
            let mut input = FrameInput::new(w, h, 1.0, t);
            input.max_side = 4096;
            // Effects with feedback need a few frames to build up.
            let frames = if effect.feedback { 30 } else { 1 };
            let mut pic = cpu.render(&instance, &input);
            for k in 1..frames {
                input.time = t + k as f32 / 30.0;
                pic = cpu.render(&instance, &input);
            }
            let took = started.elapsed() / frames;
            let path = dir.join(format!("{name}-{t}.png"));
            let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
            let mut enc = png::Encoder::new(file, pic.width, pic.height);
            enc.set_color(png::ColorType::Rgba);
            let mut wr = enc.write_header()?;
            // Unpremultiply for viewing.
            let px: Vec<u8> = pic
                .pixels
                .chunks(4)
                .flat_map(|p| {
                    let a = p[3];
                    if a == 0 {
                        [0, 0, 0, 0]
                    } else {
                        [0, 1, 2]
                            .map(|k| {
                                ((u32::from(p[k]) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                            })
                            .into_iter()
                            .chain([a])
                            .collect::<Vec<_>>()
                            .try_into()
                            .unwrap_or([0; 4])
                    }
                })
                .collect();
            wr.write_image_data(&px)?;
            println!(
                "{name} t={t}: {}x{} in {:.2} ms, cost {}",
                pic.width,
                pic.height,
                took.as_secs_f64() * 1e3,
                effect.cost
            );
        }
    }
    Ok(())
}
