//! The board document: areas on a column grid, widgets in the areas, saved as versioned JSON
//! that peers receive and render. Every change is an operation that checks the board stays
//! whole (no area outside the grid, no two areas on the same cells).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The format this crate writes; older formats are read and brought up to date.
pub const FORMAT: u32 = 1;

/// A widget placed in an area: its kind (a registry id) and its settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    /// Unique on the board.
    pub id: String,
    /// The registry id of its kind.
    pub kind: String,
    /// Its settings, by key.
    #[serde(default)]
    pub settings: Map<String, Value>,
}

/// An area of the board: a titled place on the grid holding widgets top to bottom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Area {
    /// Unique on the board.
    pub id: String,
    /// Its title (may be empty).
    #[serde(default)]
    pub title: String,
    /// Its first column (0-based) and how many it spans.
    pub col: u32,
    /// Columns spanned (at least 1).
    pub cols: u32,
    /// Its first row (0-based).
    pub row: u32,
    /// Rows spanned (at least 1).
    pub rows: u32,
    /// Its widgets, top to bottom.
    #[serde(default)]
    pub widgets: Vec<Placed>,
}

impl Area {
    fn overlaps(&self, other: &Area) -> bool {
        // A board from a peer may hold any numbers: their sums are taken where they fit.
        let end = |at: u32, span: u32| u64::from(at) + u64::from(span);
        u64::from(self.col) < end(other.col, other.cols)
            && u64::from(other.col) < end(self.col, self.cols)
            && u64::from(self.row) < end(other.row, other.rows)
            && u64::from(other.row) < end(self.row, self.rows)
    }
}

/// A board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Board {
    /// The format it was written in.
    pub version: u32,
    /// Columns of the grid.
    pub columns: u32,
    /// The areas.
    pub areas: Vec<Area>,
}

impl Default for Board {
    /// An empty board of 12 columns.
    fn default() -> Self {
        Board { version: FORMAT, columns: 12, areas: Vec::new() }
    }
}

/// Why a document or a change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardError {
    /// Not a board (what is wrong).
    Invalid(String),
    /// Written by a newer version of the format.
    TooNew(u32),
    /// It would put an area outside the grid.
    OutsideGrid,
    /// It would put two areas on the same cells.
    Overlap(String),
    /// No such area or widget.
    Missing(String),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Invalid(e) => write!(f, "not a board: {e}"),
            BoardError::TooNew(v) => write!(f, "this board was made by a newer version (format {v})"),
            BoardError::OutsideGrid => f.write_str("that would go past the edge of the board"),
            BoardError::Overlap(a) => write!(f, "that would cover the area {a}"),
            BoardError::Missing(id) => write!(f, "{id} is not on the board"),
        }
    }
}

impl std::error::Error for BoardError {}

/// The last row an area may end on: the grid the board is laid out on numbers its lines in
/// 16 bits, and a row past them cannot be placed (nor a board that tall be laid out).
const LAST_ROW: u64 = i16::MAX as u64 - 1;

/// A fresh id with `prefix` that the board does not use yet.
fn fresh(board: &Board, prefix: &str) -> String {
    let used = |id: &str| board.areas.iter().any(|a| a.id == id || a.widgets.iter().any(|w| w.id == id));
    (1..).map(|n| format!("{prefix}{n}")).find(|id| !used(id)).unwrap_or_else(|| prefix.to_owned())
}

impl Board {
    /// Reads a board, bringing older formats up to date and checking it is whole.
    pub fn from_json(text: &str) -> Result<Board, BoardError> {
        let mut v: Value = serde_json::from_str(text).map_err(|e| BoardError::Invalid(e.to_string()))?;
        let version = v.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version > u64::from(FORMAT) {
            return Err(BoardError::TooNew(u32::try_from(version).unwrap_or(u32::MAX)));
        }
        // Format 0 (drafts before this crate) had no version and called areas "sections".
        if version == 0
            && let Some(o) = v.as_object_mut()
        {
            if let Some(s) = o.remove("sections") {
                o.insert("areas".into(), s);
            }
            o.insert("version".into(), Value::from(FORMAT));
            o.entry("columns").or_insert(Value::from(12));
        }
        let b: Board = serde_json::from_value(v).map_err(|e| BoardError::Invalid(e.to_string()))?;
        b.check()?;
        Ok(Board { version: FORMAT, ..b })
    }

