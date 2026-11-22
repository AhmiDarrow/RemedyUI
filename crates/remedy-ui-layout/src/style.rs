//! Layout styles in RemedyUI's own terms, translated to taffy in one place.

use remedy_ui_core::Insets;
use taffy::style_helpers::{fr, length, max_content, min_content, minmax, percent};

/// How an element lays out its children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Display {
    /// A row or column (CSS flexbox).
    #[default]
    Flex,
    /// Rows and columns of tracks (CSS grid).
    Grid,
    /// Children stacked in block flow (documents).
    Block,
    /// Not laid out and takes no space.
    None,
}

/// The main axis of a flex container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: a box lays out along one of two axes.
pub enum Axis {
    /// Children side by side, in the text direction.
    #[default]
    Row,
    /// Children top to bottom.
    Column,
}

/// A length.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum Length {
    /// Decided by content and the container.
    #[default]
    Auto,
    /// Logical pixels.
    Px(f32),
    /// A fraction of the container (0.0 to 1.0).
    Percent(f32),
    /// Advances of the digit "0" in the element's own font (CSS `ch`): `Ch(60.0)` is a measure
    /// of about sixty characters. Resolved with the node's [`LayoutStyle::ch`], which the view
    /// sets from the element's text style.
    Ch(f32),
}

/// Alignment of children across the main axis, or of one child.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Align {
    /// At the start.
    Start,
    /// Centered.
    Center,
    /// At the end.
    End,
    /// Stretched to fill.
    #[default]
    Stretch,
    /// First baselines lined up.
    Baseline,
}

/// Distribution of children along the main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Justify {
    /// Packed at the start.
    #[default]
    Start,
    /// Packed in the center.
    Center,
    /// Packed at the end.
    End,
    /// Equal space between children.
    SpaceBetween,
    /// Equal space around children.
    SpaceAround,
    /// Equal space between children and at the edges.
    SpaceEvenly,
}

/// A grid track's size.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Track {
    /// Logical pixels.
    Px(f32),
    /// A fraction of the container.
    Percent(f32),
    /// A share of the remaining space.
    Fr(f32),
    /// As small as the content allows.
    MinContent,
    /// As large as the content wants.
    MaxContent,
    /// Between two sizes.
    MinMax(Box<Track>, Box<Track>),
    /// The tracks repeated: a number of times, or as many times as fit (CSS `repeat()`), as
    /// in `Repeat(Repeat::AutoFill, vec![MinMax(Px(180), Fr(1))])` for a poster grid. Only at
    /// the top level of [`LayoutStyle::columns`] or [`LayoutStyle::rows`]; nested, it is `auto`.
    Repeat(Repeat, Vec<Track>),
}

/// How many times [`Track::Repeat`] repeats its tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Repeat {
    /// Exactly this many times.
    Count(u16),
    /// As many times as fit the container, keeping empty tracks (CSS `auto-fill`).
    AutoFill,
    /// As many times as fit, collapsing tracks with no items (CSS `auto-fit`).
    AutoFit,
}

/// How an element is positioned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Position {
    /// In the flow of its parent.
    #[default]
    Relative,
    /// Taken out of the flow and placed by its insets within the parent.
    Absolute,
    /// Laid out in the flow, then kept by the compositor within its nearest scroll container
    /// (the window when there is none) while that scrolls, as CSS `position: sticky`: each
    /// inset side not [`AUTO`] is how close the box keeps to that edge of the scroll
    /// container's viewport, and it never leaves its parent's content box. Scrolling moves it
    /// with no new layout. The insets default to zero on every side: set the sides that should
    /// not stick to [`AUTO`] (a header that sticks to the top: `top: 0.0`, the rest `AUTO`).
    Sticky,
}

/// Text direction for layout: start and end follow it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: text runs left to right or right to left.
pub enum Direction {
    /// The parent's direction (left to right at the root), as CSS `direction` inherits.
    #[default]
    Inherit,
    /// Left to right.
    Ltr,
    /// Right to left: rows, padding, margins, insets and alignment mirror.
    Rtl,
}

