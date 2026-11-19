//! Control flow in the tree: context for subtrees, conditional subtrees ([`show`]), error
//! boundaries ([`catch_errors`]) and suspense boundaries ([`suspense`]).
//!
//! Each builds its content inside a reactive scope of its own, owned by the element that
//! holds it: hiding a `show`, retrying a boundary or switching a suspense disposes what the
//! old content made (its signals, effects and resources, which cancel their fetches) before
//! the new content is built.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, Location, catch_unwind};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_reactive::{ErrorSink, Memo, Scope, Signal, Suspense, provide_context, untrack};
use remedy_ui_tree::ElementId;

use crate::element::{El, Ui};
use crate::motion::{Motion, Target};
use crate::prop::{IntoProp, Prop};

// ---------- context ----------

impl El {
    /// The reactive scope that owns what is made for the element (its bindings, and what is
    /// built with [`El::child_with`]).
    pub fn scope(&self) -> Option<Scope> {
        self.ui.tree().scope(self.id)
    }

    /// Provides `value` to the element's subtree: descendants read it with
    /// [`El::use_context`], and code built inside the element ([`El::child_with`], its
    /// bindings) with [`remedy_ui_reactive::use_context`]. A value that changes is provided
    /// as a [`Signal`].
    pub fn provide<T: 'static>(self, value: T) -> Self {
        if let Some(s) = self.scope() {
            s.provide_context(value);
        }
        self
    }

    /// The nearest `T` provided to this element: by itself or an ancestor in the tree
    /// ([`El::provide`]), or else where it was built (the reactive owner it was made under).
    pub fn use_context<T: Clone + 'static>(&self) -> Option<T> {
        let tree = self.ui.tree();
        let mut cur = Some(self.id);
        while let Some(id) = cur {
            if let Some(v) = tree.scope(id).and_then(|s| s.own_context::<T>()) {
                return Some(v);
            }
            cur = tree.parent(id);
        }
        self.scope().and_then(|s| s.use_context::<T>())
    }

    /// Builds a child inside this element's scope, so it reads what the element provides
    /// at build time and is disposed with it, and adds it at the end.
    pub fn child_with(self, build: impl FnOnce() -> El) -> Self {
        let child = match self.scope() {
            Some(s) => s.run(build).ok(),
            None => Some(build()),
        };
        match child {
            Some(c) => self.child(c),
            None => self,
        }
    }
}

// ---------- failures ----------

/// Where failing code ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailurePhase {
    /// Building elements.
    Build,
    /// An event handler.
    Handler,
    /// A binding or effect.
    Effect,
    /// A canvas painter.
    Paint,
}

impl FailurePhase {
    /// Where it failed, as a phrase: "while building", "in an event handler", "in a binding
    /// or effect", "while painting".
    pub fn describe(self) -> &'static str {
        match self {
            FailurePhase::Build => "while building",
            FailurePhase::Handler => "in an event handler",
            FailurePhase::Effect => "in a binding or effect",
            FailurePhase::Paint => "while painting",
        }
    }
}

/// A failure an error boundary caught: what failed, where in the tree, and where in the
/// source.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CaughtError {
    /// The panic message.
    pub message: String,
    /// The failing element's path, such as `root/content/settings/save`.
    pub element_path: String,
    /// Where the failing element (or effect) was created in the source.
    pub location: Option<&'static Location<'static>>,
    /// What was running.
    pub phase: FailurePhase,
    /// The error boundary that took it ([`Ui::retry_failure`] builds its content again), or
    /// `None` when no boundary was above the failing code.
    pub boundary: Option<ElementId>,
}

impl std::fmt::Display for CaughtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(l) = self.location {
            write!(f, " ({}:{})", l.file(), l.line())?;
        }
        Ok(())
    }
}

impl std::error::Error for CaughtError {}

/// Told about every failure a window catches ([`Ui::set_error_hook`]).
pub type ErrorHook = Rc<dyn Fn(&CaughtError)>;

/// Retries an error boundary: its content is built again.
#[derive(Clone, Copy, Debug)]
pub struct Retry(Signal<Option<CaughtError>>);

impl Retry {
    /// Builds the content again.
    pub fn retry(&self) {
        let _ = self.0.try_set(None);
    }
}

