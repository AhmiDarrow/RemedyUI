//! Sorting and selection for tables and trees.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::hash::Hash;
use std::rc::Rc;

use remedy_ui_core::input::Modifiers;
use remedy_ui_core::reactive::Signal;

/// What a column sorts by, extracted from a row so sorting can run off the UI thread.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SortValue {
    /// Text, compared the way people expect: case-insensitively, with digit runs as numbers
    /// (`file2` before `file10`).
    Text(String),
    /// A number.
    Number(f64),
    /// No value: sorts after every value either way.
    Missing,
}

/// Which way a column sorts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: ascending or descending.
pub enum SortDirection {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

/// Compares text naturally: case-insensitive, digit runs by value, then exact text to break
/// ties so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (n0, m0) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let ord = n0.len().cmp(&m0.len()).then_with(|| n0.cmp(m0));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn compare(a: &SortValue, b: &SortValue) -> Ordering {
    match (a, b) {
        (SortValue::Text(x), SortValue::Text(y)) => natural_cmp(x, y),
        (SortValue::Number(x), SortValue::Number(y)) => x.total_cmp(y),
        (SortValue::Number(_), SortValue::Text(_)) => Ordering::Less,
        (SortValue::Text(_), SortValue::Number(_)) => Ordering::Greater,
        (SortValue::Missing, SortValue::Missing) => Ordering::Equal,
        (SortValue::Missing, _) => Ordering::Greater,
        (_, SortValue::Missing) => Ordering::Less,
    }
}

/// The row order for `values` sorted `dir`: indices into `values`. Stable, and missing
/// values stay last in both directions.
pub fn sorted_order(values: &[SortValue], dir: SortDirection) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|&i, &j| {
        let (a, b) =
            (values.get(i).unwrap_or(&SortValue::Missing), values.get(j).unwrap_or(&SortValue::Missing));
        match (a, b) {
            (SortValue::Missing, SortValue::Missing) => Ordering::Equal,
            (SortValue::Missing, _) => Ordering::Greater,
            (_, SortValue::Missing) => Ordering::Less,
            _ => match dir {
                SortDirection::Ascending => compare(a, b),
                SortDirection::Descending => compare(b, a),
            },
        }
    });
    idx
}

/// Above this many rows, sorting runs on a worker thread and the result arrives later.
pub const SORT_OFF_THREAD: usize = 20_000;

/// Which rows are selected, the keyboard cursor, and the anchor for Shift ranges.
#[derive(Clone)]
pub struct Selection<K: Clone + Eq + Hash + 'static> {
    /// The selected keys, in the order they were chosen.
    pub selected: Signal<Vec<K>>,
    /// The row the keyboard is on.
    pub cursor: Signal<Option<K>>,
    anchor: Rc<RefCell<Option<K>>>,
    /// More than one row can be selected.
    pub multiple: bool,
}

impl<K: Clone + Eq + Hash + 'static> std::fmt::Debug for Selection<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Selection(multiple {})", self.multiple)
    }
}

impl<K: Clone + Eq + Hash + 'static> Selection<K> {
    /// A selection of one row at a time, or of many.
    pub fn new(multiple: bool) -> Self {
        Selection {
            selected: Signal::new(Vec::new()),
            cursor: Signal::new(None),
            anchor: Rc::default(),
            multiple,
        }
    }

    /// True when `k` is selected (tracked).
    pub fn contains(&self, k: &K) -> bool {
        self.selected.with(|s| s.contains(k))
    }

    /// Selects only `k` and puts the cursor and anchor there.
    pub fn select_only(&self, k: K) {
        self.selected.set(vec![k.clone()]);
        *self.anchor.borrow_mut() = Some(k.clone());
        self.cursor.set(Some(k));
    }

    /// A click or a keyboard move to `k` among `order` (the rows as shown): Ctrl (Command on
    /// Apple platforms, given as `toggle`) adds or removes it, Shift selects the range from
    /// the anchor, otherwise it alone is selected.
    pub fn choose(&self, k: K, mods: Modifiers, toggle: bool, order: &[K]) {
        if self.multiple && mods.shift {
            let anchor = self.anchor.borrow().clone().unwrap_or_else(|| k.clone());
            let a = order.iter().position(|x| *x == anchor);
            let b = order.iter().position(|x| *x == k);
            if let (Some(a), Some(b)) = (a, b) {
                let (lo, hi) = (a.min(b), a.max(b));
                self.selected.set(order.get(lo..=hi).map(<[K]>::to_vec).unwrap_or_default());
            }
            self.cursor.set(Some(k));
        } else if self.multiple && toggle {
            self.selected.update(|s| {
                if let Some(i) = s.iter().position(|x| *x == k) {
                    s.remove(i);
                } else {
                    s.push(k.clone());
                }
            });
            *self.anchor.borrow_mut() = Some(k.clone());
            self.cursor.set(Some(k));
        } else {
            self.select_only(k);
        }
    }

    /// Selects every row of `order`.
    pub fn all(&self, order: &[K]) {
        if self.multiple {
            self.selected.set(order.to_vec());
        }
    }

    /// Selects nothing.
    pub fn clear(&self) {
        self.selected.set(Vec::new());
    }

    /// Drops keys no longer among `present` (rows that were removed).
    pub fn retain(&self, present: &std::collections::HashSet<K>) {
        self.selected.update(|s| s.retain(|k| present.contains(k)));
        if self.cursor.get_untracked().ok().flatten().is_some_and(|c| !present.contains(&c)) {
            self.cursor.set(None);
        }
    }
}
