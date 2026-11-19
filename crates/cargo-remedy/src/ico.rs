//! Icons: reading a PNG, resampling it and writing Windows `.ico` files. The default icon new
//! apps start with is drawn by `remedy-templates`.

pub use remedy_templates::image::{Rgba, default_icon};

/// Sizes a Windows icon carries (Explorer, taskbar, Start, alt-tab at every DPI).
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Reads a PNG as straight RGBA8 (gray, palette and 16-bit images are converted).
pub fn read_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let size = r.output_buffer_size().ok_or("the image is too large")?;
    let mut buf = vec![0u8; size];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let px = (info.width * info.height) as usize;
    let data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|c| {
                [c.first(), c.get(1), c.get(2)].map(|v| v.copied().unwrap_or(0)).into_iter().chain([255])
            })
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|c| {
                let (g, a) = (c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(255));
                [g, g, g, a]
            })
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    if data.len() != px * 4 {
        return Err("unexpected PNG layout".into());
    }
    Ok(Rgba { width: info.width, height: info.height, data })
}

/// Writes straight RGBA8 as a PNG.
pub fn write_png(img: &Rgba) -> Result<Vec<u8>, String> {
    remedy_templates::image::write_png(img).map_err(|e| e.to_string())
}

/// Resamples to `size`×`size` by averaging the source area under each pixel, in
/// premultiplied alpha so transparent edges do not darken.
pub fn resize(src: &Rgba, size: u32) -> Rgba {
    let (sw, sh) = (src.width.max(1), src.height.max(1));
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let y0 = (y * sh) / size;
        let y1 = (((y + 1) * sh) / size).max(y0 + 1).min(sh);
        for x in 0..size {
            let x0 = (x * sw) / size;
            let x1 = (((x + 1) * sw) / size).max(x0 + 1).min(sw);
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * sw + sx) * 4) as usize;
                    if let Some(p) = src.data.get(i..i + 4) {
                        let a = u64::from(p.get(3).copied().unwrap_or(0));
                        for (k, c) in acc.iter_mut().take(3).enumerate() {
                            *c += u64::from(p.get(k).copied().unwrap_or(0)) * a;
                        }
                        acc[3] += a;
                        n += 1;
                    }
                }
            }
            let a = acc[3];
            for c in acc.iter().take(3) {
                data.push((c + a / 2).checked_div(a).unwrap_or(0).min(255) as u8);
            }
            data.push((a + n / 2).checked_div(n).unwrap_or(0) as u8);
        }
    }
    Rgba { width: size, height: size, data }
}

/// A Windows icon holding `img` at every size in [`ICO_SIZES`], each stored as PNG.
pub fn ico(img: &Rgba) -> Result<Vec<u8>, String> {
    let pngs: Vec<Vec<u8>> =
        ICO_SIZES.iter().map(|s| write_png(&resize(img, *s))).collect::<Result<_, _>>()?;
    let count = pngs.len() as u16;
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (size, data) in ICO_SIZES.iter().zip(&pngs) {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in pngs {
        out.extend(data);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_hold_every_size_as_png_and_round_trip() {
        let img = default_icon([61, 139, 95]);
        let png = write_png(&img).unwrap_or_default();
        assert_eq!(read_png(&png).ok().as_ref(), Some(&img));
        let small = resize(&img, 16);
        assert_eq!((small.width, small.data.len()), (16, 16 * 16 * 4));
        assert_eq!(small.data.get(3), Some(&0), "the corner stays transparent");
        let center = small.data.get((8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4).unwrap_or_default().to_vec();
        assert_eq!(center.get(3), Some(&255), "the middle is opaque");
        let ico = ico(&img).unwrap_or_default();
        assert_eq!(ico.get(..6), Some(&[0, 0, 1, 0, ICO_SIZES.len() as u8, 0][..]));
        // The 256 px entry is written as 0 and points at a PNG.
        let last = 6 + 16 * (ICO_SIZES.len() - 1);
        assert_eq!(ico.get(last), Some(&0));
        let off = u32::from_le_bytes(
            ico.get(last + 12..last + 16).and_then(|b| b.try_into().ok()).unwrap_or_default(),
        ) as usize;
        assert_eq!(ico.get(off..off + 4), Some(&b"\x89PNG"[..]));
    }
}
