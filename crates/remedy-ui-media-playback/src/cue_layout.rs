//! Where cues go on the picture, by WebVTT's rules for rendering cues: each cue's box from
//! its `position`, `size` and `align`, its lines wrapped to the box, its `line` placing it
//! down the picture (or across it, for vertical text), and then, from step 10 of "apply
//! WebVTT cue settings" on, moved so it overlaps no cue placed before it and stays in the
//! title area (the picture, above the bar). ASS cues come as the same settings, so cues of
//! one alignment stack as ASS's collision handling stacks them: bottom ones upward, top ones
//! downward.

use remedy_ui_core::{Rect, Size};

use crate::subtitles::{Cue, CueAlign, CueAnchor, CueLine, Vertical};

/// One line of a laid-out cue: its text and its box (the text and its padding), on the
/// picture. A vertical cue's line runs down its box, one upright character after another.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlacedLine {
    pub text: String,
    pub rect: Rect,
}

/// A laid-out cue.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlacedCue {
    pub lines: Vec<PlacedLine>,
    pub vertical: Option<Vertical>,
}

impl PlacedCue {
    /// The box around all its lines.
    #[cfg(test)]
    pub fn bounds(&self) -> Rect {
        let mut it = self.lines.iter().map(|l| l.rect);
        let first = it.next().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        it.fold(first, |a, r| {
            Rect::new(a.min.x.min(r.min.x), a.min.y.min(r.min.y), a.max.x.max(r.max.x), a.max.y.max(r.max.y))
        })
    }
}

/// The picture shown in `area` at `aspect` (width over height), as big as fits and
/// centred (`VideoFit::Contain`): WebVTT's video viewport. All of `area` without an aspect.
pub(crate) fn viewport(area: Size, aspect: Option<f32>) -> Rect {
    let full = Rect::new(0.0, 0.0, area.width, area.height);
    let Some(a) = aspect.filter(|a| a.is_finite() && *a > 0.0) else { return full };
    if area.width <= 0.0 || area.height <= 0.0 {
        return full;
    }
    let (w, h) = if area.width / area.height > a {
        (area.height * a, area.height)
    } else {
        (area.width, area.width / a)
    };
    let (x, y) = ((area.width - w) / 2.0, (area.height - h) / 2.0);
    Rect::new(x, y, x + w, y + h)
}

/// The cue's axes on the picture: inline along its lines, block across them (down for
/// horizontal text, leftward for `rl`, rightward for `lr`).
#[derive(Clone, Copy)]
struct Frame {
    vp: Rect,
    vertical: Option<Vertical>,
}

impl Frame {
    fn inline_len(&self) -> f32 {
        if self.vertical.is_some() { self.vp.height() } else { self.vp.width() }
    }
    fn block_len(&self) -> f32 {
        if self.vertical.is_some() { self.vp.width() } else { self.vp.height() }
    }
    /// The rectangle `i0..i1` along the lines and `b0..b1` across them.
    fn rect(&self, i0: f32, i1: f32, b0: f32, b1: f32) -> Rect {
        let vp = self.vp;
        match self.vertical {
            None => Rect::new(vp.min.x + i0, vp.min.y + b0, vp.min.x + i1, vp.min.y + b1),
            Some(Vertical::Rl) => Rect::new(vp.max.x - b1, vp.min.y + i0, vp.max.x - b0, vp.min.y + i1),
            Some(Vertical::Lr) => Rect::new(vp.min.x + b0, vp.min.y + i0, vp.min.x + b1, vp.min.y + i1),
        }
    }
    /// A rectangle on the picture in this frame's terms: `(i0, i1, b0, b1)`.
    fn ranges(&self, r: Rect) -> (f32, f32, f32, f32) {
        let vp = self.vp;
        match self.vertical {
            None => (r.min.x - vp.min.x, r.max.x - vp.min.x, r.min.y - vp.min.y, r.max.y - vp.min.y),
            Some(Vertical::Rl) => {
                (r.min.y - vp.min.y, r.max.y - vp.min.y, vp.max.x - r.max.x, vp.max.x - r.min.x)
            }
            Some(Vertical::Lr) => {
                (r.min.y - vp.min.y, r.max.y - vp.min.y, r.min.x - vp.min.x, r.max.x - vp.min.x)
            }
        }
    }
}

