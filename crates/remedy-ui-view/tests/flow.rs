//! Context, conditional content, error and suspense boundaries, reactive properties, typed
//! tokens, element motion and pseudo-locales, checked through the accessibility tree where
//! people (and screen readers) would notice.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, Insets, Size, Vec2};
use remedy_ui_core::reactive::{Executor, Job, Resource, Signal, run_posted, use_context};
use remedy_ui_core::style::{ColorRole, Dp, Radius, Space, States, Style, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::motion::{Easing, Motion, Target};
use remedy_ui_view::{El, FailurePhase, Request, Transition, Ui, catch_errors, show, suspense};

/// Unwraps what a test needs, failing it with `why`.
trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

/// Fails the code under test the way a bug would.
fn fail(why: &'static str) -> ! {
    std::panic::resume_unwind(Box::new(why))
}

fn fonts() -> FontCollection {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    f
}

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule(
        "button",
        None,
        States::NONE,
        StylePatch::new().padding(Insets { top: 5.0, end: 9.0, bottom: 5.0, start: 9.0 }),
    );
    let ui = Ui::new(Theme::dark_forest(), sheet, fonts());
    ui.connect_accessibility();
    ui
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
}

/// Names the screen reader meets, in order.
fn names(ui: &Ui) -> Vec<String> {
    frame(ui);
    ui.reader_view().into_iter().filter_map(|n| n.name).collect()
}

fn label(ui: &Ui, s: &str) -> El {
    ui.el("label").text(s.to_owned())
}

/// Fires every timer the view asked for (as the runner would once they are due).
fn fire_timers(ui: &Ui) {
    for r in ui.take_requests() {
        if let Request::Timer { id, .. } = r {
            ui.fire_timer(id);
        }
    }
}

// ---------- context ----------

#[test]
fn context_flows_to_a_subtree_at_build_time_and_through_the_tree() {
    let ui = ui();
    let scale = Signal::new(1.5f32);
    let seen_at_build = Rc::new(Cell::new(None));
    let s2 = seen_at_build.clone();
    let inner = Rc::new(RefCell::new(None));
    let i2 = inner.clone();
    let outer = ui.el("panel").column().provide(scale).provide(7u32).child_with(|| {
        s2.set(use_context::<Signal<f32>>().map(|s| s.get()));
        let deep = ui.el("card").provide(9u32);
        let leaf = ui.el("label");
        *i2.borrow_mut() = Some(leaf.clone());
        deep.child(leaf)
    });
    let _ = ui.root().child(outer.clone());
    assert_eq!(seen_at_build.get(), Some(1.5), "built inside the element: its context is there");
    let leaf = inner.borrow().clone().must("built");
    assert_eq!(leaf.use_context::<u32>(), Some(9), "the nearest provider wins");
    assert_eq!(outer.use_context::<u32>(), Some(7));
    let sig = leaf.use_context::<Signal<f32>>().must("from the outer element");
    sig.set(2.0);
    assert_eq!(scale.get(), 2.0, "a provided signal is the same signal");
    assert_eq!(ui.root().use_context::<u32>(), None, "nothing flows upward");
}

// ---------- show ----------

#[test]
fn show_builds_and_disposes_content_as_its_condition_changes() {
    let ui = ui();
    let open = Signal::new(false);
    let unrelated = Signal::new(0);
    let builds = Rc::new(Cell::new(0));
    let runs = Rc::new(Cell::new(0));
    let (b2, r2, ui2) = (builds.clone(), runs.clone(), ui.clone());
    let probe = Signal::new(0);
    let content = show(
        &ui,
        move || open.get() || unrelated.get() > 100,
        move || {
            b2.set(b2.get() + 1);
            let r3 = r2.clone();
            label(&ui2, "Details").bind(move || {
                let _ = probe.get();
                r3.set(r3.get() + 1);
            })
        },
    )
    .fallback({
        let ui = ui.clone();
        move || label(&ui, "Nothing yet")
    });
    let _ = ui.root().child(content).child(label(&ui, "Footer"));
    assert_eq!(names(&ui), ["Nothing yet", "Footer"]);
    open.set(true);
    assert_eq!(names(&ui), ["Details", "Footer"]);
    // Other reads of the condition do not rebuild what shows.
    unrelated.set(5);
    assert_eq!(builds.get(), 1);
    probe.set(1);
    assert_eq!(runs.get(), 2);
    open.set(false);
    assert_eq!(names(&ui), ["Nothing yet", "Footer"]);
    probe.set(2);
    assert_eq!(runs.get(), 2, "the hidden content's bindings were disposed with it");
    open.set(true);
    assert_eq!(builds.get(), 2, "it is built afresh each time it appears");
}

