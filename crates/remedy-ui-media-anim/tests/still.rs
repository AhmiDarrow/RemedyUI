//! An animated file given to the still-image loader shows its first frame, as the animation
//! element's poster does.

mod common;

use common::*;
use remedy_ui_media_image::{decode, dimensions};

fn pixel(img: &remedy_ui_engine::paint::Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    img.pixels.get(i..i + 4).and_then(|s| s.try_into().ok()).unwrap_or([9; 4])
}

fn files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        // The first frame covers only part of the picture, and the second is the whole.
        (
            "gif",
            gif(
                4,
                4,
                None,
                &[
                    GifFrame::part((1, 1, 2, 2), GREEN, 10, ::gif::DisposalMethod::Keep),
                    GifFrame::solid(4, 4, BLUE, 10),
                ],
            ),
        ),
        // A fallback image that is not a frame, then a first frame covering part.
        (
            "apng",
            apng(
                4,
                4,
                0,
                Some([255, 255, 0, 255]),
                &[
                    ApngFrame::part(
                        (1, 1, 2, 2),
                        [0, 255, 0, 255],
                        100,
                        ::png::DisposeOp::None,
                        ::png::BlendOp::Source,
                    ),
                    ApngFrame::solid(4, 4, B, 100),
                ],
            ),
        ),
        (
            "webp",
            webp(
                4,
                4,
                0,
                &[WebpFrame::part((0, 0, 2, 2), G, 100, false, false), WebpFrame::solid(4, 4, B, 100)],
            ),
        ),
    ]
}

#[test]
fn an_animated_file_in_the_still_loader_shows_its_first_frame_at_the_picture_size() {
    remedy_ui_media_anim::register();
    for (name, bytes) in files() {
        assert_eq!(dimensions(&bytes).ok(), Some((4, 4)), "{name}");
        let d = decode(&bytes, None).unwrap_or_else(|e| unreachable!("{name}: {e}"));
        assert_eq!((d.image.width, d.image.height, d.source_size), (4, 4, (4, 4)), "{name}");
        // The first frame, drawn on a transparent picture: not the fallback and not frame two.
        let inside = pixel(&d.image, 1, 1);
        assert_eq!(inside, G, "{name}: {inside:?}");
        let corner = pixel(&d.image, 3, 3);
        assert_eq!(corner, [0, 0, 0, 0], "{name}: the rest of the picture is clear");
        let top = pixel(&d.image, 0, 0);
        assert_eq!(top, if name == "webp" { G } else { [0, 0, 0, 0] }, "{name}");
        // And scaled down while decoding.
        let small = decode(&bytes, Some((2, 2))).unwrap_or_else(|e| unreachable!("{name}: {e}"));
        assert_eq!((small.image.width, small.image.height), (2, 2), "{name}");
    }
}
