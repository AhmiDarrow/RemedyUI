//! The app's memory budget decides whether an animation is kept whole or streamed. This runs
//! in a binary of its own: the budget is shared by the whole process.

mod common;

use std::sync::Arc;

use common::*;
use remedy_ui_media_anim::{Fit, Source, animated, resident_frames, set_memory_budget};

/// 40 frames of 16×16, each in its own color shade.
fn forty() -> Vec<u8> {
    let frames: Vec<ApngFrame> =
        (0..40u32).map(|i| ApngFrame::solid(16, 16, [i as u8 * 6, 40, 90, 255], 100)).collect();
    apng(16, 16, 0, None, &frames)
}

/// Plays one animation through twice, and says the most frames it held.
fn most_held(budget: Option<u64>) -> usize {
    set_memory_budget(budget);
    let mut s = Screen::new();
    let el =
        animated(&s.ui, Source::Bytes(Arc::from(forty())), "Forty", Fit::Contain).width(16.0).height(16.0);
    s.ui.root().child(el);
    s.frame();
    s.wait("playing", |s| s.timers.len() == 1 && s.color() == [0, 40, 90, 255]);
    let mut most = resident_frames();
    for i in 1..=80u32 {
        let was = s.color();
        s.fire();
        s.wait("the next frame", |s| s.color() != was && s.timers.len() == 1);
        assert_eq!(s.color(), [(i % 40) as u8 * 6, 40, 90, 255], "frame {i}");
        most = most.max(resident_frames());
    }
    s.ui.root().clear();
    s.frame();
    most
}

#[test]
fn a_budget_in_bytes_sets_how_many_frames_an_animation_keeps() {
    // Without a budget, forty small frames are kept whole.
    assert_eq!(most_held(None), 40);
    // A frame is 1 KiB here. A budget for ten of them makes it stream through a window.
    let streamed = most_held(Some(10 * 1024));
    assert!(streamed <= 10, "kept {streamed}");
    assert!(streamed >= 3, "kept {streamed}");
    // A budget for all of them (and more) keeps it whole again.
    assert_eq!(most_held(Some(64 * 1024)), 40);
    set_memory_budget(None);
}
