//! Layout behavior.

use remedy_ui_core::{Insets, Scale};
use remedy_ui_layout::*;

type R = Result<(), LayoutError>;

#[test]
fn rows_grow_gap_and_pad() -> R {
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::row();
        v_.gap = (10.0, 0.0);
        v_.padding = Insets::all(4.0);
        v_
    })?;
    let a = t.node(&LayoutStyle::fixed(50.0, 20.0))?;
    let b = t.node(&{
        let mut v_ = LayoutStyle::fixed(0.0, 20.0);
        v_.grow = 1.0;
        v_
    })?;
    let c = t.node(&{
        let mut v_ = LayoutStyle::fixed(0.0, 20.0);
        v_.grow = 2.0;
        v_
    })?;
    t.set_children(root, &[a, b, c])?;
    t.compute(root, Some(300.0), None)?;
    // 300 - 8 padding - 50 - 20 gaps = 222 free, split 1:2.
    assert_eq!(t.rect(a).map(|r| r.min.x), Some(4.0));
    assert_eq!(t.rect(b).map(|r| (r.min.x, r.width())), Some((64.0, 74.0)));
    assert_eq!(t.rect(c).map(|r| (r.min.x, r.width())), Some((148.0, 148.0)));
    assert_eq!(t.rect(root).map(|r| r.height()), Some(28.0), "content plus padding");
    Ok(())
}

#[test]
fn right_to_left_mirrors_rows_and_logical_insets() -> R {
    let mut t = LayoutTree::new();
    let style = {
        let mut v_ = LayoutStyle::row();
        v_.direction = Direction::Rtl;
        v_.padding = Insets { start: 10.0, ..Insets::ZERO };
        v_
    };
    let root = t.node(&style)?;
    let first = t.node(&LayoutStyle::fixed(40.0, 10.0))?;
    let second = t.node(&LayoutStyle::fixed(40.0, 10.0))?;
    t.set_children(root, &[first, second])?;
    t.compute(root, Some(200.0), None)?;
    // Start is the right edge: the first child sits at the right, inside the start padding.
    assert_eq!(t.rect(first).map(|r| r.max.x), Some(190.0));
    assert_eq!(t.rect(second).map(|r| r.max.x), Some(150.0));

    // Children inherit the direction: a nested row with start padding mirrors too.
    let nested = t.node(&{
        let mut v_ = LayoutStyle::fixed(60.0, 10.0);
        v_.padding = Insets { start: 5.0, ..Insets::ZERO };
        v_
    })?;
    let leaf = t.node(&LayoutStyle::fixed(10.0, 10.0))?;
    t.set_children(nested, &[leaf])?;
    t.set_children(root, &[first, second, nested])?;
    t.compute(root, Some(200.0), None)?;
    assert_eq!(t.rect(leaf).map(|r| r.max.x), Some(55.0), "start padding on the right of the nested row");
    Ok(())
}

#[test]
fn columns_center_and_justify() -> R {
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::fixed(100.0, 100.0).into_column();
        v_.align_items = Align::Center;
        v_.justify = Justify::SpaceBetween;
        v_
    })?;
    let a = t.node(&LayoutStyle::fixed(20.0, 10.0))?;
    let b = t.node(&LayoutStyle::fixed(40.0, 10.0))?;
    t.set_children(root, &[a, b])?;
    t.compute(root, None, None)?;
    assert_eq!(t.rect(a).map(|r| (r.min.x, r.min.y)), Some((40.0, 0.0)));
    assert_eq!(t.rect(b).map(|r| (r.min.x, r.min.y)), Some((30.0, 90.0)));
    Ok(())
}

trait IntoColumn {
    fn into_column(self) -> Self;
}

impl IntoColumn for LayoutStyle {
    fn into_column(self) -> Self {
        {
            let mut v_ = self;
            v_.axis = Axis::Column;
            v_
        }
    }
}

