//! The sparkline: a small line of recent values.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Point;
use remedy_ui_paint::{Brush, Item, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

use crate::series::{PlotBox, Range, Samples, extent, plot};

/// How the sparkline looks.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SparklineOptions {
    /// The span of values shown. Auto fits the values on screen.
    pub range: Range,
    /// How many values the width stands for. `None` uses the source's capacity (a
    /// [`History`](crate::History)), or else however many values there are.
    pub window: Option<usize>,
    /// Shade the area under the line.
    pub fill: bool,
    /// Mark the newest value with a dot.
    pub dot: bool,
    /// The line's width.
    pub line_width: f32,
    /// The width. `None` fills the available width.
    pub width: Option<f32>,
    /// The height. `None` takes it from the theme.
    pub height: Option<f32>,
}

impl SparklineOptions {
    /// Sets `range`: the span of values shown.
    pub fn range(mut self, value: Range) -> Self {
        self.range = value;
        self
    }

    /// Sets `window`: how many values the width stands for.
    pub fn window(mut self, value: usize) -> Self {
        self.window = Some(value);
        self
    }

    /// Sets `fill`: shade the area under the line.
    pub fn fill(mut self, value: bool) -> Self {
        self.fill = value;
        self
    }

    /// Sets `dot`: mark the newest value with a dot.
    pub fn dot(mut self, value: bool) -> Self {
        self.dot = value;
        self
    }

    /// Sets `line_width`: the line's width.
    pub fn line_width(mut self, value: f32) -> Self {
        self.line_width = value;
        self
    }

    /// Sets `width`: the width.
    pub fn width(mut self, value: f32) -> Self {
        self.width = Some(value);
        self
    }

    /// Sets `height`: the height.
    pub fn height(mut self, value: f32) -> Self {
        self.height = Some(value);
        self
    }
}

impl Default for SparklineOptions {
    fn default() -> Self {
        SparklineOptions {
            range: Range::Auto,
            window: None,
            fill: true,
            dot: true,
            line_width: 1.5,
            width: None,
            height: None,
        }
    }
}

/// A small line chart of `values`, newest on the right. It repaints only when the values
/// change. Screen readers hear `label` and the newest value.
#[track_caller]
pub fn sparkline<S: Samples + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    values: Signal<S>,
    options: SparklineOptions,
) -> El {
    let label = label.into();
    let unit = ui.theme().spacing.unit;
    let height = options.height.unwrap_or(unit * 8.0);
    let el = ui.el("sparkline").height(height);
    let el = match options.width {
        Some(w) => el.width(w),
        None => el.fill_width(),
    };
    let SparklineOptions { range, window, fill, dot, line_width, .. } = options;
    let el = el.semantics(Semantics::new(Role::Image).label(label.clone())).paint(move |cx| {
        // Room so the stroke and the dot are not cut at the edges.
        let pad = if dot { dot_radius(line_width) } else { line_width / 2.0 };
        let drawn = values.try_with(|s| {
            let (lowest, highest) = extent(s)?;
            let range = range.resolve(lowest, highest);
            let window = window.or_else(|| s.capacity()).unwrap_or_else(|| s.len());
            let area =
                PlotBox { width: (cx.size.width - 2.0 * pad).max(0.0), height: cx.size.height, margin: pad };
            let runs = plot(s, window, range, area);
            let newest = s.len().checked_sub(1).and_then(|i| s.get(i)).is_some_and(f32::is_finite);
            Some((runs, newest))
        });
        let Ok(Some((runs, newest))) = drawn else { return };
        let color = cx.style.foreground;
        let bottom = cx.size.height;
        let moved: Vec<Vec<Point>> =
            runs.into_iter().map(|r| r.into_iter().map(|p| Point::new(p.x + pad, p.y)).collect()).collect();
        for run in &moved {
            let (Some(first), Some(last)) = (run.first(), run.last()) else { continue };
            if fill && run.len() > 1 {
                let mut area = Path::new();
                area.move_to(Point::new(first.x, bottom));
                for p in run {
                    area.line_to(*p);
                }
                area.line_to(Point::new(last.x, bottom));
                area.close();
                cx.list.push(Item::FillPath {
                    path: area,
                    brush: Brush::Solid(color.with_alpha(0.18)),
                    even_odd: false,
                });
            }
            if run.len() > 1 {
                let mut line = Path::new();
                for (i, p) in run.iter().enumerate() {
                    if i == 0 {
                        line.move_to(*p);
                    } else {
                        line.line_to(*p);
                    }
                }
                let stroke = Stroke {
                    width: line_width,
                    cap: StrokeCap::Round,
                    join: StrokeJoin::Round,
                    dashes: Vec::new(),
                };
                cx.list.push(Item::StrokePath { path: line, stroke, brush: Brush::Solid(color) });
            }
        }
        if dot
            && newest
            && let Some(p) = moved.last().and_then(|r| r.last())
        {
            cx.list.push(Item::FillPath {
                path: Path::circle(*p, dot_radius(line_width)),
                brush: Brush::Solid(color),
                even_odd: false,
            });
        }
    });

    // Screen readers hear the newest value, and only when it changes.
    let heard = Rc::new(RefCell::new(String::new()));
    let sem_el = el.clone();
    el.bind(move || {
        let latest = values.with(|s| s.len().checked_sub(1).and_then(|i| s.get(i)).filter(|v| v.is_finite()));
        let text = latest.map_or_else(|| "no data".to_owned(), |v| format!("latest {v:.2}"));
        if *heard.borrow() != text {
            let _ = sem_el
                .clone()
                .semantics(Semantics::new(Role::Image).label(label.clone()).value(text.clone()));
            *heard.borrow_mut() = text;
        }
    })
}

/// The newest-value dot's radius for a line width.
fn dot_radius(line_width: f32) -> f32 {
    (line_width * 1.6).max(2.0)
}
