//! Idle timing and sensitive copies.

use std::time::Duration;

use remedy_ui_secure::{IdleTimer, SensitiveCopy};

fn s(v: u64) -> Duration {
    Duration::from_secs(v)
}

#[test]
fn idle_counts_from_the_last_real_input() {
    let mut t = IdleTimer::new(Some(s(60)), s(0));
    assert_eq!(t.deadline(), Some(s(60)));
    assert!(!t.is_idle(s(59)));
    t.input(s(30));
    assert!(!t.is_idle(s(60)), "input pushed the deadline");
    assert!(t.is_idle(s(90)));
    t.input(s(10));
    assert_eq!(t.deadline(), Some(s(90)), "an older input never moves it back");
    t.set_timeout(None);
    assert!(!t.is_idle(s(10_000)), "off means never");
    assert_eq!(t.deadline(), None);
}

#[test]
fn a_secret_is_cleared_only_while_it_is_still_on_the_clipboard() {
    let c = SensitiveCopy::new("hunter2-recovery", s(5), s(30));
    assert_eq!(c.clear_at(), s(35));
    assert!(c.still_there("hunter2-recovery".to_owned()));
    assert!(!c.still_there("something the user copied since".to_owned()));
    assert!(!c.still_there(String::new()));
    let printed = format!("{c:?}");
    assert!(!printed.contains("hunter2"), "the text is never kept");
}