#[test]
fn grid_tracks_share_fractions() -> R {
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::default();
        v_.display = Display::Grid;
        v_.columns = vec![Track::Px(60.0), Track::Fr(1.0), Track::Fr(3.0)];
        v_.gap = (5.0, 0.0);
        v_
    })?;
    let cells: Vec<LayoutNode> = (0..3).map(|_| t.node(&LayoutStyle::default())).collect::<Result<_, _>>()?;
    t.set_children(root, &cells)?;
    t.compute(root, Some(370.0), None)?;
    let widths: Vec<f32> = cells.iter().filter_map(|c| t.rect(*c).map(|r| r.width())).collect();
    // 370 - 60 - 10 gaps = 300, split 1:3.
    assert_eq!(widths, [60.0, 75.0, 225.0]);
    Ok(())
}

#[test]
fn grid_items_sit_on_the_lines_and_spans_they_ask_for() -> R {
    // A 4-column board, 100 px a column and row: one area at column 2 spanning 2, row 1;
    // another at column 1 spanning 4, row 2 spanning 2.
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::default();
        v_.display = Display::Grid;
        v_.columns = vec![Track::Fr(1.0); 4];
        v_.rows = vec![Track::Px(100.0); 3];
        v_
    })?;
    let a = t.node(&{
        let mut v_ = LayoutStyle::default();
        v_.grid_column = Some((2, 2));
        v_.grid_row = Some((1, 1));
        v_
    })?;
    let b = t.node(&{
        let mut v_ = LayoutStyle::default();
        v_.grid_column = Some((1, 4));
        v_.grid_row = Some((2, 2));
        v_
    })?;
    t.set_children(root, &[a, b])?;
    t.compute(root, Some(400.0), None)?;
    let ra = t.rect(a).unwrap_or_default();
    let rb = t.rect(b).unwrap_or_default();
    assert_eq!((ra.min.x, ra.min.y, ra.width(), ra.height()), (100.0, 0.0, 200.0, 100.0));
    assert_eq!((rb.min.x, rb.min.y, rb.width(), rb.height()), (0.0, 100.0, 400.0, 200.0));
    Ok(())
}

#[test]
fn measured_leaves_wrap_like_text() -> R {
    // A "paragraph" 30 px per 100 px of width, needing 400 px on one line.
    let mut t = LayoutTree::new();
    let root = t.node(&LayoutStyle::column())?;
    let text = t.measured(&LayoutStyle::default(), |m: MeasureInput| {
        let width = match (m.known_width, m.available_width) {
            (Some(w), _) => w,
            (None, Available::Px(w)) => w.min(400.0),
            (None, Available::MinContent) => 60.0,
            (None, Available::MaxContent) => 400.0,
        };
        let lines = (400.0 / width.max(1.0)).ceil();
        remedy_ui_core::Size::new(width, lines * 30.0)
    })?;
    t.add_child(root, text)?;
    t.compute(root, Some(200.0), None)?;
    assert_eq!(t.rect(text).map(|r| (r.width(), r.height())), Some((200.0, 60.0)));
    t.compute(root, Some(1000.0), None)?;
    assert_eq!(t.rect(text).map(|r| r.height()), Some(30.0));
    Ok(())
}

#[test]
fn padded_measured_leaves_wrap_inside_their_padding() -> R {
    // The same "paragraph" with 20 px of padding a side, stretched to a 200 px column: it
    // wraps in the 160 px inside the padding (3 lines), not in the 200 px box (2 lines).
    let mut t = LayoutTree::new();
    let root = t.node(&LayoutStyle::column())?;
    let text = t.measured(
        &{
            let mut v_ = LayoutStyle::default();
            v_.padding = Insets::all(20.0);
            v_
        },
        |m: MeasureInput| {
            let width = match (m.known_width, m.available_width) {
                (Some(w), _) => w,
                (None, Available::Px(w)) => w.min(400.0),
                (None, Available::MinContent) => 60.0,
                (None, Available::MaxContent) => 400.0,
            };
            let lines = (400.0 / width.max(1.0)).ceil();
            remedy_ui_core::Size::new(width, lines * 30.0)
        },
    )?;
    t.add_child(root, text)?;
    t.compute(root, Some(200.0), None)?;
    assert_eq!(t.rect(text).map(|r| (r.width(), r.height())), Some((200.0, 3.0 * 30.0 + 40.0)));
    Ok(())
}

