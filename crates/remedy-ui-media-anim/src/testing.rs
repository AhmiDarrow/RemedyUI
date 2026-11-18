//! Makes small animated files for tests, with the encoders in the dependency tree. Shared by
//! the unit tests and (by path) the integration tests.
#![allow(dead_code, missing_docs)]

use std::borrow::Cow;

/// A palette of eight colors: transparent, red, green, blue, white, black, yellow, gray.
pub const PALETTE: [[u8; 3]; 8] = [
    [0, 0, 0],
    [255, 0, 0],
    [0, 255, 0],
    [0, 0, 255],
    [255, 255, 255],
    [0, 0, 0],
    [255, 255, 0],
    [128, 128, 128],
];

pub const T: u8 = 0;
pub const RED: u8 = 1;
pub const GREEN: u8 = 2;
pub const BLUE: u8 = 3;
pub const WHITE: u8 = 4;
pub const BLACK: u8 = 5;
pub const YELLOW: u8 = 6;

/// Straight RGBA of a palette color (index 0 is transparent).
pub fn rgba(i: u8) -> [u8; 4] {
    if i == T {
        return [0, 0, 0, 0];
    }
    let [r, g, b] = PALETTE.get(usize::from(i)).copied().unwrap_or([0, 0, 0]);
    [r, g, b, 255]
}

pub struct GifFrame {
    /// Left, top, width, height.
    pub rect: (u16, u16, u16, u16),
    /// Palette indices, row by row.
    pub px: Vec<u8>,
    /// Hundredths of a second.
    pub delay: u16,
    pub dispose: gif::DisposalMethod,
}

impl GifFrame {
    /// A frame covering a `w`×`h` picture in one palette color.
    pub fn solid(w: u16, h: u16, color: u8, delay: u16) -> GifFrame {
        GifFrame {
            rect: (0, 0, w, h),
            px: vec![color; usize::from(w) * usize::from(h)],
            delay,
            dispose: gif::DisposalMethod::Keep,
        }
    }

    pub fn part(rect: (u16, u16, u16, u16), color: u8, delay: u16, dispose: gif::DisposalMethod) -> GifFrame {
        GifFrame { rect, px: vec![color; usize::from(rect.2) * usize::from(rect.3)], delay, dispose }
    }
}

/// A GIF `w`×`h`. `repeat` is the loop count extension (none when `None`: plays once).
pub fn gif(w: u16, h: u16, repeat: Option<gif::Repeat>, frames: &[GifFrame]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let pal: Vec<u8> = PALETTE.iter().flatten().copied().collect();
        let mut enc = gif::Encoder::new(&mut out, w, h, &pal).unwrap_or_else(|e| unreachable!("{e}"));
        if let Some(r) = repeat {
            enc.set_repeat(r).unwrap_or_else(|e| unreachable!("{e}"));
        }
        for f in frames {
            let fr = gif::Frame {
                delay: f.delay,
                dispose: f.dispose,
                transparent: Some(T),
                left: f.rect.0,
                top: f.rect.1,
                width: f.rect.2,
                height: f.rect.3,
                buffer: Cow::Owned(f.px.clone()),
                ..gif::Frame::default()
            };
            enc.write_frame(&fr).unwrap_or_else(|e| unreachable!("{e}"));
        }
    }
    out
}

pub struct ApngFrame {
    /// Left, top, width, height.
    pub rect: (u32, u32, u32, u32),
    /// Straight RGBA, row by row.
    pub rgba: Vec<u8>,
    /// Numerator and denominator of the delay in seconds.
    pub delay: (u16, u16),
    pub dispose: png::DisposeOp,
    pub blend: png::BlendOp,
}

impl ApngFrame {
    pub fn solid(w: u32, h: u32, color: [u8; 4], ms: u16) -> ApngFrame {
        ApngFrame {
            rect: (0, 0, w, h),
            rgba: color.repeat((w * h) as usize),
            delay: (ms, 1000),
            dispose: png::DisposeOp::None,
            blend: png::BlendOp::Source,
        }
    }

    pub fn part(
        rect: (u32, u32, u32, u32),
        color: [u8; 4],
        ms: u16,
        dispose: png::DisposeOp,
        blend: png::BlendOp,
    ) -> ApngFrame {
        ApngFrame { rect, rgba: color.repeat((rect.2 * rect.3) as usize), delay: (ms, 1000), dispose, blend }
    }
}

