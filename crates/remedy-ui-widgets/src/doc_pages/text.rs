//! Page text: selection across runs and pages, copying, and search.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use remedy_ui_core::{Point, Rect};

use crate::doc_pages::source::PageText;

/// A place in a document's text: between two characters of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextPos {
    /// The page.
    pub page: usize,
    /// The run on the page.
    pub run: usize,
    /// Characters before the place, in the run.
    pub offset: usize,
}

/// A search hit: the text from `start` to `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Match {
    /// Where it starts.
    pub start: TextPos,
    /// Where it ends (after its last character).
    pub end: TextPos,
}

impl Match {
    /// The page it is on.
    pub fn page(&self) -> usize {
        self.start.page
    }
}

/// Where a search is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchStatus {
    /// What is searched for (empty: no search).
    pub query: String,
    /// Hits found so far.
    pub matches: usize,
    /// The hit shown, counting from 0.
    pub current: Option<usize>,
    /// Pages searched so far.
    pub searched: usize,
    /// True once every page was searched.
    pub done: bool,
}

/// What the view knows about page text.
#[derive(Default)]
pub(crate) struct TextState {
    pub(crate) pages: HashMap<usize, Option<Arc<PageText>>>,
    pub(crate) pending: HashSet<usize>,
    /// Anchor and focus of the selection.
    pub(crate) selection: Option<(TextPos, TextPos)>,
    pub(crate) search: Search,
}

#[derive(Default)]
pub(crate) struct Search {
    pub(crate) query: Vec<char>,
    pub(crate) text: String,
    pub(crate) generation: u64,
    pub(crate) matches: Vec<Match>,
    pub(crate) current: Option<usize>,
    pub(crate) searched: usize,
    pub(crate) done: bool,
    pub(crate) cancel: Option<crate::doc_pages::source::Cancel>,
}

/// One character, folded for a search that ignores case (one character in, one out, so
/// positions map back).
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// The query as searched for.
pub(crate) fn fold_query(q: &str) -> Vec<char> {
    q.chars().map(fold).collect()
}

/// Every hit of `query` on a page. Runs join with a line break, so a hit can cross runs
/// that read on (a phrase wrapped onto the next line matches when the query has a space or
/// line break there).
pub(crate) fn find_on_page(page: usize, text: &PageText, query: &[char]) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut flat: Vec<char> = Vec::new();
    let mut at: Vec<(usize, usize)> = Vec::new();
    for (r, run) in text.runs.iter().enumerate() {
        for (i, c) in run.text.chars().enumerate() {
            flat.push(fold(c));
            at.push((r, i));
        }
        flat.push(' ');
        at.push((r, run.len()));
    }
    // Any white space in the text matches any white space in the query.
    let space = |c: char| c.is_whitespace();
    let eq = |a: char, b: char| a == b || (space(a) && space(b));
    let mut out = Vec::new();
    let n = query.len();
    let mut i = 0;
    while i + n <= flat.len() {
        let hit = flat.get(i..i + n).is_some_and(|w| w.iter().zip(query).all(|(a, b)| eq(*a, *b)));
        if hit {
            let s = at.get(i).copied().unwrap_or((0, 0));
            let e = at.get(i + n - 1).copied().unwrap_or(s);
            out.push(Match {
                start: TextPos { page, run: s.0, offset: s.1 },
                end: TextPos { page, run: e.0, offset: e.1 + 1 },
            });
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// The run and character boundary nearest `p` (page points) on a page.
pub(crate) fn hit(page: usize, text: &PageText, p: Point) -> Option<TextPos> {
    let mut best: Option<(f32, usize)> = None;
    for (r, run) in text.runs.iter().enumerate() {
        let b = run.bounds;
        // Distance to the run's box, vertical distance counting most: a press beside a line
        // belongs to that line.
        let dx = (b.min.x - p.x).max(p.x - b.max.x).max(0.0);
        let dy = (b.min.y - p.y).max(p.y - b.max.y).max(0.0);
        let d = dy * 4.0 + dx;
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, r));
        }
    }
    let (_, r) = best?;
    let run = text.runs.get(r)?;
    Some(TextPos { page, run: r, offset: run.boundary_at(p) })
}