#[test]
fn absolute_rects_and_snapping() -> R {
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::column();
        v_.padding = Insets::all(3.3);
        v_
    })?;
    let inner = t.node(&{
        let mut v_ = LayoutStyle::row();
        v_.padding = Insets::all(1.1);
        v_
    })?;
    let leaf = t.node(&LayoutStyle::fixed(10.15, 7.7))?;
    t.set_children(root, &[inner])?;
    t.set_children(inner, &[leaf])?;
    t.compute(root, Some(100.0), None)?;
    let abs = t.absolute_rect(root, leaf).map(|r| r.min.x);
    assert!(abs.is_some_and(|x| (x - 4.4).abs() < 1e-4));
    let scale = Scale::new(1.25).unwrap_or_default();
    let snapped = t.snapped_rect(root, leaf, scale).map(|r| [r.min.x, r.max.x, r.min.y, r.max.y]);
    for edge in snapped.unwrap_or_default() {
        assert_eq!(scale.to_device(edge).fract(), 0.0, "{edge} is on the device grid");
    }
    Ok(())
}

#[test]
fn absolute_insets_follow_the_parent_or_stay_physical() -> R {
    let pinned = |dir: Direction, physical: bool| {
        let mut v_ = LayoutStyle::fixed(20.0, 10.0);
        v_.position = Position::Absolute;
        v_.inset = Insets { top: 0.0, start: 30.0, end: AUTO, bottom: AUTO };
        v_.direction = dir;
        v_.inset_physical = physical;
        v_
    };
    for (parent_rtl, child, physical, left) in [
        // Start is the parent's start: its right edge in a right-to-left parent.
        (true, Direction::Inherit, false, 150.0),
        // A box with its own direction is still placed by its parent's (as CSS does).
        (true, Direction::Ltr, false, 150.0),
        (false, Direction::Rtl, false, 30.0),
        // Physical insets: a window point stays where it is.
        (true, Direction::Inherit, true, 30.0),
        (true, Direction::Rtl, true, 30.0),
    ] {
        let mut t = LayoutTree::new();
        let root = t.node(&{
            let mut v_ = LayoutStyle::fixed(200.0, 100.0);
            v_.direction = if parent_rtl { Direction::Rtl } else { Direction::Ltr };
            v_
        })?;
        let c = t.node(&pinned(child, physical))?;
        t.set_children(root, &[c])?;
        t.compute(root, Some(200.0), Some(100.0))?;
        assert_eq!(t.rect(c).map(|r| r.min.x), Some(left), "{parent_rtl} {child:?} {physical}");
    }
    Ok(())
}

#[test]
fn sticky_boxes_stay_in_the_flow_unshifted() -> R {
    // Sticking is the compositor's: layout places a sticky box where a relative one with no
    // offsets would be, and its siblings follow it as usual.
    let mut t = LayoutTree::new();
    let root = t.node(&LayoutStyle::column())?;
    let first = t.node(&LayoutStyle::fixed(50.0, 10.0))?;
    let sticky = t.node(&{
        let mut v_ = LayoutStyle::fixed(50.0, 20.0);
        v_.position = Position::Sticky;
        v_.inset = Insets { top: 5.0, start: 7.0, end: AUTO, bottom: AUTO };
        v_
    })?;
    let after = t.node(&LayoutStyle::fixed(50.0, 10.0))?;
    t.set_children(root, &[first, sticky, after])?;
    t.compute(root, Some(100.0), Some(100.0))?;
    assert_eq!(t.rect(sticky).map(|r| (r.min.x, r.min.y)), Some((0.0, 10.0)));
    assert_eq!(t.rect(after).map(|r| r.min.y), Some(30.0));
    Ok(())
}

