//! [`set_paint_limits`] moves the filter bounds. One test, so no other test in this binary sees
//! the bounds changed.

use remedy_ui_paint::{
    Filter, FilterBudget, MAX_FILTER_STD_DEV, PaintLimits, paint_limits, set_paint_limits,
};

#[test]
fn the_filter_bounds_follow_the_paint_limits() {
    assert_eq!(paint_limits(), PaintLimits::DEFAULT);
    let huge = Filter::Blur { radius: 1000.0 };
    assert_eq!(huge.limited(1.0).std_dev(), MAX_FILTER_STD_DEV);
    set_paint_limits(PaintLimits::default().filter_std_dev(256.0).backdrop_depth(8));
    assert_eq!(huge.limited(1.0).std_dev(), 256.0);
    assert_eq!(FilterBudget::default(), FilterBudget::full());
    assert_eq!(paint_limits().backdrop_depth, 8);
    // A bound that is not a number takes nothing.
    set_paint_limits(PaintLimits::default().filter_std_dev(f32::NAN));
    assert_eq!(huge.limited(1.0).std_dev(), 0.0);
    set_paint_limits(PaintLimits::default());
    assert_eq!(FilterBudget::full(), FilterBudget::FULL);
}
