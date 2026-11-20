//! The effect graph: what `.rfx` compiles to and what both renderers run.
//!
//! An [`Effect`] is a list of [`Stage`]s writing buffers: per-pixel [`Program`]s (shade
//! stages), separable blurs and particle splats. A program is straight-line code for a small
//! register machine whose instructions are the kernel library: arithmetic, math, noise,
//! sampling. Every value is four floats; scalars are kept splatted across all four lanes, so
//! component-wise instructions broadcast with no extra work. There are no jumps: every
//! instruction runs for every pixel, so a program's cost is known before it runs and does not
//! depend on what it reads (no data-dependent timing).
//!
//! The CPU interpreter (`vm`) and the GPU kernel (`wgsl`) implement exactly these
//! instructions; their numbering is part of the GPU kernel's input format.

use std::sync::Arc;

use remedy_ui_paint::Blend;

/// One instruction of the kernel library. The numbering is the GPU kernel's opcode table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
#[non_exhaustive]
#[allow(missing_docs, reason = "each opcode is documented in docs/effects/rfx.md and by OPS below")]
pub enum Op {
    Const = 0,
    Input = 1,
    Mov = 2,
    Add = 3,
    Sub = 4,
    Mul = 5,
    Div = 6,
    Mod = 7,
    Min = 8,
    Max = 9,
    Pow = 10,
    Atan2 = 11,
    Step = 12,
    Neg = 13,
    Abs = 14,
    Sign = 15,
    Floor = 16,
    Ceil = 17,
    Fract = 18,
    Round = 19,
    Sqrt = 20,
    Exp = 21,
    Log = 22,
    Sin = 23,
    Cos = 24,
    Tan = 25,
    Asin = 26,
    Acos = 27,
    Atan = 28,
    Clamp = 29,
    Mix = 30,
    Smoothstep = 31,
    Select = 32,
    Lt = 33,
    Le = 34,
    Gt = 35,
    Ge = 36,
    Eq = 37,
    Ne = 38,
    And = 39,
    Or = 40,
    Not = 41,
    Dot = 42,
    Length = 43,
    Normalize = 44,
    Swizzle = 45,
    Combine = 46,
    Hash = 47,
    Noise = 48,
    Perlin = 49,
    Simplex = 50,
    Voronoi = 51,
    Sample = 52,
    Fma = 53,
    Exp2 = 54,
    Log2 = 55,
    Trunc = 56,
    Noise3 = 57,
    Perlin3 = 58,
}

/// How many operands each opcode reads, its cost weight and its name in tables.
pub(crate) const OPS: &[(Op, u8, u32, &str)] = &[
    (Op::Const, 0, 0, "const"),
    (Op::Input, 0, 0, "input"),
    (Op::Mov, 1, 0, "mov"),
    (Op::Add, 2, 1, "add"),
    (Op::Sub, 2, 1, "sub"),
    (Op::Mul, 2, 1, "mul"),
    (Op::Div, 2, 2, "div"),
    (Op::Mod, 2, 3, "mod"),
    (Op::Min, 2, 1, "min"),
    (Op::Max, 2, 1, "max"),
    (Op::Pow, 2, 6, "pow"),
    (Op::Atan2, 2, 6, "atan2"),
    (Op::Step, 2, 1, "step"),
    (Op::Neg, 1, 1, "neg"),
    (Op::Abs, 1, 1, "abs"),
    (Op::Sign, 1, 1, "sign"),
    (Op::Floor, 1, 1, "floor"),
    (Op::Ceil, 1, 1, "ceil"),
    (Op::Fract, 1, 1, "fract"),
    (Op::Round, 1, 1, "round"),
    (Op::Sqrt, 1, 3, "sqrt"),
    (Op::Exp, 1, 4, "exp"),
    (Op::Log, 1, 4, "log"),
    (Op::Sin, 1, 4, "sin"),
    (Op::Cos, 1, 4, "cos"),
    (Op::Tan, 1, 5, "tan"),
    (Op::Asin, 1, 5, "asin"),
    (Op::Acos, 1, 5, "acos"),
    (Op::Atan, 1, 5, "atan"),
    (Op::Clamp, 3, 1, "clamp"),
    (Op::Mix, 3, 2, "mix"),
    (Op::Smoothstep, 3, 4, "smoothstep"),
    (Op::Select, 3, 1, "select"),
    (Op::Lt, 2, 1, "lt"),
    (Op::Le, 2, 1, "le"),
    (Op::Gt, 2, 1, "gt"),
    (Op::Ge, 2, 1, "ge"),
    (Op::Eq, 2, 1, "eq"),
    (Op::Ne, 2, 1, "ne"),
    (Op::And, 2, 1, "and"),
    (Op::Or, 2, 1, "or"),
    (Op::Not, 1, 1, "not"),
    (Op::Dot, 2, 2, "dot"),
    (Op::Length, 1, 3, "length"),
    (Op::Normalize, 1, 4, "normalize"),
    (Op::Swizzle, 1, 0, "swizzle"),
    (Op::Combine, 4, 0, "combine"),
    (Op::Hash, 1, 6, "hash"),
    (Op::Noise, 1, 14, "noise"),
    (Op::Perlin, 1, 18, "perlin"),
    (Op::Simplex, 1, 22, "simplex"),
    (Op::Voronoi, 1, 60, "voronoi"),
    (Op::Sample, 1, 10, "sample"),
    (Op::Fma, 3, 1, "fma"),
    (Op::Exp2, 1, 4, "exp2"),
    (Op::Log2, 1, 4, "log2"),
    (Op::Trunc, 1, 1, "trunc"),
    (Op::Noise3, 1, 28, "noise3"),
    (Op::Perlin3, 1, 36, "perlin3"),
];

