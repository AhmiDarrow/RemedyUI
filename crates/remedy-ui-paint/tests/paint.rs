//! Display lists, transforms and paths.

use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_paint::*;

#[test]
fn affine_transforms_compose_in_order() {
    let t = Affine::scale(2.0, 2.0).then(Affine::translate(Vec2::new(10.0, 0.0)));
    assert_eq!(t.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0), "scale, then move");
    let r = Affine::rotate(std::f32::consts::FRAC_PI_2).apply(Point::new(1.0, 0.0));
    assert!((r.x).abs() < 1e-6 && (r.y - 1.0).abs() < 1e-6);
    assert!(Affine::translate(Vec2::new(3.0, 4.0)).is_translation());
    let b = Affine::rotate(std::f32::consts::FRAC_PI_4).apply_rect(Rect::new(-1.0, -1.0, 1.0, 1.0));
    assert!((b.width() - 2.0 * 2f32.sqrt()).abs() < 1e-4);
}

#[test]
fn bounds_follow_transforms_and_clips() {
    let mut l = DisplayList::new();
    assert_eq!(l.bounds(), None);
    l.push(Item::PushTransform(Affine::translate(Vec2::new(100.0, 0.0))));
    l.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Brush::Solid(Color::BLACK));
    l.push(Item::PopTransform);
    assert_eq!(l.bounds(), Some(Rect::new(100.0, 0.0, 110.0, 10.0)));
    l.push_clip(Rect::new(0.0, 0.0, 5.0, 5.0), CornerRadii::ZERO);
    l.fill_rect(Rect::new(0.0, 0.0, 50.0, 50.0), Brush::Solid(Color::BLACK));
    l.pop_clip();
    assert_eq!(l.bounds(), Some(Rect::new(0.0, 0.0, 110.0, 10.0)), "the big rect is clipped to 5x5");
}

#[test]
fn shadows_extend_past_their_shape() {
    let mut l = DisplayList::new();
    l.push(Item::Shadow {
        rect: Rect::new(10.0, 10.0, 20.0, 20.0),
        radii: CornerRadii::ZERO,
        offset: Vec2::new(0.0, 2.0),
        blur: 4.0,
        spread: 0.0,
        color: Color::BLACK,
    });
    let b = l.bounds().unwrap_or(Rect::ZERO);
    assert!(b.min.x < 10.0 && b.max.y > 22.0);
}

#[test]
fn paths_have_bounds() {
    let c = Path::circle(Point::new(10.0, 10.0), 5.0);
    assert_eq!(c.bounds(), Some(Rect::new(5.0, 5.0, 15.0, 15.0)));
    assert_eq!(c.elements().len(), 6);
    let mut p = Path::new();
    p.move_to(Point::new(0.0, 0.0)).line_to(Point::new(4.0, 3.0));
    assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 4.0, 3.0)));
}

#[test]
fn filter_groups_grow_by_their_reach() {
    let mut l = DisplayList::new();
    l.push(Item::PushTransform(Affine::scale(2.0, 2.0)));
    l.push_filter(Filter::Blur { radius: 2.0 });
    l.fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), Brush::Solid(Color::BLACK));
    l.pop_filter();
    l.push(Item::PopTransform);
    // Three standard deviations (6 units) around the rect, then scaled by two.
    assert_eq!(l.bounds(), Some(Rect::new(8.0, 8.0, 52.0, 52.0)));
    assert!(l.has_filters());

    let mut s = DisplayList::new();
    s.push_filter(Filter::DropShadow { offset: Vec2::new(0.0, 10.0), blur: 4.0, color: Color::BLACK });
    s.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Brush::Solid(Color::BLACK));
    s.pop_filter();
    // The shape itself, and its shadow (sigma 2, reach 6) moved down by 10.
    assert_eq!(s.bounds(), Some(Rect::new(-6.0, 0.0, 16.0, 26.0)));

    // A clip around the group cuts the grown area; a clip inside it cuts only the source.
    let mut c = DisplayList::new();
    c.push_clip(Rect::new(0.0, 0.0, 12.0, 12.0), CornerRadii::ZERO);
    c.push_filter(Filter::Blur { radius: 1.0 });
    c.push_clip(Rect::new(0.0, 0.0, 5.0, 5.0), CornerRadii::ZERO);
    c.fill_rect(Rect::new(0.0, 0.0, 50.0, 50.0), Brush::Solid(Color::BLACK));
    c.pop_clip();
    c.pop_filter();
    c.pop_clip();
    assert_eq!(c.bounds(), Some(Rect::new(0.0, 0.0, 8.0, 8.0)));
    assert!(Filter::Blur { radius: 0.0 }.is_identity());
    assert_eq!(Filter::Blur { radius: f32::NAN }.std_dev(), 0.0, "non-finite radii draw unblurred");
}

#[test]
fn a_blur_looks_the_same_at_high_dpi() {
    // A 40-pixel glass blur stays 40 at 200%, not clamped to half the cap.
    let blur = Filter::Blur { radius: 40.0 };
    assert_eq!(blur.limited(2.0).std_dev(), 40.0);
    assert_eq!(blur.limited(1.0).std_dev(), 40.0);
    // The cap still holds, in the filter's units; zoomed out it holds in device pixels.
    let huge = Filter::Blur { radius: 1.0e9 };
    assert_eq!(huge.limited(2.0).std_dev(), MAX_FILTER_STD_DEV);
    assert_eq!(huge.limited(0.5).std_dev() * 0.5, MAX_FILTER_STD_DEV);
}