/// Hooks the view into the tree's and the reactive runtime's failure paths: a panicking
/// handler, binding or effect of one of this window's elements is contained at the nearest
/// boundary (or, with none, logged and contained there).
pub(crate) fn install_failure_handling(ui: &Ui) {
    let weak = ui.downgrade();
    ui.tree().set_failure_handler(Some(Rc::new(move |at, message| {
        if let Some(ui) = weak.upgrade() {
            ui.report_failure(at, FailurePhase::Handler, message, None);
        }
    })));
    let weak = ui.downgrade();
    let id = remedy_ui_reactive::add_panic_handler(move |c| {
        let Some(ui) = weak.upgrade() else { return false };
        let Some(el) = ui.element_of_binding(c.node) else { return false };
        ui.report_failure(el, FailurePhase::Effect, c.message.clone(), c.created_at);
        true
    });
    ui.0.borrow_mut().panic_handler = id;
}

impl Ui {
    /// Records a failure of `at`'s code with the nearest error boundary above it (the
    /// boundary unmounts its content and shows its fallback), the window's failure list and
    /// the app's error hook. With no boundary it is logged; the rest of the window carries
    /// on either way. Returns whether a boundary took it.
    pub fn report_failure(
        &self,
        at: ElementId,
        phase: FailurePhase,
        message: String,
        location: Option<&'static Location<'static>>,
    ) -> bool {
        let tree = self.tree();
        let (boundary, err, hook) = {
            let Ok(i) = self.0.try_borrow() else { return false };
            let location = location.or_else(|| i.els.get(&at).and_then(|d| d.created_at));
            let mut cur = Some(at);
            let mut found = None;
            while let Some(e) = cur {
                if let Some(s) = i.boundaries.get(&e) {
                    found = Some((e, *s));
                    break;
                }
                cur = tree.parent(e);
            }
            let err = CaughtError {
                message,
                element_path: tree.path(at),
                location,
                phase,
                boundary: found.map(|f| f.0),
            };
            (found.map(|f| f.1), err, i.error_hook.clone())
        };
        let rev = match self.0.try_borrow_mut() {
            Ok(mut i) => {
                i.failures.push(err.clone());
                Some(i.failures_rev)
            }
            Err(_) => None,
        };
        if let Some(r) = rev {
            let _ = r.try_update(|v| *v += 1);
        }
        if let Some(h) = hook {
            h(&err);
        }
        match boundary {
            Some(s) => {
                // The first failure stays: what broke first explains what broke after.
                let _ = s.try_update_if(|cur| {
                    if cur.is_none() {
                        *cur = Some(err);
                        true
                    } else {
                        false
                    }
                });
                true
            }
            None => {
                remedy_ui_trace::tracing::error!("{} at {} ({:?})", err, err.element_path, err.phase);
                false
            }
        }
    }

    /// Calls `f` with every failure caught in this window (by a boundary or not), for the
    /// app's own logging or crash reports. `None` removes it.
    pub fn set_error_hook(&self, f: Option<ErrorHook>) {
        self.0.borrow_mut().error_hook = f;
    }

    /// The failures caught in this window so far, oldest first. Reading it inside a binding
    /// follows the list (the inspector's Failures page does).
    pub fn failures(&self) -> Vec<CaughtError> {
        let rev = self.0.borrow().failures_rev;
        let _ = rev.try_with(|_| ());
        self.0.borrow().failures.clone()
    }

    /// Forgets the failures caught so far (boundaries keep showing their fallbacks).
    pub fn clear_failures(&self) {
        let rev = {
            let mut i = self.0.borrow_mut();
            i.failures.clear();
            i.failures_rev
        };
        let _ = rev.try_update(|v| *v += 1);
    }

    /// True while the error boundary `boundary` shows a failure.
    pub fn is_failed(&self, boundary: ElementId) -> bool {
        let s = self.0.borrow().boundaries.get(&boundary).copied();
        s.is_some_and(|s| s.try_with(Option::is_some).unwrap_or(false))
    }

    /// Builds the content of the error boundary `boundary` again (as its fallback's
    /// [`Retry`] does). False when it is gone or shows no failure.
    pub fn retry_failure(&self, boundary: ElementId) -> bool {
        let s = self.0.borrow().boundaries.get(&boundary).copied();
        let Some(s) = s else { return false };
        let mut took = false;
        let _ = s.try_update_if(|f| {
            took = f.take().is_some();
            took
        });
        took
    }
}

