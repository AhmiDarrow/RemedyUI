//! `.rfx` syntax tree to an [`Effect`]: type checking, function inlining, loop unrolling,
//! branch flattening (`if` becomes a select, so every pixel runs the same instructions),
//! constant folding, common-subexpression elimination, hoisting of per-frame work out of
//! per-pixel programs, dead-code removal, register allocation, and the limits that keep a
//! hostile effect cheap.

use std::collections::HashMap;

use remedy_ui_paint::Blend;

use crate::Limits;
use crate::ir::{
    Effect, Input, Instr, Op, Param, ParamType, Particles, Program, Slot, Source, Splat, Stage, StageKind,
};
use crate::syntax::{BinOp, Diagnostic, Expr, File, FnDecl, Item, Span, Stmt, with_suggestion};

/// Where an item starts.
fn item_at(item: &Item) -> Span {
    match item {
        Item::Setting(_, _, at) | Item::Const(_, _, at) | Item::Output(_, at) => *at,
        Item::Fn(f) => f.at,
        Item::Pass { at, .. } | Item::Filter { at, .. } | Item::Particles { at, .. } => *at,
    }
}

type R<T> = Result<T, Diagnostic>;

/// A value's shape: 1 to 4 components, or a matrix of 3 or 4 columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ty {
    F,
    V2,
    V3,
    V4,
    M3,
    M4,
}

impl Ty {
    pub(crate) fn n(self) -> usize {
        match self {
            Ty::F => 1,
            Ty::V2 => 2,
            Ty::V3 => 3,
            Ty::V4 | Ty::M3 | Ty::M4 => 4,
        }
    }

    pub(crate) fn vec(n: usize) -> Ty {
        match n {
            1 => Ty::F,
            2 => Ty::V2,
            3 => Ty::V3,
            _ => Ty::V4,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Ty::F => "float",
            Ty::V2 => "vec2",
            Ty::V3 => "vec3",
            Ty::V4 => "vec4",
            Ty::M3 => "mat3",
            Ty::M4 => "mat4",
        }
    }

    fn parse(s: &str) -> Option<Ty> {
        Some(match s {
            "float" | "bool" => Ty::F,
            "vec2" => Ty::V2,
            "vec3" => Ty::V3,
            "vec4" | "color" => Ty::V4,
            "mat3" => Ty::M3,
            "mat4" => Ty::M4,
            _ => return None,
        })
    }
}

/// How often a node's value changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Rate {
    /// Known now.
    Const,
    /// Once a frame (parameters, time, size).
    Frame,
    /// Every pixel or particle.
    Each,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Leaf {
    Const([u32; 4]),
    Param(u16),
    Input(u16),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum NodeKey {
    Leaf(Leaf),
    Op(Op, u16, [u32; 4]),
}

#[derive(Clone, Debug)]
struct Node {
    key: NodeKey,
    rate: Rate,
}

const NONE: u32 = u32::MAX;

/// The value graph of one effect, shared by every program in it (so a value used by two
/// passes is folded and hoisted once).
#[derive(Default)]
struct Graph {
    nodes: Vec<Node>,
    index: HashMap<NodeKey, u32>,
    limit: usize,
}

/// A compile-time value.
#[derive(Clone, Debug)]
pub(crate) enum Val {
    /// A node, with its shape (matrices are their columns).
    V(u32, Ty),
    M(Vec<u32>, Ty),
    /// Something `sample` reads.
    Src(Source),
    /// An output not set yet.
    Unset,
}

impl Graph {
    fn add(&mut self, key: NodeKey, at: Span) -> R<u32> {
        if let Some(&id) = self.index.get(&key) {
            return Ok(id);
        }
        if self.nodes.len() >= self.limit {
            return Err(Diagnostic::new(
                at,
                format!("the effect is too large: more than {} values after unrolling", self.limit),
            ));
        }
        let rate = match &key {
            NodeKey::Leaf(Leaf::Const(_)) => Rate::Const,
            NodeKey::Leaf(Leaf::Param(_)) => Rate::Frame,
            NodeKey::Leaf(Leaf::Input(i)) => match Input::from_u16(*i) {
                Some(Input::Time | Input::Size | Input::Aspect | Input::Light | Input::Count) => Rate::Frame,
                _ => Rate::Each,
            },
            NodeKey::Op(op, _, args) => {
                if *op == Op::Sample {
                    Rate::Each
                } else {
                    args.iter().filter(|a| **a != NONE).map(|a| self.rate(*a)).max().unwrap_or(Rate::Const)
                }
            }
        };
        let id = self.nodes.len() as u32;
        self.nodes.push(Node { key: key.clone(), rate });
        self.index.insert(key, id);
        Ok(id)
    }

    fn rate(&self, id: u32) -> Rate {
        self.nodes.get(id as usize).map_or(Rate::Const, |n| n.rate)
    }

    fn konst(&mut self, v: [f32; 4], at: Span) -> R<u32> {
        let v = v.map(|x| if x == 0.0 { 0.0 } else { x }); // one zero
        self.add(NodeKey::Leaf(Leaf::Const(v.map(f32::to_bits))), at)
    }

    fn const_of(&self, id: u32) -> Option<[f32; 4]> {
        match self.nodes.get(id as usize).map(|n| &n.key) {
            Some(NodeKey::Leaf(Leaf::Const(b))) => Some(b.map(f32::from_bits)),
            _ => None,
        }
    }

    /// An operation, folded when every operand is known.
    fn op(&mut self, op: Op, imm: u16, args: &[u32], at: Span) -> R<u32> {
        let mut a = [NONE; 4];
        for (slot, x) in a.iter_mut().zip(args) {
            *slot = *x;
        }
        let consts: Option<Vec<[f32; 4]>> = args.iter().map(|x| self.const_of(*x)).collect();
        if let Some(c) = consts
            && !matches!(op, Op::Sample | Op::Input | Op::Const)
        {
            let get = |k: usize| c.get(k).copied().unwrap_or([0.0; 4]);
            let v = fold(op, imm, get(0), get(1), get(2), get(3));
            if v.iter().all(|x| x.is_finite()) {
                return self.konst(v, at);
            }
        }
        // Algebraic identities that keep per-pixel code short.
        let is = |g: &Graph, id: u32, v: f32| g.const_of(id).is_some_and(|c| c.iter().all(|x| *x == v));
        match op {
            Op::Mul if is(self, a[0], 1.0) => return Ok(a[1]),
            Op::Mul if is(self, a[1], 1.0) => return Ok(a[0]),
            Op::Add if is(self, a[0], 0.0) => return Ok(a[1]),
            Op::Add | Op::Sub if is(self, a[1], 0.0) => return Ok(a[0]),
            Op::Div if is(self, a[1], 1.0) => return Ok(a[0]),
            _ => {}
        }
        self.add(NodeKey::Op(op, imm, a), at)
    }
}

/// Evaluates an operation on known values (the same arithmetic the interpreter runs).
fn fold(op: Op, imm: u16, a: [f32; 4], b: [f32; 4], c: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    struct NoEnv;
    impl crate::vm::Env for NoEnv {
        fn input(&self, _: Input, out: &mut crate::vm::Reg, _: usize) {
            *out = [[0.0; crate::vm::LANES]; 4];
        }
        fn sample(&self, _: Source, _: f32, _: f32) -> [f32; 4] {
            [0.0; 4]
        }
    }
    let prog = Program {
        code: vec![Instr { op, dst: 4, src: [0, 1, 2, 3], imm }],
        consts: Vec::new(),
        regs: 5,
        outputs: vec![4],
    };
    let mut vm = crate::vm::Vm::new();
    // Load the operands through constants.
    let mut load = Program { code: Vec::new(), consts: Vec::new(), regs: 5, outputs: Vec::new() };
    for k in 0..4u8 {
        load.code.push(Instr { op: Op::Const, dst: k, src: [0; 4], imm: u16::from(k) });
    }
    load.code.extend(prog.code);
    vm.run(&load, &[a, b, c, d], &NoEnv, 1);
    let r = vm.reg(4);
    [r[0][0], r[1][0], r[2][0], r[3][0]]
}

/// What a name means in a scope.
#[derive(Clone, Debug)]
struct Binding {
    val: Val,
    mutable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// Constants and stage arguments: parameters, time and size only.
    Frame,
    /// A shade pass.
    Pixel,
    /// A particle program.
    Particle,
}

pub(crate) struct Compiler<'a> {
    g: Graph,
    limits: &'a Limits,
    fns: HashMap<String, FnDecl>,
    params: Vec<Param>,
    textures: Vec<String>,
    consts: HashMap<String, Val>,
    /// Buffers written so far: name to (index, stage scale).
    buffers: HashMap<String, u8>,
    inline_depth: u32,
    unrolled: u64,
    feedback: bool,
}