    /// The board as JSON, in the current format.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Board { version: FORMAT, ..self.clone() }).unwrap_or_default()
    }

    /// Checks the board is whole: columns in range, ids unique, areas inside the grid (its
    /// columns, and the rows a grid can number) and apart.
    pub fn check(&self) -> Result<(), BoardError> {
        if self.columns == 0 || self.columns > 48 {
            return Err(BoardError::Invalid(format!("{} columns", self.columns)));
        }
        let mut ids = std::collections::HashSet::new();
        for (i, a) in self.areas.iter().enumerate() {
            let right = u64::from(a.col) + u64::from(a.cols);
            let bottom = u64::from(a.row) + u64::from(a.rows);
            if a.cols == 0 || a.rows == 0 || right > u64::from(self.columns) || bottom > LAST_ROW {
                return Err(BoardError::OutsideGrid);
            }
            if !ids.insert(a.id.clone()) {
                return Err(BoardError::Invalid(format!("the id {} is used twice", a.id)));
            }
            for w in &a.widgets {
                if !ids.insert(w.id.clone()) {
                    return Err(BoardError::Invalid(format!("the id {} is used twice", w.id)));
                }
            }
            if let Some(o) = self.areas.iter().skip(i + 1).find(|o| o.overlaps(a)) {
                return Err(BoardError::Overlap(o.id.clone()));
            }
        }
        Ok(())
    }

    /// Rows the board spans (the lowest area's end).
    pub fn rows(&self) -> u32 {
        self.areas.iter().map(|a| a.row.saturating_add(a.rows)).max().unwrap_or(0)
    }

    /// The area `id`.
    pub fn area(&self, id: &str) -> Option<&Area> {
        self.areas.iter().find(|a| a.id == id)
    }

    /// The area holding widget `id`, and the widget's place in it.
    pub fn widget_place(&self, id: &str) -> Option<(usize, usize)> {
        self.areas
            .iter()
            .enumerate()
            .find_map(|(ai, a)| a.widgets.iter().position(|w| w.id == id).map(|wi| (ai, wi)))
    }

    /// The widget `id`.
    pub fn widget(&self, id: &str) -> Option<&Placed> {
        self.widget_place(id).and_then(|(a, w)| self.areas.get(a).and_then(|a| a.widgets.get(w)))
    }

    fn with(&self, f: impl FnOnce(&mut Board) -> Result<(), BoardError>) -> Result<Board, BoardError> {
        let mut b = self.clone();
        f(&mut b)?;
        b.check()?;
        Ok(b)
    }

    /// Adds an area at the first free place of its size, below what is there if nothing
    /// above fits; returns the new board and the area's id.
    pub fn add_area(&self, title: &str, cols: u32, rows: u32) -> Result<(Board, String), BoardError> {
        let cols = cols.clamp(1, self.columns);
        let rows = rows.max(1);
        let id = fresh(self, "area");
        for row in 0..=self.rows() {
            for col in 0..=(self.columns - cols) {
                let a = Area {
                    id: id.clone(),
                    title: title.to_owned(),
                    col,
                    cols,
                    row,
                    rows,
                    widgets: Vec::new(),
                };
                if !self.areas.iter().any(|o| o.overlaps(&a)) {
                    let b = self.with(|b| {
                        b.areas.push(a);
                        Ok(())
                    })?;
                    return Ok((b, id));
                }
            }
        }
        Err(BoardError::OutsideGrid)
    }

    /// Moves and sizes area `id` (column, row, columns, rows).
    pub fn place_area(
        &self,
        id: &str,
        col: u32,
        row: u32,
        cols: u32,
        rows: u32,
    ) -> Result<Board, BoardError> {
        let moved = Area {
            id: id.to_owned(),
            title: String::new(),
            col,
            cols: cols.max(1),
            row,
            rows: rows.max(1),
            widgets: Vec::new(),
        };
        if u64::from(col) + u64::from(moved.cols) > u64::from(self.columns) {
            return Err(BoardError::OutsideGrid);
        }
        // The error names the area that would be covered.
        if let Some(o) = self.areas.iter().find(|o| o.id != id && o.overlaps(&moved)) {
            return Err(BoardError::Overlap(o.id.clone()));
        }
        self.with(|b| {
            let a = b.areas.iter_mut().find(|a| a.id == id).ok_or_else(|| BoardError::Missing(id.into()))?;
            (a.col, a.row, a.cols, a.rows) = (col, row, cols.max(1), rows.max(1));
            Ok(())
        })
    }

    /// Renames area `id`.
    pub fn retitle(&self, id: &str, title: &str) -> Result<Board, BoardError> {
        self.with(|b| {
            let a = b.areas.iter_mut().find(|a| a.id == id).ok_or_else(|| BoardError::Missing(id.into()))?;
            a.title = title.to_owned();
            Ok(())
        })
    }

    /// Removes area `id` and its widgets.
    pub fn remove_area(&self, id: &str) -> Result<Board, BoardError> {
        self.with(|b| {
            let n = b.areas.len();
            b.areas.retain(|a| a.id != id);
            if b.areas.len() == n { Err(BoardError::Missing(id.into())) } else { Ok(()) }
        })
    }

    /// Adds a widget of `kind` with `settings` at the end of area `area`; returns its id.
    pub fn add_widget(
        &self,
        area: &str,
        kind: &str,
        settings: Map<String, Value>,
    ) -> Result<(Board, String), BoardError> {
        let id = fresh(self, "widget");
        let b = self.with(|b| {
            let a =
                b.areas.iter_mut().find(|a| a.id == area).ok_or_else(|| BoardError::Missing(area.into()))?;
            a.widgets.push(Placed { id: id.clone(), kind: kind.to_owned(), settings });
            Ok(())
        })?;
        Ok((b, id))
    }

    /// Moves widget `id` into area `to` at `index` (clamped to its end).
    pub fn move_widget(&self, id: &str, to: &str, index: usize) -> Result<Board, BoardError> {
        self.with(|b| {
            let (ai, wi) = b.widget_place(id).ok_or_else(|| BoardError::Missing(id.into()))?;
            let w = b
                .areas
                .get_mut(ai)
                .map(|a| a.widgets.remove(wi))
                .ok_or_else(|| BoardError::Missing(id.into()))?;
            let dest =
                b.areas.iter_mut().find(|a| a.id == to).ok_or_else(|| BoardError::Missing(to.into()))?;
            let at = index.min(dest.widgets.len());
            dest.widgets.insert(at, w);
            Ok(())
        })
    }

    /// Sets one setting of widget `id`.
    pub fn set_setting(&self, id: &str, key: &str, value: Value) -> Result<Board, BoardError> {
        self.with(|b| {
            let (ai, wi) = b.widget_place(id).ok_or_else(|| BoardError::Missing(id.into()))?;
            let w = b
                .areas
                .get_mut(ai)
                .and_then(|a| a.widgets.get_mut(wi))
                .ok_or_else(|| BoardError::Missing(id.into()))?;
            w.settings.insert(key.to_owned(), value);
            Ok(())
        })
    }

    /// Removes widget `id`.
    pub fn remove_widget(&self, id: &str) -> Result<Board, BoardError> {
        self.with(|b| {
            let (ai, wi) = b.widget_place(id).ok_or_else(|| BoardError::Missing(id.into()))?;
            if let Some(a) = b.areas.get_mut(ai) {
                a.widgets.remove(wi);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Map;

    use super::*;

    fn board() -> Board {
        let (b, _) = Board::default().add_area("Welcome", 8, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, _) = b.add_area("Links", 4, 1).unwrap_or_else(|e| unreachable!("{e}"));
        b
    }

    #[test]
    fn areas_fill_free_places_and_never_overlap() {
        let b = board();
        let w = b.area("area1").cloned().unwrap_or_else(|| unreachable!());
        let l = b.area("area2").cloned().unwrap_or_else(|| unreachable!());
        assert_eq!((w.col, w.row, l.col, l.row), (0, 0, 8, 0), "side by side on the first row");
        let (b, id) = b.add_area("News", 12, 2).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(b.area(&id).map(|a| (a.col, a.row)), Some((0, 1)), "a full-width one goes below");
        assert_eq!(b.place_area("area2", 6, 0, 4, 1), Err(BoardError::Overlap("area1".into())));
        assert_eq!(b.place_area("area2", 10, 0, 4, 1), Err(BoardError::OutsideGrid));
        assert!(b.place_area("area2", 8, 3, 4, 1).is_ok());
    }

    #[test]
    fn widgets_move_between_areas_and_keep_their_settings() {
        let (b, w1) = board().add_widget("area1", "text", Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        let (b, w2) = b.add_widget("area1", "links", Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        let b = b.set_setting(&w1, "title", "Hi".into()).unwrap_or_else(|e| unreachable!("{e}"));
        let b = b.move_widget(&w1, "area2", 0).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(b.widget_place(&w1), Some((1, 0)));
        assert_eq!(b.widget(&w1).and_then(|w| w.settings.get("title")).cloned(), Some("Hi".into()));
        let b = b.move_widget(&w2, "area1", 99).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(b.widget_place(&w2), Some((0, 0)));
        assert!(matches!(b.move_widget(&w2, "nowhere", 0), Err(BoardError::Missing(_))));
        let b = b.remove_area("area2").unwrap_or_else(|e| unreachable!("{e}"));
        assert!(b.widget(&w1).is_none(), "an area's widgets go with it");
    }

    #[test]
    fn documents_round_trip_and_older_ones_are_brought_up_to_date() {
        let (b, _) = board().add_widget("area1", "text", Map::new()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(Board::from_json(&b.to_json()), Ok(b));
        let old = r#"{"columns": 12, "sections": [{"id": "s1", "col": 0, "cols": 6, "row": 0, "rows": 1}]}"#;
        let up = Board::from_json(old).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!((up.version, up.areas.len()), (FORMAT, 1));
        assert_eq!(
            Board::from_json(r#"{"version": 99, "columns": 12, "areas": []}"#),
            Err(BoardError::TooNew(99))
        );
        let bad = r#"{"version": 1, "columns": 4, "areas": [{"id": "a", "col": 0, "cols": 3, "row": 0, "rows": 1}, {"id": "b", "col": 2, "cols": 2, "row": 0, "rows": 1}]}"#;
        assert_eq!(Board::from_json(bad), Err(BoardError::Overlap("b".into())));
        assert!(matches!(Board::from_json("[]"), Err(BoardError::Invalid(_))));
    }

    #[test]
    fn a_board_with_numbers_past_the_grid_is_refused_not_wrapped() {
        let far = |col: u64, cols: u64, row: u64, rows: u64| {
            format!(
                r#"{{"version": 1, "columns": 12, "areas": [
                    {{"id": "a", "col": 0, "cols": 6, "row": 0, "rows": 1}},
                    {{"id": "b", "col": {col}, "cols": {cols}, "row": {row}, "rows": {rows}}}]}}"#
            )
        };
        let max = u64::from(u32::MAX);
        // Sums that would wrap around to a place on the grid.
        assert_eq!(Board::from_json(&far(max, 2, 1, 1)), Err(BoardError::OutsideGrid));
        assert_eq!(Board::from_json(&far(6, 6, max, 2)), Err(BoardError::OutsideGrid));
        // A row no grid line reaches.
        assert_eq!(Board::from_json(&far(6, 6, 4_000_000_000, 1)), Err(BoardError::OutsideGrid));
        assert!(Board::from_json(&far(6, 6, 30_000, 1)).is_ok());
    }
}
