//! Horizontal padding for span backgrounds.
//!
//! Parley has no padding on a span, so a chip's side room is two inline boxes (U+FFFC) at the
//! ends of each run of text that shares one background. The boxes take part in line breaking,
//! so the fill does not cover the words beside the chip and a chip near the wrap width moves
//! to the next line instead of painting past it. The map sends the shaped text's offsets back
//! to the caller's text: hit tests, carets, selections and glyph ranges stay the original's.

use crate::rich::{InlineObject, RichText};
use crate::style::{TextStyle, finite_px};

/// Id of an inline box this module inserted. Not one of the caller's objects, and not reported
/// back with them. Bit 63 is outside the ids apps use for pictures and mentions.
pub(crate) const SPAN_PAD_ID: u64 = 1 << 63;

/// Offsets of the shaped text (pads inserted) against the text the caller laid out.
#[derive(Clone, Debug)]
pub(crate) struct PadMap {
    /// For each byte of the shaped text, the caller's byte it came from, plus a sentinel at
    /// the end (so an exclusive end offset maps too).
    to_orig: Vec<usize>,
    /// Shaped offset of each caller boundary on the upstream side: before an end pad, with
    /// the character before the boundary.
    upstream: Vec<usize>,
    /// Shaped offset of each caller boundary on the downstream side: after a start pad, with
    /// the character after the boundary.
    downstream: Vec<usize>,
}

impl PadMap {
    /// The caller's offset for a shaped offset.
    pub(crate) fn to_original(&self, padded: usize) -> usize {
        let i = padded.min(self.to_orig.len().saturating_sub(1));
        self.to_orig.get(i).copied().unwrap_or(0)
    }

    /// The shaped offset of a caller's boundary. Upstream sits before an end pad and
    /// downstream after a start pad, so a caret keeps the side of the boundary it belongs to
    /// when a line breaks on the padding.
    pub(crate) fn to_padded(&self, original: usize, upstream: bool) -> usize {
        let side = if upstream { &self.upstream } else { &self.downstream };
        side.get(original).copied().or_else(|| side.last().copied()).unwrap_or(0)
    }
}

/// `text` with inline boxes inserted for span-background padding, or `None` when nothing has
/// horizontal padding (a fill with no side room needs no change to the text).
pub(crate) fn apply(text: &RichText, base: &TextStyle) -> Option<(RichText, PadMap)> {
    let groups = groups(text, base);
    if groups.is_empty() {
        return None;
    }
    let src = text.as_str();
    let mut events: Vec<(usize, bool, f32)> = Vec::with_capacity(groups.len() * 2);
    for g in &groups {
        // `false` sorts first, so an end pad at a boundary is emitted before a start pad there.
        events.push((g.start, true, g.inline));
        events.push((g.end, false, g.inline));
    }
    events.sort_by_key(|e| (e.0, e.1));

    let mut padded = String::with_capacity(src.len() + events.len() * 3);
    let mut to_orig = Vec::with_capacity(src.len() + events.len() * 3 + 1);
    let mut to_pad = vec![0usize; src.len() + 1];
    let mut upstream = vec![0usize; src.len() + 1];
    let mut downstream = vec![0usize; src.len() + 1];
    let mut char_at = vec![0usize; src.len() + 1];
    let mut pads: Vec<(usize, f32)> = Vec::with_capacity(events.len());
    let mut ev = 0usize;
    let mut i = 0usize;
    while i <= src.len() {
        // `i` is a char boundary or `src.len()`, and each vec is `src.len() + 1` long.
        write_at(&mut upstream, i, padded.len());
        while let Some(width) = events.get(ev).filter(|e| e.0 == i && !e.1).map(|e| e.2) {
            emit_pad(&mut padded, &mut to_orig, &mut pads, i, width);
            ev += 1;
        }
        write_at(&mut to_pad, i, padded.len());
        while let Some(width) = events.get(ev).filter(|e| e.0 == i && e.1).map(|e| e.2) {
            emit_pad(&mut padded, &mut to_orig, &mut pads, i, width);
            ev += 1;
        }
        write_at(&mut downstream, i, padded.len());
        if i == src.len() {
            break;
        }
        let Some(ch) = src.get(i..).and_then(|rest| rest.chars().next()) else {
            break;
        };
        let n = ch.len_utf8();
        write_at(&mut char_at, i, padded.len());
        padded.push(ch);
        to_orig.extend(std::iter::repeat_n(i, n));
        i += n;
    }
    to_orig.push(src.len());
    debug_assert_eq!(ev, events.len());
    debug_assert_eq!(to_orig.len(), padded.len() + 1);

    let mut rich = RichText::plain(padded);
    for span in text.spans() {
        let a = to_pad.get(span.range.start).copied().unwrap_or(0);
        let b = to_pad.get(span.range.end).copied().unwrap_or(rich.len());
        if a < b {
            rich.add_span(a..b, span.patch.clone());
        }
    }
    let mut objects: Vec<(usize, InlineObject)> =
        text.objects().iter().map(|&(at, obj)| (char_at.get(at).copied().unwrap_or(at), obj)).collect();
    for (at, width) in pads {
        objects.push((at, InlineObject { id: SPAN_PAD_ID, width, height: 0.0, baseline: Some(0.0) }));
    }
    objects.sort_by_key(|(at, _)| *at);
    rich.set_objects(objects);
    Some((rich, PadMap { to_orig, upstream, downstream }))
}