#[test]
fn colour_matrices_reach_no_further_than_the_group_draws() {
    let m = Filter::ColorMatrix { matrix: ColorMatrix::grayscale(1.0) };
    let r = Rect::new(3.0, 4.0, 20.0, 30.0);
    assert_eq!(m.outset(r), r);
    assert_eq!(m.inset_source(r), r);
    assert_eq!(m.std_dev(), 0.0);
    assert!(!m.is_identity());
    assert!(Filter::ColorMatrix { matrix: ColorMatrix::IDENTITY }.is_identity());
    assert!(Filter::ColorMatrix { matrix: ColorMatrix::saturate(1.0) }.limited(2.0).is_identity());
    // A matrix group's bounds are its content's.
    let mut l = DisplayList::new();
    l.push(Item::PushTransform(Affine::scale(2.0, 2.0)));
    l.push_filter(m);
    l.fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), Brush::Solid(Color::BLACK));
    l.pop_filter();
    l.push(Item::PopTransform);
    assert_eq!(l.bounds(), Some(Rect::new(20.0, 20.0, 40.0, 40.0)));
    assert!(l.has_filters());
    assert_eq!(
        DisplayList::bounds_of(l.items().get(2..3).unwrap_or(&[])),
        Some(Rect::new(10.0, 10.0, 20.0, 20.0))
    );
}

#[test]
fn hostile_colour_matrices_are_limited() {
    let wild = Filter::ColorMatrix { matrix: ColorMatrix([f32::NAN; 20]) };
    assert!(wild.limited(1.0).is_identity(), "NaN entries become the identity's");
    let Filter::ColorMatrix { matrix } =
        (Filter::ColorMatrix { matrix: ColorMatrix([f32::INFINITY; 20]) }).limited(1.0)
    else {
        unreachable!("stays a matrix")
    };
    assert!(
        matrix
            .0
            .iter()
            .all(|v| v.is_finite() && v.abs() <= remedy_ui_core::color_matrix::MAX_COLOR_MATRIX_ENTRY)
    );
    const { assert!(MAX_FILTER_MATRIX_DEPTH >= 1 && MAX_FILTER_MATRIX_GROUPS >= MAX_FILTER_MATRIX_DEPTH) };
}

#[test]
fn nested_filters_share_one_bound() {
    let shadow =
        |x: f32, y: f32, blur: f32| Filter::DropShadow { offset: Vec2::new(x, y), blur, color: Color::BLACK };
    let offset = |f: Filter| match f {
        Filter::DropShadow { offset, .. } => offset,
        Filter::Blur { .. } | Filter::ColorMatrix { .. } => Vec2::ZERO,
    };
    // Alone, a filter gets the whole bound, exactly as `limited` draws it.
    let hostile = shadow(1.0e9, -1.0e9, 1.0e9);
    for scale in [0.5, 1.0, 2.0] {
        assert_eq!(FilterBudget::FULL.limit(&hostile, scale).0, hostile.limited(scale));
    }
    // Nested at 200%: the first uses all of it (1024 units is 2048 device pixels on each
    // axis), so a shadow inside falls no farther; the blur is spent too.
    let (outer, left) = FilterBudget::FULL.limit(&hostile, 2.0);
    assert_eq!(offset(outer), Vec2::new(MAX_FILTER_OFFSET / 2.0, -MAX_FILTER_OFFSET / 2.0));
    let (inner, _) = left.limit(&hostile, 2.0);
    assert_eq!(offset(inner), Vec2::ZERO);
    assert_eq!(inner.std_dev(), 0.0);
    // Partly spent: what is left, axis by axis. A shadow 2000 px right leaves 48 px to the
    // right but all 2048 down; one 10 px either way leaves 2038.
    let (_, left) = FilterBudget::FULL.limit(&shadow(2000.0, 0.0, 0.0), 1.0);
    assert_eq!(offset(left.limit(&shadow(-100.0, 3000.0, 0.0), 1.0).0), Vec2::new(-48.0, MAX_FILTER_OFFSET));
    // Blur adds up in the bound's units: 40 + 40 is past 64, so the second gets 24.
    let (_, left) = FilterBudget::FULL.limit(&Filter::Blur { radius: 40.0 }, 1.0);
    assert_eq!(left.limit(&Filter::Blur { radius: 40.0 }, 1.0).0.std_dev(), 24.0);
    // Ordinary nesting is untouched: a 4 px blur in a card's 8 px shadow 6 px down.
    let (card, left) = FilterBudget::FULL.limit(&shadow(0.0, 6.0, 16.0), 1.0);
    assert_eq!(card, shadow(0.0, 6.0, 16.0));
    assert_eq!(left.limit(&Filter::Blur { radius: 4.0 }, 1.0).0, Filter::Blur { radius: 4.0 });
    // Non-finite asks spend nothing and draw nothing, as alone.
    let (nan, left) = FilterBudget::FULL.limit(&shadow(f32::NAN, f32::INFINITY, f32::NAN), 1.0);
    assert_eq!((offset(nan), nan.std_dev(), left), (Vec2::ZERO, 0.0, FilterBudget::FULL));
}
