//! Icons: reading a PNG, resampling it and writing Windows `.ico` files. The default icon new
//! apps start with is drawn by `remedy-templates`. An app's icon is an [`IconSet`]: one
//! drawing that is resampled to every size, or several drawn sizes (pixel art, or small sizes
//! simplified by hand) that are used as they are.

pub use cargo_remedy::templates::image::{Rgba, default_icon};

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
    cargo_remedy::templates::image::write_png(img).map_err(|e| e.to_string())
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

/// Resamples to `size`×`size` by the nearest pixel (the source pixel under each new pixel's
/// center), so pixel art keeps its hard edges.
pub fn resize_nearest(src: &Rgba, size: u32) -> Rgba {
    let (sw, sh) = (src.width.max(1), src.height.max(1));
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let sy = ((2 * u64::from(y) + 1) * u64::from(sh) / (2 * u64::from(size))).min(u64::from(sh) - 1);
        for x in 0..size {
            let sx = ((2 * u64::from(x) + 1) * u64::from(sw) / (2 * u64::from(size))).min(u64::from(sw) - 1);
            let i = ((sy * u64::from(sw) + sx) * 4) as usize;
            data.extend_from_slice(src.data.get(i..i + 4).unwrap_or(&[0; 4]));
        }
    }
    Rgba { width: size, height: size, data }
}

/// An app's icon: its drawings, smallest first, and how other sizes are made from them.
#[derive(Clone, Debug, PartialEq)]
pub struct IconSet {
    drawings: Vec<Rgba>,
    pixel_art: bool,
}

impl IconSet {
    /// One drawing, resampled smoothly to every size.
    pub fn single(img: Rgba) -> IconSet {
        IconSet { drawings: vec![img], pixel_art: false }
    }

    /// Several drawings of the same picture (square, any sizes): a size that was drawn is used
    /// as it is; any other comes from the smallest larger drawing (the largest when none is
    /// larger), by the nearest pixel when `pixel_art`, else by area averages. A size given
    /// twice keeps the later drawing.
    pub fn new(drawings: Vec<Rgba>, pixel_art: bool) -> Result<IconSet, String> {
        let mut sorted: Vec<Rgba> = Vec::with_capacity(drawings.len());
        for d in drawings {
            if d.width != d.height || d.width == 0 {
                return Err(format!("an icon drawing is {}x{}; icons are square", d.width, d.height));
            }
            sorted.retain(|o| o.width != d.width);
            sorted.push(d);
        }
        if sorted.is_empty() {
            return Err("an icon needs at least one drawing".into());
        }
        sorted.sort_by_key(|d| d.width);
        Ok(IconSet { drawings: sorted, pixel_art })
    }

    /// The largest drawing.
    pub fn largest(&self) -> &Rgba {
        // `new` and `single` never leave the list empty.
        self.drawings.last().unwrap_or_else(|| unreachable!("an icon set has a drawing"))
    }

    /// The sizes drawn, smallest first.
    pub fn drawn_sizes(&self) -> Vec<u32> {
        self.drawings.iter().map(|d| d.width).collect()
    }

    /// True when other sizes are made by the nearest pixel.
    pub fn is_pixel_art(&self) -> bool {
        self.pixel_art
    }

    /// The icon `size` pixels square.
    pub fn at(&self, size: u32) -> Rgba {
        let src = self.drawings.iter().find(|d| d.width >= size).unwrap_or_else(|| self.largest());
        if src.width == size {
            src.clone()
        } else if self.pixel_art {
            resize_nearest(src, size)
        } else {
            resize(src, size)
        }
    }

    /// The picture scaled into the middle of a transparent `canvas`-pixel square, `size`
    /// pixels wide (an Android adaptive icon's layer: 108 dp with the picture in the middle).
    pub fn centered(&self, size: u32, canvas: u32) -> Rgba {
        let img = self.at(size.min(canvas));
        let off = (canvas - img.width) / 2;
        let mut data = vec![0u8; (canvas * canvas * 4) as usize];
        for y in 0..img.height {
            let from = (y * img.width * 4) as usize;
            let to = (((y + off) * canvas + off) * 4) as usize;
            let row = (img.width * 4) as usize;
            if let (Some(dst), Some(src)) = (data.get_mut(to..to + row), img.data.get(from..from + row)) {
                dst.copy_from_slice(src);
            }
        }
        Rgba { width: canvas, height: canvas, data }
    }

