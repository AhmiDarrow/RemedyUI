//! Behavior the design promises for the element tree.
#![allow(clippy::panic, clippy::indexing_slicing, reason = "tests panic on purpose and index known vectors")]

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::{Point, Rect};
use remedy_ui_core::reactive::{Effect, Signal, on_cleanup};
use remedy_ui_core::tree::*;

type R = Result<(), TreeError>;

fn el(tree: &Tree, parent: ElementId, name: &'static str) -> Result<ElementId, TreeError> {
    let e = tree.create(name);
    tree.append(parent, e)?;
    Ok(e)
}

#[test]
fn structure_rules() -> R {
    let t = Tree::new();
    let a = el(&t, t.root(), "a")?;
    let b = el(&t, a, "b")?;
    assert_eq!(t.path(b), "root/a/b");
    assert_eq!(t.append(b, a), Err(TreeError::AlreadyAttached), "attached elements must be detached first");
    t.detach(a)?;
    assert_eq!(t.append(b, a), Err(TreeError::WouldCycle));
    t.append(t.root(), a)?;
    assert_eq!(t.append(t.root(), b), Err(TreeError::AlreadyAttached));
    assert_eq!(t.remove(t.root()), Err(TreeError::Root));
    t.detach(b)?;
    t.insert(t.root(), 0, b)?;
    assert_eq!(t.children(t.root()), [b, a]);
    t.remove(a)?;
    assert!(!t.contains(a));
    assert_eq!(t.append(a, b), Err(TreeError::NotFound));
    Ok(())
}

#[test]
fn removing_an_element_disposes_its_reactive_scope() -> R {
    let t = Tree::new();
    let e = el(&t, t.root(), "row")?;
    let cleaned = Rc::new(RefCell::new(false));
    let c = cleaned.clone();
    let sig = t.in_scope(e, move || {
        on_cleanup(move || *c.borrow_mut() = true);
        Signal::new(1)
    })?;
    assert!(sig.is_alive());
    t.remove(e)?;
    assert!(*cleaned.borrow());
    assert!(!sig.is_alive());
    Ok(())
}

#[test]
fn layout_dirt_climbs_only_to_the_relayout_boundary() -> R {
    let t = Tree::new();
    let panel = el(&t, t.root(), "panel")?;
    let row = el(&t, panel, "row")?;
    let label = el(&t, row, "label")?;
    t.set_relayout_boundary(panel, true)?;
    t.take_dirty(
        Dirty::STYLE
            | Dirty::LAYOUT
            | Dirty::PAINT
            | Dirty::SEMANTICS
            | Dirty::CHILD_LAYOUT
            | Dirty::CHILD_PAINT,
    );
    assert_eq!(t.dirty_count(), 0);

    t.mark(label, Dirty::LAYOUT);
    assert!(t.dirty(label).contains(Dirty::LAYOUT));
    assert!(t.dirty(row).contains(Dirty::CHILD_LAYOUT));
    assert!(t.dirty(panel).contains(Dirty::CHILD_LAYOUT));
    assert!(t.dirty(t.root()).is_empty(), "stopped at the boundary");

    // Paint dirt goes to the nearest repaint boundary: here the root.
    t.mark(label, Dirty::PAINT);
    assert!(t.dirty(t.root()).contains(Dirty::CHILD_PAINT));

    let layout_work = t.take_dirty(Dirty::LAYOUT | Dirty::CHILD_LAYOUT);
    assert_eq!(layout_work, [panel, row, label], "tree order");
    assert_eq!(t.take_dirty(Dirty::LAYOUT), []);
    Ok(())
}

#[test]
fn a_signal_change_dirties_only_the_elements_bound_to_it() -> R {
    let t = Tree::new();
    let title = Signal::new("Inbox".to_string());
    let count = Signal::new(0);
    let mut labels = Vec::new();
    for i in 0..100 {
        let e = el(&t, t.root(), "label")?;
        let tree = t.clone();
        // Element 7 shows the title; the rest show the count.
        let bind_title = i == 7;
        t.in_scope(e, move || {
            Effect::new(move || {
                if bind_title {
                    title.with(|_| ());
                } else {
                    count.get();
                }
                tree.mark(e, Dirty::LAYOUT | Dirty::PAINT);
            });
        })?;
        labels.push(e);
    }
    t.take_dirty(
        Dirty::LAYOUT
            | Dirty::PAINT
            | Dirty::CHILD_LAYOUT
            | Dirty::CHILD_PAINT
            | Dirty::STYLE
            | Dirty::SEMANTICS,
    );
    title.set("Archive".into());
    let dirty = t.take_dirty(Dirty::LAYOUT);
    assert_eq!(dirty, [labels[7]]);
    Ok(())
}

