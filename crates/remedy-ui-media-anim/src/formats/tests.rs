//! The decoders, frame by frame: pixels after disposal and blending, delays, loops.

use super::*;
use crate::testing::*;

/// Every frame of `bytes`: the picture as straight RGBA, and its delay.
fn all(bytes: Vec<u8>) -> (Info, Vec<(Vec<u8>, Duration)>) {
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let Ok(Opened::Animated(mut f)) = open(&bytes) else { unreachable!("not an animation") };
    let mut out = Vec::new();
    while let Some(fr) = f.next().unwrap_or_else(|e| unreachable!("{e}")) {
        out.push((fr.rgba.to_vec(), fr.delay));
    }
    let i = f.info();
    (Info { size: i.size, frames: i.frames, loops: i.loops, profile: None }, out)
}

/// The pixel at (x, y) of a `w`-wide picture.
fn at(f: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    f.get(i..i + 4).and_then(|s| s.try_into().ok()).unwrap_or([9; 4])
}

fn frame_at(f: &[(Vec<u8>, Duration)], n: usize, w: usize, x: usize, y: usize) -> [u8; 4] {
    f.get(n).map_or([9; 4], |(px, _)| at(px, w, x, y))
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 2)
}

fn millis_of(f: &[(Vec<u8>, Duration)]) -> Vec<u128> {
    f.iter().map(|f| f.1.as_millis()).collect()
}

#[test]
fn gif_frames_are_disposed_as_the_file_says() {
    use ::gif::DisposalMethod as D;
    // Dispose to background: the frame's area becomes transparent, not the frame before.
    let bytes = gif(
        4,
        4,
        None,
        &[
            GifFrame::solid(4, 4, RED, 10),
            GifFrame::part((1, 1, 2, 2), GREEN, 10, D::Background),
            GifFrame::part((0, 0, 1, 1), BLUE, 10, D::Keep),
        ],
    );
    let (info, f) = all(bytes);
    assert_eq!((info.size, info.frames), ((4, 4), Some(3)));
    assert_eq!(f.len(), 3);
    assert_eq!(frame_at(&f, 1, 4, 1, 1), rgba(GREEN));
    assert_eq!(frame_at(&f, 1, 4, 0, 0), rgba(RED));
    assert_eq!(frame_at(&f, 2, 4, 1, 1), [0, 0, 0, 0], "cleared, not back to red");
    assert_eq!(frame_at(&f, 2, 4, 0, 0), rgba(BLUE));
    assert_eq!(frame_at(&f, 2, 4, 3, 3), rgba(RED));

    // Restore previous: the area goes back to what it was before the frame.
    let bytes = gif(
        4,
        4,
        None,
        &[
            GifFrame::solid(4, 4, RED, 10),
            GifFrame::part((0, 0, 2, 2), GREEN, 10, D::Previous),
            GifFrame::part((3, 3, 1, 1), BLUE, 10, D::Keep),
        ],
    );
    let (_, f) = all(bytes);
    assert_eq!(frame_at(&f, 1, 4, 1, 1), rgba(GREEN));
    assert_eq!(frame_at(&f, 2, 4, 1, 1), rgba(RED), "restored");
    assert_eq!(frame_at(&f, 2, 4, 3, 3), rgba(BLUE));

    // Transparent pixels leave what is below.
    let mut over = GifFrame::solid(4, 4, GREEN, 10);
    over.px = (0..16).map(|i| if i % 2 == 0 { T } else { GREEN }).collect();
    let (_, f) = all(gif(4, 4, None, &[GifFrame::solid(4, 4, RED, 10), over]));
    assert_eq!(frame_at(&f, 1, 4, 0, 0), rgba(RED));
    assert_eq!(frame_at(&f, 1, 4, 1, 0), rgba(GREEN));
}

#[test]
fn gif_delays_follow_the_browser_rules_and_loops_count_repeats() {
    use ::gif::Repeat;
    let frames = |d: &[u16]| d.iter().map(|d| GifFrame::solid(2, 2, RED, *d)).collect::<Vec<_>>();
    let (info, f) = all(gif(2, 2, None, &frames(&[0, 1, 2, 5, 30])));
    // Under 20 ms plays as 100 ms; 20 ms and up are kept.
    assert_eq!(millis_of(&f), vec![100, 100, 20, 50, 300]);
    // No loop extension: once. Repeat n: the first play and n more.
    assert_eq!(info.loops, Loops::Times(1));
    let (info, _) = all(gif(2, 2, Some(Repeat::Finite(2)), &frames(&[5, 5])));
    assert_eq!(info.loops, Loops::Times(3));
    let (info, _) = all(gif(2, 2, Some(Repeat::Infinite), &frames(&[5, 5])));
    assert_eq!(info.loops, Loops::Forever);
}