/// Content built in a scope of its own: removing it removes the element and disposes the
/// scope (its signals, effects and resources).
struct Mounted {
    el: El,
    scope: Option<Scope>,
}

impl Mounted {
    /// Builds `f` in a new scope owned by `host`'s scope.
    fn build(host: &El, f: impl FnOnce() -> El) -> Option<Mounted> {
        let make = || {
            let (el, scope) = Scope::new(f);
            (el, Some(scope))
        };
        let (el, scope) = match host.scope() {
            Some(s) => s.run(make).ok()?,
            None => make(),
        };
        Some(Mounted { el, scope })
    }

    fn remove(self) {
        self.el.remove();
        if let Some(s) = self.scope {
            s.dispose();
        }
    }
}

/// Binds `f` to `el`, the binding created inside the element's scope so what it reads of
/// context is the element's.
#[track_caller]
fn bind_in_scope(el: &El, name: &'static str, f: impl FnMut() + 'static) {
    let me = el.clone();
    match el.scope() {
        Some(s) => {
            let _ = s.run(move || me.bind_named(name, f));
        }
        None => {
            let _ = me.bind_named(name, f);
        }
    }
}

/// An error boundary: shows what `child` builds, and when building it, or any handler,
/// binding, effect or painter inside it, panics, unmounts that content and shows what
/// `fallback` builds from the error and a [`Retry`]. The rest of the window carries on.
/// The error also goes to the window's failure list and error hook ([`Ui::set_error_hook`]).
///
/// `child` is a builder, a closure that makes the content, not an element made beforehand:
/// the boundary calls it to build the content inside its own scope (so a panic while
/// building is caught too), and calls it again on [`Retry`] for a fresh copy. `fallback`
/// is a builder as well, called each time the content fails.
///
/// ```
/// # use remedy_ui_view::{Ui, catch_errors};
/// # fn demo(ui: &Ui) {
/// let (u, u2) = (ui.clone(), ui.clone());
/// let panel = catch_errors(
///     ui,
///     move |error, retry| {
///         u.el("button").text(format!("{} (retry)", error.message)).on_click(move || retry.retry())
///     },
///     move || u2.el("label").text("The panel"),
/// );
/// ui.root().child(panel);
/// # }
/// ```
#[track_caller]
pub fn catch_errors(
    ui: &Ui,
    fallback: impl Fn(&CaughtError, Retry) -> El + 'static,
    child: impl Fn() -> El + 'static,
) -> El {
    let host = ui.el_at("error_boundary", Location::caller()).column();
    let failed: Signal<Option<CaughtError>> = match host.scope() {
        Some(s) => s.run(|| Signal::new(None)).unwrap_or_else(|_| Signal::new(None)),
        None => Signal::new(None),
    };
    ui.0.borrow_mut().boundaries.insert(host.id(), failed);
    let shown: Rc<RefCell<Option<Mounted>>> = Rc::default();
    let (h, ui2) = (host.clone(), ui.clone());
    let run = move || {
        let state = failed.get();
        untrack(|| {
            if let Some(old) = shown.borrow_mut().take() {
                old.remove();
            }
            let built = match &state {
                None => {
                    let (ui3, h3) = (ui2.clone(), h.clone());
                    let mut broke = None;
                    let m = Mounted::build(&h, || {
                        // Effects made inside report here, even before their element is
                        // attached to the tree.
                        provide_context(ErrorSink::new(move |c| {
                            let at = ui3.element_of_binding(c.node).unwrap_or(h3.id());
                            ui3.report_failure(at, FailurePhase::Effect, c.message, c.created_at);
                        }));
                        match catch_unwind(AssertUnwindSafe(&child)) {
                            Ok(el) => el,
                            Err(p) => {
                                broke = Some(remedy_ui_reactive::panic_message(p.as_ref()));
                                ui2.el("error_boundary_empty")
                            }
                        }
                    });
                    if let Some(msg) = broke {
                        if let Some(m) = m {
                            m.remove();
                        }
                        ui2.report_failure(h.id(), FailurePhase::Build, msg, None);
                        None
                    } else {
                        m
                    }
                }
                Some(err) => {
                    let mut broke = None;
                    let m = Mounted::build(&h, || {
                        match catch_unwind(AssertUnwindSafe(|| fallback(err, Retry(failed)))) {
                            Ok(el) => el,
                            Err(p) => {
                                broke = Some(remedy_ui_reactive::panic_message(p.as_ref()));
                                ui2.el("error_boundary_empty")
                            }
                        }
                    });
                    if let Some(msg) = broke {
                        if let Some(m) = m {
                            m.remove();
                        }
                        // A failing fallback goes to the boundary above.
                        let above = h.parent().map_or(h.id(), |p| p.id());
                        ui2.report_failure(above, FailurePhase::Build, msg, None);
                        None
                    } else {
                        m
                    }
                }
            };
            if let Some(m) = built {
                let _ = h.clone().child(m.el.clone());
                *shown.borrow_mut() = Some(m);
            }
        });
    };
    bind_in_scope(&host, "catch_errors", run);
    host
}