#[test]
fn events_capture_down_then_bubble_up() -> R {
    let t = Tree::new();
    let outer = el(&t, t.root(), "outer")?;
    let inner = el(&t, outer, "inner")?;
    let log = Rc::new(RefCell::new(Vec::new()));
    for (e, name) in [(t.root(), "root"), (outer, "outer"), (inner, "inner")] {
        for phase in [Phase::Capture, Phase::Bubble] {
            let log = log.clone();
            t.on::<u32>(e, phase, move |_, v| log.borrow_mut().push(format!("{name}:{phase:?}:{v}")))?;
        }
    }
    let d = t.dispatch(inner, &7u32)?;
    assert_eq!(d.handlers_run, 6);
    assert_eq!(
        *log.borrow(),
        [
            "root:Capture:7",
            "outer:Capture:7",
            "inner:Capture:7",
            "inner:Bubble:7",
            "outer:Bubble:7",
            "root:Bubble:7"
        ]
    );
    // Other event types do not reach these handlers.
    assert_eq!(t.dispatch(inner, &"text")?.handlers_run, 0);
    Ok(())
}

#[test]
fn stop_propagation_finishes_the_current_element_only() -> R {
    let t = Tree::new();
    let outer = el(&t, t.root(), "outer")?;
    let inner = el(&t, outer, "inner")?;
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    t.on::<()>(inner, Phase::Bubble, move |cx, _| {
        l.borrow_mut().push("inner1");
        cx.stop_propagation();
        cx.set_handled();
    })?;
    let l = log.clone();
    t.on::<()>(inner, Phase::Bubble, move |_, _| l.borrow_mut().push("inner2"))?;
    let l = log.clone();
    t.on::<()>(outer, Phase::Bubble, move |_, _| l.borrow_mut().push("outer"))?;
    let d = t.dispatch(inner, &())?;
    assert_eq!(*log.borrow(), ["inner1", "inner2"]);
    assert!(d.stopped && d.handled);
    Ok(())
}

#[test]
fn a_panicking_handler_is_caught_by_its_boundary() -> R {
    let t = Tree::new();
    let card = el(&t, t.root(), "card")?;
    let failure = t.set_error_boundary(card)?;
    let button = el(&t, card, "save")?;
    let sibling = el(&t, t.root(), "sibling")?;
    let reached = Rc::new(RefCell::new(false));
    t.on::<()>(button, Phase::Bubble, |_, _| std::panic::panic_any("disk full".to_string()))?;
    let r = reached.clone();
    t.on::<()>(t.root(), Phase::Bubble, move |_, _| *r.borrow_mut() = true)?;
    let d = t.dispatch(button, &())?;
    assert_eq!(d.panicked, 1);
    assert!(*reached.borrow(), "the dispatch went on past the failure");
    assert_eq!(
        failure.get().map(|f| (f.element_path, f.message)),
        Some(("root/card/save".to_owned(), "disk full".to_owned()))
    );
    assert!(!t.contains(button), "the failed subtree was removed");
    assert!(t.contains(card) && t.contains(sibling), "the rest of the window is untouched");
    Ok(())
}

#[test]
fn guarded_code_reports_to_the_root_when_no_boundary_is_closer() -> R {
    let t = Tree::new();
    let e = el(&t, t.root(), "canvas")?;
    let root_failure = t.set_error_boundary(t.root())?;
    let r: Option<()> = t.guarded(e, || std::panic::panic_any("bad path"));
    assert_eq!(r, None);
    assert_eq!(root_failure.get().map(|f| f.message), Some("bad path".to_string()));
    Ok(())
}

#[test]
fn hit_testing_finds_the_topmost_element() -> R {
    let t = Tree::new();
    t.set_bounds(t.root(), Rect::new(0.0, 0.0, 100.0, 100.0))?;
    let back = el(&t, t.root(), "back")?;
    let front = el(&t, t.root(), "front")?;
    let child = el(&t, back, "child")?;
    t.set_bounds(back, Rect::new(0.0, 0.0, 60.0, 60.0))?;
    t.set_bounds(front, Rect::new(40.0, 40.0, 100.0, 100.0))?;
    t.set_bounds(child, Rect::new(10.0, 10.0, 20.0, 20.0))?;
    assert_eq!(t.hit_test(Point::new(50.0, 50.0)), Some(front), "later sibling is on top");
    assert_eq!(t.hit_test(Point::new(15.0, 15.0)), Some(child), "children are above parents");
    assert_eq!(t.hit_test(Point::new(30.0, 30.0)), Some(back));
    t.set_hit_mode(front, HitMode::PassThrough)?;
    assert_eq!(t.hit_test(Point::new(50.0, 50.0)), Some(back));
    t.set_hit_mode(back, HitMode::None)?;
    assert_eq!(t.hit_test(Point::new(15.0, 15.0)), Some(t.root()), "the whole subtree is ignored");
    // A child outside a clipping parent cannot be hit.
    let outside = el(&t, front, "outside")?;
    t.set_bounds(outside, Rect::new(0.0, 90.0, 10.0, 100.0))?;
    assert_eq!(t.hit_test(Point::new(5.0, 95.0)), Some(outside));
    t.set_clips(front, true)?;
    assert_eq!(t.hit_test(Point::new(5.0, 95.0)), Some(t.root()));
    Ok(())
}

