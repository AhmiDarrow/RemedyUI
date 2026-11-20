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