/// Lays `cues` out in order on the picture `vp`, inside the title area `title`. `measure`
/// gives one line of text's size along its line and across it, set across (`false`) or
/// down (`true`, for vertical cues); `pad` is a line box's padding along and across it.
pub(crate) fn layout(
    cues: &[Cue],
    vp: Rect,
    title: Rect,
    pad: (f32, f32),
    measure: &mut dyn FnMut(&str, bool) -> Size,
) -> Vec<PlacedCue> {
    let mut placed: Vec<PlacedCue> = Vec::new();
    for cue in cues {
        let s = cue.settings;
        let frame = Frame { vp, vertical: s.vertical };
        let (il, bl) = (frame.inline_len(), frame.block_len());
        let (left, size) = s.box_across();
        let (i0, i_len) = (left / 100.0 * il, size / 100.0 * il);
        // The lines, wrapped at spaces to the box (less its padding).
        let room = (i_len - 2.0 * pad.0).max(0.0);
        let mut lines: Vec<(String, Size)> = Vec::new();
        for src in cue.text.lines() {
            wrap(src, room, &mut |t: &str| measure(t, s.vertical.is_some()), &mut lines);
        }
        if lines.is_empty() {
            continue;
        }
        // Each line's box along the line, lined up in the cue's box by `align`.
        let boxes: Vec<(f32, f32, f32)> = lines
            .iter()
            .map(|(_, sz)| {
                let w = (sz.width + 2.0 * pad.0).min(i_len.max(sz.width));
                let start = match s.align {
                    CueAlign::Start | CueAlign::Left => i0,
                    CueAlign::End | CueAlign::Right => i0 + i_len - w,
                    CueAlign::Center => i0 + (i_len - w) / 2.0,
                };
                (start, w, sz.height + 2.0 * pad.1)
            })
            .collect();
        let total: f32 = boxes.iter().map(|b| b.2).sum();
        let line_h = boxes.first().map_or(0.0, |b| b.2).max(1.0);
        let (_, _, t_b0, t_b1) = frame.ranges(title);
        // The lines of cues placed before, in this cue's terms.
        let others: Vec<(f32, f32, f32, f32)> =
            placed.iter().flat_map(|p| p.lines.iter().map(|l| frame.ranges(l.rect))).collect();
        let hits = |b: f32| -> Vec<(f32, f32)> {
            // Each line on its own: a short line beside a long one leaves room.
            let mut at = b;
            let mut out = Vec::new();
            for (start, w, h) in &boxes {
                for o in &others {
                    if *start < o.1 - 0.5 && o.0 < start + w - 0.5 && at < o.3 - 0.5 && o.2 < at + h - 0.5 {
                        out.push((o.2, o.3));
                    }
                }
                at += h;
            }
            out
        };
        let inside = |b: f32| b >= t_b0 - 0.5 && b + total <= t_b1 + 0.5;
        let b = match s.line {
            // Snapped to lines: from the top for 0 on, from the foot for -1 on (`auto` is -1),
            // then stepped a line at a time away from where it started until it fits, turning
            // back once at the title area's edge (step 10's loop).
            None | Some(CueLine::Number(_)) => {
                let n = match s.line {
                    Some(CueLine::Number(n)) => n,
                    _ => -1,
                };
                let (start, mut step) = if n >= 0 {
                    (n as f32 * line_h, line_h)
                } else {
                    // The last line at the foot of the title area (above the bar), as WebVTT's
                    // `auto` lines sit at the foot of what is free.
                    (t_b1.min(bl) + (n + 1) as f32 * line_h - total, -line_h)
                };
                let mut b = start;
                let mut switched = false;
                let mut found = None;
                let tries = (2.0 * bl / line_h) as usize + 4;
                for _ in 0..tries {
                    if inside(b) && hits(b).is_empty() {
                        found = Some(b);
                        break;
                    }
                    let beyond = (step < 0.0 && b < t_b0) || (step > 0.0 && b + total > t_b1);
                    if beyond {
                        if switched {
                            break;
                        }
                        switched = true;
                        step = -step;
                    }
                    b += step;
                }
                found.unwrap_or_else(|| clamp(start, total, t_b0, t_b1))
            }
            // A percentage: the cue's start, middle or end (`line`'s alignment) there; then
            // moved out of the title area's edges, and past overlapped cues toward where its
            // alignment points (a bottom cue up, a top cue down), else the other way.
            Some(CueLine::Percent(p)) => {
                let anchor = match s.line_align.unwrap_or(CueAnchor::Start) {
                    CueAnchor::Start => 0.0,
                    CueAnchor::Center => 0.5,
                    CueAnchor::End => 1.0,
                };
                let start = clamp(p / 100.0 * bl - anchor * total, total, t_b0, t_b1);
                let dirs: [f32; 2] = match s.line_align {
                    Some(CueAnchor::End) => [-1.0, 1.0],
                    Some(CueAnchor::Center) if p > 50.0 => [-1.0, 1.0],
                    _ => [1.0, -1.0],
                };
                let mut best = None;
                for d in dirs {
                    let mut b = start;
                    for _ in 0..placed.len() * cue.text.len().max(1) + 2 {
                        let h = hits(b);
                        if h.is_empty() {
                            if inside(b) {
                                best = Some(b);
                            }
                            break;
                        }
                        b = if d < 0.0 {
                            h.iter().map(|o| o.0).fold(f32::MAX, f32::min) - total
                        } else {
                            h.iter().map(|o| o.1).fold(f32::MIN, f32::max)
                        };
                        if !inside(b) {
                            break;
                        }
                    }
                    if best.is_some() {
                        break;
                    }
                }
                best.unwrap_or(start)
            }
        };
        let mut at = b;
        let lines = lines
            .into_iter()
            .zip(boxes)
            .map(|((text, _), (start, w, h))| {
                let rect = frame.rect(start, start + w, at, at + h);
                at += h;
                PlacedLine { text, rect }
            })
            .collect();
        placed.push(PlacedCue { lines, vertical: s.vertical });
    }
    placed
}