#[test]
fn hit_testing_goes_any_depth_on_a_threads_stack() -> R {
    // Tens of thousands of nested elements on a test thread's stack (2 MB): the deepest is
    // on top, and a clip that misses part way down hides everything under it.
    const N: usize = 20_000;
    let t = Tree::new();
    let full = Rect::new(0.0, 0.0, 100.0, 100.0);
    t.set_bounds(t.root(), full)?;
    let mut chain = vec![t.root()];
    for _ in 0..N {
        let parent = chain.last().copied().unwrap_or(t.root());
        let e = el(&t, parent, "level")?;
        t.set_bounds(e, full)?;
        chain.push(e);
    }
    let p = Point::new(50.0, 50.0);
    assert_eq!(t.hit_test(p), chain.last().copied(), "the deepest element");
    let (above, mid) = (chain[N / 2 - 1], chain[N / 2]);
    t.set_bounds(mid, Rect::new(0.0, 0.0, 10.0, 10.0))?;
    t.set_clips(mid, true)?;
    assert_eq!(t.hit_test(p), Some(above), "the clip hides what is under it");
    Ok(())
}

#[test]
fn tab_order_wraps_and_respects_explicit_order() -> R {
    let t = Tree::new();
    let a = el(&t, t.root(), "a")?;
    let b = el(&t, t.root(), "b")?;
    let c = el(&t, t.root(), "c")?;
    let not_focusable = el(&t, t.root(), "label")?;
    for e in [a, b, c] {
        t.set_focusable(e, true)?;
    }
    assert_eq!(t.focus_next(), Some(a));
    assert_eq!(t.focus_next(), Some(b));
    assert_eq!(t.focus_next(), Some(c));
    assert_eq!(t.focus_next(), Some(a), "wraps");
    assert_eq!(t.focus_previous(), Some(c));
    t.set_focus_order(c, Some(1))?;
    t.blur();
    assert_eq!(t.focus_next(), Some(c), "explicit order comes first");
    assert!(!t.focus(not_focusable)?);
    // Out of the Tab order (`tabindex="-1"`): Tab passes it, focus still reaches it.
    t.set_focus_order(c, None)?;
    t.set_tab_stop(b, false)?;
    assert!(!t.is_tab_stop(b));
    assert_eq!(t.focus_candidates(t.root()), vec![a, c]);
    assert!(t.focus(b)?);
    assert_eq!(t.focus_next(), Some(c), "Tab goes on from where it stands");
    assert!(t.focus(b)?);
    assert_eq!(t.focus_previous(), Some(a));
    Ok(())
}

#[test]
fn focus_scopes_trap_tab_and_focus_events_fire() -> R {
    let t = Tree::new();
    let page_button = el(&t, t.root(), "page-button")?;
    let modal = el(&t, t.root(), "modal")?;
    let ok = el(&t, modal, "ok")?;
    let cancel = el(&t, modal, "cancel")?;
    for e in [page_button, ok, cancel] {
        t.set_focusable(e, true)?;
    }
    t.set_focus_scope(modal, true)?;
    let log = Rc::new(RefCell::new(Vec::new()));
    for (e, n) in [(page_button, "page"), (ok, "ok")] {
        let l = log.clone();
        t.on::<FocusIn>(e, Phase::Bubble, move |_, _| l.borrow_mut().push(format!("in:{n}")))?;
        let l = log.clone();
        t.on::<FocusOut>(e, Phase::Bubble, move |_, _| l.borrow_mut().push(format!("out:{n}")))?;
    }
    assert_eq!(t.focus_next(), Some(page_button), "the modal's contents are not in the page's tab order");
    t.focus(ok)?;
    assert_eq!(t.focus_next(), Some(cancel));
    assert_eq!(t.focus_next(), Some(ok), "Tab stays inside the modal");
    assert_eq!(*log.borrow(), ["in:page", "out:page", "in:ok", "out:ok", "in:ok"]);
    // Removing the modal clears focus inside it.
    t.remove(modal)?;
    assert_eq!(t.focused(), None);
    Ok(())
}

#[test]
fn payloads_hold_widget_data() -> R {
    let t = Tree::new();
    let e = el(&t, t.root(), "slider")?;
    t.set_payload(e, 0.5f32)?;
    assert_eq!(t.with_payload(e, |v: &mut f32| *v), Some(0.5));
    assert_eq!(t.with_payload(e, |v: &mut u8| *v), None, "wrong type is None, not a panic");
    Ok(())
}

#[test]
fn removing_the_focused_element_hands_focus_to_the_nearest_focusable_ancestor() -> R {
    let t = Tree::new();
    let panel = el(&t, t.root(), "panel")?;
    let list = el(&t, panel, "list")?;
    let item = el(&t, list, "item")?;
    t.set_focusable(panel, true)?;
    t.set_focusable(item, true)?;
    t.focus(item)?;
    // The list goes (a page swapped for another): keys still reach the panel.
    t.remove(list)?;
    assert_eq!(t.focused(), Some(panel));
    // With nothing focusable around it, focus clears.
    t.remove(panel)?;
    assert_eq!(t.focused(), None);
    Ok(())
}
