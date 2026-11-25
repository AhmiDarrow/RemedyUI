//! Breaking music into lines, shared by the staves.

use std::ops::Range;

/// Splits measures into lines (systems). `widths[i]` is how wide measure `i` wants to be,
/// `header` is the room each line spends on its clef and key, `first_extra` is more room the
/// first line needs (a time signature), and `available` is the line's full width. A line takes
/// as many measures as fit, and at least one.
pub fn break_systems(widths: &[f32], header: f32, first_extra: f32, available: f32) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < widths.len() {
        let extra = if out.is_empty() { first_extra } else { 0.0 };
        let mut used = header + extra;
        let mut end = start;
        while end < widths.len() {
            let w = widths.get(end).copied().unwrap_or(0.0);
            if end > start && used + w > available {
                break;
            }
            used += w;
            end += 1;
        }
        out.push(start..end);
        start = end;
    }
    out
}

/// How much to stretch the measures of a line so it fills `available`, given what it wants.
/// A line that is full stretches to its edges; the last line of a piece keeps its natural width.
pub fn stretch(used: f32, header: f32, available: f32, last: bool) -> f32 {
    let body = used - header;
    if last || body <= 0.0 || used >= available { 1.0 } else { (available - header) / body }
}

/// Where a cursor goes when asked to move through `counts` (the number of events in each
/// measure): the next or previous event, crossing measures, or `None` at the ends.
pub fn step_event(counts: &[usize], at: (usize, usize), forward: bool) -> Option<(usize, usize)> {
    let (m, e) = at;
    let len = counts.get(m).copied().unwrap_or(0);
    if forward {
        if e + 1 < len {
            return Some((m, e + 1));
        }
        (m + 1..counts.len()).find(|i| counts.get(*i).copied().unwrap_or(0) > 0).map(|i| (i, 0))
    } else {
        if e > 0 {
            return Some((m, e - 1));
        }
        (0..m)
            .rev()
            .find(|i| counts.get(*i).copied().unwrap_or(0) > 0)
            .map(|i| (i, counts.get(i).copied().unwrap_or(1) - 1))
    }
}

/// The first event of the next or previous measure that has events.
pub fn step_measure(counts: &[usize], at: (usize, usize), forward: bool) -> Option<(usize, usize)> {
    let m = at.0;
    if forward {
        (m + 1..counts.len()).find(|i| counts.get(*i).copied().unwrap_or(0) > 0).map(|i| (i, 0))
    } else {
        (0..m).rev().find(|i| counts.get(*i).copied().unwrap_or(0) > 0).map(|i| (i, 0))
    }
}

/// The first and the last event of everything.
pub fn ends(counts: &[usize]) -> Option<((usize, usize), (usize, usize))> {
    let first = counts.iter().position(|c| *c > 0)?;
    let last = counts.iter().rposition(|c| *c > 0)?;
    Some(((first, 0), (last, counts.get(last).copied().unwrap_or(1) - 1)))
}

/// Groups events that are joined by beams: runs of notes with flags that fall in the same
/// beat. `items` are each event's length in quarter notes, its number of flags and whether it
/// is a rest. Returns the event ranges, each at least two events long.
pub fn beam_runs(items: &[(f32, u8, bool)], beat: f32) -> Vec<Range<usize>> {
    let mut groups = Vec::new();
    let mut at = 0.0f32;
    let mut run: Option<(usize, i32)> = None;
    fn close(run: &mut Option<(usize, i32)>, end: usize, groups: &mut Vec<Range<usize>>) {
        if let Some((s, _)) = run.take()
            && end - s >= 2
        {
            groups.push(s..end);
        }
    }
    for (i, (quarters, flags, rest)) in items.iter().enumerate() {
        let beat_index = ((at + 1e-4) / beat).floor() as i32;
        if *rest || *flags == 0 {
            close(&mut run, i, &mut groups);
        } else {
            match run {
                Some((_, b)) if b == beat_index => {}
                _ => {
                    close(&mut run, i, &mut groups);
                    run = Some((i, beat_index));
                }
            }
        }
        at += quarters;
    }
    close(&mut run, items.len(), &mut groups);
    groups
}
