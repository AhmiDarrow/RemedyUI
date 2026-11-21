# remedy-vello-cpu: RemedyUI's copy of vello_cpu 0.4.0

This is the published `vello_cpu` 0.4.0 crate (Apache-2.0 OR MIT; see the license files, and
the upstream project at https://github.com/linebender/vello) with RemedyUI's fixes, published
under its own name, `remedy-vello-cpu`. Its library is still called `vello_cpu`, so code uses it
unchanged; `remedy-ui-render-cpu` depends on it by that name. The crate's examples, changelog
and lockfile are left out; the changes below are the only other differences.

## The fix

`src/filter/shift.rs`, `process_offset_pixel`: pixels shifted out of a filter layer's pixmap
must leave the exposed area transparent. The bound for a negative shift was computed as
`(width as i32 + dx_pixels) as u16`; when the shift is at least as large as the pixmap, that is
negative and the cast wrapped it to a huge value, so nothing was cleared and the "shifted"
pixmap kept its unshifted pixels. Filter pixmaps are cropped at the canvas's top and left
edge, so any drop shadow thrown wholly off that edge was drawn as if it had no offset at all
(vello_gpu draws nothing there, correctly). The comparison is now done in `i64`.

Covered by `shadows_thrown_off_the_top_or_left_edge_are_not_drawn` in
`crates/remedy-ui-render-cpu/tests/` and by
`shadows_thrown_off_the_top_or_left_edge_are_not_drawn_on_both_renderers` in
`crates/remedy-ui-render-gpu/tests/filters.rs`.

## The test font

The published crate's `glyph_atlas_resources_are_lazy` test (`src/render.rs`) includes a font
the crate does not ship, so with default features its tests did not compile. The font is now
in `assets/roboto/`: `Roboto-Regular.ttf` and its `LICENSE.txt` (Apache-2.0), byte-for-byte the
files upstream vello's `assets/roboto/` holds at the `sparse-strips-v0.4.0` tag. SHA-256:

- `Roboto-Regular.ttf`: `319cff6e7a31f0f2a41c475dca42890aa5d19fe16017e2290f8c1d4e14f76481`
- `LICENSE.txt`: `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`

Upstream keeps `assets/` at the repository root,
two directories above `src/`; here it sits beside `src/`, so the test's `include_bytes!` path
is `../assets/…` instead of `../../assets/…`. The font is only read by that test; the library
does not use it. The crate's tests run with
`cargo test --manifest-path vendor/vello_cpu/Cargo.toml`.

Upstream changes are taken in by hand, when RemedyUI chooses, with the fixes kept.