#[test]
fn leaving_content_stays_inert_until_its_exit_transition_ends() {
    let ui = ui();
    ui.set_frame_time(Duration::ZERO);
    let open = Signal::new(true);
    let tween = Motion::Tween { duration: Duration::from_millis(100), easing: Easing::Linear };
    let ui2 = ui.clone();
    let content = show(&ui, open, move || label(&ui2, "Toast"))
        .enter(Transition::fade(tween))
        .exit(Transition::slide(Vec2::new(0.0, 10.0), tween));
    let host = content.el();
    let _ = ui.root().child(host.clone());
    assert_eq!(names(&ui), ["Toast"]);
    open.set(false);
    frame(&ui);
    let leaving = host.child_elements();
    assert_eq!(leaving.len(), 1, "still there while it leaves");
    assert!(names(&ui).is_empty(), "but no longer in the accessibility tree");
    ui.set_frame_time(Duration::from_millis(50));
    frame(&ui);
    let mid = leaving.first().and_then(El::info).map(|i| i.style.opacity).must("alive");
    assert!(mid > 0.2 && mid < 0.8, "half faded: {mid}");
    ui.set_frame_time(Duration::from_millis(120));
    frame(&ui);
    assert!(host.child_elements().is_empty(), "removed once the exit ended");

    // Entering plays from the start values.
    open.set(true);
    ui.set_frame_time(Duration::from_millis(130));
    frame(&ui);
    let entering = host.child_elements().first().and_then(El::info).map(|i| i.style.opacity).must("shown");
    assert!(entering < 0.5, "starts faded: {entering}");
    ui.set_frame_time(Duration::from_millis(300));
    frame(&ui);
    let done = host.child_elements().first().and_then(El::info).map(|i| i.style.opacity).must("shown");
    assert_eq!(done, 1.0);
}

#[test]
fn a_show_with_nothing_to_show_takes_no_room_and_no_gap() {
    let ui = ui();
    ui.set_frame_time(Duration::ZERO);
    let which = Signal::new(0);
    let tween = Motion::Tween { duration: Duration::from_millis(100), easing: Easing::Linear };
    // Two screens take turns in one column, each growing: only the one shown fills it.
    let screen = |n: i32, name: &'static str| {
        let ui2 = ui.clone();
        show(&ui, move || which.get() == n, move || label(&ui2, name).height(20.0)).el().grow(1.0)
    };
    let (a, b) = (screen(0, "First"), screen(1, "Second"));
    let ui3 = ui.clone();
    let toast = show(&ui, move || which.get() == 2, move || label(&ui3, "Toast").height(20.0))
        .exit(Transition::fade(tween))
        .el();
    let column = ui.el("panel").column().gap(10.0).child(a.clone()).child(b.clone()).child(toast.clone());
    let _ = ui.root().child(column.height(300.0));
    frame(&ui);
    let rect = |e: &El| e.info().and_then(|i| i.bounds).must("laid out");
    assert_eq!(rect(&a).height(), 300.0, "the shown screen fills the column, with no gap for the others");
    assert_eq!(rect(&b).height(), 0.0);
    which.set(1);
    frame(&ui);
    assert_eq!((rect(&a).height(), rect(&b).height(), rect(&b).min.y), (0.0, 300.0, 0.0));

    // Leaving content keeps its room until its exit transition ends.
    which.set(2);
    frame(&ui);
    which.set(0);
    frame(&ui);
    assert_eq!(rect(&a).height(), 270.0, "the toast still leaves, one gap away");
    ui.set_frame_time(Duration::from_millis(120));
    frame(&ui);
    frame(&ui);
    assert!(toast.child_elements().is_empty());
    assert_eq!(rect(&a).height(), 300.0, "gone, it gives the room back");
}

// ---------- error boundaries ----------

