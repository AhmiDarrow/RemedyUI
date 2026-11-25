//! Prism `.rfx` effects: check a file against the limits apps or strangers get, with each
//! problem drawn under its line and what the effect costs pass by pass; render a frame of it
//! on the CPU (the reference renderer) to a PNG; list the built-in library.

use std::collections::BTreeMap;

use remedy_ui_engine::effects::ir::{ParamType, StageKind};
use remedy_ui_engine::effects::{
    CompileError, CpuEffect, Effect, FrameInput, Instance, LIBRARY, Limits, Picture, Value,
};
use serde::{Deserialize, Serialize};

use crate::{Result, ToolError, Workspace};

/// An effect to check or render: its source, a file, or a built-in effect's name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct EffectInput {
    /// The `.rfx` source itself. Give this, `path` or `name`.
    #[serde(default)]
    pub source: Option<String>,
    /// A `.rfx` file to read (inside the workspace).
    #[serde(default)]
    pub path: Option<String>,
    /// A built-in effect (`fire`, `aurora`…; see the library).
    #[serde(default)]
    pub name: Option<String>,
    /// Check against the limits for strangers' effects (themes, scripts, downloads) instead
    /// of an app's own.
    #[serde(default)]
    pub untrusted: bool,
}

impl EffectInput {
    /// The effect in `source`.
    pub fn from_source(source: impl Into<String>) -> Self {
        EffectInput { source: Some(source.into()), ..Default::default() }
    }

    /// The effect in the file at `path`.
    pub fn from_path(path: impl Into<String>) -> Self {
        EffectInput { path: Some(path.into()), ..Default::default() }
    }

    /// The built-in effect `name`.
    pub fn built_in(name: impl Into<String>) -> Self {
        EffectInput { name: Some(name.into()), ..Default::default() }
    }

    /// Checked against the limits for strangers' effects.
    pub fn untrusted(mut self, on: bool) -> Self {
        self.untrusted = on;
        self
    }
}

/// A problem in an effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct EffectError {
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1.
    pub column: u32,
    /// What is wrong (with "did you mean" when a name is a slip away).
    pub message: String,
}

/// One of an effect's parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct EffectParam {
    /// Its name.
    pub name: String,
    /// `float`, `vec2`, `vec3` or `color`.
    pub kind: String,
    /// Its default, as many numbers as it has.
    pub default: Vec<f32>,
    /// The range values are clamped to, if any.
    pub range: Option<(f32, f32)>,
}

/// What one pass costs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct PassCost {
    /// Its name (`glow.blur` for a bloom's stages).
    pub name: String,
    /// `shade`, `blur` or `particles`.
    pub kind: String,
    /// Its resolution relative to the element's device pixels.
    pub scale: f32,
    /// Instructions its program runs a pixel (a particle, for particles; 0 for a blur).
    pub instructions: usize,
    /// The summed weight of those instructions (the cost model).
    pub cost: u32,
    /// Particles (for particles).
    pub particles: u32,
}

/// The outcome of a check.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct EffectCheck {
    /// It compiles within the limits.
    pub ok: bool,
    /// The first problem, if any.
    pub error: Option<EffectError>,
    /// Every problem drawn under its line, as a compiler shows them.
    pub report: Option<String>,
    /// The effect's name.
    pub name: String,
    /// Frames a second it asks for.
    pub fps: u32,
    /// Its resolution relative to the element's device pixels.
    pub scale: f32,
    /// How it combines with what is under it.
    pub blend: String,
    /// It reads its last frame.
    pub feedback: bool,
    /// Its cost per element pixel, and the most the limits allow.
    pub cost: u32,
    /// The cost limit checked against.
    pub cost_limit: u32,
    /// Its parameters.
    pub params: Vec<EffectParam>,
    /// Its texture parameters.
    pub textures: Vec<String>,
    /// Its passes, in order.
    pub passes: Vec<PassCost>,
}

/// A frame to render.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct RenderInput {
    /// The effect.
    #[serde(flatten)]
    pub effect: EffectInput,
    /// The element's width, logical pixels (default 480).
    #[serde(default = "default_width")]
    pub width: f32,
    /// The element's height, logical pixels (default 320).
    #[serde(default = "default_height")]
    pub height: f32,
    /// Device pixels per logical pixel (default 1).
    #[serde(default = "one")]
    pub scale: f32,
    /// The effect's time, seconds (default its still frame's).
    #[serde(default)]
    pub time: Option<f32>,
    /// Parameter values by name: one number for a float, two or three for vectors, four for a
    /// color (0 to 1). Clamped like any host's values.
    #[serde(default)]
    pub values: BTreeMap<String, Vec<f32>>,
    /// What the picture is drawn over, `#rrggbb` (default a night blue, so light effects
    /// show); `none` keeps transparency.
    #[serde(default)]
    pub background: Option<String>,
}

