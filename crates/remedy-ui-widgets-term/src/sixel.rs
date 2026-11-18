//! Sixel graphics (DEC's raster pictures in a DCS string): `"` raster attributes, `#`
//! color registers (RGB or HLS), `!` repeats, `$` carriage return, `-` next band of six
//! rows, and the sixel characters `?`..`~` themselves.
//!
//! The picture never grows past the size the terminal can show (its columns and rows in
//! pixels, as the UI laid it out): a repeat count or raster size past that is cut, so a few
//! bytes can never claim gigabytes.

use crate::term::Rgb;

/// What the decoder is in the middle of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    None,
    Repeat,
    Color,
    Raster,
}

/// VT340's default color registers, in percent.
const VT340: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (20, 20, 80),
    (80, 13, 13),
    (20, 80, 20),
    (80, 20, 80),
    (20, 80, 80),
    (80, 80, 20),
    (53, 53, 53),
    (26, 26, 26),
    (33, 33, 60),
    (60, 26, 26),
    (33, 60, 33),
    (60, 33, 60),
    (33, 60, 60),
    (60, 60, 33),
    (80, 80, 80),
];

fn percent(v: u32) -> u8 {
    u8::try_from(v.min(100) * 255 / 100).unwrap_or(255)
}

/// DEC's HLS (hue 0 is blue, 120 red, 240 green; lightness and saturation in percent).
fn hls(h: u32, l: u32, s: u32) -> Rgb {
    let h = ((h + 240) % 360) as f32;
    let l = l.min(100) as f32 / 100.0;
    let s = s.min(100) as f32 / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to(r), to(g), to(b))
}

/// Decodes one sixel picture, byte by byte.
pub(crate) struct Decoder {
    max_w: usize,
    max_h: usize,
    w: usize,
    h: usize,
    /// Straight RGBA; alpha 0 where nothing was drawn.
    px: Vec<u8>,
    used_w: usize,
    used_h: usize,
    x: usize,
    band: usize,
    color: usize,
    palette: Vec<Rgb>,
    cmd: Cmd,
    nums: [u32; 5],
    n: usize,
    /// Unset pixels stay transparent (P2 = 1); otherwise they take color register 0.
    transparent: bool,
}

impl Decoder {
    /// A decoder for a picture of at most `max_w` × `max_h` pixels; `p2` is the DCS's second
    /// parameter (1: unset pixels are transparent).
    pub(crate) fn new(max_w: usize, max_h: usize, p2: u16) -> Decoder {
        Decoder {
            max_w: max_w.max(1),
            max_h: max_h.max(1),
            w: 0,
            h: 0,
            px: Vec::new(),
            used_w: 0,
            used_h: 0,
            x: 0,
            band: 0,
            color: 0,
            palette: VT340
                .iter()
                .map(|(r, g, b)| (percent(u32::from(*r)), percent(u32::from(*g)), percent(u32::from(*b))))
                .collect(),
            cmd: Cmd::None,
            nums: [0; 5],
            n: 0,
            transparent: p2 == 1,
        }
    }

    /// Makes the canvas at least `w` × `h` (within the limits), keeping what is drawn.
    fn grow(&mut self, w: usize, h: usize) {
        let w = w.min(self.max_w);
        let h = h.min(self.max_h);
        if w <= self.w && h <= self.h {
            return;
        }
        // Grow by doubling, so a picture drawn left to right costs few copies.
        let nw = if w > self.w { w.max(self.w * 2).min(self.max_w) } else { self.w };
        let nh = if h > self.h { h.max(self.h * 2).min(self.max_h) } else { self.h };
        let mut px = vec![0u8; nw * nh * 4];
        for y in 0..self.h {
            let from = y * self.w * 4;
            let to = y * nw * 4;
            if let (Some(src), Some(dst)) =
                (self.px.get(from..from + self.w * 4), px.get_mut(to..to + self.w * 4))
            {
                dst.copy_from_slice(src);
            }
        }
        self.px = px;
        self.w = nw;
        self.h = nh;
    }

