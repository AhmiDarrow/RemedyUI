# 16. Effects: Prism

Prism is RemedyUI's effects system: fire, water, caustics, glow and bloom, light shafts,
particles, starfields, grain, CRT screens, raymarched 3D objects and volumes, described in
`.rfx` and drawn by RemedyUI's own kernels. The language reference is
[docs/effects/rfx.md](../effects/rfx.md); this document is the design.

## Goals

1. **As capable as a game engine's 2D and procedural effects.** Multi-pass graphs at their
   own resolutions, feedback, particles with trails, signed-distance shapes, a noise library,
   cameras, lighting (GGX), raymarched 3D scenes and volumes.
2. **Safe for strangers.** People's themes and RemedyLua scripts use all of it. None of it is
   shader code from them.
3. **Cheap by construction.** Effects render at a fraction of the element's resolution, at
   their own frame rate under the window's ambient policy, only while seen.
4. **One picture on every renderer.** The GPU kernel and the CPU interpreter follow one
   specification and are tested against each other.
5. **Parity.** Built-in themes use the same language and limits as people's themes.

## Pieces

| Crate | Layer | What |
|---|---|---|
| `remedy-ui-engine::effects` | 3 | The language (lexer, parser), the compiler, the IR, the limits and cost model, the CPU interpreter and renderer, the frame logic both renderers share (per-frame values, particle simulation, buffer sizes), the built-in library |
| `remedy-ui-engine::render_gpu::effects` | 4 | `GpuEffect`: the kernel (one WGSL interpreter), the blur and particle pipelines, buffers per effect |
| `remedy-ui-engine::media_effects` | 5 | `effect_view`: the element. Picks the renderer, ticks under the ambient policy, keeps the still frame, runs the CPU quality ladder |
| `remedy-ui` | 6 | `remedy_ui::effects` (feature `effects`, in `full`) |

## The pipeline

```text
.rfx ──lex/parse──▶ syntax tree ──compile──▶ value graph (hash-consed, folded)
     ──codegen──▶ Effect { stages: [Shade(Program) | Blur | Particles], uniforms: Program }
     ──per frame──▶ per-frame values (CPU, once) ─▶ each stage on the GPU kernel or the CPU
```

- **Values** are four floats; scalars are splatted across the lanes, so component-wise
  instructions broadcast for free. Types (float, vec2–4, mat3/4) exist only in the compiler.
- **Programs** are straight-line code for a register machine (64 registers by default,
  `Limits::max_registers`, up to 255). The
  instructions are the kernel library: arithmetic, transcendental functions, comparisons and
  selects, swizzle and combine, hashing, value/gradient/simplex/cellular noise (2D and 3D),
  sampling. There are no jumps, so a program's cost is a sum known at compile time and does
  not depend on the data it reads.
- **The compiler** inlines functions, unrolls loops, flattens `if` into selects, folds
  constants, shares common subexpressions (the graph is hash-consed), hoists per-frame values
  (made only of parameters, time and size) into one program run once a frame on the CPU, removes
  dead code and allocates registers. Higher-level functions (fbm, shapes, cameras, lighting)
  lower to instructions; many are written in `.rfx` itself (the prelude).
- **Diagnostics** are located (`line:column`) and say what to do: a misspelled function,
  name, parameter, pass or type gets "did you mean" when one is a keystroke or two away, and
  an effect over its cost budget is reported at its costliest pass with the passes that cost
  most. `CompileError::report(source)` draws each problem under its line with a caret, for
  theme editors, the devtools and command lines.
- **Stages**: a shade stage runs a program for every pixel of its buffer; a blur is a
  separable Gaussian (two passes, up to 32 taps a side); bloom lowers to a bright pass at half
  resolution, a blur and a combine; particles are simulated on the CPU (the program runs once
  per particle) and splatted as capsules.

## The GPU kernel

One fragment shader interprets every program. The program's instructions live in an
`Rgba32Uint` texture and its constants in an `Rgba32Float` texture, so programs of thousands
of instructions run on every backend, GL 3.3 and WebGL2 included (no storage buffers needed).
The kernel runs exactly the program's instructions, as many as the effect's `Limits` let it
have (the CPU interpreter runs the same). Its bindings are the effect's four textures
(`MAX_TEXTURES`), eight of its earlier passes (`MAX_SAMPLED_PASSES`, mapped per pass) and its
last frame; nothing else. With the code and constant textures that is 15 sampled textures,
and WebGPU guarantees 16 to one shader stage, so these two counts are fixed by the GPU API,
not settings. Its register file is a WGSL private array of 64 (a size fixed when the shader
is built; a larger one costs every pixel of every effect); a program needing more, up to the
encoding's 255, runs on a wide form with 256, built the first time one does. Pipelines are
otherwise made once per device; loading a theme or changing an effect compiles nothing. Effects render into buffers the element owns, copied into one of two
registered textures the display list places (`Item::Texture`).

