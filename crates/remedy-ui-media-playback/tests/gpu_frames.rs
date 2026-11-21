//! Pictures that stay on the GPU (Windows: Media Foundation on a D3D11 device, shared with the
//! renderer's D3D12 device; Android: MediaCodec into an image reader, its hardware buffers
//! imported into the renderer's Vulkan device) show the same colours at the same times as
//! pictures copied through CPU memory, through a seek, and never pass through CPU memory on the
//! way.
//!
//! A test binary of its own: which GPU takes pictures is process-wide, and the other playback
//! tests read their pictures' bytes.
#![cfg(any(windows, target_os = "android"))]

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

#[cfg(windows)]
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

/// Android: the pictures cannot be read back (an opaque format), and the reader lends only so
/// many at once, so each is drawn as it is decoded and compared with the copied one drawn.
#[cfg(target_os = "android")]
#[test]
fn pictures_on_the_gpu_match_pictures_in_cpu_memory() {
    let host = platform_host().unwrap_or_else(|e| unreachable!("MediaCodec: {e}"));
    // No renderer has said it takes pictures on the GPU yet: they come in CPU memory.
    assert_eq!(remedy_ui_paint::gpu_video_adapter(), None);
    let cpu = pictures(&*host);
    assert!(cpu.iter().all(|(_, f)| f.gpu.is_none() && !f.data.is_empty()));
    let new = || {
        GpuRenderer::new(
            GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}")),
            None,
        )
    };
    let mut w = new();
    let copied: Vec<Vec<u8>> = cpu.iter().map(|(_, c)| draw(&mut w, c)).collect();
    if remedy_ui_paint::gpu_video_adapter().is_none() {
        eprintln!("this GPU cannot import hardware buffers, skipping");
        return;
    }
    let mut g = new();
    let mut dec = host.open(&MediaSource::file(clip())).unwrap_or_else(|e| unreachable!("open: {e}"));
    let (mut n, mut last) = (0usize, None);
    let mut check = |at: Duration, p: &Picture, last: &mut Option<VideoFrame>| {
        let f = frame(1000 + n as u64, p);
        let Some((at_c, c)) = cpu.get(n) else { unreachable!("more pictures than copied ones") };
        assert_eq!(*at_c, at, "picture {n} at the same time");
        assert_eq!((c.width, c.height), (f.width, f.height));
        assert!(f.gpu.is_some() && f.data.is_empty(), "the picture at {at:?} came through CPU memory");
        let shown = draw(&mut g, &f);
        // In place: only the converted picture is held, no plane textures to upload into.
        assert_eq!(g.image_bytes(), u64::from(f.width * f.height * 4), "at {at:?}");
        let worst = worst(&shown, copied.get(n).map_or(&[][..], |v| v));
        assert!(worst <= 2, "at {at:?} the GPU shows the same colours (off by {worst})");
        // Held until the next one shows, as a video surface holds its current frame.
        *last = Some(f);
        n += 1;
    };
    let mut read = |dec: &mut Box<dyn Decoder>, most: usize, last: &mut Option<VideoFrame>| {
        let mut k = 0;
        while k < most {
            match dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
                Some(Sample::Video { at, picture, .. }) => {
                    check(at, &picture, last);
                    k += 1;
                }
                Some(_) => {}
                None => break,
            }
        }
    };
    read(&mut dec, usize::MAX, &mut last);
    dec.seek(Duration::from_millis(1500)).unwrap_or_else(|e| unreachable!("seek: {e}"));
    read(&mut dec, 12, &mut last);
    assert_eq!(n, cpu.len(), "as many pictures");
}

/// The process's CPU time so far, in milliseconds (all threads: Media Foundation decodes on
/// its own), and its private memory and peak working set in MB.
#[cfg(windows)]
fn process_usage() -> (f64, f64, f64) {
    let script = format!(
        "$p = Get-Process -Id {}; \"$($p.TotalProcessorTime.TotalMilliseconds) $($p.PrivateMemorySize64) $($p.PeakWorkingSet64)\"",
        std::process::id()
    );
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let v: Vec<f64> = text.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
    (at(0), at(1) / 1e6, at(2) / 1e6)
}

/// Per picture: process CPU time and wall time to decode it and draw it on the GPU (waiting for
/// the GPU), on the copy path or the GPU path. `REMEDY_BENCH_CLIP` names the clip and
/// `REMEDY_BENCH_PATH` the path (`copy` or `gpu`); one path per process, so memory is its own.
#[cfg(windows)]
#[test]
#[ignore = "a measurement: run by hand with a long clip"]
fn measure_a_clip() {
    let (Ok(clip), Ok(path)) = (std::env::var("REMEDY_BENCH_CLIP"), std::env::var("REMEDY_BENCH_PATH"))
    else {
        eprintln!("set REMEDY_BENCH_CLIP and REMEDY_BENCH_PATH");
        return;
    };
    let host = platform_host().unwrap_or_else(|e| unreachable!("Media Foundation: {e}"));
    let ctx = GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}"));
    let mut g = GpuRenderer::new(ctx, None);
    if path == "copy" {
        remedy_ui_paint::set_gpu_video_adapter(None);
    }
    let mut dec = host.open(&MediaSource::file(&clip)).unwrap_or_else(|e| unreachable!("open: {e}"));
    let v = dec.info().video.clone().unwrap_or_else(|| unreachable!("a picture"));
    let target = g.create_target(v.width, v.height).unwrap_or_else(|e| unreachable!("target: {e}"));
    let rect = Rect::new(0.0, 0.0, v.width as f32, v.height as f32);
    let (cpu0, _, _) = process_usage();
    let start = std::time::Instant::now();
    let (mut times, mut on_gpu) = (Vec::new(), 0usize);
    let mut shown: Option<VideoFrame> = None;
    while let Some(s) = dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
        let Sample::Video { picture, .. } = s else { continue };
        let t = std::time::Instant::now();
        on_gpu += usize::from(picture.gpu.is_some());
        let mut list = DisplayList::new();
        let f = frame(times.len() as u64, &picture);
        list.push(Item::Video { frame: f.clone(), rect, radii: CornerRadii::ZERO });
        g.draw(&list, remedy_ui_paint::Affine::IDENTITY, Color::BLACK, &target)
            .unwrap_or_else(|e| unreachable!("draw {}: {e}", times.len()));
        // Held until the next one shows, as a video surface holds its current frame.
        shown = Some(f);
        let _ = g.context().device.poll(remedy_ui_render_gpu::wgpu::PollType::wait_indefinitely());
        times.push((start.elapsed(), t.elapsed()));
    }
    drop(shown);
    let wall = start.elapsed();
    let (cpu1, private, peak) = process_usage();
    let n = times.len().max(1) as f64;
    let mut shows: Vec<f64> = times.iter().map(|(_, d)| d.as_secs_f64() * 1e3).collect();
    shows.sort_by(f64::total_cmp);
    let p95 = shows.get(shows.len() * 95 / 100).copied().unwrap_or(0.0);
    eprintln!(
        "{path} {}x{}: {} pictures ({on_gpu} on the GPU); per picture: CPU {:.2} ms, frame time {:.2} ms \
         (draw {:.2} ms, p95 {:.2} ms); private memory {:.0} MB, peak working set {:.0} MB",
        v.width,
        v.height,
        times.len(),
        (cpu1 - cpu0) / n,
        wall.as_secs_f64() * 1e3 / n,
        shows.iter().sum::<f64>() / n,
        p95,
        private,
        peak,
    );
}