impl Op {
    /// The opcode with number `n`.
    pub fn from_u8(n: u8) -> Option<Op> {
        OPS.iter().find(|(o, ..)| *o as u8 == n).map(|(o, ..)| *o)
    }

    /// Operands read.
    pub fn arity(self) -> u8 {
        OPS.iter().find(|(o, ..)| *o == self).map_or(0, |e| e.1)
    }

    /// Cost weight: roughly ALU operations per pixel, calibrated so a sample is about ten.
    pub fn cost(self) -> u32 {
        OPS.iter().find(|(o, ..)| *o == self).map_or(1, |e| e.2)
    }

    /// Its name in listings.
    pub fn name(self) -> &'static str {
        OPS.iter().find(|(o, ..)| *o == self).map_or("?", |e| e.3)
    }
}

/// What a program reads with [`Op::Input`] (the instruction's immediate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum Input {
    /// The pixel's position in the effect, 0 to 1 on each axis (pixel centers).
    Uv = 0,
    /// The pixel's position in logical pixels.
    Px = 1,
    /// The effect's size in logical pixels.
    Size = 2,
    /// Effect time in seconds (wraps every hour).
    Time = 3,
    /// Width over height.
    Aspect = 4,
    /// Particles: the particle's number, from 0.
    Id = 5,
    /// Particles: a random number 0 to 1 that changes every time the particle respawns.
    Seed = 6,
    /// Particles: seconds since the particle (re)spawned.
    Age = 7,
    /// Particles: its lifetime in seconds.
    Life = 8,
    /// Particles: age over life, 0 to 1.
    Progress = 9,
    /// Particles: the number of particles.
    Count = 10,
    /// Logical pixels per buffer pixel of the stage being drawn (for one-pixel-wide work).
    Texel = 11,
    /// Draw lighter (ambient light mode, power saving): 1, else 0.
    Light = 12,
}

impl Input {
    /// The input with number `n`.
    pub fn from_u16(n: u16) -> Option<Input> {
        use Input::*;
        [Uv, Px, Size, Time, Aspect, Id, Seed, Age, Life, Progress, Count, Texel, Light]
            .into_iter()
            .find(|i| *i as u16 == n)
    }
}

/// One instruction: an opcode, a destination register, up to four operand registers and a
/// 16-bit immediate (a constant index, an input, a swizzle, a component count, a slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Instr {
    /// The kernel.
    pub op: Op,
    /// The register written.
    pub dst: u8,
    /// Operand registers (unused ones are zero).
    pub src: [u8; 4],
    /// The immediate.
    pub imm: u16,
}

impl Instr {
    /// The two 32-bit words the GPU kernel reads.
    pub fn encode(&self) -> [u32; 2] {
        let [a, b, c, d] = self.src;
        [
            u32::from(self.op as u8) | u32::from(self.dst) << 8 | u32::from(a) << 16 | u32::from(b) << 24,
            u32::from(c) | u32::from(d) << 8 | u32::from(self.imm) << 16,
        ]
    }

    /// The instruction those words hold.
    pub fn decode(w: [u32; 2]) -> Option<Instr> {
        let op = Op::from_u8((w[0] & 0xff) as u8)?;
        Some(Instr {
            op,
            dst: (w[0] >> 8) as u8,
            src: [(w[0] >> 16) as u8, (w[0] >> 24) as u8, w[1] as u8, (w[1] >> 8) as u8],
            imm: (w[1] >> 16) as u16,
        })
    }
}