// ---------- show ----------

/// How content arrives or leaves: it starts from (or ends at) these values, moving with
/// `motion`. Under reduced motion it is instant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Transition {
    /// Where entering content starts and leaving content ends, such as
    /// `[Target::Opacity(0.0), Target::Translate(Vec2::new(0.0, 8.0))]`.
    pub from: Vec<Target>,
    /// How it moves.
    pub motion: Motion,
}

impl Transition {
    /// Content starting from (or ending at) `from`, moving with `motion`.
    pub fn new(from: Vec<Target>, motion: Motion) -> Self {
        Transition { from, motion }
    }

    /// A fade with `motion`.
    pub fn fade(motion: Motion) -> Self {
        Transition { from: vec![Target::Opacity(0.0)], motion }
    }

    /// A fade that also slides by `offset`.
    pub fn slide(offset: remedy_ui_core::Vec2, motion: Motion) -> Self {
        Transition { from: vec![Target::Opacity(0.0), Target::Translate(offset)], motion }
    }
}

/// The rest value of each property a transition moves.
fn rest(t: &Target) -> Target {
    match t {
        Target::Opacity(_) => Target::Opacity(1.0),
        Target::Translate(_) => Target::Translate(remedy_ui_core::Vec2::ZERO),
        Target::Scale(_) => Target::Scale(1.0),
        other => *other,
    }
}

/// Plays `t` in on `el`: it jumps to the start values and moves to rest.
pub fn play_enter(el: &El, t: &Transition) {
    for f in &t.from {
        el.set_animated(*f);
        el.animate(rest(f), t.motion);
    }
}

/// Plays `t` out on `el`, which is inert (outside hit testing and accessibility) while it
/// leaves, then calls `gone` (which removes it).
pub fn play_exit(el: &El, t: &Transition, gone: impl FnOnce() + 'static) {
    let _ = el.ui.tree().set_hit_mode(el.id, remedy_ui_tree::HitMode::None);
    let _ = el.clone().semantics(remedy_ui_a11y::Semantics::new(remedy_ui_a11y::Role::Group).hidden(true));
    if t.from.is_empty() {
        gone();
        return;
    }
    let left = Rc::new(std::cell::Cell::new(t.from.len()));
    let gone = Rc::new(RefCell::new(Some(gone)));
    for f in &t.from {
        let (left, gone) = (left.clone(), gone.clone());
        el.animate_then(*f, t.motion, move || {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0
                && let Some(g) = gone.borrow_mut().take()
            {
                g();
            }
        });
    }
}

/// Content shown while a condition holds ([`show`]): built when the condition becomes
/// true, disposed when it becomes false, with an optional fallback in between and
/// transitions in and out. It becomes an element with [`Show::el`] (or `El::from`, so it
/// can be passed to [`El::child`]).
#[must_use = "a Show does nothing until it becomes an element"]
pub struct Show {
    ui: Ui,
    when: Prop<bool>,
    build: Rc<dyn Fn() -> El>,
    fallback: Option<Rc<dyn Fn() -> El>>,
    enter: Option<Transition>,
    exit: Option<Transition>,
    at: &'static Location<'static>,
}

impl std::fmt::Debug for Show {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Show").field("enter", &self.enter).field("exit", &self.exit).finish()
    }
}