#[test]
fn trees_of_any_depth_lay_out_on_a_threads_stack() -> R {
    // Taffy recurses once per level: thousands of nested flex, grid and block containers lay
    // out on a test thread's stack (2 MB) all the same, each level inset by its padding, and
    // a measured leaf at the bottom.
    const N: usize = 4000;
    for display in [Display::Flex, Display::Grid, Display::Block] {
        let mut t = LayoutTree::new();
        let leaf = t.measured(&LayoutStyle::default(), |_| remedy_ui_core::Size::new(10.0, 10.0))?;
        let mut top = leaf;
        for _ in 0..N {
            let mut s = LayoutStyle::default();
            s.display = display;
            s.padding = Insets::all(1.0);
            let parent = t.node(&s)?;
            t.add_child(parent, top)?;
            top = parent;
        }
        t.compute(top, None, None)?;
        let at = t.absolute_rect(top, leaf).map(|r| (r.min.x, r.min.y, r.width(), r.height()));
        assert_eq!(at, Some((N as f32, N as f32, 10.0, 10.0)), "{display:?}: the leaf, {N} levels in");
        assert_eq!(t.rect(top).map(|r| r.width()), Some(10.0 + 2.0 * N as f32), "{display:?}");
    }
    Ok(())
}

fn poster_grid(repeat: Repeat, width: f32, n: usize) -> Result<Vec<f32>, LayoutError> {
    let mut t = LayoutTree::new();
    let root = t.node(&{
        let mut v_ = LayoutStyle::default();
        v_.display = Display::Grid;
        // Portal's `repeat(auto-fill, minmax(180px, 1fr))`.
        v_.columns = vec![Track::Repeat(
            repeat,
            vec![Track::MinMax(Box::new(Track::Px(180.0)), Box::new(Track::Fr(1.0)))],
        )];
        v_.gap = (10.0, 10.0);
        v_
    })?;
    let cells: Vec<LayoutNode> = (0..n).map(|_| t.node(&LayoutStyle::default())).collect::<Result<_, _>>()?;
    t.set_children(root, &cells)?;
    t.compute(root, Some(width), None)?;
    Ok(cells.iter().filter_map(|c| t.rect(*c).map(|r| r.min.x)).collect())
}

#[test]
fn auto_fill_minmax_columns_fill_the_width() -> R {
    // 1280: (1280 + 10) / 190 = 6 columns of (1280 - 50) / 6 = 205.
    let xs = poster_grid(Repeat::AutoFill, 1280.0, 8)?;
    assert_eq!(xs.get(..7), Some(&[0.0, 215.0, 430.0, 645.0, 860.0, 1075.0, 0.0][..]));
    // 390: two columns of 190.
    assert_eq!(poster_grid(Repeat::AutoFill, 390.0, 3)?, [0.0, 200.0, 0.0]);
    // Narrower than one track: still one.
    assert_eq!(poster_grid(Repeat::AutoFill, 120.0, 2)?, [0.0, 0.0]);
    Ok(())
}

#[test]
fn auto_fit_collapses_empty_tracks_and_count_repeats() -> R {
    // Two items at 1280: auto-fill keeps six tracks, auto-fit collapses four and the two grow.
    assert_eq!(poster_grid(Repeat::AutoFill, 1280.0, 2)?, [0.0, 215.0]);
    assert_eq!(poster_grid(Repeat::AutoFit, 1280.0, 2)?, [0.0, 645.0]);
    assert_eq!(
        poster_grid(Repeat::Count(3), 1280.0, 4)?.get(3),
        Some(&0.0),
        "three columns, the fourth wraps"
    );
    Ok(())
}
