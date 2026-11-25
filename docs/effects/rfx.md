# `.rfx`: the Prism effects language

`.rfx` describes an **effect**: an animated, procedural picture (fire, water, a starfield,
glow, a raymarched 3D object) that RemedyUI draws in an element. Effects are data, not
shader code. The compiler checks them against cost limits and turns them into programs for
a fixed set of kernels, which run on the GPU or, with the same results, on the CPU. Apps,
built-in themes, people's themes and RemedyLua scripts all use the same language and the same
kernels.

- Crate: `remedy-ui-effects` (`remedy_ui::effects` with the `effects` feature).
- Element: `effect_view(ui, Instance::new(effect))` (`remedy-ui-media-effects`); in `.rui`
  markup, `effect(name: "fire", param: "intensity", value: $heat) { … }` once the app calls
  `remedy_ui::effects::register_markup` (built-in effects only).
- Library: 38 built-in effects in `crates/remedy-ui-engine/effects/effects/*.rfx`, reachable by
  name with `library("fire")`. Read them as examples; anything they do, a theme can do too.
- Design: [docs/design/16-effects.md](../design/16-effects.md).

## A first effect

```rfx
// A pool of warm light that breathes.
effect lamp(color tint = #ffc46b, float radius = 0.35 in 0.01..2) {
    fps 24;          // frames a second it wants (the window's ambient rate also caps it)
    scale 0.25;      // drawn at a quarter of the element's resolution: soft light needs no more
    blend plus;      // added to what is under the element, like light

    const pulse = 0.85 + 0.15 * sin(time * 2.0);   // once a frame, not per pixel

    pass light {
        let d = length((uv - vec2(0.5)) * vec2(aspect, 1.0)) / radius;
        out = vec4(tint.rgb, exp(-d * d * 3.0) * pulse);
    }
}
```

```rust
use remedy_ui::effects::{Effect, Instance, effect_view};

let effect = Effect::compile(SOURCE)?;                 // or compile_with(SOURCE, &Limits::untrusted())
let (el, view) = effect_view(ui, Instance::new(effect.shared()));
ui.root().child(el.width(300.0).height(200.0));
view.set("radius", 0.5)?;                              // by name, typed, clamped
```

## File structure

One effect per file:

```text
effect NAME ( PARAMETERS ) { ITEMS }
```

Comments are `// …` and `/* … */`.

### Parameters

What the host (an app, a theme, a script) sets. Each has a type, a default and an optional
range; values from the host are clamped to the range, and non-finite numbers keep the
default.

| Declaration | Meaning |
|---|---|
| `float k = 0.5 in 0..1` | A number, clamped to 0–1 |
| `vec2 at = (0.5, 0.5)` | Two numbers (`(x, y)` is a vector literal) |
| `vec3 v = (1.0, 0.0, 0.0)` | Three numbers |
| `color c = #ff8a1f` | A colour: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, components 0–1 |
| `texture art` | A picture the host binds (`EffectView::set_texture`); at most 4 |

### Items

| Item | Meaning |
|---|---|
| `fps N;` | Frames a second wanted, 1–60 (30 for untrusted effects). Default 30 |
| `scale K;` | Resolution relative to the element's device pixels, 1/16–1. Default 0.5 |
| `blend MODE;` | How the output combines with what is under it: `normal`, `plus`, `screen`, `multiply` |
| `still T;` | The time (seconds) of the still frame shown when motion is paused. Default 0 |
| `const NAME = EXPR;` | A value computed once a frame from parameters, `time`, `size` |
| `fn NAME(a: TYPE, …) -> TYPE { … return EXPR; }` | A function (inlined; no recursion) |
| `pass NAME [scale K] { … out = EXPR; }` | A per-pixel program writing a buffer |
| `pass NAME [scale K] = blur(SRC, RADIUS);` | A Gaussian blur of an earlier pass (radius in logical pixels) |
| `pass NAME [scale K] = bloom(SRC, THRESHOLD, RADIUS, STRENGTH);` | Bright parts blurred and added back (three stages) |
| `particles NAME [over SRC] (OPTIONS) { … }` | A particle emitter writing a buffer |
| `output NAME;` | Which pass is the picture (default: the last) |