/// The boxes (page points) of the characters from `a` to `b` on `page`.
pub(crate) fn rects(page: usize, text: &PageText, a: TextPos, b: TextPos) -> Vec<Rect> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    if page < a.page || page > b.page {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (r, run) in text.runs.iter().enumerate() {
        let here = TextPos { page, run: r, offset: 0 };
        let end_here = TextPos { page, run: r, offset: run.len() };
        if end_here <= a || here >= b {
            continue;
        }
        let from = if a.page == page && a.run == r { a.offset } else { 0 };
        let to = if b.page == page && b.run == r { b.offset } else { run.len() };
        if to <= from {
            continue;
        }
        let first = run.char_rect(from);
        let last = run.char_rect(to - 1);
        out.push(first.union(last));
    }
    out
}

/// The text from `a` to `b` across pages whose text is known; runs on separate lines join
/// with a line break, runs on one line with a space, pages with a blank line.
pub(crate) fn between(pages: &HashMap<usize, Option<Arc<PageText>>>, a: TextPos, b: TextPos) -> String {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let mut out = String::new();
    for page in a.page..=b.page {
        let Some(Some(text)) = pages.get(&page) else { continue };
        if page != a.page && !out.is_empty() {
            out.push_str("\n\n");
        }
        let mut prev: Option<Rect> = None;
        for (r, run) in text.runs.iter().enumerate() {
            let here = TextPos { page, run: r, offset: 0 };
            let end_here = TextPos { page, run: r, offset: run.len() };
            if end_here <= a || here >= b {
                continue;
            }
            let from = if a.page == page && a.run == r { a.offset } else { 0 };
            let to = if b.page == page && b.run == r { b.offset } else { run.len() };
            if let Some(p) = prev {
                let same_line = (p.center().y - run.bounds.center().y).abs() < run.bounds.height() / 2.0;
                out.push(if same_line { ' ' } else { '\n' });
            }
            out.push_str(&run.slice(from..to));
            prev = Some(run.bounds);
        }
    }
    out
}

/// A page's whole text, for screen readers.
pub(crate) fn whole(text: &PageText) -> String {
    let mut pages = HashMap::new();
    pages.insert(0, Some(Arc::new(text.clone())));
    let end = text.runs.len().checked_sub(1).map_or(TextPos::default(), |r| TextPos {
        page: 0,
        run: r,
        offset: text.runs.get(r).map_or(0, |run| run.len()),
    });
    between(&pages, TextPos::default(), end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_pages::source::TextRun;

    fn page() -> PageText {
        PageText {
            runs: vec![
                TextRun::new("Hello world", Rect::new(10.0, 10.0, 120.0, 22.0)),
                TextRun::new("second line", Rect::new(10.0, 30.0, 120.0, 42.0)),
            ],
        }
    }

    #[test]
    fn search_ignores_case_and_crosses_lines() {
        let t = page();
        let hits = find_on_page(3, &t, &fold_query("WORLD"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits.first().map(|m| (m.start.offset, m.end.offset)), Some((6, 11)));
        let across = find_on_page(3, &t, &fold_query("world second"));
        assert_eq!(across.len(), 1, "a phrase wrapped onto the next line is found");
        assert_eq!(across.first().map(|m| (m.start.run, m.end.run)), Some((0, 1)));
        assert!(find_on_page(3, &t, &[]).is_empty());
    }

    #[test]
    fn selection_hits_rects_and_text() {
        let t = page();
        let a = hit(0, &t, Point::new(10.0, 15.0)).unwrap_or_default();
        let b = hit(0, &t, Point::new(52.0, 35.0)).unwrap_or_default();
        assert_eq!((a.run, a.offset), (0, 0));
        assert_eq!((b.run, b.offset), (1, 4), "the boundary nearest the point");
        let mut pages = HashMap::new();
        pages.insert(0, Some(Arc::new(t.clone())));
        assert_eq!(between(&pages, b, a), "Hello world\nseco", "either direction");
        assert_eq!(rects(0, &t, a, b).len(), 2);
        assert_eq!(whole(&t), "Hello world\nsecond line");
    }
}