    /// The color at the largest drawing's top-left corner, as `#rrggbb`, when it is opaque
    /// (the background an Android adaptive icon fills around the picture); `None` for a
    /// see-through corner.
    pub fn corner_color(&self) -> Option<String> {
        let px = self.largest().data.get(..4)?;
        match px {
            [r, g, b, 255] => Some(format!("#{r:02x}{g:02x}{b:02x}")),
            _ => None,
        }
    }
}

/// A Windows icon holding `set` at every size in [`ICO_SIZES`] and every drawn size up to
/// 256, each stored as PNG.
pub fn ico(set: &IconSet) -> Result<Vec<u8>, String> {
    let mut sizes: Vec<u32> = ICO_SIZES.to_vec();
    sizes.extend(set.drawn_sizes().into_iter().filter(|s| *s <= 256));
    sizes.sort_unstable();
    sizes.dedup();
    let pngs: Vec<Vec<u8>> = sizes.iter().map(|s| write_png(&set.at(*s))).collect::<Result<_, _>>()?;
    let count = u16::try_from(pngs.len()).map_err(|_| "too many icon sizes")?;
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (size, data) in sizes.iter().zip(&pngs) {
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

/// Reads a PNG file as an icon drawing.
pub fn read_png_file(path: &std::path::Path) -> Result<Rgba, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("icon {}: {e}", path.display()))?;
    read_png(&bytes).map_err(|e| format!("icon {}: {e}", path.display()))
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
        let ico = ico(&IconSet::single(img.clone())).unwrap_or_default();
        assert_eq!(ico.get(..6), Some(&[0, 0, 1, 0, ICO_SIZES.len() as u8, 0][..]));
        // The 256 px entry is written as 0 and points at a PNG.
        let last = 6 + 16 * (ICO_SIZES.len() - 1);
        assert_eq!(ico.get(last), Some(&0));
        let off = u32::from_le_bytes(
            ico.get(last + 12..last + 16).and_then(|b| b.try_into().ok()).unwrap_or_default(),
        ) as usize;
        assert_eq!(ico.get(off..off + 4), Some(&b"\x89PNG"[..]));
    }

    fn flat(size: u32, v: u8) -> Rgba {
        Rgba { width: size, height: size, data: vec![v; (size * size * 4) as usize] }
    }

    #[test]
    fn icon_sets_use_drawn_sizes_and_scale_pixel_art_by_the_nearest_pixel() {
        assert!(IconSet::new(Vec::new(), false).is_err());
        assert!(IconSet::new(vec![Rgba { width: 2, height: 3, data: vec![0; 24] }], false).is_err());
        let set = IconSet::new(vec![flat(64, 64), flat(16, 16), flat(32, 32)], true)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(set.drawn_sizes(), [16, 32, 64]);
        assert!(set.at(16).data.iter().all(|&c| c == 16), "a drawn size as it is");
        assert!(set.at(20).data.iter().all(|&c| c == 32), "the next larger drawing");
        assert_eq!(set.at(128).width, 128);
        // A 2 x 2 checkerboard stays black and white at 4 x 4.
        let board = Rgba {
            width: 2,
            height: 2,
            data: [[255, 255, 255, 255], [0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255]].concat(),
        };
        let sharp = IconSet::new(vec![board.clone()], true).unwrap_or_else(|e| unreachable!("{e}")).at(4);
        assert!(sharp.data.iter().all(|&c| c == 0 || c == 255));
        assert_eq!(sharp.data.get(..8), Some(&[255; 8][..]));
        let smooth = IconSet::single(board).at(1);
        assert_eq!(smooth.data.get(..3), Some(&[128, 128, 128][..]));
        // The .ico holds the standard sizes and the drawn ones.
        let ico = ico(&set).unwrap_or_default();
        let count = usize::from(ico.get(4).copied().unwrap_or(0));
        assert_eq!(count, ICO_SIZES.len(), "16, 32 and 64 are standard sizes already");
        assert_eq!(set.corner_color(), None, "a see-through corner");
        let opaque = Rgba { width: 1, height: 1, data: vec![1, 2, 3, 255] };
        assert_eq!(IconSet::single(opaque).corner_color().as_deref(), Some("#010203"));
        let c = set.centered(72, 108);
        assert_eq!((c.width, c.data.get(3), c.data.get((54 * 108 + 54) * 4 + 3)), (108, Some(&0), Some(&64)));
    }
}
