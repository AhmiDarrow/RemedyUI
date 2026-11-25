//! Line diffs: the fewest lines removed and added to turn one text into another (Myers'
//! algorithm), for diff views.

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffLine {
    /// In both: the line, its number in the old text and in the new one (from 1).
    Same(String, usize, usize),
    /// Only in the old text.
    Removed(String, usize),
    /// Only in the new text.
    Added(String, usize),
}

/// The most edit steps searched for a shortest diff. The search remembers a row per step
/// (about `MAX_EDITS`² numbers at worst, 32 MB), so texts further apart get a coarse diff.
const MAX_EDITS: usize = 2000;

/// The lines of `old` and `new` as a diff, in order: removals before the additions that
/// replace them. Texts more than 2000 changed lines apart (after the lines they start and
/// end with in common) show that middle as wholly removed, then wholly added.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = if old.is_empty() { Vec::new() } else { old.split('\n').collect() };
    let b: Vec<&str> = if new.is_empty() { Vec::new() } else { new.split('\n').collect() };
    // The lines both start and end with are the same in a shortest diff: search the rest.
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a.iter().skip(pre).rev().zip(b.iter().skip(pre).rev()).take_while(|(x, y)| x == y).count();
    let (a_end, b_end) = (a.len() - suf, b.len() - suf);
    let mid_a = a.get(pre..a_end).unwrap_or_default();
    let mid_b = b.get(pre..b_end).unwrap_or_default();
    let mut out = Vec::with_capacity(a.len().max(b.len()));
    out.extend(a.iter().take(pre).enumerate().map(|(i, s)| DiffLine::Same((*s).to_owned(), i + 1, i + 1)));
    if !shortest(mid_a, mid_b, pre, &mut out) {
        out.extend(mid_a.iter().enumerate().map(|(i, s)| DiffLine::Removed((*s).to_owned(), pre + i + 1)));
        out.extend(mid_b.iter().enumerate().map(|(i, s)| DiffLine::Added((*s).to_owned(), pre + i + 1)));
    }
    let tail = a.iter().skip(a_end).enumerate();
    out.extend(tail.map(|(i, s)| DiffLine::Same((*s).to_owned(), a_end + i + 1, b_end + i + 1)));
    out
}