Every eighth frame an effect brackets its passes with two timestamp queries (a query set kept
per effect) and reads them back once the GPU is done, never waiting for it
(`GpuEffect::take_gpu_time`). Devices without timestamp queries or compute passes (GL 3.3,
WebGL2) measure nothing and pay nothing. A window snapshot of a GPU window reads back the
frame the compositor keeps, so it shows the effects ([04](04-rendering.md)).

## The CPU path

The interpreter runs one instruction over 64 pixels at a time (component-wise instructions
are one tight loop per opcode, which the compiler vectorizes) and splits large buffers across
threads (bit-identical to one thread). Every instruction is total.

## Quality ladders

On a window the CPU renderer draws, the element measures each frame and steps down a ladder
when over its budget (`set_cpu_budget`, 4 ms by default): half resolution, a quarter at no more
than 15 fps, then the still frame; it climbs back when frames are cheap. On the GPU it holds
the sampled GPU time to `set_gpu_budget` (2 ms by default): half resolution, then a quarter at
no more than 30 fps, and never the still frame; it climbs back after a run of cheap measures.
Where the device takes no timestamps the GPU path relies on the static cost and the effect's
scale. `EffectStats` reports the step and the latest GPU time.

## Safety

- **No code from strangers reaches the GPU.** The kernel is ours, written and tested once.
- **Bounded cost before anything runs.** Limits on size, nesting, inlining, loop
  iterations, instructions, registers, cost per pixel and per effect, passes, particles and
  frame rate are checked at compile time ([rfx.md](../effects/rfx.md#limits-and-cost));
  `Limits::untrusted` holds people's themes and scripts to about a few octaves of noise at full
  resolution, or a raymarched scene at reduced scale. The red-team tests compile hostile
  sources (deep nesting, exponential inlining, recursion, nested and huge loops, huge sources,
  too many of everything, garbage bytes) and require each to fail within milliseconds with a
  located reason.
- **Bounded cost while it runs**, on both renderers alike: all of one effect's buffers hold
  at most `MAX_EFFECT_PIXELS` (4 Mi) pixels, a larger element drawing its effect at a lower
  resolution, and particles cover at most `MAX_OVERDRAW` (8) times their buffer's pixels a
  frame. Both are defaults: `set_max_effect_pixels` and `set_max_overdraw` change them for
  the process.
- **No side channels.** Effects sample only their own inputs (filters that read other pixels
  leak them through timing), programs have no
  data-dependent control flow, and nothing flows back to the theme: no readback, no timing,
  only coarse frame time in.
- **Total arithmetic**: no NaN or infinity reaches a pixel; parameters are clamped to their
  ranges.
- **Filters on element content are not Prism programs.** The blur, drop-shadow, colour-matrix
  (saturate, hue-rotate, grayscale and the rest) and backdrop filters a style can ask for
  ([04](04-rendering.md)) are fixed engine operations: their cost depends on the size of what
  they filter, never on its pixels, and nothing they compute flows
  back to the theme, so reading what is behind an element opens no timing channel. Prism
  programs still sample only their own inputs.

## Frames and budgets

An effect element repaints at `min(effect fps, ambient rate)` while seen, in its own
compositor layer, so a frame costs the effect and nothing around it. In Light mode and while
saving power it draws light (`light` = 1, half the particles). Paused, Still, reduced motion
and hidden show the still frame (`still`), drawn once per size and parameter change.

Costs measured (release, 4-core CI container; the GPU is llvmpipe, a software rasterizer, so
GPU numbers are an upper bound):

- The CPU interpreter: the 30 original built-in effects take 0.3–30 ms a frame at 480 × 320 logical
  pixels at their own scales; raymarched 3D (orb, crystal, volumetric clouds) are the GPU's,
  and step down on the CPU.
- The eight added later (shimmer, mesh gradient, confetti, smoke, galaxy, code rain, frost,
  retro grid) take 0.4–12 ms a frame at 320 × 200 logical pixels on the CPU (release, the
  same container; frost, galaxy and code rain are the costliest).
- GPU and CPU agree: every instruction family bit for bit, every built-in effect within half a
  level on average.

## Markup

`.rui` markup has the `effect` element (`remedy_ui::effects::register_markup`: a built-in
effect by name, one parameter driven by a number signal, children drawn over it); markup offers
only built-in effects, never effect source. 3D beyond effects: [17-3d.md](17-3d.md).
