//! Box-drawing and block characters drawn as shapes, so frames and bars that programs draw
//! join seamlessly from cell to cell whatever the font (many fonts lack them, or draw them a
//! hair short of the cell's edges).

use remedy_ui_core::{Color, Rect};
use remedy_ui_paint::{Brush, DisplayList};

/// The four arms of U+2500–U+257F: up, right, down, left; 0 none, 1 light, 2 heavy, 3
/// double. `None` for characters left to the font (diagonals).
fn arms(c: char) -> Option<[u8; 4]> {
    const LINES: [&str; 76] = [
        "0101", "0202", "1010", "2020", "0101", "0202", "1010", "2020", "0101", "0202", "1010", "2020",
        "0110", "0210", "0120", "0220", "0011", "0012", "0021", "0022", "1100", "1200", "2100", "2200",
        "1001", "1002", "2001", "2002", "1110", "1210", "2110", "1120", "2120", "2210", "1220", "2220",
        "1011", "1012", "2011", "1021", "2021", "2012", "1022", "2022", "0111", "0112", "0211", "0212",
        "0121", "0122", "0221", "0222", "1101", "1102", "1201", "1202", "2101", "2102", "2201", "2202",
        "1111", "1112", "1211", "1212", "2111", "1121", "2121", "2112", "2211", "1122", "1221", "2212",
        "1222", "2122", "2221", "2222",
    ];
    const DASHED_DOUBLE: [&str; 4] = ["0101", "0202", "1010", "2020"];
    const DOUBLE: [&str; 29] = [
        "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033", "1300", "3100", "3300", "1003",
        "3001", "3003", "1310", "3130", "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303",
        "3101", "3303", "1313", "3131", "3333",
    ];
    const ROUND: [&str; 4] = ["0110", "0011", "1001", "1100"];
    const HALF: [&str; 12] =
        ["0001", "1000", "0100", "0010", "0002", "2000", "0200", "0020", "0201", "1020", "0102", "2010"];
    let n = c as u32;
    let spec = match n {
        0x2500..=0x254B => LINES.get((n - 0x2500) as usize),
        0x254C..=0x254F => DASHED_DOUBLE.get((n - 0x254C) as usize),
        0x2550..=0x256C => DOUBLE.get((n - 0x2550) as usize),
        0x256D..=0x2570 => ROUND.get((n - 0x256D) as usize),
        0x2574..=0x257F => HALF.get((n - 0x2574) as usize),
        _ => None,
    }?;
    let mut out = [0u8; 4];
    for (o, b) in out.iter_mut().zip(spec.bytes()) {
        *o = b - b'0';
    }
    Some(out)
}

/// Rounds `v` logical pixels to whole device pixels.
fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

/// Draws `c` into `cell` in `color` when it is a box-drawing or block character; false when
/// the font should draw it.
pub(crate) fn draw(list: &mut DisplayList, c: char, cell: Rect, color: Color, scale: f32) -> bool {
    let (x0, y0, w, h) = (cell.min.x, cell.min.y, cell.width(), cell.height());
    let fill = |list: &mut DisplayList, x: f32, y: f32, ww: f32, hh: f32, col: Color| {
        let r = Rect::new(snap(x, scale), snap(y, scale), snap(x + ww, scale), snap(y + hh, scale));
        if r.width() > 0.0 && r.height() > 0.0 {
            list.fill_rect(r, Brush::Solid(col));
        }
    };
    let n = c as u32;
    match n {
        0x2580..=0x259F => {
            let e = |k: f32| k / 8.0;
            match n {
                0x2580 => fill(list, x0, y0, w, h * 0.5, color),
                0x2581..=0x2587 => {
                    let part = e((n - 0x2580) as f32);
                    fill(list, x0, y0 + h * (1.0 - part), w, h * part, color);
                }
                0x2588 => fill(list, x0, y0, w, h, color),
                0x2589..=0x258F => {
                    let part = e((0x2590 - n) as f32);
                    fill(list, x0, y0, w * part, h, color);
                }
                0x2590 => fill(list, x0 + w * 0.5, y0, w * 0.5, h, color),
                0x2591..=0x2593 => {
                    let a = 0.25 * (n - 0x2590) as f32;
                    fill(list, x0, y0, w, h, color.with_alpha(color.a * a));
                }
                0x2594 => fill(list, x0, y0, w, h / 8.0, color),
                0x2595 => fill(list, x0 + w * 7.0 / 8.0, y0, w / 8.0, h, color),
                _ => {
                    // Quadrants: upper left, upper right, lower left, lower right.
                    let q: [bool; 4] = match n {
                        0x2596 => [false, false, true, false],
                        0x2597 => [false, false, false, true],
                        0x2598 => [true, false, false, false],
                        0x2599 => [true, false, true, true],
                        0x259A => [true, false, false, true],
                        0x259B => [true, true, true, false],
                        0x259C => [true, true, false, true],
                        0x259D => [false, true, false, false],
                        0x259E => [false, true, true, false],
                        _ => [false, true, true, true],
                    };
                    let (hw, hh) = (w * 0.5, h * 0.5);
                    for (i, on) in q.iter().enumerate() {
                        if *on {
                            let (dx, dy) = (if i % 2 == 1 { hw } else { 0.0 }, if i >= 2 { hh } else { 0.0 });
                            fill(list, x0 + dx, y0 + dy, hw, hh, color);
                        }
                    }
                }
            }
            true
        }
        _ => {
            let Some([up, right, down, left]) = arms(c) else { return false };
            let light = snap((w / 8.0).max(1.0 / scale), scale).max(1.0 / scale);
            let thick = |weight: u8| if weight == 2 { light * 2.0 } else { light };
            let cx = snap(x0 + w * 0.5 - light * 0.5, scale) + light * 0.5;
            let cy = snap(y0 + h * 0.5 - light * 0.5, scale) + light * 0.5;
            // How far arms reach past the center, so they meet the crossing lines.
            let reach_v = [up, down]
                .iter()
                .map(|a| if *a == 3 { light * 1.5 } else { thick(*a) * 0.5 })
                .fold(0.0, f32::max);
            let reach_h = [left, right]
                .iter()
                .map(|a| if *a == 3 { light * 1.5 } else { thick(*a) * 0.5 })
                .fold(0.0, f32::max);
            let horizontal = |list: &mut DisplayList, from: f32, to: f32, weight: u8| {
                if weight == 3 {
                    for off in [-light, light] {
                        fill(list, from, cy + off - light * 0.5, to - from, light, color);
                    }
                } else if weight > 0 {
                    let t = thick(weight);
                    fill(list, from, cy - t * 0.5, to - from, t, color);
                }
            };
            let vertical = |list: &mut DisplayList, from: f32, to: f32, weight: u8| {
                if weight == 3 {
                    for off in [-light, light] {
                        fill(list, cx + off - light * 0.5, from, light, to - from, color);
                    }
                } else if weight > 0 {
                    let t = thick(weight);
                    fill(list, cx - t * 0.5, from, t, to - from, color);
                }
            };
            horizontal(list, x0, cx + reach_v, left);
            horizontal(list, cx - reach_v, x0 + w, right);
            vertical(list, y0, cy + reach_h, up);
            vertical(list, cy - reach_h, y0 + h, down);
            true
        }
    }
}