/// Appends the shortest diff of `a` and `b` (Myers' greedy search) to `out`, numbering lines
/// after the `pre` lines before them; `false`, with nothing appended, when it would take more
/// than [`MAX_EDITS`] steps.
fn shortest(a: &[&str], b: &[&str], pre: usize, out: &mut Vec<DiffLine>) -> bool {
    let (n, m) = (a.len(), b.len());
    let max = (n + m).min(MAX_EDITS);
    // `v` holds diagonals -max-1..=max+1; a row of `trace` holds -d..=d of its step d.
    let get = |v: &[usize], k: isize, off: isize| {
        usize::try_from(k + off).ok().and_then(|i| v.get(i)).copied().unwrap_or(0)
    };
    let off = max as isize + 1;
    let mut v = vec![0usize; 2 * max + 3];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    let mut found = false;
    'search: for d in 0..=max as isize {
        let mut k = -d;
        while k <= d {
            let down = k == -d || (k != d && get(&v, k - 1, off) < get(&v, k + 1, off));
            let mut x = if down { get(&v, k + 1, off) } else { get(&v, k - 1, off) + 1 };
            let mut y = (x as isize - k).max(0) as usize;
            while x < n && y < m && a.get(x) == b.get(y) {
                x += 1;
                y += 1;
            }
            if let Some(slot) = v.get_mut((k + off) as usize) {
                *slot = x;
            }
            if x >= n && y >= m {
                found = true;
                break 'search;
            }
            k += 2;
        }
        trace.push(v.get((off - d) as usize..=(off + d) as usize).unwrap_or_default().to_vec());
    }
    if !found {
        return false;
    }
    // Back from the end: each step was one removal or addition after a run of equal lines.
    let mut back = Vec::new();
    let (mut x, mut y) = (n, m);
    let same = |back: &mut Vec<DiffLine>, x: usize, y: usize| {
        back.push(DiffLine::Same(a.get(x).copied().unwrap_or("").to_owned(), pre + x + 1, pre + y + 1));
    };
    for d in (1..=trace.len()).rev() {
        let prev = trace.get(d - 1).map(Vec::as_slice).unwrap_or_default();
        let (di, po) = (d as isize, d as isize - 1);
        let k = x as isize - y as isize;
        let down = k == -di || (k != di && get(prev, k - 1, po) < get(prev, k + 1, po));
        let pk = if down { k + 1 } else { k - 1 };
        let px = get(prev, pk, po);
        let py = (px as isize - pk).max(0) as usize;
        while x > px && y > py {
            x -= 1;
            y -= 1;
            same(&mut back, x, y);
        }
        if down {
            y -= 1;
            back.push(DiffLine::Added(b.get(y).copied().unwrap_or("").to_owned(), pre + y + 1));
        } else {
            x -= 1;
            back.push(DiffLine::Removed(a.get(x).copied().unwrap_or("").to_owned(), pre + x + 1));
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        same(&mut back, x, y);
    }
    out.extend(back.into_iter().rev());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use DiffLine::*;

    fn apply(d: &[DiffLine]) -> (String, String) {
        let old: Vec<&str> = d
            .iter()
            .filter_map(|l| match l {
                Same(s, ..) | Removed(s, _) => Some(s.as_str()),
                Added(..) => None,
            })
            .collect();
        let new: Vec<&str> = d
            .iter()
            .filter_map(|l| match l {
                Same(s, ..) | Added(s, _) => Some(s.as_str()),
                Removed(..) => None,
            })
            .collect();
        (old.join("\n"), new.join("\n"))
    }

    #[test]
    fn a_change_in_the_middle() {
        let d = diff_lines("a\nb\nc\nd", "a\nB\nc\nd\ne");
        assert_eq!(
            d,
            [
                Same("a".into(), 1, 1),
                Removed("b".into(), 2),
                Added("B".into(), 2),
                Same("c".into(), 3, 3),
                Same("d".into(), 4, 4),
                Added("e".into(), 5),
            ]
        );
    }

    #[test]
    fn diffs_rebuild_both_texts_and_are_minimal() {
        let cases = [
            ("", ""),
            ("", "x\ny"),
            ("x\ny", ""),
            ("same\nsame", "same\nsame"),
            ("a\nb\nc\na\nb\nb\na", "c\nb\na\nb\na\nc"),
            ("fn main() {\n    a();\n}\n", "fn main() {\n    b();\n    a();\n}\n"),
        ];
        for (old, new) in cases {
            let d = diff_lines(old, new);
            assert_eq!(apply(&d), (old.to_owned(), new.to_owned()), "{old:?} -> {new:?}");
        }
        // The classic example: 5 edits.
        let d = diff_lines("a\nb\nc\na\nb\nb\na", "c\nb\na\nb\na\nc");
        assert_eq!(d.iter().filter(|l| !matches!(l, Same(..))).count(), 5);
    }

    #[test]
    fn far_apart_texts_get_a_coarse_diff_between_what_they_share() {
        let lines = |tag: &str| (0..1100).map(|i| format!("{tag}{i}")).collect::<Vec<_>>().join("\n");
        let old = format!("top\n{}\nend", lines("old"));
        let new = format!("top\n{}\nmore\nend", lines("new"));
        let d = diff_lines(&old, &new);
        assert_eq!(apply(&d), (old, new));
        assert_eq!(d.first(), Some(&Same("top".into(), 1, 1)));
        assert_eq!(d.last(), Some(&Same("end".into(), 1102, 1103)));
        assert_eq!(d.get(1), Some(&Removed("old0".into(), 2)));
        assert_eq!(d.get(1101), Some(&Added("new0".into(), 2)));
    }

    #[test]
    fn a_small_change_in_a_long_text_is_found_between_its_shared_ends() {
        let body = (0..5000).map(|i| i.to_string()).collect::<Vec<_>>();
        let mut changed = body.clone();
        changed.insert(2500, "new".into());
        let d = diff_lines(&body.join("\n"), &changed.join("\n"));
        let edits: Vec<&DiffLine> = d.iter().filter(|l| !matches!(l, Same(..))).collect();
        assert_eq!(edits, [&Added("new".into(), 2501)]);
        assert_eq!(d.last(), Some(&Same("4999".into(), 5000, 5001)));
    }
}