A pass's `scale` is relative to the effect's: `pass glow scale 0.5` at `scale 0.5` draws at
a quarter of the element's resolution. Later passes read earlier ones with `sample(name, uv)`.
`last` is the effect's previous output frame (feedback: trails, ripples, smoke); the first
frame reads transparent black.

### Particles

```rfx
particles sparks over flame (count: 120, life: 2.5, splat: soft, blend: plus) {
    pos = vec2(rand(1.0) * size.x, size.y * (1.0 - progress));   // logical pixels
    radius = mix(2.0, 0.5, progress);                             // logical pixels
    color = vec4(1.0, 0.6, 0.2, 1.0 - progress);                  // straight alpha
    trail = vec2(0.0, 8.0);                                       // drawn as a capsule to pos + trail
}
```

Particles are **stateless**: each is a pure function of its number, the time and its
respawn, so pausing, scrubbing and resuming cost nothing and never drift. The program runs
once per particle per frame. Options: `count` (constant, at most 8192, 2048 untrusted; light
mode draws half), `life` (seconds, may use parameters and time), `splat` (`soft`, `disc`,
`ring`), `blend` (`plus`, `normal`, `screen`). `over` draws them over an earlier pass.

Inputs in a particle program: `id`, `seed` (random, new each respawn), `age`, `life`,
`progress` (age / life), `count`, `time`, `size`, `aspect`, `light`, and `rand(k)`: a random
number 0–1 for this particle's respawn, a different one for each `k`.

## Statements

| Statement | Meaning |
|---|---|
| `let x = EXPR;` | A name for a value |
| `var x = EXPR;` | A variable; `x = …`, `x += …`, `-=`, `*=`, `/=` change it |
| `if COND { … } else { … }` | Both branches run and every variable they change is selected per pixel: no pixel waits on another, and the cost does not depend on the data |
| `for i in A..B { … }` | Whole-number constant bounds; unrolled at compile time (at most 256 iterations a loop, 4096 in all; 128 and 1024 untrusted) |
| `return EXPR;` | The last statement of a function |

A pass must set `out`: a `vec4` colour (straight alpha), a `vec3` (opaque) or a `float`
(grey).

## Expressions and types

Types: `float`, `vec2`, `vec3`, `vec4` (also `color`), `mat3`, `mat4`. Numbers and vectors
combine component-wise; a `float` combines with any vector. Operators: `+ - * / %`,
comparisons `< <= > >= == !=` (1.0 or 0.0), `&& || !`, `c ? a : b` (both sides evaluated).
Swizzles: `.x .y .z .w` or `.r .g .b .a`, up to four (`v.xy`, `c.rgb`, `p.yx`). Constructors:
`vec2(…)`, `vec3(…)`, `vec4(…)`, `float(…)`, `mat3(c0, c1, c2)`, `mat4(c0, c1, c2, c3)` (columns),
and tuples `(x, y)`. `M * v`, `v * M`, `M * M`. `pi` and `tau` are constants.

Every operation is total: dividing by zero gives 0, square roots and logarithms of
negatives are taken at 0, and no result is NaN or infinite, so a hostile program cannot
poison a picture.

### Inputs (passes)

| Name | Type | Meaning |
|---|---|---|
| `uv` | vec2 | Position in the effect, 0–1 on each axis, at pixel centres |
| `px` | vec2 | Position in logical pixels |
| `size` | vec2 | The element's size in logical pixels |
| `aspect` | float | Width / height |
| `time` | float | Effect time in seconds (wraps every hour); the frame's time on the window's frame clock |
| `texel` | float | Logical pixels per buffer pixel of this pass (for one-pixel-wide work) |
| `light` | float | 1 when the window asks for lighter drawing (Light mode, power saving), else 0 |