/// Compiles a parsed file.
pub(crate) fn compile(file: &File, prelude: &[FnDecl], limits: &Limits) -> R<Effect> {
    let mut c = Compiler {
        g: Graph { limit: limits.max_values, ..Graph::default() },
        limits,
        fns: HashMap::new(),
        params: Vec::new(),
        textures: Vec::new(),
        consts: HashMap::new(),
        buffers: HashMap::new(),
        inline_depth: 0,
        unrolled: 0,
        feedback: false,
    };
    for f in prelude {
        c.fns.insert(f.name.clone(), f.clone());
    }
    // Parameters.
    for p in &file.params {
        if p.ty == "texture" {
            if c.textures.len() >= limits.max_textures {
                return Err(Diagnostic::new(p.at, format!("at most {} textures", limits.max_textures)));
            }
            c.check_new_name(&p.name, p.at)?;
            c.textures.push(p.name.clone());
            continue;
        }
        let ty = match p.ty.as_str() {
            "float" => ParamType::Float,
            "vec2" => ParamType::Vec2,
            "vec3" => ParamType::Vec3,
            "color" => ParamType::Color,
            other => {
                return Err(Diagnostic::new(
                    p.at,
                    format!("`{other}` is not a parameter type (float, vec2, vec3, color, texture)"),
                ));
            }
        };
        if c.params.len() >= limits.max_params {
            return Err(Diagnostic::new(p.at, format!("at most {} parameters", limits.max_params)));
        }
        c.check_new_name(&p.name, p.at)?;
        let default = match &p.default {
            Some(e) => {
                let v = c.expr(e, &mut Vec::new(), Ctx::Frame)?;
                let (id, t) = c.vector(&v, e.span())?;
                let Some(k) = c.g.const_of(id) else {
                    return Err(Diagnostic::new(e.span(), "a default must be a constant"));
                };
                let want = ty.components();
                if t.n() != want && t != Ty::F {
                    return Err(Diagnostic::new(
                        e.span(),
                        format!("the default is a {}, the parameter a {}", t.name(), ty.name()),
                    ));
                }
                if t == Ty::F && want > 1 { [k[0]; 4] } else { k }
            }
            None if ty == ParamType::Color => [1.0, 1.0, 1.0, 1.0],
            None => [0.0; 4],
        };
        if let Some((lo, hi)) = p.range
            && !matches!(lo.partial_cmp(&hi), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
        {
            return Err(Diagnostic::new(p.at, "the range's low end is above its high end"));
        }
        let param = Param { name: p.name.clone(), ty, default, range: p.range };
        let default = param.clamp(default);
        c.params.push(Param { default, ..param });
    }

    let mut fps = 30u32;
    let mut scale = 0.5f32;
    let mut blend = Blend::Normal;
    let mut still = 0.0f32;
    let mut stages: Vec<Stage> = Vec::new();
    let mut output: Option<(String, Span)> = None;
    let mut frame_roots: Vec<(u32, Ty)> = Vec::new();
    let mut pending: Vec<PendingStage> = Vec::new();

    for item in &file.items {
        match item {
            Item::Setting(name, e, at) => {
                let constant = |c: &mut Compiler<'_>| -> R<f32> {
                    let v = c.expr(e, &mut Vec::new(), Ctx::Frame)?;
                    let (id, _) = c.vector(&v, e.span())?;
                    c.g.const_of(id)
                        .map(|k| k[0])
                        .ok_or_else(|| Diagnostic::new(e.span(), "must be a constant"))
                };
                match name.as_str() {
                    "fps" => {
                        let v = constant(&mut c)?;
                        if !(1.0..=limits.max_fps as f32).contains(&v) {
                            return Err(Diagnostic::new(*at, format!("fps must be 1 to {}", limits.max_fps)));
                        }
                        fps = v as u32;
                    }
                    "scale" => {
                        let v = constant(&mut c)?;
                        if !(1.0 / 16.0..=1.0).contains(&v) {
                            return Err(Diagnostic::new(*at, "scale must be between 0.0625 and 1"));
                        }
                        scale = v;
                    }
                    "still" => {
                        let v = constant(&mut c)?;
                        if !(0.0..=3600.0).contains(&v) {
                            return Err(Diagnostic::new(*at, "still must be a time from 0 to 3600 seconds"));
                        }
                        still = v;
                    }
                    "blend" => {
                        let Expr::Name(mode, _) = e else {
                            return Err(Diagnostic::new(*at, "blend is normal, plus, screen or multiply"));
                        };
                        blend = match mode.as_str() {
                            "normal" => Blend::Normal,
                            "plus" | "add" => Blend::Plus,
                            "screen" => Blend::Screen,
                            "multiply" => Blend::Multiply,
                            _ => {
                                return Err(Diagnostic::new(
                                    *at,
                                    "blend is normal, plus, screen or multiply",
                                ));
                            }
                        };
                    }
                    _ => unreachable!("the parser gives only these settings"),
                }
            }
            Item::Const(name, e, at) => {
                c.check_new_name(name, *at)?;
                let v = c.expr(e, &mut Vec::new(), Ctx::Frame)?;
                c.consts.insert(name.clone(), v);
            }
            Item::Fn(f) => {
                if BUILTINS.contains(&f.name.as_str()) {
                    return Err(Diagnostic::new(f.at, format!("`{}` is a built-in function", f.name)));
                }
                c.fns.insert(f.name.clone(), f.clone());
            }
            Item::Pass { name, scale: s, body, at } => {
                c.check_new_name(name, *at)?;
                let mut scope = vec![HashMap::new()];
                let outs = c.block_outputs(body, &mut scope, Ctx::Pixel, &["out"], *at)?;
                let out = outs
                    .first()
                    .cloned()
                    .flatten()
                    .ok_or_else(|| Diagnostic::new(*at, "a pass must set `out`"))?;
                let color = c.as_color(out, *at)?;
                let s = c.stage_scale(*s, *at)?;
                c.new_buffer(name, *at, stages.len() + pending.len())?;
                pending.push(PendingStage::Shade {
                    name: name.clone(),
                    roots: vec![color],
                    scale: s,
                    at: *at,
                });
            }
            Item::Filter { name, scale: s, call, at } => {
                c.check_new_name(name, *at)?;
                let Expr::Call(f, args, cat) = call else {
                    return Err(Diagnostic::new(
                        *at,
                        "a filter pass is `pass name = blur(src, radius);` or `bloom(…)`",
                    ));
                };
                let src_of = |c: &Compiler<'_>, e: Option<&(Option<String>, Expr)>| -> R<u8> {
                    match e {
                        Some((_, Expr::Name(n, s))) => c.buffers.get(n).copied().ok_or_else(|| {
                            let known = c.buffers.keys().map(String::as_str);
                            Diagnostic::new(
                                *s,
                                with_suggestion(format!("`{n}` is not an earlier pass"), n, known),
                            )
                        }),
                        _ => Err(Diagnostic::new(*cat, "the first argument is the pass to filter")),
                    }
                };
                let s = c.stage_scale(*s, *at)?;
                match f.as_str() {
                    "blur" => {
                        if args.len() != 2 {
                            return Err(Diagnostic::new(*cat, "blur(src, radius)"));
                        }
                        let src = src_of(&c, args.first())?;
                        let radius = c.frame_scalar(args.get(1), *cat)?;
                        frame_roots.push(radius);
                        c.new_buffer(name, *at, stages.len() + pending.len())?;
                        pending.push(PendingStage::Blur {
                            name: name.clone(),
                            src,
                            radius: frame_roots.len() - 1,
                            scale: s,
                            at: *at,
                        });
                    }
                    "bloom" => {
                        if args.len() != 4 {
                            return Err(Diagnostic::new(*cat, "bloom(src, threshold, radius, strength)"));
                        }
                        let src = src_of(&c, args.first())?;
                        let threshold = c.frame_scalar(args.get(1), *cat)?;
                        let radius = c.frame_scalar(args.get(2), *cat)?;
                        let strength = c.frame_scalar(args.get(3), *cat)?;
                        // Bright parts at half resolution, blurred, added back.
                        let base = stages.len() + pending.len();
                        let bright = c.bloom_bright(src, threshold.0, *cat)?;
                        let bright_buf =
                            u8::try_from(base).map_err(|_| Diagnostic::new(*at, "too many passes"))?;
                        pending.push(PendingStage::Shade {
                            name: format!("{name}.bright"),
                            roots: vec![bright],
                            scale: s * 0.5,
                            at: *at,
                        });
                        frame_roots.push(radius);
                        pending.push(PendingStage::Blur {
                            name: format!("{name}.blur"),
                            src: bright_buf,
                            radius: frame_roots.len() - 1,
                            scale: s * 0.5,
                            at: *at,
                        });
                        let combined = c.bloom_combine(src, bright_buf + 1, strength.0, *cat)?;
                        c.new_buffer(name, *at, base + 2)?;
                        pending.push(PendingStage::Shade {
                            name: name.clone(),
                            roots: vec![combined],
                            scale: s,
                            at: *at,
                        });
                    }
                    other => {
                        return Err(Diagnostic::new(
                            *cat,
                            format!("`{other}` is not a filter (blur, bloom); write other filters as passes"),
                        ));
                    }
                }
            }
            Item::Particles { name, over, opts, body, at } => {
                c.check_new_name(name, *at)?;
                let src = match over {
                    Some(o) => Some(c.buffers.get(o).copied().ok_or_else(|| {
                        let known = c.buffers.keys().map(String::as_str);
                        Diagnostic::new(
                            *at,
                            with_suggestion(format!("`{o}` is not an earlier pass"), o, known),
                        )
                    })?),
                    None => None,
                };
                let mut count = 100u32;
                let mut life = None;
                let mut splat = Splat::Soft;
                let mut pblend = Blend::Plus;
                for (k, e) in opts {
                    match k.as_str() {
                        "count" => {
                            let v = c.expr(e, &mut Vec::new(), Ctx::Frame)?;
                            let (id, _) = c.vector(&v, e.span())?;
                            let n =
                                c.g.const_of(id)
                                    .map(|k| k[0])
                                    .ok_or_else(|| Diagnostic::new(e.span(), "count must be a constant"))?;
                            if !(1.0..=limits.max_particles as f32).contains(&n) {
                                return Err(Diagnostic::new(
                                    e.span(),
                                    format!("count must be 1 to {}", limits.max_particles),
                                ));
                            }
                            count = n as u32;
                        }
                        "life" => life = Some(c.frame_scalar(Some(&(None, e.clone())), e.span())?),
                        "splat" => {
                            splat = match e {
                                Expr::Name(n, _) if n == "soft" => Splat::Soft,
                                Expr::Name(n, _) if n == "disc" => Splat::Disc,
                                Expr::Name(n, _) if n == "ring" => Splat::Ring,
                                _ => return Err(Diagnostic::new(e.span(), "splat is soft, disc or ring")),
                            }
                        }
                        "blend" => {
                            pblend = match e {
                                Expr::Name(n, _) if n == "plus" || n == "add" => Blend::Plus,
                                Expr::Name(n, _) if n == "normal" => Blend::Normal,
                                Expr::Name(n, _) if n == "screen" => Blend::Screen,
                                _ => {
                                    return Err(Diagnostic::new(
                                        e.span(),
                                        "particle blend is plus, normal or screen",
                                    ));
                                }
                            }
                        }
                        other => {
                            return Err(Diagnostic::new(
                                e.span(),
                                format!("`{other}` is not a particle option (count, life, splat, blend)"),
                            ));
                        }
                    }
                }
                let life = match life {
                    Some(l) => l,
                    None => (c.g.konst([2.0; 4], *at)?, Ty::F),
                };
                frame_roots.push(life);
                let mut scope = vec![HashMap::new()];
                let outs = c.block_outputs(
                    body,
                    &mut scope,
                    Ctx::Particle,
                    &["pos", "radius", "color", "trail"],
                    *at,
                )?;
                let mut it = outs.into_iter();
                let pos = it
                    .next()
                    .flatten()
                    .ok_or_else(|| Diagnostic::new(*at, "particles must set `pos` (logical pixels)"))?;
                let pos = c.expect_ty(pos, Ty::V2, "pos", *at)?;
                let radius = match it.next().flatten() {
                    Some(v) => c.expect_ty(v, Ty::F, "radius", *at)?,
                    None => c.g.konst([2.0; 4], *at)?,
                };
                let color = match it.next().flatten() {
                    Some(v) => c.as_color(v, *at)?,
                    None => c.g.konst([1.0; 4], *at)?,
                };
                let trail = match it.next().flatten() {
                    Some(v) => c.expect_ty(v, Ty::V2, "trail", *at)?,
                    None => c.g.konst([0.0; 4], *at)?,
                };
                c.new_buffer(name, *at, stages.len() + pending.len())?;
                pending.push(PendingStage::Particles {
                    name: name.clone(),
                    src,
                    roots: vec![pos, radius, color, trail],
                    count,
                    life: frame_roots.len() - 1,
                    splat,
                    blend: pblend,
                    at: *at,
                });
            }
            Item::Output(name, at) => output = Some((name.clone(), *at)),
        }
        if pending.len() + stages.len() > limits.max_stages {
            return Err(Diagnostic::new(
                item_at(item),
                format!("more than {} passes (bloom counts as three)", limits.max_stages),
            ));
        }
    }
    if pending.is_empty() {
        return Err(Diagnostic::new(Span { line: 1, col: 1 }, "an effect needs at least one pass"));
    }
    // The output: the last stage, or the one named (moved last by copying it with a pass).
    if let Some((name, at)) = output {
        let idx = *c.buffers.get(&name).ok_or_else(|| {
            let known = c.buffers.keys().map(String::as_str);
            Diagnostic::new(at, with_suggestion(format!("`{name}` is not a pass"), &name, known))
        })?;
        if usize::from(idx) + 1 != pending.len() {
            let uv = c.g.add(NodeKey::Leaf(Leaf::Input(Input::Uv as u16)), at)?;
            let copy = c.g.op(Op::Sample, Source::Buffer(idx).encode(), &[uv], at)?;
            pending.push(PendingStage::Shade { name: "output".into(), roots: vec![copy], scale: 1.0, at });
        }
    }

    // Per-frame values: what every stage and program hoists, in one program run once a frame.
    let mut hoisted: Vec<u32> = frame_roots.iter().map(|(id, _)| *id).collect();
    let mut stage_roots: Vec<Vec<u32>> = Vec::new();
    for p in &pending {
        stage_roots.push(match p {
            PendingStage::Shade { roots, .. } | PendingStage::Particles { roots, .. } => roots.clone(),
            PendingStage::Blur { .. } => Vec::new(),
        });
    }
    let mut programs = Vec::new();
    for roots in &stage_roots {
        programs.push(if roots.is_empty() { None } else { Some(c.codegen(roots, &mut hoisted, false)?) });
    }
    let uniforms = c.codegen(&hoisted, &mut Vec::new(), true)?;

    let mut cost = 0.0f64;
    // Each stage's share of the cost, for a located, itemised error when the effect is over.
    let mut shares: Vec<(String, f64, Span)> = Vec::new();
    for (p, prog) in pending.into_iter().zip(programs) {
        let at = match &p {
            PendingStage::Shade { at, .. }
            | PendingStage::Blur { at, .. }
            | PendingStage::Particles { at, .. } => *at,
        };
        let prog = prog.unwrap_or_default();
        let (stage, pixel_cost) = match p {
            PendingStage::Shade { name, scale: s, at, .. } => {
                if prog.code.len() > limits.max_instructions {
                    return Err(Diagnostic::new(
                        at,
                        format!(
                            "pass `{name}` is {} instructions; the most is {}",
                            prog.code.len(),
                            limits.max_instructions
                        ),
                    ));
                }
                if prog.cost() > limits.max_pass_cost {
                    return Err(Diagnostic::new(
                        at,
                        format!(
                            "pass `{name}` costs {} a pixel; the most is {}",
                            prog.cost(),
                            limits.max_pass_cost
                        ),
                    ));
                }
                let mut sampled: Vec<u16> = prog
                    .code
                    .iter()
                    .filter(|i| i.op == Op::Sample && (16..48).contains(&i.imm))
                    .map(|i| i.imm)
                    .collect();
                sampled.sort_unstable();
                sampled.dedup();
                if sampled.len() > MAX_SAMPLED_PASSES {
                    return Err(Diagnostic::new(
                        at,
                        format!(
                            "pass `{name}` samples {} other passes; the most is {MAX_SAMPLED_PASSES}",
                            sampled.len()
                        ),
                    ));
                }
                let w = f64::from(prog.cost()) * f64::from(s * s);
                (Stage { name, kind: StageKind::Shade(prog), scale: s }, w)
            }
            PendingStage::Blur { name, src, radius, scale: s, .. } => (
                Stage { name, kind: StageKind::Blur { src, radius }, scale: s },
                BLUR_COST * f64::from(s * s),
            ),
            PendingStage::Particles { name, src, count, life, splat, blend, .. } => {
                let work = u64::from(count) * u64::from(prog.cost() + 40);
                if work > limits.max_particle_work {
                    return Err(Diagnostic::new(
                        at,
                        format!(
                            "particles `{name}` cost {work} a frame; the most is {}",
                            limits.max_particle_work
                        ),
                    ));
                }
                let em = Particles { count, life, program: prog, splat, blend };
                (Stage { name, kind: StageKind::Particles { src, emitter: em }, scale: 1.0 }, 2.0)
            }
        };
        cost += pixel_cost;
        shares.push((stage.name.clone(), pixel_cost * f64::from(scale * scale), at));
        stages.push(stage);
    }
    let cost = (cost * f64::from(scale * scale)).ceil() as u32;
    if cost > limits.max_effect_cost {
        // Located at the costliest pass, naming the ones that cost most.
        shares.sort_by(|a, b| b.1.total_cmp(&a.1));
        let at = shares.first().map_or(Span { line: 1, col: 1 }, |s| s.2);
        let most: Vec<String> =
            shares.iter().take(3).map(|(n, c, _)| format!("`{n}` {}", c.ceil() as u64)).collect();
        return Err(Diagnostic::new(
            at,
            format!(
                "the effect costs {cost} an element pixel; the most is {} (most of it: {}; lower `scale`, \
                 a pass's scale, or simplify)",
                limits.max_effect_cost,
                most.join(", ")
            ),
        ));
    }
    if uniforms.code.len() > limits.max_instructions {
        return Err(Diagnostic::new(
            Span { line: 1, col: 1 },
            format!(
                "the per-frame values (constants, settings, particle lives) are {} instructions; the most is {}",
                uniforms.code.len(),
                limits.max_instructions
            ),
        ));
    }
    let reads_time = |p: &Program| p.code.iter().any(|i| i.op == Op::Input && i.imm == Input::Time as u16);
    let animated = c.feedback
        || reads_time(&uniforms)
        || stages.iter().any(|s| match &s.kind {
            StageKind::Shade(p) => reads_time(p),
            StageKind::Particles { .. } => true,
            StageKind::Blur { .. } => false,
        });
    Ok(Effect {
        name: file.name.clone(),
        params: c.params,
        textures: c.textures,
        uniforms,
        stages,
        fps,
        scale,
        blend,
        still,
        feedback: c.feedback,
        animated,
        cost,
    })
}