    /// Ends the parameters of a `#` or `"` command.
    fn finish_cmd(&mut self) {
        let nums = self.nums;
        let n = self.n + 1;
        match self.cmd {
            Cmd::Color => {
                let reg = nums.first().copied().unwrap_or(0) as usize;
                if n >= 5 {
                    let [_, kind, a, b, c] = nums;
                    let rgb = if kind == 1 { hls(a, b, c) } else { (percent(a), percent(b), percent(c)) };
                    // Registers are numbered by the program; a high number grows the table
                    // only as far as that number (at most 65 536 entries: parameters are 16 bits).
                    let reg = reg.min(u16::MAX as usize);
                    if self.palette.len() <= reg {
                        self.palette.resize(reg + 1, (0, 0, 0));
                    }
                    if let Some(p) = self.palette.get_mut(reg) {
                        *p = rgb;
                    }
                }
                self.color = reg;
            }
            Cmd::Raster => {
                // Pan;Pad;Ph;Pv: the picture's size, when the program says it.
                let (w, h) =
                    (nums.get(2).copied().unwrap_or(0) as usize, nums.get(3).copied().unwrap_or(0) as usize);
                if w > 0 && h > 0 {
                    self.grow(w, h);
                    self.used_w = self.used_w.max(w.min(self.max_w));
                    self.used_h = self.used_h.max(h.min(self.max_h));
                }
            }
            Cmd::Repeat | Cmd::None => {}
        }
        if self.cmd != Cmd::Repeat {
            self.cmd = Cmd::None;
            self.nums = [0; 5];
            self.n = 0;
        }
    }

    fn draw(&mut self, bits: u8, count: usize) {
        let y0 = self.band * 6;
        if y0 >= self.max_h {
            self.x = self.x.saturating_add(count);
            return;
        }
        let end = self.x.saturating_add(count).min(self.max_w);
        if end > self.x {
            self.grow(end, y0 + 6);
            let (r, g, b) = self.palette.get(self.color).copied().unwrap_or((0, 0, 0));
            for bit in 0..6usize {
                if bits & (1 << bit) == 0 {
                    continue;
                }
                let y = y0 + bit;
                if y >= self.h {
                    break;
                }
                for x in self.x..end {
                    let i = (y * self.w + x) * 4;
                    if let Some(p) = self.px.get_mut(i..i + 4) {
                        p.copy_from_slice(&[r, g, b, 255]);
                    }
                }
                self.used_h = self.used_h.max(y + 1);
            }
            self.used_w = self.used_w.max(end);
        }
        self.x = self.x.saturating_add(count);
    }

    /// Takes one byte of the picture.
    pub(crate) fn put(&mut self, b: u8) {
        match b {
            b'0'..=b'9' => {
                if let Some(v) = self.nums.get_mut(self.n) {
                    *v = v.saturating_mul(10).saturating_add(u32::from(b - b'0'));
                }
            }
            b';' => {
                if self.n + 1 < self.nums.len() {
                    self.n += 1;
                }
            }
            b'?'..=b'~' => {
                let count = if self.cmd == Cmd::Repeat {
                    self.nums.first().copied().unwrap_or(1).max(1) as usize
                } else {
                    1
                };
                if self.cmd != Cmd::Repeat {
                    self.finish_cmd();
                }
                self.cmd = Cmd::None;
                self.nums = [0; 5];
                self.n = 0;
                self.draw(b - b'?', count);
            }
            b'!' | b'#' | b'"' | b'$' | b'-' => {
                self.finish_cmd();
                self.cmd = Cmd::None;
                self.nums = [0; 5];
                self.n = 0;
                match b {
                    b'!' => self.cmd = Cmd::Repeat,
                    b'#' => self.cmd = Cmd::Color,
                    b'"' => self.cmd = Cmd::Raster,
                    b'$' => self.x = 0,
                    _ => {
                        self.x = 0;
                        self.band += 1;
                    }
                }
            }
            _ => {}
        }
    }

    /// The finished picture as premultiplied RGBA8, cut to what was drawn; `None` when
    /// nothing was.
    pub(crate) fn finish(mut self) -> Option<(u32, u32, Vec<u8>)> {
        self.finish_cmd();
        let (w, h) = (self.used_w.min(self.w), self.used_h.min(self.h));
        if w == 0 || h == 0 {
            return None;
        }
        let bg = self.palette.first().copied().unwrap_or((0, 0, 0));
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = y * self.w * 4;
            for x in 0..w {
                let p = self.px.get(row + x * 4..row + x * 4 + 4).unwrap_or(&[0, 0, 0, 0]);
                match p {
                    [r, g, b, 255] => out.extend_from_slice(&[*r, *g, *b, 255]),
                    _ if !self.transparent => out.extend_from_slice(&[bg.0, bg.1, bg.2, 255]),
                    _ => out.extend_from_slice(&[0, 0, 0, 0]),
                }
            }
        }
        Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?, out))
    }
}