Locals may reuse these names (`let light = …`).

## Built-in functions

**Maths** (component-wise): `abs sign floor ceil fract round trunc sqrt exp exp2 log log2 sin
cos tan asin acos atan atan2(y, x) min max pow mod step(edge, x) clamp saturate mix lerp
smoothstep fma select(if_false, if_true, cond) radians degrees inverse_lerp(a, b, x)
remap(x, a, b, c, d)`. `pow(x, n)` with a negative `x` and an odd whole `n` is negative.

**Vectors**: `dot length distance normalize cross reflect refract(i, n, eta) transpose`.

**Noise** (results 0–1, deterministic, identical on both renderers): `hash(p)` (a random
number per whole-number cell of `p`), `hash2 hash3 hash4` (several), `noise(p)` (value noise,
vec2 or vec3), `perlin(p)` (gradient noise, vec2 or vec3), `simplex(p)` (vec2), `voronoi(p)`
(vec2 → vec3: distance to the nearest and second-nearest point, and the nearest cell's random
number), `fbm(p, octaves[, gain[, lacunarity]])`, `ridged(…)`, `turbulence(…)` (octaves a
constant 1–8).

**Pictures**: `sample(source, uv)` reads a texture parameter (repeating), an earlier pass or
`last` (clamped at the edges) with bilinear filtering, as straight alpha.
`height_normal(source, uv, strength)` is a surface normal from a picture's brightness. Particle
programs cannot sample.

**Colour**: `luma(c)`, `hsv(h, s, v)`, `palette(t, a, b, c, d)` (cosine palette),
`ramp3(t, a, b, c)`, `ramp4(t, a, b, c, d)`, `over(top, bottom)`, `screen(a, b)`,
`tonemap(c)`, `aces(c)`, `vignette(uv, amount)`.

**2D shapes** (signed distances, negative inside): `sd_circle sd_box sd_round_box sd_segment
sd_ring sd_triangle sd_star`, `smin smax` (smooth union), `fill(d)` (anti-aliased coverage
over one buffer pixel), `stroke(d, width)`, `glow_of(d, radius)`.

**2D space**: `rotate(p, angle)`, `polar(p)`, `from_polar(r, a)`, `centered(uv)` (origin in
the middle, aspect-corrected), `tile(p, cell)`, `barrel(uv, k)` (CRT curvature),
`warp(p, amount, octaves)` (domain warping).

**3D**: shapes `sd_sphere sd_box3 sd_round_box3 sd_torus sd_capsule sd_cylinder sd_plane
sd_octahedron`; space `rotate_x rotate_y rotate_z repeat3 twist`; cameras
`look_at(eye, target, up) -> mat3`, `camera_ray(uv, eye, target, fov) -> vec3`,
`project(point, eye, target, fov) -> vec3` (screen uv and depth).

**Lighting**: `lambert(n, l)`, `blinn(n, l, v, shininess)`, `fresnel(cos, f0)`,
`ggx(n, l, v, roughness, f0)` (Cook–Torrance), `rim(n, v, power)`,
`fog(color, fog_color, distance, density)`, `sky(dir, horizon, zenith)`.

### 3D in practice

Raymarch a distance field with a fixed number of steps (no early exit: every pixel does the
same work, so the cost is known):

```rfx
fn map(p: vec3) -> float { return smin(sd_sphere(p, 1.0), sd_torus(rotate_x(p, time), 1.4, 0.2), 0.3); }

pass scene {
    let eye = vec3(0.0, 0.5, -3.5);
    let rd = camera_ray(uv, eye, vec3(0.0), 0.9);
    var t = 0.0;
    for i in 0..48 { t += map(eye + rd * t); }
    let p = eye + rd * t;
    let e = 0.003;
    let n = normalize(vec3(map(p + vec3(e, 0.0, 0.0)) - map(p - vec3(e, 0.0, 0.0)),
                           map(p + vec3(0.0, e, 0.0)) - map(p - vec3(0.0, e, 0.0)),
                           map(p + vec3(0.0, 0.0, e)) - map(p - vec3(0.0, 0.0, e))));
    let hit = step(map(p), 0.01);
    out = vec3(lambert(n, normalize(vec3(-0.5, 0.8, -0.4)))) * hit;
}
```