fn default_width() -> f32 {
    480.0
}
fn default_height() -> f32 {
    320.0
}
fn one() -> f32 {
    1.0
}

impl RenderInput {
    /// A frame of `effect` at its defaults.
    pub fn new(effect: EffectInput) -> Self {
        RenderInput {
            effect,
            width: default_width(),
            height: default_height(),
            scale: 1.0,
            time: None,
            values: BTreeMap::new(),
            background: None,
        }
    }
}

/// A rendered frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct RenderOutput {
    /// The PNG's size in pixels: the element's device pixels (its logical size times the
    /// scale), with the effect's picture stretched over it smoothly, as the element draws it.
    /// At most 4096 on a side.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The resolution the effect itself drew at (its `scale` statement and quality step make
    /// it smaller than the element).
    pub effect_width: u32,
    /// Height the effect drew at.
    pub effect_height: u32,
    /// The time drawn.
    pub time: f32,
    /// How long the CPU took, milliseconds (an upper bound for the GPU kernel).
    pub cpu_ms: f64,
    /// Values given that the effect has no parameter of, or of another type.
    pub ignored: Vec<String>,
    /// The PNG (straight-alpha RGBA). Not serialized.
    #[serde(skip)]
    pub png: Vec<u8>,
    /// Where the PNG was written, when asked.
    pub png_file: Option<String>,
}

/// One built-in effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct LibraryEffect {
    /// Its name (`library("fire")`).
    pub name: String,
    /// What it is, from the first comment line of its source.
    pub summary: String,
    /// Its parameters' names.
    pub params: Vec<String>,
    /// Its cost per element pixel.
    pub cost: u32,
}

fn source(input: &EffectInput, ws: &Workspace) -> Result<String> {
    match (&input.source, &input.path, &input.name) {
        (Some(s), _, _) => Ok(s.clone()),
        (None, Some(p), _) => {
            let path = ws.resolve(p)?;
            std::fs::read_to_string(&path).map_err(|e| ToolError::new(format!("{}: {e}", path.display())))
        }
        (None, None, Some(n)) => remedy_ui_engine::effects::library_source(n).map(str::to_owned).ok_or_else(|| {
            let names: Vec<&str> = LIBRARY.iter().map(|(n, _)| *n).collect();
            ToolError::new(format!("no built-in effect `{n}`; they are {}", names.join(", ")))
        }),
        (None, None, None) => {
            Err(ToolError::new("give the effect as `source`, a file as `path`, or a built-in `name`"))
        }
    }
}

fn limits(input: &EffectInput) -> Limits {
    if input.untrusted { Limits::untrusted() } else { Limits::default() }
}

fn compile(
    input: &EffectInput,
    ws: &Workspace,
) -> Result<(String, std::result::Result<Effect, CompileError>)> {
    let src = source(input, ws)?;
    let compiled = Effect::compile_with(&src, &limits(input));
    Ok((src, compiled))
}

fn kind_name(t: ParamType) -> String {
    match t {
        ParamType::Float => "float",
        ParamType::Vec2 => "vec2",
        ParamType::Vec3 => "vec3",
        ParamType::Color => "color",
        _ => "value",
    }
    .to_owned()
}