struct Group {
    start: usize,
    end: usize,
    inline: f32,
}

/// Maximal runs of text that share one background with horizontal padding.
fn groups(text: &RichText, base: &TextStyle) -> Vec<Group> {
    let segs = crate::engine::resolved_backgrounds(text, base);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some((range, bg)) = segs.get(i) {
        let inline = bg.map(|b| finite_px(b.padding_inline)).unwrap_or(0.0);
        if bg.is_none() || inline <= 0.0 {
            i += 1;
            continue;
        }
        let start = range.start;
        let mut end = range.end;
        let want = *bg;
        i += 1;
        while segs.get(i).is_some_and(|(_, next)| *next == want) {
            if let Some((range, _)) = segs.get(i) {
                end = range.end;
            }
            i += 1;
        }
        if start < end {
            out.push(Group { start, end, inline });
        }
    }
    out
}

/// `slots` is sized to every byte of the source plus one, and `i` is a char boundary.
fn write_at(slots: &mut [usize], i: usize, value: usize) {
    if let Some(slot) = slots.get_mut(i) {
        *slot = value;
    }
}

fn emit_pad(
    padded: &mut String,
    to_orig: &mut Vec<usize>,
    pads: &mut Vec<(usize, f32)>,
    at: usize,
    width: f32,
) {
    let index = padded.len();
    padded.push('\u{FFFC}');
    to_orig.extend(std::iter::repeat_n(at, '\u{FFFC}'.len_utf8()));
    pads.push((index, width));
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, reason = "these tests assert on the chip they just built")]
mod tests {
    use remedy_ui_core::Color;

    use super::*;
    use crate::style::{SpanBackground, TextStylePatch};

    fn chip() -> SpanBackground {
        SpanBackground::new(Color::rgb8(40, 40, 48)).padding(4.0, 1.0).radius(3.0)
    }

    #[test]
    fn pads_wrap_the_span_and_offsets_round_trip() {
        let text =
            RichText::new().text("use ").styled("code", TextStylePatch::background(chip())).text(" here");
        let (rich, map) = apply(&text, &TextStyle::default()).expect("the chip has side room");
        let stripped: String = rich.as_str().chars().filter(|c| *c != '\u{FFFC}').collect();
        assert_eq!(stripped, text.as_str());
        assert_eq!(rich.as_str().chars().filter(|c| *c == '\u{FFFC}').count(), 2);
        let span = rich.spans().iter().find(|s| s.patch.background.is_some()).expect("the span");
        let covered = rich.as_str().get(span.range.clone()).unwrap_or("");
        assert_eq!(covered.chars().filter(|c| *c == '\u{FFFC}').count(), 2, "{covered:?}");
        assert!(covered.contains("code"), "{covered:?}");
        let src = text.as_str();
        for (i, _) in src.char_indices().chain(std::iter::once((src.len(), '\0'))) {
            assert_eq!(map.to_original(map.to_padded(i, true)), i, "upstream {i}");
            assert_eq!(map.to_original(map.to_padded(i, false)), i, "downstream {i}");
        }
        // The word's own bytes map back to the caller's "code".
        let code = rich.as_str().find("code").expect("code");
        assert_eq!(map.to_original(code), src.find("code").unwrap_or(0));
        assert_eq!(map.to_original(code + 4), src.find("code").unwrap_or(0) + 4);
    }

    #[test]
    fn the_same_background_on_adjacent_spans_is_one_chip() {
        let bg = chip();
        let text = RichText::new()
            .styled("ab", TextStylePatch::background(bg))
            .styled("cd", TextStylePatch::background(bg));
        let (rich, _) = apply(&text, &TextStyle::default()).expect("pads");
        assert_eq!(rich.as_str().chars().filter(|c| *c == '\u{FFFC}').count(), 2);
    }

    #[test]
    fn different_chips_keep_their_own_side_room() {
        let a = chip();
        let b = SpanBackground::new(Color::rgb8(80, 20, 20)).padding_inline(2.0);
        let text = RichText::new()
            .styled("ab", TextStylePatch::background(a))
            .styled("cd", TextStylePatch::background(b));
        let (rich, _) = apply(&text, &TextStyle::default()).expect("pads");
        assert_eq!(rich.as_str().chars().filter(|c| *c == '\u{FFFC}').count(), 4);
    }

    #[test]
    fn a_fill_with_no_side_room_does_not_rewrite_the_text() {
        let bg = SpanBackground::new(Color::BLACK).radius(2.0).padding_block(1.0);
        let text = RichText::new().styled("code", TextStylePatch::background(bg));
        assert!(apply(&text, &TextStyle::default()).is_none());
    }

    #[test]
    fn a_callers_inline_object_keeps_its_id_beside_a_chip() {
        let mut text = RichText::new().text("a").object(InlineObject {
            id: 3,
            width: 10.0,
            height: 10.0,
            baseline: None,
        });
        text.add_span(0..1, TextStylePatch::background(chip()));
        let (rich, _) = apply(&text, &TextStyle::default()).expect("pads");
        let ids: Vec<u64> = rich.objects().iter().map(|(_, o)| o.id).collect();
        assert!(ids.contains(&3), "{ids:?}");
        assert!(ids.contains(&SPAN_PAD_ID), "{ids:?}");
        assert_eq!(ids.iter().filter(|id| **id == 3).count(), 1);
    }
}
