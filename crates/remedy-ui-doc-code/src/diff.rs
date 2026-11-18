//! Line diffs: the fewest lines removed and added to turn one text into another (Myers'
//! algorithm), for diff views.

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// In both: the line, its number in the old text and in the new one (from 1).
    Same(String, usize, usize),
    /// Only in the old text.
    Removed(String, usize),
    /// Only in the new text.
    Added(String, usize),
}

/// The lines of `old` and `new` as a diff, in order: removals before the additions that
/// replace them.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = if old.is_empty() { Vec::new() } else { old.split('\n').collect() };
    let b: Vec<&str> = if new.is_empty() { Vec::new() } else { new.split('\n').collect() };
    let (n, m) = (a.len(), b.len());
    let max = n + m;
    let off = max as isize;
    let get = |v: &[usize], k: isize| v.get((k + off) as usize).copied().unwrap_or(0);
    // Myers' greedy search: after step d, `trace[d]` holds the furthest x on each diagonal k.
    let mut v = vec![0usize; 2 * max + 2];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    'search: for d in 0..=max as isize {
        let mut k = -d;
        while k <= d {
            let down = k == -d || (k != d && get(&v, k - 1) < get(&v, k + 1));
            let mut x = if down { get(&v, k + 1) } else { get(&v, k - 1) + 1 };
            let mut y = (x as isize - k).max(0) as usize;
            while x < n && y < m && a.get(x) == b.get(y) {
                x += 1;
                y += 1;
            }
            if let Some(slot) = v.get_mut((k + off) as usize) {
                *slot = x;
            }
            if x >= n && y >= m {
                trace.push(v.clone());
                break 'search;
            }
            k += 2;
        }
        trace.push(v.clone());
    }
    // Back from the end: each step was one removal or addition after a run of equal lines.
    let mut out = Vec::new();
    let (mut x, mut y) = (n, m);
    let same = |out: &mut Vec<DiffLine>, x: usize, y: usize| {
        out.push(DiffLine::Same(a.get(x).copied().unwrap_or("").to_owned(), x + 1, y + 1));
    };
    for d in (1..trace.len()).rev() {
        let prev = trace.get(d - 1).map(Vec::as_slice).unwrap_or_default();
        let di = d as isize;
        let k = x as isize - y as isize;
        let down = k == -di || (k != di && get(prev, k - 1) < get(prev, k + 1));
        let pk = if down { k + 1 } else { k - 1 };
        let px = get(prev, pk);
        let py = (px as isize - pk).max(0) as usize;
        while x > px && y > py {
            x -= 1;
            y -= 1;
            same(&mut out, x, y);
        }
        if down {
            y -= 1;
            out.push(DiffLine::Added(b.get(y).copied().unwrap_or("").to_owned(), y + 1));
        } else {
            x -= 1;
            out.push(DiffLine::Removed(a.get(x).copied().unwrap_or("").to_owned(), x + 1));
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        same(&mut out, x, y);
    }
    out.reverse();
    out
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
}
