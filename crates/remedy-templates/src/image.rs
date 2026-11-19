//! The pictures a new app starts with: its icon and the document template's sample pages,
//! drawn in code and written as PNG.

/// Writing a PNG failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageError(pub(crate) String);

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not write the PNG: {}", self.0)
    }
}

impl std::error::Error for ImageError {}

/// Straight-alpha RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Pixels, row by row.
    pub data: Vec<u8>,
}

/// Writes straight RGBA8 as a PNG.
pub fn write_png(img: &Rgba) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| ImageError(e.to_string()))?;
        w.write_image_data(&img.data).map_err(|e| ImageError(e.to_string()))?;
    }
    Ok(out)
}

/// A 512 px default icon: a rounded square in `color` with a light ring, antialiased.
pub fn default_icon(color: [u8; 3]) -> Rgba {
    let size = 512u32;
    let s = size as f32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let inside = |x: f32, y: f32| -> (bool, bool) {
        // Rounded square with 22% corner radius, inset 6%.
        let (m, r) = (s * 0.06, s * 0.22);
        let (cx, cy) = (x.clamp(m + r, s - m - r), y.clamp(m + r, s - m - r));
        let sq = (x - cx).powi(2) + (y - cy).powi(2) <= r * r && x >= m && x <= s - m && y >= m && y <= s - m;
        let d = ((x - s / 2.0).powi(2) + (y - s / 2.0).powi(2)).sqrt();
        (sq, (s * 0.20..=s * 0.28).contains(&d))
    };
    for py in 0..size {
        for px in 0..size {
            let (mut cov, mut ring) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                    let (a, b) = inside(x, y);
                    cov += u32::from(a);
                    ring += u32::from(a && b);
                }
            }
            let t = ring as f32 / cov.max(1) as f32;
            let mix = |c: u8| (f32::from(c) * (1.0 - t) + 240.0 * t).round() as u8;
            data.extend([mix(color[0]), mix(color[1]), mix(color[2]), (cov * 255 / 16) as u8]);
        }
    }
    Rgba { width: size, height: size, data }
}
