//! Pictures that stay on the GPU (Windows: Media Foundation on a D3D11 device, shared with the
//! renderer's D3D12 device) show the same colours at the same times as pictures copied
//! through CPU memory, through a seek, and never pass through CPU memory on the way.
//!
//! A test binary of its own: which GPU takes pictures is process-wide, and the other playback
//! tests read their pictures' bytes.
#![cfg(windows)]

use std::time::Duration;

use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_media_playback::*;
use remedy_ui_paint::{DisplayList, Item, VideoFrame};
use remedy_ui_render_gpu::{GpuContext, GpuRenderer, LadderOptions};

fn clip() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mp4")
}

fn frame(id: u64, p: &Picture) -> VideoFrame {
    let mut f = VideoFrame::packed(1, id, p.width, p.height, p.format, p.data.clone());
    f.planes = p.planes;
    f.matrix = p.matrix;
    f.full_range = p.full_range;
    f.gpu = p.gpu.clone();
    f
}

/// Every picture of the clip from the start, then from a seek to 1.5 s: its time and frame.
fn pictures(host: &dyn MediaHost) -> Vec<(Duration, VideoFrame)> {
    let mut dec = host.open(&MediaSource::file(clip())).unwrap_or_else(|e| unreachable!("open: {e}"));
    let mut out = Vec::new();
    let read = |dec: &mut Box<dyn Decoder>, out: &mut Vec<(Duration, VideoFrame)>, most: usize| {
        let mut n = 0;
        while n < most {
            match dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
                Some(Sample::Video { at, picture, .. }) => {
                    out.push((at, frame(out.len() as u64, &picture)));
                    n += 1;
                }
                Some(_) => {}
                None => break,
            }
        }
    };
    read(&mut dec, &mut out, usize::MAX);
    dec.seek(Duration::from_millis(1500)).unwrap_or_else(|e| unreachable!("seek: {e}"));
    read(&mut dec, &mut out, 12);
    out
}

fn draw(g: &mut GpuRenderer, f: &VideoFrame) -> Vec<u8> {
    let mut list = DisplayList::new();
    let rect = Rect::new(0.0, 0.0, f.width as f32, f.height as f32);
    list.push(Item::Video { frame: f.clone(), rect, radii: CornerRadii::ZERO });
    match g.render(&list, f.width, f.height, 1.0, Color::BLACK) {
        Ok(out) => out.pixels,
        Err(e) => unreachable!("render: {e}"),
    }
}

fn worst(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

#[test]
fn pictures_on_the_gpu_match_pictures_in_cpu_memory() {
    let host = platform_host().unwrap_or_else(|e| unreachable!("Media Foundation: {e}"));
    // No renderer has said it takes pictures on the GPU yet: they come in CPU memory.
    assert_eq!(remedy_ui_paint::gpu_video_adapter(), None);
    let cpu = pictures(&*host);
    assert!(cpu.iter().all(|(_, f)| f.gpu.is_none() && !f.data.is_empty()));

    let ctx = GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}"));
    let mut g = GpuRenderer::new(ctx, None);
    if remedy_ui_paint::gpu_video_adapter().is_none() {
        eprintln!("this GPU cannot sample NV12 textures shared from D3D11, skipping");
        return;
    }
    let gpu = pictures(&*host);
    assert_eq!(gpu.len(), cpu.len(), "as many pictures");
    for ((at_c, c), (at_g, f)) in cpu.iter().zip(&gpu) {
        assert_eq!(at_c, at_g, "at the same times");
        assert_eq!((c.width, c.height), (f.width, f.height));
        // Stayed on the GPU: no bytes, so the sample's buffer was never locked and copied.
        assert!(f.gpu.is_some() && f.data.is_empty(), "the picture at {at_g:?} came through CPU memory");
        let (a, b) = (c.to_rgba().unwrap_or_default(), f.to_rgba().unwrap_or_default());
        assert!(worst(&a, &b) <= 2, "read back at {at_g:?}, the colours match");
        // Red for the first second, green for the second, blue for the third.
        let mid = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        let rgb = b.get(mid..mid + 3).unwrap_or_default();
        let strongest = (0..3).max_by_key(|&i| rgb.get(i).copied().unwrap_or(0));
        assert_eq!(strongest, Some((at_g.as_secs() as usize).min(2)), "at {at_g:?}: {rgb:?}");
    }
    // Drawn by the GPU renderer, sampled in place: the same pixels as the copied picture.
    let mut w = GpuRenderer::new(
        GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}")),
        None,
    );
    for i in [0, 30, 60, gpu.len() - 1] {
        let (Some((at, c)), Some((_, f))) = (cpu.get(i), gpu.get(i)) else { unreachable!("{i}") };
        let shown = draw(&mut g, f);
        // In place: only the converted picture is held, no plane textures to upload into.
        assert_eq!(g.image_bytes(), u64::from(f.width * f.height * 4), "at {at:?}");
        let copied = draw(&mut w, c);
        assert!(worst(&shown, &copied) <= 2, "at {at:?} the GPU shows the same colours");
    }
}
