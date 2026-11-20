//! **Prism**, RemedyUI's effects system: animated, procedural pictures (fire, water,
//! caustics, glow and bloom, light shafts, particles, starfields, paper and film grain, CRT
//! screens, raymarched 3D scenes, volumetric clouds) described in `.rfx`, a small language
//! whose programs are data, run on our own GPU kernels or on the CPU, and are safe to load
//! from strangers.
//!
//! ```
//! use remedy_ui_effects::{Effect, Instance, Value};
//!
//! let effect = Effect::compile(r#"
//!     effect ember_glow(color heat = #ff7a1a, float intensity = 0.8 in 0..1) {
//!         fps 24;
//!         scale 0.5;
//!         blend plus;
//!         pass flame {
//!             let n = fbm(uv * 3.0 + vec2(0.0, -time * 0.6), 4);
//!             out = vec4(heat.rgb, smoothstep(0.45, 0.9, n) * intensity);
//!         }
//!         pass glow = bloom(flame, 0.5, 12.0, 0.8);
//!     }
//! "#).map_err(|e| e.to_string())?;
//! assert_eq!(effect.fps, 24);
//!
//! // Hosts (an app, a theme, a script) set parameters by name; values are clamped.
//! let mut ember = Instance::new(effect.shared());
//! ember.set("intensity", Value::Float(4.0))?;
//! assert_eq!(ember.get("intensity"), Some(Value::Float(1.0)));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # How it works
//!
//! - **The language** (`.rfx`, reference in `docs/effects/rfx.md`): typed values (`float`,
//!   `vec2`–`vec4`, `color`, `mat3`, `mat4`), functions, `let`/`var`, `if`, `for` loops with
//!   constant bounds, and a library of math, noise (value, Perlin, simplex, cellular, fbm),
//!   colour, 2D and 3D shapes, cameras and lighting. An effect is a graph of **passes**:
//!   per-pixel programs, blurs, blooms and particle emitters, each writing a buffer later
//!   passes can sample, at its own resolution; `last` is the previous frame (trails, ripples).
//! - **The compiler** inlines functions, unrolls loops and turns `if` into selects, so every
//!   pixel runs the same straight-line code: its cost is known before it runs and does not
//!   depend on what it reads. It folds constants, merges repeated work, and hoists what only
//!   changes once a frame (parameters, time) out of the per-pixel code.
//! - **The kernels**: programs are instructions for a register machine whose instructions
//!   are our own kernels (arithmetic, transcendental functions, hashing, noise, sampling). The
//!   GPU runs them in one fixed shader written and tested by us
//!   (`remedy_ui_render_gpu::effects`); the CPU runs them in [`vm`], batched 64 pixels at a
//!   time. Both follow one specification, and the tests hold them to the same pictures.
//! - **Safety**: themes and scripts never supply shader code. Every program is checked
//!   against [`Limits`] (instructions, registers, cost per pixel, passes, particles, loop
//!   iterations, nesting, size) when it is compiled, so a hostile effect cannot hang the GPU
//!   (no unbounded loops, a fixed instruction budget) or the CPU. Programs sample only their
//!   own textures and passes, never other content, and nothing they compute flows back to the
//!   theme: no timing side channel to read what is on screen.
//! - **Cheap by construction**: effects render at a fraction of the element's resolution
//!   (`scale`), at their own frame rate under the window's ambient policy, only while seen;
//!   reduced motion and paused motion show a still frame.
//!
//! Layer 3: depends only on the core types and the display list.

mod builtins;
mod compile;
mod cpu;
pub mod ir;
mod library;
mod syntax;
pub mod vm;

use std::sync::{Arc, OnceLock};

pub use compile::MAX_SAMPLED_PASSES;
pub use cpu::{
    CpuEffect, FrameInput, MAX_EFFECT_PIXELS, MAX_OVERDRAW, Picture, Sprite, blur_radius, blur_weights,
    buffer_size, cap_overdraw, constants, fit, particles, uniforms,
};
pub use ir::{Effect, Param, ParamType};
pub use library::{LIBRARY, library, library_source};
pub use syntax::{Diagnostic, Span};