fn boundary(ui: &Ui, child: impl Fn() -> El + 'static) -> (El, ()) {
    let ui2 = ui.clone();
    let el = catch_errors(
        ui,
        move |err, retry| {
            let r = retry;
            ui2.el("button")
                .text(format!("Failed: {}", err.message))
                .semantics(
                    Semantics::new(Role::Button)
                        .label(format!("Failed: {}", err.message))
                        .action(Action::Click),
                )
                .on_click(move || r.retry())
        },
        child,
    );
    (el, ())
}

#[test]
fn a_failing_build_shows_the_fallback_and_retry_builds_again() {
    let ui = ui();
    let attempts = Rc::new(Cell::new(0));
    let a2 = attempts.clone();
    let ui2 = ui.clone();
    let (b, _) = boundary(&ui, move || {
        a2.set(a2.get() + 1);
        if a2.get() == 1 {
            fail("no profile yet");
        }
        label(&ui2, "Profile")
    });
    let _ = ui.root().child(b).child(label(&ui, "Sidebar"));
    assert_eq!(names(&ui), ["Failed: no profile yet", "Sidebar"]);
    let failure = ui.failures().into_iter().next().must("recorded");
    assert_eq!(failure.phase, FailurePhase::Build);
    assert!(failure.element_path.contains("error_boundary"), "{}", failure.element_path);
    let fallback = ui.reader_view().into_iter().find(|n| n.role == Role::Button).must("fallback button");
    assert!(ui.accessibility_action(fallback.id, Action::Click));
    assert_eq!(names(&ui), ["Profile", "Sidebar"]);
    assert_eq!(attempts.get(), 2);
}

#[test]
fn handler_effect_and_painter_failures_are_contained_where_they_happen() {
    let ui = ui();
    let hooked = Rc::new(RefCell::new(Vec::new()));
    let h2 = hooked.clone();
    ui.set_error_hook(Some(Rc::new(move |e| h2.borrow_mut().push(e.phase))));
    // A button whose handler fails.
    let ui2 = ui.clone();
    let (b1, _) = boundary(&ui, move || {
        ui2.el("button")
            .text("Save")
            .semantics(Semantics::new(Role::Button).label("Save").action(Action::Click))
            .on_click(|| fail("disk full"))
    });
    // A binding that fails when its signal changes.
    let bad = Signal::new(false);
    let ui3 = ui.clone();
    let (b2, _) = boundary(&ui, move || {
        label(&ui3, "Count").bind(move || {
            if bad.get() {
                fail("count overflowed");
            }
        })
    });
    // A painter that fails.
    let ui4 = ui.clone();
    let (b3, _) = boundary(&ui, move || {
        ui4.el("canvas")
            .width(20.0)
            .height(20.0)
            .paint(|_| fail("bad path"))
            .semantics(Semantics::new(Role::Image).label("Chart"))
    });
    let _ = ui.root().child(b1).child(b2).child(b3).child(label(&ui, "Still here"));
    // The painter fails while the first frame draws; its fallback shows in the next.
    frame(&ui);
    let shown = names(&ui);
    assert!(shown.contains(&"Failed: bad path".to_owned()), "the painter's boundary caught it: {shown:?}");
    let save = ui.reader_view().into_iter().find(|n| n.name.as_deref() == Some("Save")).must("save button");
    ui.accessibility_action(save.id, Action::Click);
    bad.set(true);
    let shown = names(&ui);
    assert!(shown.contains(&"Failed: disk full".to_owned()), "{shown:?}");
    assert!(shown.contains(&"Failed: count overflowed".to_owned()), "{shown:?}");
    assert_eq!(shown.last().map(String::as_str), Some("Still here"), "the rest of the window carries on");
    let phases = hooked.borrow().clone();
    for p in [FailurePhase::Paint, FailurePhase::Handler, FailurePhase::Effect] {
        assert!(phases.contains(&p), "{p:?} reached the hook: {phases:?}");
    }
    let located =
        ui.failures().into_iter().all(|f| f.location.is_some_and(|l| l.file().ends_with("flow.rs")));
    assert!(located, "every failure points at the source: {:?}", ui.failures());
}