/// An APNG `w`×`h` playing `plays` times (0: forever). With `default_image`, that picture is
/// the fallback for readers that do not animate, and not a frame.
pub fn apng(w: u32, h: u32, plays: u32, default_image: Option<[u8; 4]>, frames: &[ApngFrame]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, plays).unwrap_or_else(|e| unreachable!("{e}"));
        if default_image.is_some() {
            enc.set_sep_def_img(true).unwrap_or_else(|e| unreachable!("{e}"));
        }
        let mut w_ = enc.write_header().unwrap_or_else(|e| unreachable!("{e}"));
        if let Some(c) = default_image {
            w_.write_image_data(&c.repeat((w * h) as usize)).unwrap_or_else(|e| unreachable!("{e}"));
        }
        for f in frames {
            w_.set_frame_delay(f.delay.0, f.delay.1).unwrap_or_else(|e| unreachable!("{e}"));
            // Out of the way first: size and place must fit together at every step.
            w_.set_frame_position(0, 0).unwrap_or_else(|e| unreachable!("{e}"));
            w_.set_frame_dimension(f.rect.2, f.rect.3).unwrap_or_else(|e| unreachable!("{e}"));
            w_.set_frame_position(f.rect.0, f.rect.1).unwrap_or_else(|e| unreachable!("{e}"));
            w_.set_dispose_op(f.dispose).unwrap_or_else(|e| unreachable!("{e}"));
            w_.set_blend_op(f.blend).unwrap_or_else(|e| unreachable!("{e}"));
            w_.write_image_data(&f.rgba).unwrap_or_else(|e| unreachable!("{e}"));
        }
        w_.finish().unwrap_or_else(|e| unreachable!("{e}"));
    }
    out
}

/// A still PNG of one color.
pub fn png_still(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w_ = enc.write_header().unwrap_or_else(|e| unreachable!("{e}"));
        w_.write_image_data(&color.repeat((w * h) as usize)).unwrap_or_else(|e| unreachable!("{e}"));
        w_.finish().unwrap_or_else(|e| unreachable!("{e}"));
    }
    out
}

pub struct WebpFrame {
    /// Left, top (even), width, height.
    pub rect: (u32, u32, u32, u32),
    pub rgba: Vec<u8>,
    pub ms: u32,
    /// Draw over what is there (false: replace it).
    pub blend: bool,
    /// Clear the frame's area to the background after it.
    pub dispose: bool,
}

impl WebpFrame {
    pub fn solid(w: u32, h: u32, color: [u8; 4], ms: u32) -> WebpFrame {
        WebpFrame {
            rect: (0, 0, w, h),
            rgba: color.repeat((w * h) as usize),
            ms,
            blend: false,
            dispose: false,
        }
    }

    pub fn part(
        rect: (u32, u32, u32, u32),
        color: [u8; 4],
        ms: u32,
        blend: bool,
        dispose: bool,
    ) -> WebpFrame {
        WebpFrame { rect, rgba: color.repeat((rect.2 * rect.3) as usize), ms, blend, dispose }
    }
}

fn chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(name);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

fn le24(v: u32) -> [u8; 3] {
    let [a, b, c, _] = v.to_le_bytes();
    [a, b, c]
}

/// A lossless still WebP's VP8L chunk data.
fn vp8l(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut file = Vec::new();
    image_webp::WebPEncoder::new(&mut file)
        .encode(rgba, w, h, image_webp::ColorType::Rgba8)
        .unwrap_or_else(|e| unreachable!("{e}"));
    // RIFF, size, WEBP, then "VP8L", its size and data.
    let size = file.get(16..20).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes) as usize;
    file.get(20..20 + size).map(<[u8]>::to_vec).unwrap_or_default()
}

/// An animated WebP `w`×`h` playing `loops` times (0: forever).
pub fn webp(w: u32, h: u32, loops: u16, frames: &[WebpFrame]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"WEBP");
    // VP8X: animation and alpha flags, then the canvas size minus one.
    let mut vp8x = vec![0x02 | 0x10, 0, 0, 0];
    vp8x.extend_from_slice(&le24(w - 1));
    vp8x.extend_from_slice(&le24(h - 1));
    chunk(&mut body, b"VP8X", &vp8x);
    // ANIM: background color (BGRA), loop count.
    let mut anim = vec![0, 0, 0, 0];
    anim.extend_from_slice(&loops.to_le_bytes());
    chunk(&mut body, b"ANIM", &anim);
    for f in frames {
        let mut anmf = Vec::new();
        anmf.extend_from_slice(&le24(f.rect.0 / 2));
        anmf.extend_from_slice(&le24(f.rect.1 / 2));
        anmf.extend_from_slice(&le24(f.rect.2 - 1));
        anmf.extend_from_slice(&le24(f.rect.3 - 1));
        anmf.extend_from_slice(&le24(f.ms));
        // Bit 1: do not blend. Bit 0: dispose to background.
        anmf.push((u8::from(!f.blend) << 1) | u8::from(f.dispose));
        chunk(&mut anmf, b"VP8L", &vp8l(f.rect.2, f.rect.3, &f.rgba));
        chunk(&mut body, b"ANMF", &anmf);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}