/// An element's layout style. All horizontal properties are logical (start and end), so a
/// right-to-left layout mirrors with no other change.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayoutStyle {
    /// Layout mode.
    pub display: Display,
    /// Direction of start and end.
    pub direction: Direction,
    /// Main axis (flex).
    pub axis: Axis,
    /// Reverse the main axis (flex).
    pub reverse: bool,
    /// Wrap onto more lines (flex).
    pub wrap: bool,
    /// Width.
    pub width: Length,
    /// Height.
    pub height: Length,
    /// Minimum width.
    pub min_width: Length,
    /// Minimum height.
    pub min_height: Length,
    /// Maximum width.
    pub max_width: Length,
    /// Maximum height.
    pub max_height: Length,
    /// Width divided by height, when one side is free.
    pub aspect_ratio: Option<f32>,
    /// Space inside the edges.
    pub padding: Insets,
    /// Space outside the edges.
    pub margin: Insets,
    /// Space between children: (between columns, between rows).
    pub gap: (f32, f32),
    /// Share of free space this child takes (flex).
    pub grow: f32,
    /// Share of overflow this child gives up (flex).
    pub shrink: f32,
    /// Starting size before growing or shrinking (flex).
    pub basis: Length,
    /// Cross-axis alignment of children.
    pub align_items: Align,
    /// This child's own cross-axis alignment.
    pub align_self: Option<Align>,
    /// Main-axis distribution of children.
    pub justify: Justify,
    /// Positioning.
    pub position: Position,
    /// Offsets for absolute positioning, or the edges a sticky box keeps to: logical (`start` is the right edge in a right-to-left
    /// parent), unless `inset_physical`.
    pub inset: Insets,
    /// The insets are physical: `start` is the left edge and `end` the right whatever the
    /// direction, for a box placed at a point (an overlay at a window point, a selection
    /// band at a content point) while its own content reads right to left.
    pub inset_physical: bool,
    // (A side set to [`AUTO`] is decided by the box's size and its other sides.)
    /// Column tracks (grid).
    pub columns: Vec<Track>,
    /// Row tracks (grid).
    pub rows: Vec<Track>,
    /// In a grid: the column line it starts on (1 is the first) and how many columns it
    /// spans; `None` places it automatically.
    pub grid_column: Option<(i16, u16)>,
    /// In a grid: the row line it starts on and how many rows it spans.
    pub grid_row: Option<(i16, u16)>,
    /// The advance of "0" in the element's font, in logical pixels: what one
    /// [`Length::Ch`] is. The view sets it from the element's text style; 0 until then.
    pub ch: f32,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        LayoutStyle {
            display: Display::Flex,
            direction: Direction::Inherit,
            axis: Axis::Row,
            reverse: false,
            wrap: false,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            aspect_ratio: None,
            padding: Insets::ZERO,
            margin: Insets::ZERO,
            gap: (0.0, 0.0),
            grow: 0.0,
            shrink: 1.0,
            basis: Length::Auto,
            align_items: Align::Stretch,
            align_self: None,
            justify: Justify::Start,
            position: Position::Relative,
            inset: Insets::ZERO,
            inset_physical: false,
            columns: Vec::new(),
            rows: Vec::new(),
            grid_column: None,
            grid_row: None,
            ch: 0.0,
        }
    }
}

impl LayoutStyle {
    /// A row.
    pub fn row() -> Self {
        LayoutStyle::default()
    }

    /// A column.
    pub fn column() -> Self {
        LayoutStyle { axis: Axis::Column, ..LayoutStyle::default() }
    }

    /// A fixed-size box.
    pub fn fixed(width: f32, height: f32) -> Self {
        LayoutStyle {
            width: Length::Px(width),
            height: Length::Px(height),
            shrink: 0.0,
            ..LayoutStyle::default()
        }
    }
}

fn dim(l: Length, ch: f32) -> taffy::Dimension {
    match l {
        Length::Auto => taffy::Dimension::auto(),
        Length::Px(v) => taffy::Dimension::length(v),
        Length::Percent(v) => taffy::Dimension::percent(v),
        Length::Ch(v) => taffy::Dimension::length(v * ch),
    }
}

fn lpa(l: Length, ch: f32) -> taffy::LengthPercentageAuto {
    match l {
        Length::Auto => taffy::LengthPercentageAuto::auto(),
        Length::Px(v) => taffy::LengthPercentageAuto::length(v),
        Length::Percent(v) => taffy::LengthPercentageAuto::percent(v),
        Length::Ch(v) => taffy::LengthPercentageAuto::length(v * ch),
    }
}

fn track(t: &Track) -> taffy::TrackSizingFunction {
    match t {
        Track::Px(v) => length(*v),
        Track::Percent(v) => percent(*v),
        Track::Fr(v) => fr(*v),
        Track::MinContent => min_content(),
        Track::MaxContent => max_content(),
        Track::MinMax(a, b) => minmax(track(a).min, track(b).max),
        Track::Repeat(..) => taffy::style_helpers::auto(),
    }
}

fn component<S: taffy::style::CheapCloneStr>(t: &Track) -> taffy::GridTemplateComponent<S> {
    match t {
        Track::Repeat(n, tracks) => {
            taffy::GridTemplateComponent::Repeat(taffy::style::GridTemplateRepetition {
                count: match n {
                    Repeat::Count(c) => taffy::style::RepetitionCount::Count((*c).max(1)),
                    Repeat::AutoFill => taffy::style::RepetitionCount::AutoFill,
                    Repeat::AutoFit => taffy::style::RepetitionCount::AutoFit,
                },
                tracks: tracks.iter().map(track).collect(),
                line_names: Vec::new(),
            })
        }
        t => taffy::GridTemplateComponent::Single(track(t)),
    }
}

