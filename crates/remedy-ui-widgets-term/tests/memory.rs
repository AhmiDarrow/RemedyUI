//! The automatic scrollback budget follows the machine's memory when it changes (its own
//! test binary: the machine's memory is process-wide).

use remedy_ui_widgets_term::*;

#[test]
fn the_automatic_budget_follows_the_machines_memory() {
    // 64 MB of memory: a budget of 256 KB.
    remedy_ui_view::device::set_total_memory(Some(64 << 20));
    let mut e = Emulator::new(40, 5);
    let line = format!("{}\r\n", "x".repeat(38));
    for _ in 0..20_000 {
        e.feed(line.as_bytes());
    }
    let small = e.history_len();
    assert!(e.history_bytes() <= 256 << 10, "{}", e.history_bytes());
    // More memory: the budget grows with it, and more lines are kept.
    remedy_ui_view::device::set_total_memory(Some(1 << 30));
    for _ in 0..20_000 {
        e.feed(line.as_bytes());
    }
    assert!(e.history_len() > small * 4, "{} after {small}", e.history_len());
    // Less again: the scrollback shrinks to the new budget at once.
    remedy_ui_view::device::set_total_memory(Some(64 << 20));
    e.feed(b"!");
    assert!(e.history_bytes() <= 256 << 10, "{}", e.history_bytes());
}
