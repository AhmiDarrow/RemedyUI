//! Contrast balancing.

use remedy_ui_core::Color;

/// WCAG 2 AA contrast for normal text.
pub const CONTRAST_AA: f32 = 4.5;
/// WCAG 2 AAA contrast for normal text.
pub const CONTRAST_AAA: f32 = 7.0;

/// `fg` moved in Oklab lightness, as little as needed, until it has at least `ratio` contrast
/// against the opaque `bg`. Hue and chroma are kept, so a theme's colors keep their
/// character. If no lightness reaches the ratio (an impossible request), the best found is
/// returned: black or white, whichever contrasts more.
pub fn ensure_contrast(fg: Color, bg: Color, ratio: f32) -> Color {
    let opaque = fg.over(bg);
    if opaque.contrast(bg) >= ratio {
        return fg;
    }
    let [l, a, b, alpha] = fg.to_oklab();
    let towards_light = bg.luminance() < 0.18;
    // Search the lightness between the color's own and the extreme in that direction.
    let (mut lo, mut hi) = if towards_light { (l, 1.0f32) } else { (0.0f32, l) };
    let target_ok = |lightness: f32| Color::from_oklab(lightness, a, b, alpha).over(bg).contrast(bg) >= ratio;
    let extreme = if towards_light { hi } else { lo };
    if !target_ok(extreme) {
        let (black, white) = (Color::BLACK, Color::WHITE);
        return if black.contrast(bg) >= white.contrast(bg) { black } else { white };
    }
    for _ in 0..32 {
        let mid = (lo + hi) / 2.0;
        let ok = target_ok(mid);
        // Move the boundary that keeps the answer as close to the original lightness as possible.
        match (towards_light, ok) {
            (true, true) => hi = mid,
            (true, false) => lo = mid,
            (false, true) => lo = mid,
            (false, false) => hi = mid,
        }
    }
    Color::from_oklab(if towards_light { hi } else { lo }, a, b, alpha)
}