/// What one effect may cost, checked when it is compiled. [`Limits::default`] is for code the
/// app ships; [`Limits::untrusted`] for themes and scripts from strangers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Bytes of source.
    pub max_source: usize,
    /// Tokens in the source.
    pub max_tokens: usize,
    /// How deep expressions and blocks nest.
    pub max_depth: u32,
    /// How deep functions call functions (all calls are inlined; there is no recursion).
    pub max_inline_depth: u32,
    /// Iterations of one loop.
    pub max_loop: u32,
    /// Loop iterations in all, after nesting.
    pub max_unrolled: u64,
    /// Distinct values in the effect after unrolling (compile memory).
    pub max_values: usize,
    /// Instructions in one program.
    pub max_instructions: usize,
    /// Registers one program may use.
    pub max_registers: u8,
    /// Cost of one pass per pixel (summed instruction weights).
    pub max_pass_cost: u32,
    /// Cost of the whole effect per element pixel: every pass's cost weighted by its
    /// resolution (a pass at half resolution costs a quarter).
    pub max_effect_cost: u32,
    /// Passes (bloom counts as three).
    pub max_stages: usize,
    /// Parameters.
    pub max_params: usize,
    /// Texture parameters.
    pub max_textures: usize,
    /// Particles in one emitter.
    pub max_particles: u32,
    /// Particles times their program's cost, per emitter per frame.
    pub max_particle_work: u64,
    /// Frames a second.
    pub max_fps: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_source: 256 * 1024,
            max_tokens: 100_000,
            max_depth: 64,
            max_inline_depth: 16,
            max_loop: 256,
            max_unrolled: 4096,
            max_values: 100_000,
            max_instructions: 8192,
            max_registers: 64,
            max_pass_cost: 40_000,
            max_effect_cost: 20_000,
            max_stages: 16,
            max_params: 32,
            max_textures: 4,
            max_particles: 8192,
            max_particle_work: 2_000_000,
            max_fps: 60,
        }
    }
}

impl Limits {
    /// Limits for effects from strangers: people's themes, downloaded packages, scripts. A
    /// whole effect costs at most about what a few octaves of noise at full resolution do, or
    /// a raymarched scene at a quarter; ambient frame rates; fewer particles.
    pub fn untrusted() -> Self {
        Limits {
            max_source: 64 * 1024,
            max_tokens: 20_000,
            max_depth: 48,
            max_inline_depth: 8,
            max_loop: 128,
            max_unrolled: 1024,
            max_values: 30_000,
            max_instructions: 4096,
            max_registers: 48,
            max_pass_cost: 16_000,
            max_effect_cost: 2_500,
            max_stages: 10,
            max_params: 16,
            max_textures: 4,
            max_particles: 2048,
            max_particle_work: 400_000,
            max_fps: 30,
        }
    }
}

/// Why a source did not compile: every problem found, with where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    /// The problems (the first stops compiling, so usually one).
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, d) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{}: {}", d.at, d.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// Every problem drawn under its line of `source` ([`Diagnostic::render`]): what a theme
    /// editor, the devtools or a command line shows an author.
    ///
    /// ```
    /// use remedy_ui_effects::Effect;
    /// let src = "effect e {\n    pass p { out = vec4(fmb(uv, 3)); }\n}";
    /// let Err(e) = Effect::compile(src) else { unreachable!("it does not compile") };
    /// let report = e.report(src);
    /// assert!(report.contains("did you mean `fbm`?"));
    /// assert!(report.contains("2 |     pass p { out = vec4(fmb(uv, 3)); }"));
    /// ```
    pub fn report(&self, source: &str) -> String {
        self.diagnostics.iter().map(|d| d.render(source)).collect::<Vec<_>>().join("\n")
    }
}

impl From<Diagnostic> for CompileError {
    fn from(d: Diagnostic) -> Self {
        CompileError { diagnostics: vec![d] }
    }
}

fn prelude() -> &'static [syntax::FnDecl] {
    static PRELUDE: OnceLock<Vec<syntax::FnDecl>> = OnceLock::new();
    PRELUDE.get_or_init(|| {
        let parsed = syntax::lex(builtins::PRELUDE, 1_000_000)
            .and_then(|t| syntax::Parser::new(t, 64).file())
            .map(|f| {
                f.items
                    .into_iter()
                    .filter_map(|i| match i {
                        syntax::Item::Fn(f) => Some(f),
                        _ => None,
                    })
                    .collect()
            });
        parsed.unwrap_or_default()
    })
}

impl Effect {
    /// Compiles `.rfx` source with the default (app) [`Limits`].
    pub fn compile(source: &str) -> Result<Effect, CompileError> {
        Self::compile_with(source, &Limits::default())
    }

    /// Compiles `.rfx` source with `limits`: [`Limits::untrusted`] for themes and scripts.
    pub fn compile_with(source: &str, limits: &Limits) -> Result<Effect, CompileError> {
        if source.len() > limits.max_source {
            return Err(Diagnostic::new(
                Span { line: 1, col: 1 },
                format!("the source is larger than {} bytes", limits.max_source),
            )
            .into());
        }
        let tokens = syntax::lex(source, limits.max_tokens)?;
        let file = syntax::Parser::new(tokens, limits.max_depth).file()?;
        Ok(compile::compile(&file, prelude(), limits)?)
    }
}