#[test]
fn without_a_boundary_a_failing_handler_is_logged_and_the_window_goes_on() {
    let ui = ui();
    let clicks = Rc::new(Cell::new(0));
    let c2 = clicks.clone();
    let broken = ui.el("button").text("Broken").on_click(|| fail("oops"));
    let fine = ui.el("button").text("Fine").on_click(move || c2.set(c2.get() + 1));
    let _ = ui.root().child(broken.clone()).child(fine.clone());
    frame(&ui);
    assert!(ui.accessibility_action(broken.node_id(), Action::Click));
    assert!(ui.accessibility_action(fine.node_id(), Action::Click));
    assert_eq!(clicks.get(), 1);
    assert_eq!(ui.failures().len(), 1);
    assert_eq!(names(&ui), ["Broken", "Fine"]);
}

// ---------- suspense ----------

/// An executor holding jobs until the test runs them, so loading takes as long as a test
/// says.
#[derive(Clone, Default)]
struct Held(Arc<Mutex<Vec<Job>>>);

impl Executor for Held {
    fn spawn(&self, job: Job) {
        if let Ok(mut q) = self.0.lock() {
            q.push(job);
        }
    }
}

impl Held {
    fn run_all(&self) {
        let jobs: Vec<Job> = self.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for j in jobs {
            j();
        }
        run_posted();
    }
}

#[test]
fn suspense_shows_its_placeholder_late_and_long_enough_then_the_content() {
    let ui = ui();
    let held = Held::default();
    let user = Signal::new(1u32);
    let (ui2, ui3, h2) = (ui.clone(), ui.clone(), held.clone());
    let s = suspense(
        &ui,
        move || label(&ui2, "Loading profile"),
        move || {
            let r: Resource<String, ()> = Resource::with_executor(
                Arc::new(h2.clone()),
                move || user.get(),
                |id, _| Ok(format!("User {id}")),
            );
            ui3.el("label").text(move || r.get().unwrap_or_default())
        },
    )
    .delay(Duration::from_millis(100))
    .min_shown(Duration::from_millis(300));
    let _ = ui.root().child(s);
    assert!(names(&ui).is_empty(), "before the delay nothing shows: no flash");
    fire_timers(&ui); // the delay ends
    assert_eq!(names(&ui), ["Loading profile"]);
    held.run_all(); // the data arrives, but the placeholder has not been seen long enough
    assert_eq!(names(&ui), ["Loading profile"]);
    fire_timers(&ui); // the minimum time ends
    assert_eq!(names(&ui), ["User 1"]);

    // A quick reload shows no placeholder at all.
    user.set(2);
    assert!(names(&ui).is_empty() || names(&ui) == ["User 1"]);
    held.run_all();
    assert_eq!(names(&ui), ["User 2"]);
}

#[test]
fn removing_a_suspense_cancels_what_it_was_fetching() {
    let ui = ui();
    let held = Held::default();
    let delivered = Rc::new(Cell::new(false));
    let (ui2, ui3, h2, d2) = (ui.clone(), ui.clone(), held.clone(), delivered.clone());
    let s = suspense(
        &ui,
        move || label(&ui2, "Loading"),
        move || {
            let r: Resource<u32, ()> = Resource::with_executor(
                Arc::new(h2.clone()),
                || (),
                |(), c| {
                    assert!(c.is_cancelled(), "the fetch sees it is no longer wanted");
                    Ok(1)
                },
            );
            let d3 = d2.clone();
            ui3.el("label").bind(move || d3.set(r.get().is_some()))
        },
    )
    .el();
    let _ = ui.root().child(s.clone());
    frame(&ui);
    s.remove();
    held.run_all();
    assert!(!delivered.get());
}

// ---------- reactive properties ----------

#[test]
fn properties_take_values_signals_and_closures() {
    let ui = ui();
    let name = Signal::new(String::from("Ada"));
    let busy = Signal::new(false);
    let primary = Signal::new(true);
    let b = ui
        .el("button")
        .text(name)
        .disabled(move || busy.get())
        .variant(move || if primary.get() { "primary" } else { "quiet" })
        .style(StylePatch::new().radius(4.0))
        .style(move || Style::new().opacity(if busy.get() { 0.5 } else { 1.0 }));
    let _ = ui.root().child(b.clone());
    assert_eq!(names(&ui), ["Ada"]);
    name.set("Grace".into());
    busy.set(true);
    primary.set(false);
    assert_eq!(names(&ui), ["Grace"]);
    let info = b.info().must("alive");
    assert!(info.states.contains(States::DISABLED));
    assert_eq!(info.variant, "quiet");
    assert_eq!(info.style.opacity, 0.5);
    assert_eq!(info.style.radius, 4.0, "the bound layer goes over the constants, not instead");
    // Setting a property again replaces its binding.
    let b = b.text("Fixed");
    name.set("Ignored".into());
    assert_eq!(names(&ui), ["Fixed"]);
    let _ = b;
}