/// Where a program's constant comes from each frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    /// A literal.
    Value([f32; 4]),
    /// Parameter `n` of the effect (its value this frame).
    Param(usize),
    /// The value of uniform expression `n` this frame (see [`Effect::uniforms`]).
    Uniform(usize),
}

/// Straight-line code for one pixel (or one particle).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Program {
    /// The instructions, in order.
    pub code: Vec<Instr>,
    /// The constant table [`Op::Const`] reads.
    pub consts: Vec<Slot>,
    /// Registers used.
    pub regs: u8,
    /// Registers holding the results, in order (a shade program's colour, a particle
    /// program's position, radius, colour and trail).
    pub outputs: Vec<u8>,
}

impl Program {
    /// The summed cost weights: what one run costs.
    pub fn cost(&self) -> u32 {
        self.code.iter().map(|i| i.op.cost()).sum()
    }
}

/// What a sampling instruction's immediate names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A texture parameter, by its index among the effect's textures.
    Texture(u8),
    /// The buffer an earlier stage wrote.
    Buffer(u8),
    /// The effect's previous output frame (feedback: trails, ripples, smoke).
    Feedback,
}

impl Source {
    /// The immediate encoding: 0..16 textures, 16..48 buffers, 255 feedback.
    pub fn encode(self) -> u16 {
        match self {
            Source::Texture(n) => u16::from(n),
            Source::Buffer(n) => 16 + u16::from(n),
            Source::Feedback => 255,
        }
    }

    /// The source an immediate names.
    pub fn decode(imm: u16) -> Option<Source> {
        match imm {
            0..16 => Some(Source::Texture(imm as u8)),
            16..48 => Some(Source::Buffer((imm - 16) as u8)),
            255 => Some(Source::Feedback),
            _ => None,
        }
    }
}

/// How particles are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Splat {
    /// A soft round falloff (glow, embers, fireflies, dust).
    #[default]
    Soft,
    /// A crisp anti-aliased disc (snow, bubbles, stars).
    Disc,
    /// A ring (ripples, bubbles' rims).
    Ring,
}

/// A particle emitter: `count` stateless particles, each a pure function of its number, its
/// respawn and time. The program runs once per particle per frame (on the CPU, for both
/// renderers) and outputs position (logical pixels), radius, colour and a trail vector.
#[derive(Clone, Debug, PartialEq)]
pub struct Particles {
    /// How many (the most; light mode halves it).
    pub count: u32,
    /// Lifetime in seconds: the index of a per-frame value.
    pub life: usize,
    /// The per-particle program: outputs `pos`, `radius`, `color`, `trail`.
    pub program: Program,
    /// How each is drawn.
    pub splat: Splat,
    /// How they combine with the buffer.
    pub blend: Blend,
}

/// One step of an effect.
#[derive(Clone, Debug, PartialEq)]
pub enum StageKind {
    /// Runs `program` for every pixel of a new buffer.
    Shade(Program),
    /// A separable Gaussian blur of buffer `src`, radius (logical pixels) from a uniform.
    Blur {
        /// The buffer blurred.
        src: u8,
        /// The per-frame value holding the radius (logical pixels).
        radius: usize,
    },
    /// Particles drawn over a copy of buffer `src` (or over transparency).
    Particles {
        /// The buffer drawn over.
        src: Option<u8>,
        /// The emitter.
        emitter: Particles,
    },
}

/// A stage and the buffer it writes.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    /// Its name in the source (diagnostics, devtools).
    pub name: String,
    /// What it does.
    pub kind: StageKind,
    /// Its buffer's resolution relative to the effect's (1, 1/2, 1/4…).
    pub scale: f32,
}

/// A parameter's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParamType {
    /// One number.
    Float,
    /// Two numbers.
    Vec2,
    /// Three numbers.
    Vec3,
    /// A colour (straight alpha, sRGB-encoded like the rest of RemedyUI).
    Color,
}

impl ParamType {
    /// The number of components.
    pub fn components(self) -> usize {
        match self {
            ParamType::Float => 1,
            ParamType::Vec2 => 2,
            ParamType::Vec3 => 3,
            ParamType::Color => 4,
        }
    }

    /// The name in `.rfx`.
    pub fn name(self) -> &'static str {
        match self {
            ParamType::Float => "float",
            ParamType::Vec2 => "vec2",
            ParamType::Vec3 => "vec3",
            ParamType::Color => "color",
        }
    }
}