/// Earlier passes one pass may sample (the GPU kernel's texture bindings).
pub const MAX_SAMPLED_PASSES: usize = 8;

/// The weight of a blur pass per pixel (two passes of up to 33 taps).
const BLUR_COST: f64 = 140.0;

enum PendingStage {
    Shade {
        name: String,
        roots: Vec<u32>,
        scale: f32,
        at: Span,
    },
    Blur {
        name: String,
        src: u8,
        radius: usize,
        scale: f32,
        at: Span,
    },
    Particles {
        name: String,
        src: Option<u8>,
        roots: Vec<u32>,
        count: u32,
        life: usize,
        splat: Splat,
        blend: Blend,
        at: Span,
    },
}

/// Every built-in function name (user functions may not take them).
pub(crate) const BUILTINS: &[&str] = &[
    "abs",
    "sign",
    "floor",
    "ceil",
    "fract",
    "round",
    "trunc",
    "sqrt",
    "exp",
    "exp2",
    "log",
    "log2",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "min",
    "max",
    "pow",
    "mod",
    "step",
    "clamp",
    "saturate",
    "mix",
    "smoothstep",
    "fma",
    "select",
    "dot",
    "length",
    "distance",
    "normalize",
    "cross",
    "reflect",
    "refract",
    "hash",
    "hash2",
    "hash3",
    "hash4",
    "noise",
    "perlin",
    "simplex",
    "voronoi",
    "fbm",
    "ridged",
    "turbulence",
    "sample",
    "vec2",
    "vec3",
    "vec4",
    "color",
    "float",
    "mat3",
    "mat4",
    "transpose",
    "rand",
    "radians",
    "degrees",
    "lerp",
    "luma",
    "inverse_lerp",
    "remap",
    "height_normal",
    "rgb",
    "rgba",
];