/// `b` moved so `b..b + len` stays within `lo..hi` (at `lo` when it cannot).
fn clamp(b: f32, len: f32, lo: f32, hi: f32) -> f32 {
    b.min(hi - len).max(lo)
}

/// `src` broken at spaces into lines no wider than `room` (a word wider than it on its own
/// line), each with its measured size.
fn wrap(src: &str, room: f32, measure: &mut dyn FnMut(&str) -> Size, out: &mut Vec<(String, Size)>) {
    let mut line = String::new();
    let mut size = Size::new(0.0, 0.0);
    for word in src.split(' ') {
        let next = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        let s = measure(&next);
        if s.width > room && !line.is_empty() {
            out.push((std::mem::take(&mut line), size));
            size = measure(word);
            line = word.to_owned();
        } else {
            line = next;
            size = s;
        }
    }
    if !line.trim().is_empty() {
        out.push((line, size));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitles::CueSettings;

    /// 10 px per character, 20 px high.
    fn measure(t: &str, _: bool) -> Size {
        Size::new(t.chars().count() as f32 * 10.0, 20.0)
    }

    fn cue(text: &str, settings: &str) -> Cue {
        Cue { text: text.into(), settings: CueSettings::parse_webvtt(settings), ..Default::default() }
    }

    fn run(cues: &[Cue], vp: Rect, title: Rect) -> Vec<Rect> {
        layout(cues, vp, title, (0.0, 0.0), &mut measure).iter().map(PlacedCue::bounds).collect()
    }

    const VP: Rect = Rect {
        min: remedy_ui_core::Point { x: 0.0, y: 0.0 },
        max: remedy_ui_core::Point { x: 400.0, y: 200.0 },
    };

    #[test]
    fn the_viewport_is_the_letterboxed_picture() {
        assert_eq!(viewport(Size::new(400.0, 400.0), Some(2.0)), Rect::new(0.0, 100.0, 400.0, 300.0));
        assert_eq!(viewport(Size::new(400.0, 100.0), Some(2.0)), Rect::new(100.0, 0.0, 300.0, 100.0));
        assert_eq!(viewport(Size::new(400.0, 100.0), None), Rect::new(0.0, 0.0, 400.0, 100.0));
    }

    #[test]
    fn auto_cues_stack_up_from_the_foot_and_clear_the_bar() {
        let cues = [cue("one", "align:start"), cue("two", "align:start")];
        // The title area stops 30 px above the foot (the bar).
        let r = run(&cues, VP, Rect::new(0.0, 0.0, 400.0, 170.0));
        assert_eq!(r, [Rect::new(0.0, 150.0, 30.0, 170.0), Rect::new(0.0, 130.0, 30.0, 150.0)]);
    }

    #[test]
    fn line_numbers_snap_and_step_away_from_each_other() {
        let cues = [cue("top", "line:0"), cue("also top", "line:0"), cue("third up", "line:-3")];
        let r = run(&cues, VP, VP);
        assert_eq!(r.first().map(|b| b.min.y), Some(0.0));
        // The second steps down a line; the third sits three lines up from the foot.
        assert_eq!(r.get(1).map(|b| b.min.y), Some(20.0));
        assert_eq!(r.get(2).map(|b| b.min.y), Some(140.0));
    }

    #[test]
    fn a_percentage_line_honours_its_alignment() {
        let two = "a\nb";
        let at = |s: &str| run(&[cue(two, s)], VP, VP).first().map(|b| (b.min.y, b.max.y));
        assert_eq!(at("line:50%,start"), Some((100.0, 140.0)));
        assert_eq!(at("line:50%,center"), Some((80.0, 120.0)));
        assert_eq!(at("line:50%,end"), Some((60.0, 100.0)));
        // Kept inside the title area.
        assert_eq!(at("line:100%"), Some((160.0, 200.0)));
    }

    #[test]
    fn overlapping_percentage_cues_move_the_way_they_point() {
        // Two bottom-aligned cues at the same place (ASS \an2 with a margin): the second goes up.
        let cues = [cue("first", "line:90%,end"), cue("second", "line:90%,end")];
        let r = run(&cues, VP, VP);
        assert_eq!(
            r.iter().map(|b| (b.min.y, b.max.y)).collect::<Vec<_>>(),
            [(160.0, 180.0), (140.0, 160.0)]
        );
        // Two top-aligned: the second goes down.
        let cues = [cue("first", "line:10%"), cue("second", "line:10%")];
        let r = run(&cues, VP, VP);
        assert_eq!(r.iter().map(|b| b.min.y).collect::<Vec<_>>(), [20.0, 40.0]);
        // Side by side they do not collide.
        let cues = [
            cue("l", "line:10% position:0% size:20% align:left"),
            cue("r", "line:10% position:100% size:20% align:right"),
        ];
        assert_eq!(run(&cues, VP, VP).iter().map(|b| b.min.y).collect::<Vec<_>>(), [20.0, 20.0]);
    }

    #[test]
    fn position_size_and_wrapping() {
        // A 50% box from 25%: 200 px from 100; "aaaa bbbb cccc" is 140 px, wrapped to two lines.
        let r = layout(
            &[cue("aaaa bbbbbbbbbbbbbbbb cccc", "line:0 position:25% size:50% align:left")],
            VP,
            VP,
            (0.0, 0.0),
            &mut measure,
        );
        let lines: Vec<(&str, Rect)> =
            r.iter().flat_map(|c| c.lines.iter().map(|l| (l.text.as_str(), l.rect))).collect();
        assert_eq!(
            lines,
            [
                ("aaaa", Rect::new(100.0, 0.0, 140.0, 20.0)),
                ("bbbbbbbbbbbbbbbb", Rect::new(100.0, 20.0, 260.0, 40.0)),
                ("cccc", Rect::new(100.0, 40.0, 140.0, 60.0))
            ]
        );
    }

    #[test]
    fn vertical_cues_run_down_and_stack_across() {
        // rl: lines along the height, the first line at the right; lr: at the left.
        let rl = run(&[cue("abc", "vertical:rl line:0")], VP, VP);
        assert_eq!(rl, [Rect::new(380.0, 85.0, 400.0, 115.0)]);
        let lr = run(&[cue("abc", "vertical:lr line:0 align:start")], VP, VP);
        assert_eq!(lr, [Rect::new(0.0, 0.0, 20.0, 30.0)]);
        // Two rl cues at line 0: the second steps leftward.
        let two = run(&[cue("abc", "vertical:rl line:0"), cue("def", "vertical:rl line:0")], VP, VP);
        assert_eq!(two.get(1).map(|b| b.min.x), Some(360.0));
    }
}
