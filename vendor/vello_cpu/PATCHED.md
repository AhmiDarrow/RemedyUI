# vello_cpu 0.4.0, patched for RemedyUI

This is the published `vello_cpu` 0.4.0 crate (Apache-2.0 OR MIT; see the license files) with
one fix, used through `[patch.crates-io]` in the workspace `Cargo.toml`. The crate's examples,
changelog and lockfile are left out; nothing else differs from the published crate.

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
`crates/remedy-ui-render-gpu/tests/filters.rs`. (The published crate's own tests need assets
it does not ship, so they are not run here.)

Drop this directory and the `[patch.crates-io]` entry once a vello_cpu release carries a fix.