/// Shows what `build` makes while `when` is true: a signal, a closure or a constant. The
/// content is built in a scope of its own each time it appears and disposed (its effects,
/// signals and resources with it) when it goes.
/// While it shows nothing (no fallback, nothing leaving), it takes no room and no gap in its
/// row or column.
///
/// ```
/// # use remedy_ui_view::{Ui, show};
/// # use remedy_ui_reactive::Signal;
/// # fn demo(ui: &Ui) {
/// let error = Signal::new(None::<String>);
/// let banner = show(ui, move || error.get().is_some(), {
///     let ui = ui.clone();
///     move || ui.el("label").text(move || error.get().unwrap_or_default())
/// })
/// .el();
/// # let _ = banner;
/// # }
/// ```
#[track_caller]
pub fn show(ui: &Ui, when: impl IntoProp<bool>, build: impl Fn() -> El + 'static) -> Show {
    Show {
        ui: ui.clone(),
        when: when.into_prop(),
        build: Rc::new(build),
        fallback: None,
        enter: None,
        exit: None,
        at: Location::caller(),
    }
}

impl Show {
    /// Shows what `f` makes while the condition is false.
    pub fn fallback(mut self, f: impl Fn() -> El + 'static) -> Self {
        self.fallback = Some(Rc::new(f));
        self
    }

    /// Plays `t` as content appears (not for what shows when it is first built).
    pub fn enter(mut self, t: Transition) -> Self {
        self.enter = Some(t);
        self
    }

    /// Plays `t` as content leaves; it stays, inert, until the transition ends.
    pub fn exit(mut self, t: Transition) -> Self {
        self.exit = Some(t);
        self
    }

    /// The element holding the content (a column the content fills).
    pub fn el(self) -> El {
        let Show { ui, when, build, fallback, enter: t_in, exit: t_out, at } = self;
        let host = ui.el_at("show", at).column();
        // Only a change of the condition's value rebuilds, not every change it reads.
        let when = match host.scope() {
            Some(s) => s.run(|| Memo::new(move || when.get())).ok(),
            None => None,
        };
        let Some(when) = when else { return host };
        let current: Rc<RefCell<Option<(bool, Mounted)>>> = Rc::default();
        let h = host.clone();
        let mut first = true;
        let run = move || {
            let on = when.get();
            untrack(|| {
                if current.borrow().as_ref().is_some_and(|(was, _)| *was == on) {
                    return;
                }
                if let Some((_, old)) = current.borrow_mut().take() {
                    match &t_out {
                        Some(t) => {
                            let (el, h2) = (old.el.clone(), h.clone());
                            play_exit(&el, t, move || {
                                old.remove();
                                collapse_if_empty(&h2);
                            });
                        }
                        None => old.remove(),
                    }
                }
                let make = if on { Some(build.clone()) } else { fallback.clone() };
                if let Some(m) = make.and_then(|make| Mounted::build(&h, || make())) {
                    let _ = h.clone().child(m.el.clone());
                    if let (Some(t), false) = (&t_in, first) {
                        play_enter(&m.el, t);
                    }
                    *current.borrow_mut() = Some((on, m));
                }
                // Showing nothing (no fallback, nothing leaving), it takes no room and no gap
                // in its row or column, as content that is not there would not.
                collapse_if_empty(&h);
                first = false;
            });
        };
        bind_in_scope(&host, "show", run);
        host
    }
}

/// Collapses `host` while it holds nothing, and opens it again once it does.
fn collapse_if_empty(host: &El) {
    host.set_collapsed(host.ui.tree().children(host.id).is_empty());
}

impl From<Show> for El {
    fn from(s: Show) -> El {
        s.el()
    }
}

// ---------- suspense ----------

/// A boundary that shows a placeholder while resources read inside its content are still
/// loading ([`suspense`]).
#[must_use = "a SuspenseBoundary does nothing until it becomes an element"]
pub struct SuspenseBoundary {
    ui: Ui,
    placeholder: Rc<dyn Fn() -> El>,
    child: Rc<dyn Fn() -> El>,
    delay: Duration,
    min_shown: Duration,
    at: &'static Location<'static>,
}

impl std::fmt::Debug for SuspenseBoundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SuspenseBoundary")
            .field("delay", &self.delay)
            .field("min_shown", &self.min_shown)
            .finish()
    }
}

/// How long a suspense waits before showing its placeholder, by default: a load that ends
/// sooner shows no placeholder at all ("every action answers within 100 ms").
pub const SUSPENSE_DELAY: Duration = Duration::from_millis(100);

/// How long a suspense keeps its placeholder once shown, by default, so it never flashes
/// (the long end of the design's 100–300 ms motion range).
pub const SUSPENSE_MIN_SHOWN: Duration = Duration::from_millis(300);

