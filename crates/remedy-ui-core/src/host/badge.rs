//! The app's badge: a count or a dot on its taskbar button, Dock tile or launcher icon.

use crate::host::TrayImage;

/// What the app's badge shows (Windows' taskbar overlay, the macOS Dock tile, the Linux
/// dock's launcher entry, a browser's installed-app icon).
///
/// ```
/// use remedy_ui_core::host::Badge;
///
/// assert_eq!(Badge::count(0), None, "nothing to show is no badge");
/// assert_eq!(Badge::count(3).map(Badge::label), Some("3".to_owned()));
/// assert_eq!(Badge::Count(250).label(), "99+");
/// assert_eq!(Badge::Count(12).short_label(), "9+", "what fits a 16-pixel overlay");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Badge {
    /// A number: unread messages, waiting downloads.
    Count(u32),
    /// Something needs a look, with no number to give.
    Dot,
}

/// The badge's red (Windows' and macOS's alert red, readable under white text at 4.6:1).
const RED: [u8; 3] = [0xC4, 0x2B, 0x1C];

/// Digits and `+` in a 3 × 5 grid, one row per entry, high bit on the left.
const GLYPHS: [(char, [u8; 5]); 11] = [
    ('0', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('1', [0b010, 0b110, 0b010, 0b010, 0b111]),
    ('2', [0b111, 0b001, 0b111, 0b100, 0b111]),
    ('3', [0b111, 0b001, 0b111, 0b001, 0b111]),
    ('4', [0b101, 0b101, 0b111, 0b001, 0b001]),
    ('5', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('6', [0b111, 0b100, 0b111, 0b101, 0b111]),
    ('7', [0b111, 0b001, 0b010, 0b010, 0b010]),
    ('8', [0b111, 0b101, 0b111, 0b101, 0b111]),
    ('9', [0b111, 0b101, 0b111, 0b001, 0b111]),
    ('+', [0b000, 0b010, 0b111, 0b010, 0b000]),
];

impl Badge {
    /// A count badge, or none for zero.
    pub fn count(n: u32) -> Option<Badge> {
        (n > 0).then_some(Badge::Count(n))
    }

    /// What a badge with room for text says: the number up to 99, then `99+`; `•` for
    /// [`Badge::Dot`].
    pub fn label(self) -> String {
        match self {
            Badge::Count(n) if n > 99 => "99+".to_owned(),
            Badge::Count(n) => n.to_string(),
            Badge::Dot => "\u{2022}".to_owned(),
        }
    }

    /// What a badge with room for two characters says: the digit up to 9, then `9+`; empty for
    /// [`Badge::Dot`].
    pub fn short_label(self) -> String {
        match self {
            Badge::Count(n) if n > 9 => "9+".to_owned(),
            Badge::Count(n) => n.to_string(),
            Badge::Dot => String::new(),
        }
    }

    /// The badge drawn `size` pixels square (premultiplied RGBA8): a red disc with
    /// [`Badge::short_label`] in white, for platforms that take a picture (Windows' taskbar
    /// overlay). A dot is a smaller disc.
    pub fn image(self, size: u32) -> TrayImage {
        let size = size.max(1);
        let n = size as usize;
        let mut rgba = vec![0u8; n * n * 4];
        let c = size as f32 / 2.0;
        let r = match self {
            Badge::Dot => c * 0.6,
            Badge::Count(_) => c,
        };
        let pixels = rgba.as_chunks_mut::<4>().0;
        for (i, px) in pixels.iter_mut().enumerate() {
            let (x, y) = (i % n, i / n);
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let cover = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            let a = (cover * 255.0).round() as u8;
            let [cr, cg, cb] = RED.map(|ch| ((u16::from(ch) * u16::from(a) + 127) / 255) as u8);
            *px = [cr, cg, cb, a];
        }
        let text = self.short_label();
        let glyphs: Vec<[u8; 5]> = text
            .chars()
            .filter_map(|ch| GLYPHS.iter().find(|(g, _)| *g == ch).map(|(_, rows)| *rows))
            .collect();
        if glyphs.is_empty() {
            return TrayImage { size, rgba };
        }
        // Each glyph is 3 cells wide with a cell between glyphs; 7 cells fit two in the disc.
        let scale = (size / 8).max(1) as usize;
        let width = (glyphs.len() * 4 - 1) * scale;
        let height = 5 * scale;
        let left = n.saturating_sub(width) / 2;
        let top = n.saturating_sub(height) / 2;
        for (i, rows) in glyphs.iter().enumerate() {
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) == 0 {
                        continue;
                    }
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let x = left + (i * 4 + col) * scale + sx;
                            let y = top + row * scale + sy;
                            if x < n
                                && let Some(px) = pixels.get_mut(y * n + x)
                            {
                                // White over the disc, at the disc's own coverage.
                                let a = px[3];
                                *px = [a, a, a, a];
                            }
                        }
                    }
                }
            }
        }
        TrayImage { size, rgba }
    }
}

#[cfg(test)]
mod tests {
    use super::Badge;

    fn at(img: &crate::host::TrayImage, x: u32, y: u32) -> [u8; 4] {
        img.rgba.as_chunks::<4>().0.get((y * img.size + x) as usize).copied().unwrap_or_default()
    }

    #[test]
    fn labels_shorten_to_fit() {
        assert_eq!(Badge::Count(7).label(), "7");
        assert_eq!(Badge::Count(99).label(), "99");
        assert_eq!(Badge::Count(100).label(), "99+");
        assert_eq!(Badge::Count(9).short_label(), "9");
        assert_eq!(Badge::Count(10).short_label(), "9+");
        assert_eq!(Badge::Dot.short_label(), "");
    }

    #[test]
    fn the_image_is_a_red_disc_with_white_digits() {
        let img = Badge::Count(1).image(16);
        assert_eq!(img.size, 16);
        assert_eq!(img.rgba.len(), 16 * 16 * 4);
        assert_eq!(at(&img, 0, 0)[3], 0, "the corners stay clear");
        // The "1"'s stem runs down the middle column of its glyph.
        let white =
            (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).filter(|&(x, y)| at(&img, x, y) == [255; 4]);
        assert!(white.count() >= 7 * 4, "the digit is drawn in white at scale 2");
        let red = at(&img, 2, 8);
        assert!(red[0] > red[1] && red[3] == 255, "the disc is opaque red: {red:?}");
        // Premultiplied: no channel exceeds alpha anywhere.
        assert!(img.rgba.as_chunks::<4>().0.iter().all(|[r, g, b, a]| r <= a && g <= a && b <= a));
    }

    #[test]
    fn two_glyphs_fit_and_a_dot_is_smaller() {
        let img = Badge::Count(42).image(16);
        let row: Vec<bool> = (0..16).map(|x| at(&img, x, 8) == [255; 4]).collect();
        assert!(row.iter().filter(|w| **w).count() >= 4, "both '9' and '+' cross the middle row: {row:?}");
        let dot = Badge::Dot.image(16);
        assert_eq!(at(&dot, 1, 8)[3], 0, "a dot leaves the rim clear");
        assert_eq!(at(&dot, 8, 8)[3], 255);
        assert!(dot.rgba.as_chunks::<4>().0.iter().all(|p| *p != [255; 4]), "a dot has no text");
    }
}