`orb.rfx`, `crystal.rfx` and `volumetric_clouds.rfx` show shadows, ambient occlusion, GGX,
fog and volumes.

## Limits and cost

Every program is checked when it is compiled. A source over a limit is refused with the
reason and where.

| Limit | Apps (`Limits::default`) | Strangers (`Limits::untrusted`) |
|---|---|---|
| Source size / tokens | 256 KiB / 100,000 | 64 KiB / 20,000 |
| Nesting depth / function call depth | 64 / 16 | 48 / 8 |
| Loop iterations (one loop / in all) | 256 / 4096 | 128 / 1024 |
| Instructions a program | 8192 | 4096 |
| Registers a program | 64 | 48 |
| Cost a pass, per pixel | 40,000 | 16,000 |
| **Cost of the effect, per element pixel** | 20,000 | 2,500 |
| Passes (bloom counts three) | 16 | 10 |
| Parameters / textures | 32 / 4 | 16 / 4 |
| Particles an emitter / particle work a frame | 8192 / 2,000,000 | 2048 / 400,000 |
| Frames a second | 60 | 30 |
| Earlier passes one pass samples | 8 | 8 |

**Cost** is the sum of the instruction weights a pixel runs (arithmetic 1, division 2,
trigonometry 4–6, a texture sample 10, value noise 14, gradient noise 18–36, cellular noise
60), after inlining and unrolling. A pass's cost is weighted by its resolution squared: a pass
at half resolution costs a quarter. The effect's cost (`Effect::cost`) is per element pixel at
full resolution, so heavy effects stay affordable by drawing at a lower `scale`. The built-in
effects all fit the untrusted limits.

## What the compiler does

- Inlines functions, unrolls loops and turns `if` into selects: every pixel runs the same
  straight-line code.
- Folds constants and merges repeated work (`noise(uv * 4.0)` written twice is computed once).
- **Hoists** what only changes once a frame (parameters, `time`, `size` and anything made only
  from them) out of per-pixel code into one per-frame program run on the CPU.
- Allocates registers, reusing them as values die.
- Refuses what is over the limits, with `line:column: what`. A misspelled function, name,
  parameter, pass or type gets a suggestion (`` `fmb` is not a function; did you mean `fbm`? ``);
  an effect over its cost budget is reported at its costliest pass, naming the passes that
  cost most. `CompileError::report(source)` draws each problem under its line:

  ```text
  error: `fmb` is not a function; did you mean `fbm`?
   --> 3:20
    |
  3 |         out = vec4(fmb(uv, 3));
    |                    ^
  ```

## Safety

- Effects are data. Themes and scripts never supply GPU code; the GPU runs one fixed kernel
  RemedyUI ships and tests, which interprets the program and stops after a fixed number of
  instructions whatever the program says.
- Costs are bounded before anything runs, so an effect cannot hang the GPU (no unbounded
  loops) or stall the CPU.
- An effect samples only its own textures, its own passes and its own last frame: never other
  windows, elements, messages or the screen. Nothing it computes flows back to the theme or
  script, and time reaches it only as the coarse frame time, so it has no timing side channel
  to read what is on screen.
- Parameter values are clamped; all arithmetic is total.

## Both renderers

The GPU kernel and the CPU interpreter implement one specification; noise and hashing are
integer-exact. The test suite holds every instruction family and every built-in effect on the
GPU to the CPU reference (`crates/remedy-ui-engine/tests/render_gpu/effects.rs`), and the CPU to golden
images (`crates/remedy-ui-engine/tests/effects/golden.rs`).