impl Compiler<'_> {
    fn check_new_name(&self, name: &str, at: Span) -> R<()> {
        if self.params.iter().any(|p| p.name == name)
            || self.textures.iter().any(|t| t == name)
            || self.consts.contains_key(name)
            || self.buffers.contains_key(name)
            || ["out", "last"].contains(&name)
        {
            return Err(Diagnostic::new(at, format!("`{name}` is already a name here")));
        }
        Ok(())
    }

    fn new_buffer(&mut self, name: &str, at: Span, index: usize) -> R<()> {
        let i = u8::try_from(index)
            .ok()
            .filter(|i| *i < 32)
            .ok_or_else(|| Diagnostic::new(at, "too many passes"))?;
        self.buffers.insert(name.to_owned(), i);
        Ok(())
    }

    fn stage_scale(&self, s: Option<f32>, at: Span) -> R<f32> {
        match s {
            None => Ok(1.0),
            Some(v) if (1.0 / 16.0..=1.0).contains(&v) => Ok(v),
            Some(_) => Err(Diagnostic::new(at, "a pass's scale is between 0.0625 and 1 (of the effect's)")),
        }
    }

    fn frame_scalar(&mut self, e: Option<&(Option<String>, Expr)>, at: Span) -> R<(u32, Ty)> {
        let Some((_, e)) = e else { return Err(Diagnostic::new(at, "an argument is missing")) };
        let v = self.expr(e, &mut Vec::new(), Ctx::Frame)?;
        let (id, t) = self.vector(&v, e.span())?;
        if t != Ty::F {
            return Err(Diagnostic::new(e.span(), format!("expected a float, found a {}", t.name())));
        }
        Ok((id, t))
    }

    pub(crate) fn input(&mut self, i: Input, at: Span) -> R<u32> {
        self.g.add(NodeKey::Leaf(Leaf::Input(i as u16)), at)
    }

    fn bloom_bright(&mut self, src: u8, threshold: u32, at: Span) -> R<u32> {
        // bright = c * smoothstep(threshold, threshold + 0.25, luma(c)), straight alpha.
        let uv = self.input(Input::Uv, at)?;
        let c = self.g.op(Op::Sample, Source::Buffer(src).encode(), &[uv], at)?;
        let lum_w = self.g.konst([0.2126 * 1.0, 0.7152, 0.0722, 0.0], at)?;
        let l = self.g.op(Op::Dot, 3, &[c, lum_w], at)?;
        let quarter = self.g.konst([0.25; 4], at)?;
        let hi = self.g.op(Op::Add, 0, &[threshold, quarter], at)?;
        let k = self.g.op(Op::Smoothstep, 0, &[threshold, hi, l], at)?;
        let alpha = self.g.op(Op::Swizzle, swz(&[3, 3, 3, 3]), &[c], at)?;
        let a = self.g.op(Op::Mul, 0, &[alpha, k], at)?;
        self.with_alpha(c, a, at)
    }

    fn bloom_combine(&mut self, src: u8, blurred: u8, strength: u32, at: Span) -> R<u32> {
        // Premultiplied: out = src + blurred * strength, as straight colour.
        let uv = self.input(Input::Uv, at)?;
        let a = self.g.op(Op::Sample, Source::Buffer(src).encode(), &[uv], at)?;
        let b = self.g.op(Op::Sample, Source::Buffer(blurred).encode(), &[uv], at)?;
        let pa = self.premul(a, at)?;
        let pb = self.premul(b, at)?;
        let sum = self.g.op(Op::Fma, 0, &[pb, strength, pa], at)?;
        let zero = self.g.konst([0.0; 4], at)?;
        let one = self.g.konst([1.0; 4], at)?;
        let sum = self.g.op(Op::Clamp, 0, &[sum, zero, one], at)?;
        // Back to straight alpha.
        let alpha = self.g.op(Op::Swizzle, swz(&[3, 3, 3, 3]), &[sum], at)?;
        let rgb = self.g.op(Op::Div, 0, &[sum, alpha], at)?;
        self.with_alpha(rgb, alpha, at)
    }

    fn premul(&mut self, c: u32, at: Span) -> R<u32> {
        let a = self.g.op(Op::Swizzle, swz(&[3, 3, 3, 3]), &[c], at)?;
        let p = self.g.op(Op::Mul, 0, &[c, a], at)?;
        self.with_alpha(p, a, at)
    }

    /// `rgb`'s first three components with `alpha` (a splatted scalar) as the fourth.
    fn with_alpha(&mut self, rgb: u32, alpha: u32, at: Span) -> R<u32> {
        let r = self.g.op(Op::Swizzle, swz(&[0, 0, 0, 0]), &[rgb], at)?;
        let gg = self.g.op(Op::Swizzle, swz(&[1, 1, 1, 1]), &[rgb], at)?;
        let b = self.g.op(Op::Swizzle, swz(&[2, 2, 2, 2]), &[rgb], at)?;
        self.g.op(Op::Combine, 0, &[r, gg, b, alpha], at)
    }

    pub(crate) fn vector(&self, v: &Val, at: Span) -> R<(u32, Ty)> {
        match v {
            Val::V(id, t) => Ok((*id, *t)),
            Val::M(_, t) => {
                Err(Diagnostic::new(at, format!("expected a number or vector, found a {}", t.name())))
            }
            Val::Src(_) => {
                Err(Diagnostic::new(at, "a texture or pass can only be sampled: sample(name, uv)"))
            }
            Val::Unset => Err(Diagnostic::new(at, "read before it is set")),
        }
    }

    fn expect_ty(&self, v: Val, ty: Ty, what: &str, at: Span) -> R<u32> {
        let (id, t) = self.vector(&v, at)?;
        if t != ty {
            return Err(Diagnostic::new(at, format!("`{what}` is a {}, not a {}", ty.name(), t.name())));
        }
        Ok(id)
    }

    /// A colour from a float (grey), vec3 (opaque) or vec4.
    fn as_color(&mut self, v: Val, at: Span) -> R<u32> {
        let (id, t) = self.vector(&v, at)?;
        match t {
            Ty::V4 => Ok(id),
            Ty::V3 => {
                let one = self.g.konst([1.0; 4], at)?;
                self.with_alpha(id, one, at)
            }
            Ty::F => {
                let one = self.g.konst([1.0; 4], at)?;
                self.with_alpha(id, one, at)
            }
            _ => Err(Diagnostic::new(at, format!("expected a colour, found a {}", t.name()))),
        }
    }

    /// Runs a block and returns the final values of `outs` (assignable names it sets).
    fn block_outputs(
        &mut self,
        body: &[Stmt],
        scope: &mut Vec<HashMap<String, Binding>>,
        ctx: Ctx,
        outs: &[&str],
        _at: Span,
    ) -> R<Vec<Option<Val>>> {
        if let Some(top) = scope.last_mut() {
            for o in outs {
                top.insert((*o).to_owned(), Binding { val: Val::Unset, mutable: true });
            }
        }
        // Outputs start unset: marked by a placeholder that reading refuses.
        self.stmts(body, scope, ctx, None)?;
        let top = scope.first().cloned().unwrap_or_default();
        Ok(outs
            .iter()
            .map(|o| {
                top.get(*o).and_then(|b| if matches!(b.val, Val::Unset) { None } else { Some(b.val.clone()) })
            })
            .collect())
    }

    fn lookup(&self, scope: &[HashMap<String, Binding>], name: &str) -> Option<Binding> {
        scope.iter().rev().find_map(|s| s.get(name).cloned())
    }

    fn assign(&mut self, scope: &mut [HashMap<String, Binding>], name: &str, val: Val, at: Span) -> R<()> {
        for s in scope.iter_mut().rev() {
            if let Some(b) = s.get_mut(name) {
                if !b.mutable {
                    return Err(Diagnostic::new(at, format!("`{name}` is a `let`; use `var` to change it")));
                }
                // Outputs and variables keep their shape.
                if let (Val::V(_, a), Val::V(_, b2)) = (&b.val, &val)
                    && a != b2
                {
                    return Err(Diagnostic::new(
                        at,
                        format!("`{name}` is a {}, not a {}", a.name(), b2.name()),
                    ));
                }
                b.val = val;
                return Ok(());
            }
        }
        Err(Diagnostic::new(at, format!("`{name}` is not a variable here")))
    }

    /// Runs statements. `ret` collects a function's return value.
    fn stmts(
        &mut self,
        body: &[Stmt],
        scope: &mut Vec<HashMap<String, Binding>>,
        ctx: Ctx,
        mut ret: Option<&mut Option<Val>>,
    ) -> R<()> {
        for (i, s) in body.iter().enumerate() {
            match s {
                Stmt::Let { name, value, mutable, at } => {
                    // Locals may shadow inputs (a local `light`, `seed`); not the outputs.
                    if name == "out" || name == "last" {
                        return Err(Diagnostic::new(*at, format!("`{name}` is a built-in name")));
                    }
                    let v = self.expr(value, scope, ctx)?;
                    if let Some(top) = scope.last_mut() {
                        top.insert(name.clone(), Binding { val: v, mutable: *mutable });
                    }
                }
                Stmt::Assign { name, op, value, at } => {
                    let v = self.expr(value, scope, ctx)?;
                    let v = match op {
                        None => v,
                        Some(op) => {
                            let cur = self.lookup(scope, name).ok_or_else(|| {
                                Diagnostic::new(*at, format!("`{name}` is not a variable here"))
                            })?;
                            if matches!(cur.val, Val::Unset) {
                                return Err(Diagnostic::new(
                                    *at,
                                    format!("`{name}` is read before it is set"),
                                ));
                            }
                            self.binary(*op, cur.val, v, *at)?
                        }
                    };
                    self.assign(scope, name, v, *at)?;
                }
                Stmt::For { var, from, to, body, at } => {
                    let n = to.saturating_sub(*from);
                    if n < 0 || n > i64::from(self.limits.max_loop) {
                        return Err(Diagnostic::new(
                            *at,
                            format!("a loop runs 0 to {} times", self.limits.max_loop),
                        ));
                    }
                    self.unrolled += n as u64;
                    if self.unrolled > self.limits.max_unrolled {
                        return Err(Diagnostic::new(
                            *at,
                            format!("more than {} loop iterations in all", self.limits.max_unrolled),
                        ));
                    }
                    for k in *from..*to {
                        let iv = self.g.konst([k as f32; 4], *at)?;
                        let mut inner = HashMap::new();
                        inner.insert(var.clone(), Binding { val: Val::V(iv, Ty::F), mutable: false });
                        scope.push(inner);
                        let r = self.stmts(body, scope, ctx, None);
                        scope.pop();
                        r?;
                    }
                }
                Stmt::If { cond, then, otherwise, at } => {
                    let c = self.expr(cond, scope, ctx)?;
                    let (cid, ct) = self.vector(&c, cond.span())?;
                    if ct != Ty::F {
                        return Err(Diagnostic::new(
                            cond.span(),
                            "a condition is a float (non-zero is true)",
                        ));
                    }
                    // Known conditions pick a branch; others run both and select.
                    if let Some(k) = self.g.const_of(cid) {
                        scope.push(HashMap::new());
                        let r = self.stmts(if k[0] > 0.5 { then } else { otherwise }, scope, ctx, None);
                        scope.pop();
                        r?;
                        continue;
                    }
                    let before: Vec<HashMap<String, Binding>> = scope.clone();
                    scope.push(HashMap::new());
                    self.stmts(then, scope, ctx, None)?;
                    scope.pop();
                    let after_then = std::mem::replace(scope, before.clone());
                    scope.push(HashMap::new());
                    self.stmts(otherwise, scope, ctx, None)?;
                    scope.pop();
                    // Every variable either branch changed: select per variable.
                    for (lvl, (t, e)) in after_then.iter().zip(scope.iter_mut()).enumerate() {
                        for (name, tb) in t {
                            let Some(eb) = e.get_mut(name) else { continue };
                            let orig = before.get(lvl).and_then(|m| m.get(name));
                            let same = |a: &Val, b: &Val| match (a, b) {
                                (Val::V(x, _), Val::V(y, _)) => x == y,
                                (Val::Src(a), Val::Src(b)) => a == b,
                                (Val::Unset, Val::Unset) => true,
                                _ => false,
                            };
                            if same(&tb.val, &eb.val) {
                                continue;
                            }
                            let unset = |v: &Val| matches!(v, Val::Unset);
                            if unset(&tb.val) || unset(&eb.val) {
                                if orig.is_some_and(|o| unset(&o.val)) {
                                    return Err(Diagnostic::new(
                                        *at,
                                        format!(
                                            "`{name}` is set in one branch only; set it before the `if` too"
                                        ),
                                    ));
                                }
                                continue;
                            }
                            let (Val::V(a, ta), Val::V(b, tb2)) = (&eb.val, &tb.val) else {
                                return Err(Diagnostic::new(
                                    *at,
                                    format!("`{name}` cannot be chosen by a condition"),
                                ));
                            };
                            if ta != tb2 {
                                return Err(Diagnostic::new(
                                    *at,
                                    format!(
                                        "`{name}` is a {} in one branch, a {} in the other",
                                        tb2.name(),
                                        ta.name()
                                    ),
                                ));
                            }
                            let sel = self.g.op(Op::Select, 0, &[*a, *b, cid], *at)?;
                            eb.val = Val::V(sel, *ta);
                        }
                    }
                }
                Stmt::Return(e, at) => {
                    let Some(slot) = ret.as_deref_mut() else {
                        return Err(Diagnostic::new(*at, "`return` is only for functions"));
                    };
                    if i + 1 != body.len() {
                        return Err(Diagnostic::new(*at, "`return` is the last statement of a function"));
                    }
                    *slot = Some(self.expr(e, scope, ctx)?);
                }
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr, scope: &mut Vec<HashMap<String, Binding>>, ctx: Ctx) -> R<Val> {
        match e {
            Expr::Num(n, at) => Ok(Val::V(self.g.konst([*n; 4], *at)?, Ty::F)),
            Expr::Color(c, at) => Ok(Val::V(self.g.konst(*c, *at)?, Ty::V4)),
            Expr::Name(name, at) => self.name(name, *at, scope, ctx),
            Expr::Unary(op, x, at) => {
                let v = self.expr(x, scope, ctx)?;
                match (op, &v) {
                    ('-', Val::M(cols, t)) => {
                        let mut out = Vec::new();
                        for c in cols {
                            out.push(self.g.op(Op::Neg, 0, &[*c], *at)?);
                        }
                        Ok(Val::M(out, *t))
                    }
                    _ => {
                        let (id, t) = self.vector(&v, *at)?;
                        let o = if *op == '-' { Op::Neg } else { Op::Not };
                        Ok(Val::V(self.g.op(o, 0, &[id], *at)?, t))
                    }
                }
            }
            Expr::Binary(op, a, b, at) => {
                let a = self.expr(a, scope, ctx)?;
                let b = self.expr(b, scope, ctx)?;
                self.binary(*op, a, b, *at)
            }
            Expr::Ternary(c, a, b, at) => {
                let cv = self.expr(c, scope, ctx)?;
                let (cid, ct) = self.vector(&cv, *at)?;
                if ct != Ty::F {
                    return Err(Diagnostic::new(*at, "a condition is a float"));
                }
                let a = self.expr(a, scope, ctx)?;
                let b = self.expr(b, scope, ctx)?;
                let (x, tx) = self.vector(&a, *at)?;
                let (y, ty) = self.vector(&b, *at)?;
                let t = unify(tx, ty).ok_or_else(|| {
                    Diagnostic::new(*at, format!("the two choices are a {} and a {}", tx.name(), ty.name()))
                })?;
                Ok(Val::V(self.g.op(Op::Select, 0, &[y, x, cid], *at)?, t))
            }
            Expr::Field(x, f, at) => {
                let v = self.expr(x, scope, ctx)?;
                let (id, t) = self.vector(&v, *at)?;
                let mut lanes = Vec::new();
                for ch in f.chars() {
                    let k = match ch {
                        'x' | 'r' => 0,
                        'y' | 'g' => 1,
                        'z' | 'b' => 2,
                        'w' | 'a' => 3,
                        _ => {
                            return Err(Diagnostic::new(
                                *at,
                                format!("`.{f}` is not a swizzle (xyzw or rgba)"),
                            ));
                        }
                    };
                    if k >= t.n() {
                        return Err(Diagnostic::new(*at, format!("a {} has no `.{ch}`", t.name())));
                    }
                    lanes.push(k);
                }
                if lanes.is_empty() || lanes.len() > 4 {
                    return Err(Diagnostic::new(*at, "a swizzle has 1 to 4 components"));
                }
                let n = lanes.len();
                // A single component is splatted (scalars fill every lane).
                let first = lanes.first().copied().unwrap_or(0);
                let pattern: Vec<u8> = if n == 1 {
                    vec![first as u8; 4]
                } else {
                    (0..4).map(|i| lanes.get(i).copied().unwrap_or(first) as u8).collect()
                };
                Ok(Val::V(self.g.op(Op::Swizzle, swz(&pattern), &[id], *at)?, Ty::vec(n)))
            }
            Expr::Call(name, args, at) => self.call(name, args, *at, scope, ctx),
        }
    }

    fn name(&mut self, name: &str, at: Span, scope: &[HashMap<String, Binding>], ctx: Ctx) -> R<Val> {
        if let Some(b) = self.lookup(scope, name) {
            if matches!(b.val, Val::Unset) {
                return Err(Diagnostic::new(at, format!("`{name}` is read before it is set")));
            }
            return Ok(b.val);
        }
        if let Some(v) = self.consts.get(name) {
            return Ok(v.clone());
        }
        if let Some((i, p)) = self.params.iter().enumerate().find(|(_, p)| p.name == name) {
            let ty = Ty::vec(p.ty.components());
            return Ok(Val::V(self.g.add(NodeKey::Leaf(Leaf::Param(i as u16)), at)?, ty));
        }
        if let Some(i) = self.textures.iter().position(|t| t == name) {
            return Ok(Val::Src(Source::Texture(i as u8)));
        }
        if let Some(&i) = self.buffers.get(name) {
            if ctx != Ctx::Pixel {
                return Err(Diagnostic::new(at, "only passes sample other passes"));
            }
            return Ok(Val::Src(Source::Buffer(i)));
        }
        match name {
            "pi" => return Ok(Val::V(self.g.konst([std::f32::consts::PI; 4], at)?, Ty::F)),
            "tau" => return Ok(Val::V(self.g.konst([std::f32::consts::TAU; 4], at)?, Ty::F)),
            "last" => {
                if ctx != Ctx::Pixel {
                    return Err(Diagnostic::new(at, "only passes read the last frame"));
                }
                self.feedback = true;
                return Ok(Val::Src(Source::Feedback));
            }
            _ => {}
        }
        if let Some((_, input, ty, allowed)) = INPUTS.iter().find(|(n, ..)| *n == name) {
            if !allowed.contains(&ctx) {
                let where_ = match ctx {
                    Ctx::Frame => "in constants and settings",
                    Ctx::Pixel => "in a pass",
                    Ctx::Particle => "in particles",
                };
                return Err(Diagnostic::new(at, format!("`{name}` cannot be used {where_}")));
            }
            return Ok(Val::V(self.input(*input, at)?, *ty));
        }
        let mut known: Vec<&str> = Vec::new();
        for frame in scope {
            known.extend(frame.keys().map(String::as_str));
        }
        known.extend(self.consts.keys().map(String::as_str));
        known.extend(self.params.iter().map(|p| p.name.as_str()));
        known.extend(self.textures.iter().map(String::as_str));
        known.extend(self.buffers.keys().map(String::as_str));
        known.extend(["pi", "tau", "last"]);
        known.extend(INPUTS.iter().map(|(n, ..)| *n));
        Err(Diagnostic::new(at, with_suggestion(format!("`{name}` is not defined"), name, known)))
    }

    fn binary(&mut self, op: BinOp, a: Val, b: Val, at: Span) -> R<Val> {
        // Matrix products.
        if op == BinOp::Mul {
            match (&a, &b) {
                (Val::M(cols, mt), Val::V(v, vt)) if vt.n() == cols.len() && *vt != Ty::F => {
                    // M * v = Σ col_k * v[k]
                    let mut acc: Option<u32> = None;
                    for (k, c) in cols.iter().enumerate() {
                        let vk = self.g.op(Op::Swizzle, swz(&[k as u8; 4]), &[*v], at)?;
                        acc = Some(match acc {
                            None => self.g.op(Op::Mul, 0, &[*c, vk], at)?,
                            Some(s) => self.g.op(Op::Fma, 0, &[*c, vk, s], at)?,
                        });
                    }
                    let n = if *mt == Ty::M3 { 3 } else { 4 };
                    return Ok(Val::V(acc.unwrap_or(*v), Ty::vec(n)));
                }
                (Val::V(v, vt), Val::M(cols, _)) if vt.n() == cols.len() && *vt != Ty::F => {
                    // v * M = (dot(v, col_0), dot(v, col_1), …)
                    let n = cols.len();
                    let mut comps = Vec::new();
                    for c in cols {
                        comps.push(self.g.op(Op::Dot, n as u16, &[*v, *c], at)?);
                    }
                    return self.combine(&comps, at).map(|id| Val::V(id, Ty::vec(n)));
                }
                (Val::M(a_cols, t), Val::M(b_cols, t2)) if t == t2 => {
                    let mut out = Vec::new();
                    for bc in b_cols {
                        let prod =
                            self.binary(BinOp::Mul, a.clone(), Val::V(*bc, Ty::vec(a_cols.len())), at)?;
                        let (id, _) = self.vector(&prod, at)?;
                        out.push(id);
                    }
                    return Ok(Val::M(out, *t));
                }
                (Val::M(cols, t), Val::V(s, Ty::F)) | (Val::V(s, Ty::F), Val::M(cols, t)) => {
                    let mut out = Vec::new();
                    for c in cols {
                        out.push(self.g.op(Op::Mul, 0, &[*c, *s], at)?);
                    }
                    return Ok(Val::M(out, *t));
                }
                _ => {}
            }
        }
        let (x, tx) = self.vector(&a, at)?;
        let (y, ty) = self.vector(&b, at)?;
        let t = unify(tx, ty).ok_or_else(|| {
            Diagnostic::new(
                at,
                format!(
                    "a {} and a {} cannot be combined; convert one (vec3(v, 1.0), v.xy)",
                    tx.name(),
                    ty.name()
                ),
            )
        })?;
        let o = match op {
            BinOp::Add => Op::Add,
            BinOp::Sub => Op::Sub,
            BinOp::Mul => Op::Mul,
            BinOp::Div => Op::Div,
            BinOp::Mod => Op::Mod,
            BinOp::Lt => Op::Lt,
            BinOp::Le => Op::Le,
            BinOp::Gt => Op::Gt,
            BinOp::Ge => Op::Ge,
            BinOp::Eq => Op::Eq,
            BinOp::Ne => Op::Ne,
            BinOp::And => Op::And,
            BinOp::Or => Op::Or,
        };
        Ok(Val::V(self.g.op(o, 0, &[x, y], at)?, t))
    }

    /// A vector of scalars (each splatted) from their first lanes.
    pub(crate) fn combine(&mut self, comps: &[u32], at: Span) -> R<u32> {
        let get = |k: usize| comps.get(k).copied().or(comps.last().copied()).unwrap_or(NONE);
        self.g.op(Op::Combine, 0, &[get(0), get(1), get(2), get(3)], at)
    }

    /// Every component of `v` as a splatted scalar.
    pub(crate) fn components(&mut self, v: &Val, at: Span) -> R<Vec<u32>> {
        match v {
            Val::V(id, Ty::F) => Ok(vec![*id]),
            Val::V(id, t) => {
                (0..t.n()).map(|k| self.g.op(Op::Swizzle, swz(&[k as u8; 4]), &[*id], at)).collect()
            }
            Val::M(cols, t) => {
                let n = if *t == Ty::M3 { 3 } else { 4 };
                let mut out = Vec::new();
                for c in cols {
                    for k in 0..n {
                        out.push(self.g.op(Op::Swizzle, swz(&[k as u8; 4]), &[*c], at)?);
                    }
                }
                Ok(out)
            }
            Val::Src(_) | Val::Unset => Err(Diagnostic::new(at, "a texture or pass is not a value")),
        }
    }

    fn call(
        &mut self,
        name: &str,
        args: &[(Option<String>, Expr)],
        at: Span,
        scope: &mut Vec<HashMap<String, Binding>>,
        ctx: Ctx,
    ) -> R<Val> {
        // Constructors take any mix of values.
        if let Some(n) = match name {
            "float" => Some(1),
            "vec2" => Some(2),
            "vec3" => Some(3),
            "vec4" | "color" | "rgba" => Some(4),
            "rgb" => Some(3),
            _ => None,
        } {
            let mut comps = Vec::new();
            for (_, a) in args {
                let v = self.expr(a, scope, ctx)?;
                comps.extend(self.components(&v, a.span())?);
            }
            let first = comps.first().copied().unwrap_or(NONE);
            return match comps.len() {
                1 if n > 1 => Ok(Val::V(first, Ty::vec(n))),
                k if k == n => {
                    if n == 1 {
                        Ok(Val::V(first, Ty::F))
                    } else {
                        Ok(Val::V(self.combine(&comps, at)?, Ty::vec(n)))
                    }
                }
                k => Err(Diagnostic::new(at, format!("{name}(…) needs {n} components, given {k}"))),
            };
        }
        if name == "mat3" || name == "mat4" {
            let n = if name == "mat3" { 3 } else { 4 };
            let mut cols = Vec::new();
            for (_, a) in args {
                let v = self.expr(a, scope, ctx)?;
                let (id, t) = self.vector(&v, a.span())?;
                if t.n() != n || t == Ty::F {
                    return Err(Diagnostic::new(a.span(), format!("{name} takes {n} vec{n} columns")));
                }
                cols.push(id);
            }
            if cols.len() == 1 && args.len() == 1 {
                // mat3(s): s on the diagonal.
            }
            if cols.len() != n {
                return Err(Diagnostic::new(at, format!("{name} takes {n} columns")));
            }
            return Ok(Val::M(cols, if n == 3 { Ty::M3 } else { Ty::M4 }));
        }
        // User and prelude functions, inlined.
        if let Some(f) = self.fns.get(name).cloned() {
            return self.inline(&f, args, at, scope, ctx);
        }
        if !BUILTINS.contains(&name) {
            let known = self.fns.keys().map(String::as_str).chain(BUILTINS.iter().copied());
            return Err(Diagnostic::new(
                at,
                with_suggestion(format!("`{name}` is not a function"), name, known),
            ));
        }
        // Built-ins: arguments first (sample takes a source).
        let mut vals = Vec::new();
        for (_, a) in args {
            vals.push((self.expr(a, scope, ctx)?, a.span()));
        }
        crate::builtins::call(self, name, &vals, at, ctx == Ctx::Particle)
    }

    fn inline(
        &mut self,
        f: &FnDecl,
        args: &[(Option<String>, Expr)],
        at: Span,
        scope: &mut Vec<HashMap<String, Binding>>,
        ctx: Ctx,
    ) -> R<Val> {
        if args.len() != f.args.len() {
            return Err(Diagnostic::new(
                at,
                format!("`{}` takes {} arguments, given {}", f.name, f.args.len(), args.len()),
            ));
        }
        self.inline_depth += 1;
        if self.inline_depth > self.limits.max_inline_depth {
            return Err(Diagnostic::new(
                at,
                format!(
                    "functions call each other more than {} deep (no recursion)",
                    self.limits.max_inline_depth
                ),
            ));
        }
        let mut frame = HashMap::new();
        for ((pname, pty), (_, e)) in f.args.iter().zip(args) {
            let v = self.expr(e, scope, ctx)?;
            let want = Ty::parse(pty).ok_or_else(|| {
                let known = ["float", "vec2", "vec3", "vec4", "color", "mat3", "mat4"];
                Diagnostic::new(f.at, with_suggestion(format!("`{pty}` is not a type"), pty, known))
            })?;
            let got = match &v {
                Val::V(_, t) | Val::M(_, t) => *t,
                Val::Src(_) | Val::Unset => {
                    return Err(Diagnostic::new(e.span(), "functions take values, not textures"));
                }
            };
            let v = match (want, got, &v) {
                (w, g, _) if w == g => v,
                // A float widens to any vector (it is already splatted).
                (w, Ty::F, Val::V(id, _)) if !matches!(w, Ty::M3 | Ty::M4) => Val::V(*id, w),
                _ => {
                    return Err(Diagnostic::new(
                        e.span(),
                        format!("`{}` takes a {} for `{pname}`, given a {}", f.name, want.name(), got.name()),
                    ));
                }
            };
            frame.insert(pname.clone(), Binding { val: v, mutable: true });
        }
        // A function sees its arguments, constants, parameters and inputs; not the caller's
        // locals.
        let mut inner = vec![frame];
        let mut ret = None;
        let r = self.stmts(&f.body, &mut inner, ctx, Some(&mut ret));
        self.inline_depth -= 1;
        r?;
        let v = ret.ok_or_else(|| Diagnostic::new(f.at, format!("`{}` must end with `return`", f.name)))?;
        let want =
            Ty::parse(&f.ret).ok_or_else(|| Diagnostic::new(f.at, format!("`{}` is not a type", f.ret)))?;
        match &v {
            Val::V(id, Ty::F) if want != Ty::F && !matches!(want, Ty::M3 | Ty::M4) => Ok(Val::V(*id, want)),
            Val::V(_, t) | Val::M(_, t) if *t == want => Ok(v),
            Val::V(_, t) | Val::M(_, t) => Err(Diagnostic::new(
                at,
                format!("`{}` returns a {}, declared {}", f.name, t.name(), want.name()),
            )),
            Val::Src(_) | Val::Unset => Err(Diagnostic::new(at, "functions return values")),
        }
    }

    /// Instructions for `roots`. Per-frame values a program uses are hoisted: their nodes go
    /// to `hoist`, and the program reads them as constants. With `all`, everything is
    /// computed here (the per-frame program itself).
    fn codegen(&mut self, roots: &[u32], hoist: &mut Vec<u32>, all: bool) -> R<Program> {
        // Which nodes the program computes: walk from the roots, stopping at constants and
        // (unless `all`) at per-frame values.
        let mut need = vec![false; self.g.nodes.len()];
        let mut stack: Vec<u32> = roots.to_vec();
        let mut slots: HashMap<u32, usize> = HashMap::new();
        let mut consts: Vec<Slot> = Vec::new();
        let slot_of = |key: Slot, consts: &mut Vec<Slot>| -> usize {
            if let Some(i) = consts.iter().position(|c| *c == key) {
                i
            } else {
                consts.push(key);
                consts.len() - 1
            }
        };
        while let Some(id) = stack.pop() {
            let Some(n) = self.g.nodes.get(id as usize) else { continue };
            if need.get(id as usize).copied().unwrap_or(true) || slots.contains_key(&id) {
                continue;
            }
            match (&n.key, n.rate) {
                (NodeKey::Leaf(Leaf::Const(b)), _) => {
                    slots.insert(id, slot_of(Slot::Value(b.map(f32::from_bits)), &mut consts));
                }
                (NodeKey::Leaf(Leaf::Param(p)), _) => {
                    slots.insert(id, slot_of(Slot::Param(usize::from(*p)), &mut consts));
                }
                (NodeKey::Op(..), Rate::Frame) if !all => {
                    let u = match hoist.iter().position(|h| *h == id) {
                        Some(u) => u,
                        None => {
                            hoist.push(id);
                            hoist.len() - 1
                        }
                    };
                    slots.insert(id, slot_of(Slot::Uniform(u), &mut consts));
                }
                (NodeKey::Op(_, _, args), _) => {
                    if let Some(x) = need.get_mut(id as usize) {
                        *x = true;
                    }
                    stack.extend(args.iter().filter(|a| **a != NONE));
                }
                (NodeKey::Leaf(Leaf::Input(_)), _) => {
                    if let Some(x) = need.get_mut(id as usize) {
                        *x = true;
                    }
                }
            }
        }
        // Constants that are roots still need a register: load them.
        let order: Vec<u32> = (0..self.g.nodes.len() as u32)
            .filter(|i| {
                need.get(*i as usize).copied().unwrap_or(false)
                    || (roots.contains(i) && slots.contains_key(i))
            })
            .collect();
        // Last use of every value, for register reuse.
        let mut last: HashMap<u32, usize> = HashMap::new();
        for (pos, id) in order.iter().enumerate() {
            if let Some(NodeKey::Op(_, _, args)) = self.g.nodes.get(*id as usize).map(|n| &n.key)
                && need.get(*id as usize).copied().unwrap_or(false)
            {
                for a in args.iter().filter(|a| **a != NONE) {
                    last.insert(*a, pos);
                }
            }
        }
        for r in roots {
            last.insert(*r, usize::MAX);
        }
        let mut reg: HashMap<u32, u8> = HashMap::new();
        let mut free: Vec<u8> = Vec::new();
        let mut next = 0u16;
        let mut code = Vec::new();
        let max = u16::from(self.limits.max_registers);
        let mut alloc = |free: &mut Vec<u8>| -> R<u8> {
            if let Some(r) = free.pop() {
                return Ok(r);
            }
            if next >= max {
                return Err(Diagnostic::new(
                    Span { line: 1, col: 1 },
                    format!("a program needs more than {max} registers; simplify it or split it into passes"),
                ));
            }
            next += 1;
            Ok((next - 1) as u8)
        };
        // Loads of constants and per-frame values happen where they are used.
        let load = |id: u32,
                    code: &mut Vec<Instr>,
                    reg: &mut HashMap<u32, u8>,
                    free: &mut Vec<u8>,
                    alloc: &mut dyn FnMut(&mut Vec<u8>) -> R<u8>|
         -> R<u8> {
            if let Some(r) = reg.get(&id) {
                return Ok(*r);
            }
            let Some(&slot) = slots.get(&id) else {
                return Err(Diagnostic::new(Span { line: 1, col: 1 }, "internal: a value has no register"));
            };
            let r = alloc(free)?;
            code.push(Instr { op: Op::Const, dst: r, src: [0; 4], imm: slot as u16 });
            reg.insert(id, r);
            Ok(r)
        };
        let mut const_regs: Vec<(u32, u8)> = Vec::new();
        for (pos, id) in order.iter().enumerate() {
            let Some(node) = self.g.nodes.get(*id as usize) else { continue };
            if !need.get(*id as usize).copied().unwrap_or(false) {
                // A constant root.
                let r = load(*id, &mut code, &mut reg, &mut free, &mut alloc)?;
                const_regs.push((*id, r));
                continue;
            }
            let ins = match &node.key {
                NodeKey::Leaf(Leaf::Input(i)) => {
                    let r = alloc(&mut free)?;
                    Instr { op: Op::Input, dst: r, src: [0; 4], imm: *i }
                }
                NodeKey::Op(op, imm, args) => {
                    let mut src = [0u8; 4];
                    // Constants and per-frame values are loaded where they are used and let go
                    // of at once (a load costs nothing): holding a shared constant from its
                    // first use to its last would tie up a register in between.
                    let mut temps: Vec<u8> = Vec::new();
                    for (k, a) in args.iter().enumerate() {
                        if *a == NONE {
                            continue;
                        }
                        let r = match (reg.get(a), slots.get(a)) {
                            (Some(r), _) => *r,
                            (None, Some(&slot)) => {
                                let r = alloc(&mut free)?;
                                code.push(Instr { op: Op::Const, dst: r, src: [0; 4], imm: slot as u16 });
                                temps.push(r);
                                r
                            }
                            (None, None) => {
                                return Err(Diagnostic::new(
                                    Span { line: 1, col: 1 },
                                    "internal: a value has no register",
                                ));
                            }
                        };
                        if let Some(s) = src.get_mut(k) {
                            *s = r;
                        }
                    }
                    // Operands used for the last time free their registers before the
                    // result takes one (the interpreter reads operands first).
                    for a in args.iter().filter(|a| **a != NONE) {
                        if last.get(a).is_some_and(|l| *l == pos)
                            && !slots.contains_key(a)
                            && let Some(r) = reg.remove(a)
                            && !free.contains(&r)
                        {
                            free.push(r);
                        }
                    }
                    for r in temps {
                        if !free.contains(&r) {
                            free.push(r);
                        }
                    }
                    let r = alloc(&mut free)?;
                    Instr { op: *op, dst: r, src, imm: *imm }
                }
                NodeKey::Leaf(_) => continue,
            };
            reg.insert(*id, ins.dst);
            code.push(ins);
        }
        let outputs = roots.iter().map(|r| reg.get(r).copied().unwrap_or(0)).collect();
        Ok(Program { code, consts, regs: next.max(1) as u8, outputs })
    }
}

/// The swizzle immediate for lanes `p` (two bits each).
pub(crate) fn swz(p: &[u8]) -> u16 {
    p.iter().take(4).enumerate().fold(0u16, |acc, (i, &k)| acc | (u16::from(k & 3) << (2 * i)))
}

pub(crate) fn unify(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        (x, y) if x == y => Some(x),
        (Ty::F, y) => Some(y),
        (x, Ty::F) => Some(x),
        _ => None,
    }
}

