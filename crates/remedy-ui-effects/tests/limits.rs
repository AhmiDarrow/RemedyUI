//! The render-time budgets ([`set_max_effect_pixels`], [`set_max_overdraw`]) and the fixed GPU
//! binding counts. One test changes the budgets, so no other test in this binary sees them
//! changed.

use remedy_ui_effects::{
    Effect, FrameInput, Limits, MAX_EFFECT_PIXELS, MAX_OVERDRAW, MAX_TEXTURES, Sprite, buffer_size,
    cap_overdraw, fit, max_effect_pixels, max_overdraw, set_max_effect_pixels, set_max_overdraw,
};

fn pixels(e: &Effect, input: &FrameInput) -> u64 {
    let fitted = fit(e, input);
    e.stages
        .iter()
        .map(|s| {
            let (w, h) = buffer_size(e, s, &fitted);
            u64::from(w) * u64::from(h)
        })
        .sum()
}

#[test]
fn the_effect_budgets_follow_their_setters() {
    let Ok(e) = Effect::compile("effect big { scale 1.0; pass p { out = 1.0; } }") else { unreachable!() };
    let mut input = FrameInput::new(4000.0, 4000.0, 1.0, 0.0);
    input.max_side = 1 << 20;
    assert_eq!((max_effect_pixels(), max_overdraw()), (MAX_EFFECT_PIXELS, MAX_OVERDRAW));
    assert!(pixels(&e, &input) <= MAX_EFFECT_PIXELS);
    set_max_effect_pixels(16_000_000);
    assert_eq!(pixels(&e, &input), 16_000_000, "the whole element at full resolution");
    set_max_effect_pixels(10_000);
    assert!(pixels(&e, &input) <= 10_000);

    let sprite = Sprite { pos: [0.0; 2], radius: 10.0, color: [1.0; 4], trail: [0.0; 2] };
    let kept = |n: usize| {
        let mut s = vec![sprite; n];
        cap_overdraw(&mut s, 1.0, 100_000);
        s.len()
    };
    let before = kept(100_000);
    set_max_overdraw(MAX_OVERDRAW * 4);
    assert!(kept(100_000) > before * 3, "four times the overdraw keeps about four times the particles");

    set_max_effect_pixels(MAX_EFFECT_PIXELS);
    set_max_overdraw(MAX_OVERDRAW);
}

#[test]
fn texture_parameters_stop_at_the_gpu_bindings() {
    let src = |n: usize| {
        let params: Vec<String> = (0..n).map(|i| format!("texture t{i}")).collect();
        format!("effect t({}) {{ scale 1.0; pass p {{ out = 1.0; }} }}", params.join(", "))
    };
    let mut limits = Limits::default();
    limits.max_textures = 64;
    assert!(Effect::compile_with(&src(MAX_TEXTURES), &limits).is_ok());
    assert!(Effect::compile_with(&src(MAX_TEXTURES + 1), &limits).is_err());
}