// ---------- typed tokens ----------

#[test]
fn tokens_lay_out_and_style_through_the_theme_and_follow_it() {
    let ui = ui();
    let row = ui.el("row").row().gap(Space::S3);
    let a = ui.el("box").width(Dp::from(Space::S4)).height(10.0);
    let b = ui.el("box").width(10.0).height(10.0);
    let button = ui
        .el("button")
        .text("Send")
        .style(Style::new().radius(Radius::R2).padding_x(Space::S4).background(ColorRole::Accent));
    let _ = ui.root().child(row.clone().child(a.clone()).child(b.clone())).child(button.clone());
    frame(&ui);
    let theme = ui.theme();
    let gap = |a: &El, b: &El| {
        b.bounds().map(|r| r.min.x).unwrap_or(0.0) - a.bounds().map(|r| r.max.x).unwrap_or(0.0)
    };
    assert_eq!(gap(&a, &b), theme.spacing.unit * 3.0);
    assert_eq!(a.bounds().map(|r| r.width()), Some(theme.spacing.unit * 4.0));
    let s = button.info().must("alive").style;
    assert_eq!(s.radius, theme.radii.medium);
    assert_eq!(s.background, theme.colors.accent);
    assert_eq!(
        (s.padding.start, s.padding.top),
        (theme.spacing.unit * 4.0, 5.0),
        "unset sides keep the theme's"
    );

    let mut other = theme.clone();
    other.spacing.unit = 8.0;
    other.radii.medium = 11.0;
    other.colors.accent = Color::rgb8(10, 120, 200);
    ui.set_theme(other, StyleSheet::new());
    frame(&ui);
    assert_eq!(gap(&a, &b), 24.0);
    assert_eq!(a.bounds().map(|r| r.width()), Some(32.0));
    let s = button.info().must("alive").style;
    assert_eq!((s.radius, s.background, s.padding.start), (11.0, Color::rgb8(10, 120, 200), 32.0));
}

// ---------- motion ----------

#[test]
fn elements_animate_on_the_frame_clock_and_turn_smoothly_when_interrupted() {
    let ui = ui();
    ui.set_frame_time(Duration::ZERO);
    let card = ui.el("card").width(50.0).height(50.0);
    let _ = ui.root().child(card.clone());
    frame(&ui);
    let opacity = |el: &El| el.info().map(|i| i.style.opacity).unwrap_or(-1.0);
    let tween = Motion::Tween { duration: Duration::from_millis(200), easing: Easing::Linear };
    card.animate(Target::Opacity(0.0), tween);
    frame(&ui);
    assert_eq!(opacity(&card), 1.0);
    assert!(ui.take_requests().contains(&Request::Frame), "a moving element asks for frames");
    ui.set_frame_time(Duration::from_millis(100));
    frame(&ui);
    assert!((opacity(&card) - 0.5).abs() < 0.01, "{}", opacity(&card));
    // A new target mid-way starts from where it is, not from the start or the end.
    card.animate(Target::Opacity(1.0), tween);
    ui.set_frame_time(Duration::from_millis(101));
    frame(&ui);
    assert!((opacity(&card) - 0.5).abs() < 0.02, "no jump: {}", opacity(&card));
    ui.set_frame_time(Duration::from_millis(400));
    frame(&ui);
    assert_eq!(opacity(&card), 1.0);
    assert!(!card.is_animating());
    let _ = ui.take_requests();
    frame(&ui);
    assert!(!ui.take_requests().contains(&Request::Frame), "settled: no more frames");

    // Reduced motion: instant.
    ui.set_reduced_motion(true);
    let done = Rc::new(Cell::new(false));
    let d2 = done.clone();
    card.animate_then(Target::Translate(Vec2::new(30.0, 0.0)), tween, move || d2.set(true));
    frame(&ui);
    assert!(done.get(), "an instant move finishes in the next frame");
    card.animate_style(&StylePatch::new().background(Color::rgb8(255, 0, 0)), tween);
    frame(&ui);
    assert_eq!(card.info().map(|i| i.style.background), Some(Color::rgb8(255, 0, 0)));
}