/// Names programs read, what they are, and where they may be used.
const INPUTS: &[(&str, Input, Ty, &[Ctx])] = &[
    ("uv", Input::Uv, Ty::V2, &[Ctx::Pixel]),
    ("px", Input::Px, Ty::V2, &[Ctx::Pixel]),
    ("texel", Input::Texel, Ty::F, &[Ctx::Pixel]),
    ("size", Input::Size, Ty::V2, &[Ctx::Pixel, Ctx::Particle, Ctx::Frame]),
    ("time", Input::Time, Ty::F, &[Ctx::Pixel, Ctx::Particle, Ctx::Frame]),
    ("aspect", Input::Aspect, Ty::F, &[Ctx::Pixel, Ctx::Particle, Ctx::Frame]),
    ("light", Input::Light, Ty::F, &[Ctx::Pixel, Ctx::Particle, Ctx::Frame]),
    ("id", Input::Id, Ty::F, &[Ctx::Particle]),
    ("seed", Input::Seed, Ty::F, &[Ctx::Particle]),
    ("age", Input::Age, Ty::F, &[Ctx::Particle]),
    ("life", Input::Life, Ty::F, &[Ctx::Particle]),
    ("progress", Input::Progress, Ty::F, &[Ctx::Particle]),
    ("count", Input::Count, Ty::F, &[Ctx::Particle]),
];

/// What the built-in library sees of the compiler.
pub(crate) trait Lower {
    fn op(&mut self, op: Op, imm: u16, args: &[u32], at: Span) -> R<u32>;
    fn konst(&mut self, v: [f32; 4], at: Span) -> R<u32>;
    fn is_const(&self, id: u32) -> Option<[f32; 4]>;
}

impl Lower for Compiler<'_> {
    fn op(&mut self, op: Op, imm: u16, args: &[u32], at: Span) -> R<u32> {
        self.g.op(op, imm, args, at)
    }
    fn konst(&mut self, v: [f32; 4], at: Span) -> R<u32> {
        self.g.konst(v, at)
    }
    fn is_const(&self, id: u32) -> Option<[f32; 4]> {
        self.g.const_of(id)
    }
}
