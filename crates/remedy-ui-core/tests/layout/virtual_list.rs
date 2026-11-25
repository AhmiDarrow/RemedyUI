//! Virtual lists: estimates, anchoring and stick-to-end.

use remedy_ui_core::layout::{JumpAlign, VirtualList};

#[test]
fn positions_use_measured_heights_and_the_estimate() {
    let mut v = VirtualList::new(10, 20.0, false);
    assert_eq!(v.total_height(), 200.0);
    v.set_height(0, 50.0);
    // The estimate becomes the average of measured rows.
    assert_eq!(v.estimate(), 50.0);
    assert_eq!(v.offset_of(1), 50.0);
    assert_eq!(v.total_height(), 500.0);
    v.set_height(1, 30.0);
    assert_eq!(v.estimate(), 40.0);
    assert_eq!(v.offset_of(2), 80.0);
    assert_eq!(v.index_at(79.0), 1);
    assert_eq!(v.index_at(80.0), 2);
}

#[test]
fn only_rows_near_the_view_are_mounted() {
    let mut v = VirtualList::new(100_000, 30.0, false);
    v.set_viewport(600.0);
    v.scroll_to(30_000.0);
    let r = v.visible_range(300.0);
    assert_eq!(r, 990..1030);
    assert!(r.len() < 50, "a hundred thousand rows, forty mounted");
}

#[test]
fn a_chat_sticks_to_the_end_until_scrolled_up() {
    let mut v = VirtualList::new(50, 40.0, true);
    v.set_viewport(400.0);
    assert!(v.is_stuck_to_end());
    assert_eq!(v.scroll_offset(), 1600.0);
    v.push(3);
    assert_eq!(v.scroll_offset(), v.total_height() - 400.0, "new messages keep the end in view");
    v.scroll_by(-500.0);
    assert!(!v.is_stuck_to_end());
    let held = v.scroll_offset();
    v.push(10);
    assert_eq!(v.scroll_offset(), held, "reading history is not interrupted");
    v.scroll_to(f32::MAX);
    assert!(v.is_stuck_to_end(), "scrolling back to the end sticks again");
}

#[test]
fn loading_history_above_keeps_the_view_still() {
    let mut v = VirtualList::new(100, 25.0, false);
    v.set_viewport(500.0);
    v.scroll_to(1000.0);
    let first = v.index_at(v.scroll_offset());
    let screen_y = v.offset_of(first) - v.scroll_offset();
    v.insert(0, 200); // older messages arrive above
    let same_row = first + 200;
    assert_eq!(v.offset_of(same_row) - v.scroll_offset(), screen_y);
}

#[test]
fn rows_above_growing_do_not_move_the_view() {
    let mut v = VirtualList::new(100, 25.0, false);
    v.set_viewport(500.0);
    v.scroll_to(1250.0);
    let first = v.index_at(v.scroll_offset());
    let before = v.offset_of(first) - v.scroll_offset();
    for i in 0..first {
        v.set_height(i, 80.0); // images above finish loading
    }
    assert!((v.offset_of(first) - v.scroll_offset() - before).abs() < 1e-3);
}

#[test]
fn removing_rows_keeps_the_view_on_what_remains() {
    let mut v = VirtualList::new(100, 10.0, false);
    v.set_viewport(100.0);
    v.scroll_to(505.0); // row 50, 5 px in
    v.remove(10..20); // above
    assert_eq!(v.index_at(v.scroll_offset()), 40);
    assert_eq!(v.scroll_offset() - v.offset_of(40), 5.0);
    v.remove(35..45); // covering the view's first row
    assert_eq!(v.index_at(v.scroll_offset()), 35);
}

#[test]
fn jumping_to_a_row_never_measured() {
    let mut v = VirtualList::new(10_000, 20.0, false);
    v.set_viewport(400.0);
    v.jump_to(5000, JumpAlign::Start);
    assert_eq!(v.index_at(v.scroll_offset()), 5000);
    v.jump_to(5000, JumpAlign::Center);
    assert_eq!(v.scroll_offset(), 5000.0 * 20.0 + 10.0 - 200.0);
    v.jump_to(9999, JumpAlign::End);
    assert_eq!(v.scroll_offset(), v.total_height() - 400.0);
}

#[test]
fn a_width_change_forgets_heights_but_holds_the_view() {
    let mut v = VirtualList::new(100, 20.0, false);
    v.set_viewport(200.0);
    for i in 0..100 {
        v.set_height(i, 20.0 + (i % 3) as f32 * 10.0);
    }
    v.scroll_to(1500.0);
    let first = v.index_at(v.scroll_offset());
    v.invalidate_heights();
    assert_eq!(v.index_at(v.scroll_offset()), first);
}

#[test]
fn values_that_are_not_numbers_do_not_poison_the_list() {
    let mut v = VirtualList::new(10, 20.0, false);
    v.set_viewport(50.0);
    v.scroll_to(40.0);
    v.scroll_to(f32::NAN);
    assert_eq!(v.scroll_offset(), 40.0, "the view stays where it was");
    v.set_height(0, f32::INFINITY);
    v.set_height(1, f32::NAN);
    assert_eq!((v.total_height(), v.scroll_offset()), (200.0, 40.0), "neither was a measurement");
    v.set_height(0, 30.0);
    assert_eq!(v.total_height(), 300.0);
}

#[test]
fn a_hundred_thousand_messages_arrive_quickly() {
    let mut v = VirtualList::new(0, 40.0, true);
    v.set_viewport(800.0);
    let t = std::time::Instant::now();
    for i in 0..100_000 {
        v.push(1);
        v.set_height(i, 36.0 + (i % 5) as f32);
    }
    assert_eq!(v.len(), 100_000);
    assert!(v.is_stuck_to_end());
    assert!(t.elapsed().as_secs_f32() < 2.0, "{:?}", t.elapsed());
}