#[test]
fn painters_get_the_frame_time() {
    let ui = ui();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s2 = seen.clone();
    let canvas = ui.el("canvas").width(10.0).height(10.0).paint(move |cx| {
        s2.borrow_mut().push(cx.time);
        cx.request_next_frame();
    });
    let _ = ui.root().child(canvas);
    for ms in [0u64, 16, 33] {
        ui.set_frame_time(Duration::from_millis(ms));
        frame(&ui);
    }
    assert_eq!(*seen.borrow(), [0u64, 16, 33].map(Duration::from_millis));
}

// ---------- pseudo-locales and accessibility actions ----------

#[test]
fn pseudo_locales_reach_screen_readers_and_the_inspector() {
    let ui = ui();
    let title = ui.el("label").text("Settings");
    let named = ui.el("button").text("Save").semantics(Semantics::new(Role::Button).label("Save"));
    let file = ui.el("label").text("report.pdf").data_text();
    let _ = ui.root().child(title.clone()).child(named).child(file);
    assert_eq!(names(&ui), ["Settings", "Save", "report.pdf"]);
    ui.set_locale("en-XA");
    let shown = names(&ui);
    assert_ne!(shown.first().map(String::as_str), Some("Settings"), "{shown:?}");
    assert_ne!(shown.get(1).map(String::as_str), Some("Save"), "explicit names too: {shown:?}");
    assert_eq!(shown.get(2).map(String::as_str), Some("report.pdf"), "data stays as it is");
    let inspected = title.info().and_then(|i| i.text).must("text");
    assert_eq!(Some(&inspected), shown.first(), "the inspector shows what is drawn and read");
    ui.set_locale("en-US");
    assert_eq!(names(&ui), ["Settings", "Save", "report.pdf"]);
}

#[test]
fn elements_take_accessibility_actions_their_own_way() {
    let ui = ui();
    let open = Signal::new(false);
    let row = ui
        .el("row")
        .semantics(move || Semantics::new(Role::TreeItem).label("Photos").expanded(open.get()))
        .on_accessibility_action(move |a| match a {
            Action::Expand => {
                open.set(true);
                true
            }
            Action::Collapse => {
                open.set(false);
                true
            }
            _ => false,
        });
    let _ = ui.root().child(row.clone());
    frame(&ui);
    assert!(ui.accessibility_action(row.node_id(), Action::Expand));
    assert!(open.get());
    assert!(ui.accessibility_action(row.node_id(), Action::Collapse));
    assert!(!open.get());
    assert!(
        !ui.accessibility_action(row.node_id(), Action::ShowContextMenu),
        "what it does not take stays false"
    );
}

#[test]
fn elements_built_while_a_binding_runs_keep_their_own_bindings() {
    let ui = ui();
    let rerun = Signal::new(0);
    let name = Signal::new(String::from("Ada"));
    let painted = Rc::new(Cell::new(0));
    let kept: Rc<RefCell<Option<El>>> = Rc::default();
    let host = ui.el("panel").column();
    let (h, k, ui2, p2) = (host.clone(), kept.clone(), ui.clone(), painted.clone());
    let host = host.bind(move || {
        let _ = rerun.get();
        remedy_ui_core::reactive::untrack(|| {
            // Built once, on the first run, and kept: the binding running again must not
            // take its bindings and painter with it.
            if k.borrow().is_none() {
                let p3 = p2.clone();
                let el = ui2.el("label").text(name).paint(move |_| {
                    let _ = name.get();
                    p3.set(p3.get() + 1);
                });
                let _ = h.clone().child(el.clone());
                *k.borrow_mut() = Some(el);
            }
        });
    });
    let _ = ui.root().child(host);
    assert_eq!(names(&ui), ["Ada"]);
    rerun.set(1);
    rerun.set(2);
    name.set("Grace".into());
    assert_eq!(names(&ui), ["Grace"], "the text binding outlived the binding that built it");
    let before = painted.get();
    name.set("Hopper".into());
    frame(&ui);
    assert!(painted.get() > before, "so did the painter's watch");
    // Removing the element ends them.
    kept.borrow().clone().must("built").remove();
    name.set("Gone".into());
    assert!(names(&ui).is_empty());
}

