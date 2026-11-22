//! [`set_fx_limits`] moves the noise size and the texture caches. A binary of its own, so no
//! other test sees them changed.

use remedy_ui_core::Color;
use remedy_ui_widgets_stage::fx::{FX_KEPT, FX_NOISE_SIDE, NoiseSpec, noise_image, set_fx_limits};

#[test]
fn the_app_sets_the_noise_size_and_what_is_kept() {
    let small = NoiseSpec::new(16, 16, 2, Color::WHITE);
    let first = noise_image(&small).map(|i| i.id);
    assert_eq!(noise_image(&small).map(|i| i.id), first, "kept and shared");
    assert!(noise_image(&NoiseSpec::new(FX_NOISE_SIDE + 1, 8, 2, Color::WHITE)).is_none());
    set_fx_limits(0, 8);
    assert!(noise_image(&NoiseSpec::new(16, 8, 2, Color::WHITE)).is_none(), "past the side");
    let tiny = NoiseSpec::new(8, 8, 2, Color::WHITE);
    let a = noise_image(&tiny).map(|i| i.id);
    assert!(a.is_some() && noise_image(&tiny).map(|i| i.id) != a, "none kept: made again");
    set_fx_limits(FX_KEPT, FX_NOISE_SIDE);
}