/// Checks an effect: compiles it against the app's limits (or strangers', with `untrusted`)
/// and reports what it costs, pass by pass.
pub fn check(input: &EffectInput, ws: &Workspace) -> Result<EffectCheck> {
    let (src, compiled) = compile(input, ws)?;
    let lim = limits(input);
    let effect = match compiled {
        Ok(e) => e,
        Err(e) => {
            let first = e.diagnostics.first();
            return Ok(EffectCheck {
                ok: false,
                error: first.map(|d| EffectError {
                    line: d.at.line,
                    column: d.at.col,
                    message: d.message.clone(),
                }),
                report: Some(e.report(&src)),
                name: String::new(),
                fps: 0,
                scale: 0.0,
                blend: String::new(),
                feedback: false,
                cost: 0,
                cost_limit: lim.max_effect_cost,
                params: Vec::new(),
                textures: Vec::new(),
                passes: Vec::new(),
            });
        }
    };
    let params = effect
        .params
        .iter()
        .map(|p| EffectParam {
            name: p.name.clone(),
            kind: kind_name(p.ty),
            default: p.default.iter().take(p.ty.components()).copied().collect(),
            range: p.range,
        })
        .collect();
    let passes = effect
        .stages
        .iter()
        .map(|s| {
            let (kind, instructions, cost, particles) = match &s.kind {
                StageKind::Shade(p) => ("shade", p.code.len(), p.cost(), 0),
                StageKind::Blur { .. } => ("blur", 0, 0, 0),
                StageKind::Particles { emitter, .. } => {
                    ("particles", emitter.program.code.len(), emitter.program.cost(), emitter.count)
                }
            };
            PassCost {
                name: s.name.clone(),
                kind: kind.into(),
                scale: s.scale * effect.scale,
                instructions,
                cost,
                particles,
            }
        })
        .collect();
    Ok(EffectCheck {
        ok: true,
        error: None,
        report: None,
        name: effect.name.clone(),
        fps: effect.fps,
        scale: effect.scale,
        blend: format!("{:?}", effect.blend).to_lowercase(),
        feedback: effect.feedback,
        cost: effect.cost,
        cost_limit: lim.max_effect_cost,
        params,
        textures: effect.textures.clone(),
        passes,
    })
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Renders a frame of an effect on the CPU (the reference renderer the GPU kernel is tested
/// against) and encodes it as a PNG, written to `png_file` too when given.
pub fn render(input: &RenderInput, ws: &Workspace, png_file: Option<&str>) -> Result<RenderOutput> {
    let (src, compiled) = compile(&input.effect, ws)?;
    let effect = compiled.map_err(|e| ToolError::new(e.report(&src)))?.shared();
    let dims_ok = |v: f32, max: f32| v.is_finite() && v >= 1.0 && v <= max;
    if !dims_ok(input.width, 4096.0) || !dims_ok(input.height, 4096.0) || !(0.25..=4.0).contains(&input.scale)
    {
        return Err(ToolError::new("width and height are 1 to 4096 logical pixels, scale 0.25 to 4"));
    }
    let mut instance = Instance::new(effect.clone());
    // Texture parameters show a test picture, so effects that transform one (a CRT screen,
    // heat shimmer) can be judged.
    for t in effect.textures.clone() {
        let _ = instance.set_texture(&t, Some(test_picture()));
    }
    let mut ignored = Vec::new();
    for (name, v) in &input.values {
        let value = match v.as_slice() {
            [x] => Value::Float(*x),
            [x, y] => Value::Vec2([*x, *y]),
            [x, y, z] => Value::Vec3([*x, *y, *z]),
            [r, g, b, a] => Value::Color([*r, *g, *b, *a]),
            _ => {
                ignored.push(name.clone());
                continue;
            }
        };
        if instance.set(name, value).is_err() {
            ignored.push(name.clone());
        }
    }
    let time = input.time.filter(|t| t.is_finite()).unwrap_or(effect.still);
    let mut cpu = CpuEffect::new();
    let started = std::time::Instant::now();
    // Effects that read their last frame build up over a few frames first.
    let frames = if effect.feedback { 12 } else { 1 };
    let mut pic = cpu.render(&instance, &FrameInput::new(input.width, input.height, input.scale, time));
    for k in 1..frames {
        let t = time + k as f32 / effect.fps.max(1) as f32;
        pic = cpu.render(&instance, &FrameInput::new(input.width, input.height, input.scale, t));
    }
    let cpu_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    let (effect_width, effect_height) = (pic.width, pic.height);
    // Stretched over the element's device pixels, as the element draws it.
    let fit = (4096.0 / (input.width * input.scale).max(input.height * input.scale)).min(1.0);
    let side = |v: f32| ((v * input.scale * fit).round() as u32).clamp(1, 4096);
    let pic = stretch(&pic, side(input.width), side(input.height));
    // Premultiplied pixels, over the background (or unpremultiplied, kept transparent).
    let bg = match input.background.as_deref() {
        Some("none") => None,
        Some(hex) => {
            Some(parse_hex(hex).ok_or_else(|| ToolError::new(format!("`{hex}` is not #rrggbb or none")))?)
        }
        None => Some([18, 22, 38]),
    };
    let px: Vec<u8> = pic
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let [r, g, b, a] = *p;
            match bg {
                Some(bg) => {
                    let over = |c: u8, k: u8| {
                        (u32::from(c) + u32::from(k) * (255 - u32::from(a)) / 255).min(255) as u8
                    };
                    [over(r, bg[0]), over(g, bg[1]), over(b, bg[2]), 255]
                }
                None => {
                    let un = |c: u8| {
                        (u32::from(c) * 255 + u32::from(a) / 2)
                            .checked_div(u32::from(a))
                            .map_or(0, |v| v.min(255) as u8)
                    };
                    [un(r), un(g), un(b), a]
                }
            }
        })
        .collect();
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, pic.width, pic.height);
        enc.set_color(png::ColorType::Rgba);
        let mut w = enc.write_header().map_err(|e| ToolError::new(e.to_string()))?;
        w.write_image_data(&px).map_err(|e| ToolError::new(e.to_string()))?;
    }
    let mut out = RenderOutput {
        width: pic.width,
        height: pic.height,
        effect_width,
        effect_height,
        time,
        cpu_ms,
        ignored,
        png,
        png_file: None,
    };
    if let Some(p) = png_file {
        let target = ws.resolve(p)?;
        std::fs::write(&target, &out.png)
            .map_err(|e| ToolError::new(format!("{}: {e}", target.display())))?;
        out.png_file = Some(target.display().to_string());
    }
    Ok(out)
}