#[test]
fn context_reaches_elements_whatever_scope_they_live_in() {
    let ui = ui();
    let seen = Rc::new(Cell::new(None));
    let s2 = seen.clone();
    let outer = ui.el("panel").provide(5u8).child_with(|| {
        let inner = ui.el("card");
        inner.child_with(|| {
            s2.set(use_context::<u8>());
            ui.el("label")
        })
    });
    let _ = ui.root().child(outer);
    assert_eq!(seen.get(), Some(5), "nested builds read context through the scopes they were made in");
}

#[test]
fn keyframes_loop_until_stopped_and_shadow_spread_animates() {
    use remedy_ui_view::motion::Repeat;
    let ui = ui();
    ui.set_frame_time(Duration::ZERO);
    let card = ui.el("card").width(50.0).height(50.0);
    let _ = ui.root().child(card.clone());
    frame(&ui);
    let step = Motion::Tween { duration: Duration::from_millis(100), easing: Easing::Linear };
    let pulse = card
        .animate_keyframes(vec![(Target::Opacity(0.2), step), (Target::Opacity(1.0), step)], Repeat::Forever);
    let opacity = |e: &El| e.info().map_or(0.0, |i| i.style.opacity);
    // Down by 100 ms, back up by 200 ms, down again by 300 ms: it loops.
    let mut seen = Vec::new();
    for ms in [0u64, 100, 200, 300, 400] {
        ui.set_frame_time(Duration::from_millis(ms));
        frame(&ui);
        frame(&ui);
        seen.push((opacity(&card) * 10.0).round() as i32);
    }
    assert!(seen.contains(&2) && seen.iter().filter(|v| **v == 10).count() >= 1, "{seen:?}");
    assert!(card.is_animating() && !pulse.is_stopped());
    pulse.stop();
    for ms in [500u64, 600, 700] {
        ui.set_frame_time(Duration::from_millis(ms));
        frame(&ui);
        frame(&ui);
    }
    assert!(!card.is_animating(), "stopped after the step it was in");
    // A bounded run stops itself, and the shadow's spread moves.
    let glow = card.animate_keyframes(vec![(Target::ShadowSpread(6.0), step)], Repeat::Times(1));
    for ms in [800u64, 900, 1000] {
        ui.set_frame_time(Duration::from_millis(ms));
        frame(&ui);
        frame(&ui);
    }
    assert!(glow.is_stopped());
    assert_eq!(card.info().map(|i| i.style.shadow.spread), Some(6.0));
}

#[test]
fn the_pointer_takes_the_shape_of_the_innermost_element_that_sets_one() {
    use remedy_ui_core::input::{PointerEvent, PointerPhase};
    use remedy_ui_view::Cursor;
    let ui = ui();
    let inner = ui.el("box").width(20.0).height(20.0);
    let outer = ui.el("box").width(100.0).height(100.0).cursor(Cursor::Grab).child(inner.clone());
    let _ = ui.root().child(outer);
    frame(&ui);
    assert_eq!(ui.cursor(), Cursor::Default, "nothing under the pointer yet");
    let at = |el: &El, dx: f32| el.bounds().map(|b| remedy_ui_core::Point::new(b.min.x + dx, b.min.y + 5.0));
    let over_inner = at(&inner, 5.0).must("laid out");
    ui.pointer(&PointerEvent::new(PointerPhase::Move, over_inner));
    assert_eq!(ui.cursor(), Cursor::Grab, "inherited from the parent");
    let _ = inner.clone().cursor(Cursor::NotAllowed);
    assert_eq!(ui.cursor(), Cursor::NotAllowed);
    ui.pointer(&PointerEvent::new(
        PointerPhase::Move,
        remedy_ui_core::Point::new(over_inner.x + 60.0, over_inner.y),
    ));
    assert_eq!(ui.cursor(), Cursor::Grab, "outside the child, still inside the parent");
}