fn align(a: Align) -> taffy::AlignItems {
    match a {
        Align::Start => taffy::AlignItems::START,
        Align::Center => taffy::AlignItems::CENTER,
        Align::End => taffy::AlignItems::END,
        Align::Stretch => taffy::AlignItems::STRETCH,
        Align::Baseline => taffy::AlignItems::BASELINE,
    }
}

/// Logical insets to taffy's physical left and right (mirrored by the caller for RTL).
fn rect_lp(i: Insets) -> taffy::Rect<taffy::LengthPercentage> {
    taffy::Rect {
        left: taffy::LengthPercentage::length(i.start),
        right: taffy::LengthPercentage::length(i.end),
        top: taffy::LengthPercentage::length(i.top),
        bottom: taffy::LengthPercentage::length(i.bottom),
    }
}

fn rect_lpa(i: Insets) -> taffy::Rect<taffy::LengthPercentageAuto> {
    let side = |v: f32| {
        if v.is_nan() { taffy::LengthPercentageAuto::auto() } else { taffy::LengthPercentageAuto::length(v) }
    };
    taffy::Rect { left: side(i.start), right: side(i.end), top: side(i.top), bottom: side(i.bottom) }
}

/// An inset side left to the layout ([`LayoutStyle::inset`]): an absolute box pinned by its
/// other sides, sized by its content.
pub const AUTO: f32 = f32::NAN;

/// A grid item from its start line and span.
fn placement((start, span): (i16, u16)) -> taffy::Line<taffy::GridPlacement> {
    taffy::Line { start: taffy::style_helpers::line(start), end: taffy::style_helpers::span(span.max(1)) }
}

/// `s` as a taffy style: `rtl` is the node's resolved direction, which its padding, margins
/// and children follow; `parent_rtl` its parent's, which its insets follow (as CSS resolves
/// an absolute box's logical insets against its containing block), so a box placed at a
/// window point stays there whatever direction its own content reads in.
pub(crate) fn to_taffy(s: &LayoutStyle, rtl: bool, parent_rtl: bool) -> taffy::Style {
    let phys = |i: Insets| if rtl { i.mirrored() } else { i };
    let inset_phys = |i: Insets| if parent_rtl && !s.inset_physical { i.mirrored() } else { i };
    taffy::Style {
        display: match s.display {
            Display::Flex => taffy::Display::Flex,
            Display::Grid => taffy::Display::Grid,
            Display::Block => taffy::Display::Block,
            Display::None => taffy::Display::None,
        },
        direction: if rtl { taffy::Direction::Rtl } else { taffy::Direction::Ltr },
        flex_direction: match (s.axis, s.reverse) {
            (Axis::Row, false) => taffy::FlexDirection::Row,
            (Axis::Row, true) => taffy::FlexDirection::RowReverse,
            (Axis::Column, false) => taffy::FlexDirection::Column,
            (Axis::Column, true) => taffy::FlexDirection::ColumnReverse,
        },
        flex_wrap: if s.wrap { taffy::FlexWrap::Wrap } else { taffy::FlexWrap::NoWrap },
        size: taffy::Size { width: dim(s.width, s.ch), height: dim(s.height, s.ch) },
        min_size: taffy::Size { width: lpa(s.min_width, s.ch), height: lpa(s.min_height, s.ch) },
        max_size: taffy::Size { width: lpa(s.max_width, s.ch), height: lpa(s.max_height, s.ch) },
        aspect_ratio: s.aspect_ratio,
        padding: rect_lp(phys(s.padding)),
        margin: rect_lpa(phys(s.margin)),
        gap: taffy::Size {
            width: taffy::LengthPercentage::length(s.gap.0),
            height: taffy::LengthPercentage::length(s.gap.1),
        },
        flex_grow: s.grow,
        flex_shrink: s.shrink,
        flex_basis: dim(s.basis, s.ch),
        align_items: align(s.align_items),
        align_self: s.align_self.map(align),
        justify_content: match s.justify {
            Justify::Start => taffy::JustifyContent::START,
            Justify::Center => taffy::JustifyContent::CENTER,
            Justify::End => taffy::JustifyContent::END,
            Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
        },
        position: match s.position {
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
            // In the flow, unshifted: the compositor moves it as its scroll container scrolls.
            Position::Sticky => taffy::Position::Relative,
        },
        inset: if s.position == Position::Sticky {
            rect_lpa(Insets::all(AUTO))
        } else {
            rect_lpa(inset_phys(s.inset))
        },
        grid_template_columns: s.columns.iter().map(component).collect(),
        grid_template_rows: s.rows.iter().map(component).collect(),
        grid_column: s.grid_column.map_or_else(taffy::Line::default, placement),
        grid_row: s.grid_row.map_or_else(taffy::Line::default, placement),
        ..taffy::Style::default()
    }
}