/// `pic` resized to `w` × `h` with bilinear filtering from pixel centres, clamped at the
/// edges: what a GPU's linear sampler does when the element draws the effect.
fn stretch(pic: &Picture, w: u32, h: u32) -> Picture {
    if (pic.width, pic.height) == (w, h) || pic.width == 0 || pic.height == 0 {
        return pic.clone();
    }
    let (sw, sh) = (pic.width as usize, pic.height as usize);
    let at = |x: usize, y: usize, c: usize| -> f32 {
        f32::from(pic.pixels.get((y.min(sh - 1) * sw + x.min(sw - 1)) * 4 + c).copied().unwrap_or(0))
    };
    let axis = |i: u32, to: u32, from: usize| {
        let f = ((i as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, from as f32 - 1.0);
        (f.floor() as usize, f.fract())
    };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let (y0, fy) = axis(y, h, sh);
        for x in 0..w {
            let (x0, fx) = axis(x, w, sw);
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - fx) + at(x0 + 1, y0, c) * fx;
                let bottom = at(x0, y0 + 1, c) * (1.0 - fx) + at(x0 + 1, y0 + 1, c) * fx;
                pixels.push((top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    Picture { width: w, height: h, pixels }
}

/// A test picture for texture parameters: a checkerboard tinted by gradients, and a disc.
fn test_picture() -> remedy_ui_engine::paint::Image {
    let (w, h) = (256u32, 160u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let check = if (x / 20 + y / 20) % 2 == 0 { 230u8 } else { 70 };
            let (dx, dy) = (x as f32 - 128.0, y as f32 - 80.0);
            let disc = (dx * dx + dy * dy).sqrt() < 46.0;
            px.extend(if disc {
                [255, 196, 70, 255]
            } else {
                [check, (x * 255 / w) as u8, (y * 255 / h) as u8, 255]
            });
        }
    }
    remedy_ui_engine::paint::Image::new(w, h, px.into())
}

/// The built-in effects, with what each is and costs.
pub fn library() -> Vec<LibraryEffect> {
    LIBRARY
        .iter()
        .map(|(name, src)| {
            let summary = src
                .lines()
                .find_map(|l| l.trim().strip_prefix("//").map(str::trim))
                .unwrap_or_default()
                .to_owned();
            let effect = remedy_ui_engine::effects::library(name);
            LibraryEffect {
                name: (*name).to_owned(),
                summary,
                params: effect
                    .as_ref()
                    .map(|e| e.params.iter().map(|p| p.name.clone()).collect())
                    .unwrap_or_default(),
                cost: effect.map_or(0, |e| e.cost),
            }
        })
        .collect()
}

/// A check as people read it: `ok` with the cost and passes, or the report.
pub fn check_text(c: &EffectCheck) -> String {
    if let Some(r) = &c.report {
        return r.clone();
    }
    let mut t = format!(
        "ok: effect `{}`, {} fps, scale {}, blend {}{}\ncost {} an element pixel (limit {})\n",
        c.name,
        c.fps,
        c.scale,
        c.blend,
        if c.feedback { ", reads its last frame" } else { "" },
        c.cost,
        c.cost_limit
    );
    for p in &c.params {
        let range = p.range.map(|(a, b)| format!(" in {a}..{b}")).unwrap_or_default();
        t.push_str(&format!("param {}: {} = {:?}{range}\n", p.name, p.kind, p.default));
    }
    for name in &c.textures {
        t.push_str(&format!("texture {name}\n"));
    }
    for p in &c.passes {
        let extra = match p.kind.as_str() {
            "particles" => format!(", {} particles", p.particles),
            _ => String::new(),
        };
        t.push_str(&format!(
            "pass {} ({}) at scale {}: {} instructions, cost {}{extra}\n",
            p.name, p.kind, p.scale, p.instructions, p.cost
        ));
    }
    t
}
