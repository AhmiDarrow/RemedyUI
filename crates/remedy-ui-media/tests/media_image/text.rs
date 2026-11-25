//! Text in SVG files draws with the fonts the app adds.

use remedy_ui_core::Color;
use remedy_ui_media::media_image::svg::{add_font, render};

const REPLAY_10: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path fill="none" stroke="currentColor" stroke-width="1.8" d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><text x="12" y="15.6" font-family="Remedy Sans" font-weight="700" font-size="6.5" text-anchor="middle">10</text></svg>"#;

#[test]
fn svg_text_draws_with_an_added_font() {
    for face in remedy_ui_fonts::all() {
        add_font(face.data.to_vec());
    }
    let img = render(REPLAY_10.as_bytes(), Some((48, 48)), Some(Color::rgb8(0, 0, 0)))
        .unwrap_or_else(|e| unreachable!("{e}"));
    // The "10" sits in x 9..15, y 11..16 of the 24-unit view box, clear of the arc; 48 px is 2x.
    let inked = (22..32)
        .flat_map(|y| (18..30).map(move |x| (x, y)))
        .filter(|&(x, y)| img.pixels.get(((y * 48 + x) * 4 + 3) as usize).is_some_and(|&a| a > 128))
        .count();
    assert!(inked > 10, "the text drew {inked} pixels");
}