/// Shows what `child` builds once every resource read inside it has loaded, and what
/// `placeholder` builds while one is loading. The content is built (hidden) at once, so its
/// resources start fetching; the placeholder appears only after [`SUSPENSE_DELAY`] and stays
/// at least [`SUSPENSE_MIN_SHOWN`], so quick loads show nothing and slow ones never flash.
#[track_caller]
pub fn suspense(
    ui: &Ui,
    placeholder: impl Fn() -> El + 'static,
    child: impl Fn() -> El + 'static,
) -> SuspenseBoundary {
    SuspenseBoundary {
        ui: ui.clone(),
        placeholder: Rc::new(placeholder),
        child: Rc::new(child),
        delay: SUSPENSE_DELAY,
        min_shown: SUSPENSE_MIN_SHOWN,
        at: Location::caller(),
    }
}

/// What a suspense shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Showing {
    /// The content.
    Content,
    /// Loading, before the delay: nothing.
    Waiting,
    /// The placeholder; `held` once it has been shown long enough.
    Placeholder { held: bool },
}

impl SuspenseBoundary {
    /// Waits `d` before showing the placeholder (zero shows it at once).
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// Keeps the placeholder at least `d` once shown.
    pub fn min_shown(mut self, d: Duration) -> Self {
        self.min_shown = d;
        self
    }

    /// The boundary's element.
    pub fn el(self) -> El {
        let SuspenseBoundary { ui, placeholder, child, delay, min_shown, at } = self;
        let host = ui.el_at("suspense", at).column();
        let Some(scope) = host.scope() else { return host };
        let Ok(registry) = scope.run(Suspense::new) else { return host };
        scope.provide_context(registry.clone());
        let Some(content) = Mounted::build(&host, || child()) else { return host };
        let content_el = content.el.clone().hidden(true);
        let host = host.child(content_el.clone()).on_removed(move || content.remove());
        let Ok(showing) = scope.run(|| Signal::new(Showing::Content)) else { return host };
        let ph: Rc<RefCell<Option<Mounted>>> = Rc::default();
        let applied: Rc<std::cell::Cell<Option<Showing>>> = Rc::default();
        let (h, ui2) = (host.clone(), ui.clone());
        let decide = move || {
            let pending = registry.pending();
            let now = showing.get();
            untrack(|| {
                let next = match (pending, now) {
                    (true, Showing::Content) if delay.is_zero() => {
                        Showing::Placeholder { held: min_shown.is_zero() }
                    }
                    (true, Showing::Content) => Showing::Waiting,
                    (true, s) | (false, s @ Showing::Placeholder { held: false }) => s,
                    (false, _) => Showing::Content,
                };
                if next == Showing::Waiting && now != Showing::Waiting {
                    ui2.after(delay, move || {
                        let held = min_shown.is_zero();
                        let _ = showing.try_update_if(|s| {
                            let wait = *s == Showing::Waiting;
                            if wait {
                                *s = Showing::Placeholder { held };
                            }
                            wait
                        });
                    });
                }
                let entering = matches!(next, Showing::Placeholder { held: false })
                    && !matches!(applied.get(), Some(Showing::Placeholder { .. }));
                if entering {
                    ui2.after(min_shown, move || {
                        let _ = showing.try_update_if(|s| {
                            let hold = *s == Showing::Placeholder { held: false };
                            if hold {
                                *s = Showing::Placeholder { held: true };
                            }
                            hold
                        });
                    });
                }
                applied.set(Some(next));
                if next != now {
                    let _ = showing.try_set(next);
                }
                let _ = content_el.clone().hidden(next != Showing::Content);
                let want = matches!(next, Showing::Placeholder { .. });
                let has = ph.borrow().is_some();
                if want && !has {
                    if let Some(m) = Mounted::build(&h, || placeholder()) {
                        let _ = h.clone().child(m.el.clone());
                        *ph.borrow_mut() = Some(m);
                    }
                } else if !want && let Some(m) = ph.borrow_mut().take() {
                    m.remove();
                }
            });
        };
        bind_in_scope(&host, "suspense", decide);
        host
    }
}

impl From<SuspenseBoundary> for El {
    fn from(s: SuspenseBoundary) -> El {
        s.el()
    }
}