#[test]
fn apng_frames_dispose_and_blend_as_the_file_says() {
    use ::png::BlendOp::{Over, Source};
    use ::png::DisposeOp as D;
    let half_green = [0, 255, 0, 128];
    let bytes = apng(
        4,
        4,
        3,
        None,
        &[
            ApngFrame::solid(4, 4, [255, 0, 0, 255], 40),
            // Over: mixed with the red below. Background: cleared after.
            ApngFrame::part((1, 1, 2, 2), half_green, 50, D::Background, Over),
            ApngFrame::part((3, 3, 1, 1), [0, 0, 255, 255], 25, D::None, Source),
            // Source: replaces, alpha and all. Previous: undone after.
            ApngFrame::part((0, 0, 2, 1), [255, 255, 255, 100], 60, D::Previous, Source),
            ApngFrame::part((3, 0, 1, 1), [0, 0, 255, 255], 70, D::None, Source),
        ],
    );
    let (info, f) = all(bytes);
    assert_eq!((info.size, info.frames, info.loops), ((4, 4), Some(5), Loops::Times(3)));
    assert_eq!(millis_of(&f), vec![40, 50, 25, 60, 70]);
    assert_eq!(frame_at(&f, 1, 4, 1, 1), [127, 128, 0, 255], "green over red");
    assert_eq!(frame_at(&f, 2, 4, 1, 1), [0, 0, 0, 0], "disposed to transparent");
    assert_eq!(frame_at(&f, 2, 4, 3, 3), [0, 0, 255, 255]);
    assert_eq!(frame_at(&f, 3, 4, 0, 0), [255, 255, 255, 100], "replaced, not mixed");
    assert_eq!(frame_at(&f, 4, 4, 0, 0), [255, 0, 0, 255], "restored to before the frame");
    assert_eq!(frame_at(&f, 4, 4, 3, 3), [0, 0, 255, 255]);
}

#[test]
fn apng_default_image_that_is_not_a_frame_is_skipped_and_delays_use_the_fraction() {
    let mut f = ApngFrame::solid(2, 2, [255, 0, 0, 255], 0);
    f.delay = (1, 4);
    let mut g = ApngFrame::solid(2, 2, [0, 255, 0, 255], 0);
    g.delay = (0, 10);
    let bytes = apng(2, 2, 0, Some([255, 255, 0, 255]), &[f, g]);
    let (info, frames) = all(bytes);
    assert_eq!((info.frames, info.loops), (Some(2), Loops::Forever));
    assert_eq!(frames.len(), 2, "the fallback image is not a frame");
    assert_eq!(frame_at(&frames, 0, 2, 0, 0), [255, 0, 0, 255]);
    assert_eq!(millis_of(&frames), vec![250, 100], "zero asks for as fast as possible");
}

#[test]
fn webp_frames_blend_and_dispose_as_the_file_says() {
    let half_green = [0, 255, 0, 128];
    let bytes = webp(
        4,
        4,
        2,
        &[
            WebpFrame::solid(4, 4, [255, 0, 0, 255], 30),
            WebpFrame::part((2, 2, 2, 2), half_green, 40, true, true),
            WebpFrame::part((0, 0, 2, 2), [0, 0, 255, 255], 50, false, false),
        ],
    );
    let (info, f) = all(bytes);
    assert_eq!((info.size, info.frames, info.loops), ((4, 4), Some(3), Loops::Times(2)));
    assert_eq!(millis_of(&f), vec![30, 40, 50]);
    assert!(near(frame_at(&f, 1, 4, 2, 2), [127, 128, 0, 255]), "{:?}", frame_at(&f, 1, 4, 2, 2));
    assert_eq!(frame_at(&f, 2, 4, 2, 2), [0, 0, 0, 0], "disposed to transparent, not to the file's hint");
    assert_eq!(frame_at(&f, 2, 4, 0, 0), [0, 0, 255, 255]);
    assert_eq!(frame_at(&f, 2, 4, 3, 0), [255, 0, 0, 255]);
}

#[test]
fn restarting_plays_the_same_frames_again() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let files = [
        gif(2, 2, None, &[GifFrame::solid(2, 2, RED, 5), GifFrame::solid(2, 2, GREEN, 5)]),
        apng(2, 2, 0, None, &[ApngFrame::solid(2, 2, red, 50), ApngFrame::solid(2, 2, green, 50)]),
        webp(2, 2, 0, &[WebpFrame::solid(2, 2, red, 50), WebpFrame::solid(2, 2, green, 50)]),
    ];
    for bytes in files {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let Ok(Opened::Animated(mut f)) = open(&bytes) else { unreachable!("not an animation") };
        let mut first = Vec::new();
        while let Some(fr) = f.next().unwrap_or_else(|e| unreachable!("{e}")) {
            first.push(at(fr.rgba, 2, 0, 0));
        }
        f.restart().unwrap_or_else(|e| unreachable!("{e}"));
        let mut second = Vec::new();
        while let Some(fr) = f.next().unwrap_or_else(|e| unreachable!("{e}")) {
            second.push(at(fr.rgba, 2, 0, 0));
        }
        assert_eq!(first, vec![red, green]);
        assert_eq!(first, second);
    }
}

#[test]
fn files_are_told_apart_by_their_headers() {
    let g = gif(2, 2, None, &[GifFrame::solid(2, 2, RED, 5)]);
    let a = apng(2, 2, 0, None, &[ApngFrame::solid(2, 2, [1, 2, 3, 255], 5)]);
    let w = webp(2, 2, 0, &[WebpFrame::solid(2, 2, [1, 2, 3, 255], 5)]);
    assert!(sniff(&g) == Kind::Gif && sniff(&a) == Kind::Apng && sniff(&w) == Kind::Webp);
    // A plain PNG, text and nothing are stills for the image loader to judge.
    assert!(sniff(&png_still(2, 2, [1, 2, 3, 255])) == Kind::Still);
    assert!(sniff(b"hello") == Kind::Still && sniff(b"") == Kind::Still);
    // A damaged animation is an error, not a still.
    let cut: Arc<[u8]> = Arc::from(g.get(..g.len() / 2).unwrap_or_default());
    assert!(matches!(open(&cut), Err(DecodeError::Malformed(_) | DecodeError::Io(_))));
}