/// A parameter value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// A `float`.
    Float(f32),
    /// A `vec2`.
    Vec2([f32; 2]),
    /// A `vec3`.
    Vec3([f32; 3]),
    /// A `color`, straight alpha, components 0 to 1.
    Color([f32; 4]),
}

impl Value {
    fn components(self) -> [f32; 4] {
        match self {
            Value::Float(x) => [x; 4],
            Value::Vec2([x, y]) => [x, y, 0.0, 0.0],
            Value::Vec3([x, y, z]) => [x, y, z, 0.0],
            Value::Color(c) => c,
        }
    }

    fn of(ty: ParamType, v: [f32; 4]) -> Value {
        match ty {
            ParamType::Float => Value::Float(v[0]),
            ParamType::Vec2 => Value::Vec2([v[0], v[1]]),
            ParamType::Vec3 => Value::Vec3([v[0], v[1], v[2]]),
            ParamType::Color => Value::Color(v),
        }
    }

    fn type_matches(self, ty: ParamType) -> bool {
        matches!(
            (self, ty),
            (Value::Float(_), ParamType::Float)
                | (Value::Vec2(_), ParamType::Vec2)
                | (Value::Vec3(_), ParamType::Vec3)
                | (Value::Color(_), ParamType::Color)
        )
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v)
    }
}

impl From<remedy_ui_core::Color> for Value {
    fn from(c: remedy_ui_core::Color) -> Self {
        Value::Color([c.r, c.g, c.b, c.a])
    }
}

/// Why a parameter could not be set.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamError {
    /// The effect has no parameter (or texture) of that name.
    Unknown(String),
    /// The value's type is not the parameter's.
    Type {
        /// The parameter.
        name: String,
        /// Its type.
        expected: &'static str,
    },
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamError::Unknown(n) => write!(f, "the effect has no parameter `{n}`"),
            ParamError::Type { name, expected } => write!(f, "`{name}` is a {expected}"),
        }
    }
}

impl std::error::Error for ParamError {}

/// One use of an effect: its parameter values and bound textures. Both renderers draw from
/// it. Cheap to clone (the compiled effect is shared).
#[derive(Clone, Debug)]
pub struct Instance {
    effect: Arc<Effect>,
    values: Vec<[f32; 4]>,
    textures: Vec<Option<remedy_ui_paint::Image>>,
    /// Bumped on every change: renderers and elements redraw when it moves.
    version: u64,
}

impl Instance {
    /// `effect` with its default parameters and no textures bound.
    pub fn new(effect: Arc<Effect>) -> Self {
        let values = effect.defaults();
        let textures = vec![None; effect.textures.len()];
        Instance { effect, values, textures, version: 0 }
    }

    /// The effect.
    pub fn effect(&self) -> &Arc<Effect> {
        &self.effect
    }

    /// Sets parameter `name` (clamped to its range; non-finite components keep the default).
    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> Result<(), ParamError> {
        let value = value.into();
        let Some((i, p)) = self.effect.param(name) else { return Err(ParamError::Unknown(name.to_owned())) };
        if !value.type_matches(p.ty) {
            return Err(ParamError::Type { name: name.to_owned(), expected: p.ty.name() });
        }
        let v = p.clamp(value.components());
        if let Some(slot) = self.values.get_mut(i)
            && *slot != v
        {
            *slot = v;
            self.version += 1;
        }
        Ok(())
    }

    /// The value of parameter `name`.
    pub fn get(&self, name: &str) -> Option<Value> {
        let (i, p) = self.effect.param(name)?;
        self.values.get(i).map(|v| Value::of(p.ty, *v))
    }

    /// Binds a picture to texture parameter `name` (`None` unbinds it; sampling an unbound
    /// texture reads transparent black).
    pub fn set_texture(
        &mut self,
        name: &str,
        image: Option<remedy_ui_paint::Image>,
    ) -> Result<(), ParamError> {
        let Some(i) = self.effect.texture(name) else { return Err(ParamError::Unknown(name.to_owned())) };
        if let Some(slot) = self.textures.get_mut(i) {
            *slot = image;
            self.version += 1;
        }
        Ok(())
    }

    /// Every parameter's value, in the effect's order.
    pub fn values(&self) -> &[[f32; 4]] {
        &self.values
    }

    /// The bound textures, in the effect's order.
    pub fn textures(&self) -> &[Option<remedy_ui_paint::Image>] {
        &self.textures
    }

    /// Counts changes: a new value means draw again.
    pub fn version(&self) -> u64 {
        self.version
    }
}