/// A value the host (an app, a theme, a script) sets on an effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Its name.
    pub name: String,
    /// Its type.
    pub ty: ParamType,
    /// The default.
    pub default: [f32; 4],
    /// The range every component is clamped to (`None`: any finite value).
    pub range: Option<(f32, f32)>,
}

impl Param {
    /// `v` made valid for this parameter: non-finite components become the default's, then
    /// every component is clamped to the range. Values from untrusted code go through here.
    pub fn clamp(&self, v: [f32; 4]) -> [f32; 4] {
        let mut out = self.default;
        for (i, (o, x)) in out.iter_mut().zip(v).enumerate() {
            if i >= self.ty.components() || !x.is_finite() {
                continue;
            }
            *o = match (self.ty, self.range) {
                (ParamType::Color, _) => x.clamp(0.0, 1.0),
                (_, Some((lo, hi))) => x.clamp(lo, hi),
                (_, None) => x.clamp(-1.0e6, 1.0e6),
            };
        }
        if self.ty.components() == 1 {
            out = [out[0]; 4];
        }
        out
    }
}

/// A compiled effect: plain data, cheap to share ([`Arc`]) between elements, windows and
/// threads.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// Its name.
    pub name: String,
    /// Values the host sets.
    pub params: Vec<Param>,
    /// Texture parameters, by name (pictures the host binds).
    pub textures: Vec<String>,
    /// What only changes once a frame (blur radii, particle lifetimes, the parts of programs
    /// that read only parameters and time): one program run once a frame on the CPU, whose
    /// outputs programs read as constants ([`Slot::Uniform`]).
    pub uniforms: Program,
    /// The stages, in order; the last one's buffer is the output.
    pub stages: Vec<Stage>,
    /// Frames a second it wants at most (the window's ambient rate also caps it).
    pub fps: u32,
    /// Resolution relative to the element's device pixels (soft effects look the same at a
    /// half or a quarter).
    pub scale: f32,
    /// How the output combines with what is under the element.
    pub blend: Blend,
    /// The time the still frame shows (reduced motion, paused, Still).
    pub still: f32,
    /// True when a stage reads the previous output frame: the effect has state.
    pub feedback: bool,
    /// The static cost of one output pixel at full resolution (see [`crate::Limits`]).
    pub cost: u32,
}

impl Effect {
    /// The parameter named `name`, with its index.
    pub fn param(&self, name: &str) -> Option<(usize, &Param)> {
        self.params.iter().enumerate().find(|(_, p)| p.name == name)
    }

    /// The texture slot named `name`.
    pub fn texture(&self, name: &str) -> Option<usize> {
        self.textures.iter().position(|t| t == name)
    }

    /// The default values of every parameter.
    pub fn defaults(&self) -> Vec<[f32; 4]> {
        self.params.iter().map(|p| p.default).collect()
    }

    /// Shared.
    pub fn shared(self) -> Arc<Effect> {
        Arc::new(self)
    }

    /// Particles drawn per frame at most.
    pub fn particle_count(&self) -> u32 {
        self.stages
            .iter()
            .map(|s| match &s.kind {
                StageKind::Particles { emitter, .. } => emitter.count,
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_round_trip_through_their_words() {
        let i = Instr { op: Op::Smoothstep, dst: 31, src: [1, 2, 3, 250], imm: 0xbeef };
        assert_eq!(Instr::decode(i.encode()), Some(i));
        assert_eq!(Instr::decode([0xff, 0]), None, "an unknown opcode is refused");
    }

    #[test]
    fn the_opcode_table_is_complete_and_dense() {
        for (n, (op, ..)) in OPS.iter().enumerate() {
            assert_eq!(*op as usize, n, "{op:?} is listed in opcode order");
            assert_eq!(Op::from_u8(n as u8), Some(*op));
        }
    }

    #[test]
    fn params_clamp_hostile_values() {
        let p = Param { name: "k".into(), ty: ParamType::Float, default: [0.5; 4], range: Some((0.0, 1.0)) };
        assert_eq!(p.clamp([f32::NAN; 4]), [0.5; 4]);
        assert_eq!(p.clamp([9.0, 0.0, 0.0, 0.0]), [1.0; 4]);
        let c = Param { name: "c".into(), ty: ParamType::Color, default: [0.0, 0.0, 0.0, 1.0], range: None };
        assert_eq!(c.clamp([2.0, -1.0, f32::INFINITY, 0.5]), [1.0, 0.0, 0.0, 0.5]);
    }
}
