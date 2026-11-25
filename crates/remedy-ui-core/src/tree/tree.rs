//! The element tree.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::reactive::{Scope, Signal, batch, panic_message};
use crate::{Arena, Id, Point, Rect};

use crate::tree::dirty::Dirty;

/// Identifies an element in a [`Tree`]. Stale ids (of removed elements) are harmless: every
/// operation on them reports [`TreeError::NotFound`] or returns nothing.
pub type ElementId = Id<Node>;

/// Why a tree operation could not be done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TreeError {
    /// The element does not exist (it was removed, or the id is from another tree).
    NotFound,
    /// The child already has a parent; remove it first.
    AlreadyAttached,
    /// Attaching would make an element its own ancestor.
    WouldCycle,
    /// The root cannot be removed or attached elsewhere.
    Root,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TreeError::NotFound => "the element does not exist",
            TreeError::AlreadyAttached => "the element already has a parent",
            TreeError::WouldCycle => "an element cannot be its own ancestor",
            TreeError::Root => "the root element cannot be moved or removed",
        })
    }
}

impl std::error::Error for TreeError {}

/// Whether pointer hit testing can land on an element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HitMode {
    /// The element and its children can be hit.
    #[default]
    Normal,
    /// The element itself is click-through; its children can still be hit.
    PassThrough,
    /// Neither the element nor anything inside it can be hit.
    None,
}

/// Event phase a handler listens in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Phase {
    /// From the root down to the target's parent, before the target.
    Capture,
    /// At the target, then from its parent up to the root.
    Bubble,
}

/// A failure caught by an error boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Failure {
    /// Path of the element whose code failed, such as `root/settings/save`.
    pub element_path: String,
    /// The panic message.
    pub message: String,
}

type HandlerFn = Rc<dyn Fn(&mut EventCx, &dyn Any)>;

#[derive(Clone)]
struct Handler {
    id: u64,
    event: TypeId,
    phase: Phase,
    f: HandlerFn,
}

/// One element's data. Only reachable through [`Tree`].
pub struct Node {
    name: Cow<'static, str>,
    parent: Option<ElementId>,
    children: Vec<ElementId>,
    dirty: Dirty,
    in_dirty_list: bool,
    relayout_boundary: bool,
    repaint_boundary: bool,
    bounds: Rect,
    clips: bool,
    hit: HitMode,
    /// Drawn over its in-flow siblings (a sticky element), so hit before them.
    raised: bool,
    focusable: bool,
    /// Out of the Tab order while focusable (HTML's `tabindex="-1"`).
    skip_tab: bool,
    focus_order: Option<i32>,
    focus_scope: bool,
    boundary: Option<Signal<Option<Failure>>>,
    handlers: Vec<Handler>,
    scope: Scope,
    payload: Option<Box<dyn Any>>,
}

struct Data {
    nodes: Arena<Node>,
    root: ElementId,
    focus: Option<ElementId>,
    dirty_list: Vec<ElementId>,
    next_handler: u64,
    marks: u64,
    /// Takes failures in place of the tree's own boundaries ([`Tree::set_failure_handler`]).
    failure_handler: Option<FailureHandler>,
}

/// Takes a failure: the element whose code failed, and the panic message.
pub type FailureHandler = Rc<dyn Fn(ElementId, String)>;

/// A window's element tree. Cloning gives another handle to the same tree, so bindings and
/// handlers can capture it.
///
/// The tree never runs app code while it is borrowed: handlers, focus events and guarded
/// closures run after the borrow ends, inside a reactive batch.
#[derive(Clone)]
pub struct Tree(Rc<RefCell<Data>>);

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.0.borrow();
        f.debug_struct("Tree").field("elements", &d.nodes.len()).field("focus", &d.focus).finish()
    }
}

/// Passed to event handlers.
#[derive(Debug)]
pub struct EventCx {
    target: ElementId,
    current: ElementId,
    phase: Phase,
    stopped: bool,
    default_prevented: bool,
    handled: bool,
    handled_by: Option<ElementId>,
}

impl EventCx {
    /// The element the event was dispatched to.
    pub fn target(&self) -> ElementId {
        self.target
    }

    /// The element whose handler is running.
    pub fn current(&self) -> ElementId {
        self.current
    }

    /// The phase the running handler is in.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Stops the event from reaching further elements. Handlers on the current element
    /// still run.
    pub fn stop_propagation(&mut self) {
        self.stopped = true;
    }

    /// Asks the engine not to do the default action for this event (for example, scroll).
    pub fn prevent_default(&mut self) {
        self.default_prevented = true;
    }

    /// Records that a handler dealt with the event.
    pub fn set_handled(&mut self) {
        self.handled = true;
        self.handled_by.get_or_insert(self.current);
    }
}

/// What happened when an event was dispatched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Dispatch {
    /// Handlers that ran (including ones that panicked).
    pub handlers_run: usize,
    /// A handler called [`EventCx::set_handled`].
    pub handled: bool,
    /// A handler called [`EventCx::stop_propagation`].
    pub stopped: bool,
    /// A handler called [`EventCx::prevent_default`].
    pub default_prevented: bool,
    /// Handlers that panicked; each was caught by an error boundary.
    pub panicked: usize,
    /// The element whose handler first marked the event handled.
    pub handled_by: Option<ElementId>,
}

/// Sent to an element when it gains focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusIn;

/// Sent to an element when it loses focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusOut;

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    /// A tree holding only its root. The root is a relayout and repaint boundary, a focus
    /// scope and an error boundary.
    #[track_caller]
    pub fn new() -> Self {
        let mut nodes = Arena::new();
        let ((), scope) = Scope::new(|| {});
        let boundary = Signal::new(None);
        let root = match nodes.insert(Node::new("root".into(), scope)) {
            Ok(id) => id,
            Err(_) => std::process::abort(),
        };
        if let Some(r) = nodes.get_mut(root) {
            r.relayout_boundary = true;
            r.repaint_boundary = true;
            r.focus_scope = true;
            r.boundary = Some(boundary);
        }
        Tree(Rc::new(RefCell::new(Data {
            nodes,
            root,
            focus: None,
            dirty_list: Vec::new(),
            next_handler: 0,
            marks: 0,
            failure_handler: None,
        })))
    }

    /// The root element.
    pub fn root(&self) -> ElementId {
        self.0.borrow().root
    }

    /// Number of elements, the root included.
    pub fn len(&self) -> usize {
        self.0.borrow().nodes.len()
    }

    /// Always false: a tree has at least its root.
    pub fn is_empty(&self) -> bool {
        false
    }

    // ---------- structure ----------

    /// A new, detached element. Its reactive scope owns anything made for it, and is disposed
    /// when the element is removed.
    #[track_caller]
    pub fn create(&self, name: impl Into<Cow<'static, str>>) -> ElementId {
        let ((), scope) = Scope::detached(|| {});
        match self.0.borrow_mut().nodes.insert(Node::new(name.into(), scope)) {
            Ok(id) => id,
            Err(_) => std::process::abort(),
        }
    }

    /// Runs `f` with the element's reactive scope as owner, so signals and effects it makes
    /// are disposed with the element.
    pub fn in_scope<R>(&self, id: ElementId, f: impl FnOnce() -> R) -> Result<R, TreeError> {
        let scope = self.0.borrow().nodes.get(id).map(|n| n.scope).ok_or(TreeError::NotFound)?;
        scope.run(f).map_err(|_| TreeError::NotFound)
    }

    /// Adds `child` as the last child of `parent`.
    pub fn append(&self, parent: ElementId, child: ElementId) -> Result<(), TreeError> {
        let len = self.0.borrow().nodes.get(parent).map(|n| n.children.len()).ok_or(TreeError::NotFound)?;
        self.insert(parent, len, child)
    }

    /// Inserts `child` among `parent`'s children at `index` (clamped to the end).
    pub fn insert(&self, parent: ElementId, index: usize, child: ElementId) -> Result<(), TreeError> {
        {
            let mut d = self.0.borrow_mut();
            if child == d.root {
                return Err(TreeError::Root);
            }
            if !d.nodes.contains(parent) {
                return Err(TreeError::NotFound);
            }
            let leaf = match d.nodes.get(child) {
                None => return Err(TreeError::NotFound),
                Some(c) if c.parent.is_some() => return Err(TreeError::AlreadyAttached),
                Some(c) => c.children.is_empty(),
            };
            // An element with no children is an ancestor of nothing but itself: only a subtree
            // needs the walk up from `parent`, so a tree is built in linear time at any depth.
            if child == parent || (!leaf && d.is_ancestor_or_self(child, parent)) {
                return Err(TreeError::WouldCycle);
            }
            if let Some(p) = d.nodes.get_mut(parent) {
                let at = index.min(p.children.len());
                p.children.insert(at, child);
            }
            if let Some(c) = d.nodes.get_mut(child) {
                c.parent = Some(parent);
            }
        }
        self.mark(parent, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        self.mark(child, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        Ok(())
    }

    /// Detaches `id` from its parent without destroying it, so it can be attached elsewhere
    /// (moving a subtree between windows or containers). Focus inside it moves to the nearest
    /// focusable element around it (a dialog whose button went away keeps its keys), or clears.
    pub fn detach(&self, id: ElementId) -> Result<(), TreeError> {
        let heir = {
            let d = self.0.borrow();
            let inside = d.focus.is_some_and(|f| d.is_ancestor_or_self(id, f));
            let mut at = d.nodes.get(id).and_then(|n| n.parent).filter(|_| inside);
            while let Some(a) = at.filter(|a| !d.nodes.get(*a).is_some_and(|n| n.focusable)) {
                at = d.nodes.get(a).and_then(|n| n.parent);
            }
            at
        };
        let parent = {
            let mut d = self.0.borrow_mut();
            if id == d.root {
                return Err(TreeError::Root);
            }
            let parent = d.nodes.get(id).ok_or(TreeError::NotFound)?.parent;
            if let Some(p) = parent.and_then(|p| d.nodes.get_mut(p)) {
                p.children.retain(|&c| c != id);
            }
            if let Some(n) = d.nodes.get_mut(id) {
                n.parent = None;
            }
            parent
        };
        self.clear_focus_within(id);
        if let Some(h) = heir {
            let _ = self.focus(h);
        }
        if let Some(p) = parent {
            self.mark(p, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        }
        Ok(())
    }

    /// Removes `id` and everything under it, disposing their reactive scopes (which runs
    /// their clean-ups).
    pub fn remove(&self, id: ElementId) -> Result<(), TreeError> {
        if self.0.borrow().nodes.get(id).is_some_and(|n| n.parent.is_some()) {
            self.detach(id)?;
        } else if id == self.root() {
            return Err(TreeError::Root);
        }
        self.clear_focus_within(id);
        let mut stack = vec![id];
        let mut removed = Vec::new();
        {
            let mut d = self.0.borrow_mut();
            while let Some(e) = stack.pop() {
                if let Some(node) = d.nodes.remove(e) {
                    stack.extend(node.children.iter().copied());
                    removed.push(node);
                }
            }
            let Data { dirty_list, nodes, .. } = &mut *d;
            dirty_list.retain(|e| nodes.contains(*e));
        }
        // Dropping payloads and handlers, and disposing (which runs clean-ups), is app code:
        // the tree is not borrowed.
        let scopes: Vec<Scope> = removed.iter().map(|n| n.scope).collect();
        drop(removed);
        for s in scopes.into_iter().rev() {
            s.dispose();
        }
        Ok(())
    }

    /// The element's parent.
    pub fn parent(&self, id: ElementId) -> Option<ElementId> {
        self.0.borrow().nodes.get(id).and_then(|n| n.parent)
    }

    /// The element's children, in order.
    pub fn children(&self, id: ElementId) -> Vec<ElementId> {
        self.0.borrow().nodes.get(id).map(|n| n.children.clone()).unwrap_or_default()
    }

    /// True when `id` exists.
    pub fn contains(&self, id: ElementId) -> bool {
        self.0.borrow().nodes.contains(id)
    }

    /// True when `ancestor` is `id` or one of its ancestors.
    pub fn is_ancestor_or_self(&self, ancestor: ElementId, id: ElementId) -> bool {
        self.0.borrow().is_ancestor_or_self(ancestor, id)
    }

    /// The element's name.
    pub fn name(&self, id: ElementId) -> Option<String> {
        self.0.borrow().nodes.get(id).map(|n| n.name.to_string())
    }

    /// Hands every failure caught in this tree (a panicking handler or guarded closure) to
    /// `f` instead of the tree's own boundaries: a view layer that keeps more per element
    /// (where it was built, its layout and paint) contains failures itself. `None` restores
    /// the tree's boundaries.
    pub fn set_failure_handler(&self, f: Option<FailureHandler>) {
        self.0.borrow_mut().failure_handler = f;
    }

    /// The reactive scope that owns what is made for the element.
    pub fn scope(&self, id: ElementId) -> Option<Scope> {
        self.0.borrow().nodes.get(id).map(|n| n.scope)
    }

    /// The element's path from the root, such as `root/settings/save`.
    pub fn path(&self, id: ElementId) -> String {
        self.0.borrow().path(id)
    }

    /// Every element under `id` (included) in tree order.
    pub fn descendants(&self, id: ElementId) -> Vec<ElementId> {
        self.0.borrow().preorder(id)
    }

    // ---------- payload ----------

    /// Stores data for whoever implements this element (a widget, a layout).
    pub fn set_payload<T: 'static>(&self, id: ElementId, value: T) -> Result<(), TreeError> {
        let old = {
            let mut d = self.0.borrow_mut();
            let n = d.nodes.get_mut(id).ok_or(TreeError::NotFound)?;
            n.payload.replace(Box::new(value))
        };
        // The payload it replaces is app data: dropped with the tree not borrowed.
        drop(old);
        Ok(())
    }

    /// Runs `f` with the element's payload, if it has one of type `T`.
    pub fn with_payload<T: 'static, R>(&self, id: ElementId, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut d = self.0.borrow_mut();
        d.nodes.get_mut(id)?.payload.as_mut()?.downcast_mut::<T>().map(f)
    }

    // ---------- dirty flags ----------

    /// Marks phases an element needs. `LAYOUT` sets `CHILD_LAYOUT` on ancestors up to the
    /// nearest relayout boundary; `PAINT` sets `CHILD_PAINT` up to the nearest repaint
    /// boundary. Ancestors already marked stop the climb.
    pub fn mark(&self, id: ElementId, flags: Dirty) {
        let mut d = self.0.borrow_mut();
        d.marks += 1;
        d.add_flags(id, flags);
        if flags.contains(Dirty::LAYOUT) {
            d.climb(id, Dirty::CHILD_LAYOUT, |n| n.relayout_boundary);
        }
        if flags.contains(Dirty::PAINT) {
            d.climb(id, Dirty::CHILD_PAINT, |n| n.repaint_boundary);
        }
    }

    /// The element's current dirty flags.
    pub fn dirty(&self, id: ElementId) -> Dirty {
        self.0.borrow().nodes.get(id).map(|n| n.dirty).unwrap_or_default()
    }

    /// Elements with any flag in `flags`, in tree order, clearing those flags. A frame phase
    /// calls this to get exactly the work it has; the cost is proportional to what is dirty.
    pub fn take_dirty(&self, flags: Dirty) -> Vec<ElementId> {
        let mut d = self.0.borrow_mut();
        let list = std::mem::take(&mut d.dirty_list);
        let mut found = Vec::new();
        let mut keep = Vec::new();
        for id in list {
            let Some(n) = d.nodes.get_mut(id) else { continue };
            if n.dirty.intersects(flags) {
                found.push(id);
                n.dirty = n.dirty.without(flags);
            }
            if n.dirty.is_empty() {
                n.in_dirty_list = false;
            } else {
                keep.push(id);
            }
        }
        d.dirty_list = keep;
        // One element is in order as it is: the whole tree is walked only to order several.
        if found.len() > 1 {
            let order = d.order_index();
            found.sort_by_key(|id| order.get(id).copied().unwrap_or(usize::MAX));
        }
        found
    }

    /// Number of elements with any dirty flag.
    pub fn dirty_count(&self) -> usize {
        self.0.borrow().dirty_list.len()
    }

    /// Makes `id` a relayout boundary (its size does not depend on its children).
    pub fn set_relayout_boundary(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.relayout_boundary = on)
    }

    /// Makes `id` a repaint boundary (it paints into its own layer).
    pub fn set_repaint_boundary(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.repaint_boundary = on)
    }

    // ---------- geometry and hit testing ----------

    /// Sets the element's bounds in window coordinates (layout does this).
    pub fn set_bounds(&self, id: ElementId, bounds: Rect) -> Result<(), TreeError> {
        self.edit(id, |n| n.bounds = bounds)
    }

    /// The element's bounds in window coordinates.
    pub fn bounds(&self, id: ElementId) -> Option<Rect> {
        self.0.borrow().nodes.get(id).map(|n| n.bounds)
    }

    /// Whether the element clips its children to its bounds (for hit testing and paint).
    pub fn set_clips(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.clips = on)
    }

    /// Whether the element is drawn over its siblings whatever their order (a sticky element
    /// stuck over the content scrolling under it): hit testing tries it before them.
    pub fn set_raised(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.raised = on)
    }

    /// How hit testing treats the element.
    pub fn set_hit_mode(&self, id: ElementId, mode: HitMode) -> Result<(), TreeError> {
        self.edit(id, |n| n.hit = mode)?;
        // Nothing inside a subtree that takes no input keeps keyboard focus.
        if mode == HitMode::None {
            self.clear_focus_within(id);
        }
        Ok(())
    }

    /// Whether keyboard focus can go to `id`: focusable, attached, and not inside a subtree
    /// that takes no input (hidden or disabled).
    pub fn can_focus(&self, id: ElementId) -> bool {
        let d = self.0.borrow();
        d.nodes.get(id).is_some_and(|n| n.focusable) && d.is_ancestor_or_self(d.root, id) && d.takes_input(id)
    }

    /// The topmost element under `p`: later children are above earlier ones, raised ones
    /// ([`Tree::set_raised`]) above the rest, and children are above their parent.
    pub fn hit_test(&self, p: Point) -> Option<ElementId> {
        let d = self.0.borrow();
        d.hit(d.root, p)
    }

    // ---------- events ----------

    /// Adds a handler for events of type `E` on `id`. Returns an id for [`Tree::off`].
    pub fn on<E: 'static>(
        &self,
        id: ElementId,
        phase: Phase,
        f: impl Fn(&mut EventCx, &E) + 'static,
    ) -> Result<u64, TreeError> {
        let mut d = self.0.borrow_mut();
        let hid = d.next_handler;
        let n = d.nodes.get_mut(id).ok_or(TreeError::NotFound)?;
        let f: HandlerFn = Rc::new(move |cx: &mut EventCx, e: &dyn Any| {
            if let Some(e) = e.downcast_ref::<E>() {
                f(cx, e);
            }
        });
        n.handlers.push(Handler { id: hid, event: TypeId::of::<E>(), phase, f });
        d.next_handler += 1;
        Ok(hid)
    }

    /// Removes a handler added with [`Tree::on`].
    pub fn off(&self, id: ElementId, handler: u64) -> Result<(), TreeError> {
        // What the handler captured is app data: dropped with the tree not borrowed.
        let mut taken: Vec<Handler> = Vec::new();
        let done = self.edit(id, |n| {
            let (gone, kept) = std::mem::take(&mut n.handlers).into_iter().partition(|h| h.id == handler);
            n.handlers = kept;
            taken = gone;
        });
        drop(taken);
        done
    }

    /// Drops every handler on `id` (an element going away for good, or the root of a window
    /// being torn down).
    pub fn clear_handlers(&self, id: ElementId) -> Result<(), TreeError> {
        // What the handlers captured is app data: dropped with the tree not borrowed.
        let mut taken: Vec<Handler> = Vec::new();
        let done = self.edit(id, |n| taken = std::mem::take(&mut n.handlers));
        drop(taken);
        done
    }

    /// Dispatches `event` to `target`: capture handlers from the root down, then the target's
    /// handlers, then bubble handlers up to the root. Handlers run in one reactive batch, so
    /// what they change updates once. A panicking handler is caught by its nearest error
    /// boundary and the dispatch goes on.
    pub fn dispatch<E: 'static>(&self, target: ElementId, event: &E) -> Result<Dispatch, TreeError> {
        let route = {
            let d = self.0.borrow();
            if !d.nodes.contains(target) {
                return Err(TreeError::NotFound);
            }
            d.route(target, TypeId::of::<E>())
        };
        let mut cx = EventCx {
            target,
            current: target,
            phase: Phase::Capture,
            stopped: false,
            default_prevented: false,
            handled: false,
            handled_by: None,
        };
        let mut out = Dispatch::default();
        batch(|| {
            let mut stopped_at = None;
            for (element, phase, f) in route {
                if let Some(at) = stopped_at
                    && at != element
                {
                    break;
                }
                cx.current = element;
                cx.phase = phase;
                out.handlers_run += 1;
                let r = catch_unwind(AssertUnwindSafe(|| f(&mut cx, event)));
                if let Err(p) = r {
                    out.panicked += 1;
                    self.report_failure(element, panic_message(p.as_ref()));
                }
                if cx.stopped && stopped_at.is_none() {
                    stopped_at = Some(element);
                }
            }
        });
        out.handled = cx.handled;
        out.handled_by = cx.handled_by;
        out.stopped = cx.stopped;
        out.default_prevented = cx.default_prevented;
        Ok(out)
    }

    // ---------- error boundaries ----------

    /// Makes `id` an error boundary. The returned signal holds the failure caught there, if
    /// any; a component shows its fallback by reading it.
    #[track_caller]
    pub fn set_error_boundary(&self, id: ElementId) -> Result<Signal<Option<Failure>>, TreeError> {
        if let Some(sig) = self.0.borrow().nodes.get(id).and_then(|n| n.boundary) {
            return Ok(sig);
        }
        let sig = self.in_scope(id, || Signal::new(None))?;
        self.edit(id, |n| n.boundary = Some(sig))?;
        Ok(sig)
    }

    /// Runs app code for `id` (a build closure, a canvas painter). A panic is caught by the
    /// nearest error boundary: its children are removed, its signal gets the failure, and
    /// `None` is returned.
    pub fn guarded<R>(&self, id: ElementId, f: impl FnOnce() -> R) -> Option<R> {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => Some(r),
            Err(p) => {
                self.report_failure(id, panic_message(p.as_ref()));
                None
            }
        }
    }

    /// Records a failure at `at` with its nearest error boundary (the root at the latest):
    /// the boundary's children are removed and its signal is set.
    pub fn report_failure(&self, at: ElementId, message: String) {
        let handler = self.0.borrow().failure_handler.clone();
        if let Some(h) = handler {
            h(at, message);
            return;
        }
        let (boundary, signal, path) = {
            let d = self.0.borrow();
            let path = d.path(at);
            let mut cur = Some(at);
            let mut found = None;
            while let Some(e) = cur {
                let Some(n) = d.nodes.get(e) else { break };
                if let Some(sig) = n.boundary {
                    found = Some((e, sig));
                    break;
                }
                cur = n.parent;
            }
            match found {
                Some((b, s)) => (b, s, path),
                None => return,
            }
        };
        for child in self.children(boundary) {
            let _ = self.remove(child);
        }
        let _ = signal.try_set(Some(Failure { element_path: path, message }));
        self.mark(boundary, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
    }

    // ---------- focus ----------

    /// The nearest focus scope containing `id` (itself included), or the root.
    pub fn focus_scope(&self, id: ElementId) -> ElementId {
        self.0.borrow().focus_scope_of(id)
    }

    /// Whether the element can take keyboard focus.
    pub fn set_focusable(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.focusable = on)?;
        if !on && self.focused() == Some(id) {
            self.blur();
        }
        Ok(())
    }

    /// Whether the element can take keyboard focus.
    pub fn is_focusable(&self, id: ElementId) -> bool {
        self.0.borrow().nodes.get(id).is_some_and(|n| n.focusable)
    }

    /// Keeps a focusable element out of the Tab order (`false`), as HTML's `tabindex="-1"`:
    /// Tab, Shift+Tab and arrow navigation pass it by, while [`focus`](Self::focus) and a
    /// click still focus it. Elements are in the order by default.
    pub fn set_tab_stop(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.skip_tab = !on)
    }

    /// Whether Tab stops at the element when it is focusable.
    pub fn is_tab_stop(&self, id: ElementId) -> bool {
        self.0.borrow().nodes.get(id).is_some_and(|n| !n.skip_tab)
    }

    /// Sets the element's position in the tab order: elements with an order come first, lowest
    /// first; the rest (`None`) follow in tree order.
    pub fn set_focus_order(&self, id: ElementId, order: Option<i32>) -> Result<(), TreeError> {
        self.edit(id, |n| n.focus_order = order)
    }

    /// Makes `id` a focus scope: Tab and Shift+Tab stay inside it (modals, menus).
    pub fn set_focus_scope(&self, id: ElementId, on: bool) -> Result<(), TreeError> {
        self.edit(id, |n| n.focus_scope = on)
    }

    /// The focused element.
    pub fn focused(&self) -> Option<ElementId> {
        let d = self.0.borrow();
        d.focus.filter(|f| d.nodes.contains(*f))
    }

    /// Moves focus to `id`, sending `FocusOut` and `FocusIn`. Returns false (and changes
    /// nothing) if the element is not focusable or not attached to the root.
    pub fn focus(&self, id: ElementId) -> Result<bool, TreeError> {
        let (old, ok) = {
            let d = self.0.borrow();
            let n = d.nodes.get(id).ok_or(TreeError::NotFound)?;
            (d.focus, n.focusable && d.is_ancestor_or_self(d.root, id) && d.takes_input(id))
        };
        if !ok {
            return Ok(false);
        }
        if old == Some(id) {
            return Ok(true);
        }
        self.0.borrow_mut().focus = Some(id);
        if let Some(old) = old.filter(|o| self.contains(*o)) {
            self.mark(old, Dirty::STYLE | Dirty::SEMANTICS);
            let _ = self.dispatch(old, &FocusOut);
        }
        self.mark(id, Dirty::STYLE | Dirty::SEMANTICS);
        let _ = self.dispatch(id, &FocusIn);
        Ok(true)
    }

    /// Clears focus.
    pub fn blur(&self) {
        let old = self.0.borrow_mut().focus.take();
        if let Some(old) = old.filter(|o| self.contains(*o)) {
            self.mark(old, Dirty::STYLE | Dirty::SEMANTICS);
            let _ = self.dispatch(old, &FocusOut);
        }
    }

    /// Moves focus to the next focusable element in the current focus scope (Tab), wrapping
    /// around. Returns the newly focused element.
    pub fn focus_next(&self) -> Option<ElementId> {
        self.focus_step(true)
    }

    /// Moves focus to the previous focusable element (Shift+Tab), wrapping around.
    pub fn focus_previous(&self) -> Option<ElementId> {
        self.focus_step(false)
    }

    /// The focusable elements in `scope`'s tab order, nested focus scopes left out: what Tab
    /// and arrow navigation choose from.
    pub fn focus_candidates(&self, scope: ElementId) -> Vec<ElementId> {
        self.0.borrow().tab_order(scope, false)
    }

    fn focus_step(&self, forward: bool) -> Option<ElementId> {
        // Elements out of the Tab order are in this list too, so Tab from one goes on from
        // where it stands, as from an HTML element with `tabindex="-1"`.
        let order: Vec<(ElementId, bool)> = {
            let d = self.0.borrow();
            let scope = d.focus_scope_of(d.focus.unwrap_or(d.root));
            d.tab_order(scope, true)
                .into_iter()
                .map(|e| (e, d.nodes.get(e).is_some_and(|n| !n.skip_tab)))
                .collect()
        };
        let len = order.len();
        let current = self.focused().and_then(|f| order.iter().position(|&(e, _)| e == f));
        let target = (1..=len)
            .map(|k| match (current, forward) {
                (None, true) => k - 1,
                (None, false) => len - k,
                (Some(i), true) => (i + k) % len,
                (Some(i), false) => (i + len * k - k) % len,
            })
            .find_map(|i| order.get(i).filter(|(_, stop)| *stop).map(|(e, _)| *e))?;
        self.focus(target).ok().filter(|&ok| ok).map(|_| target)
    }

    fn clear_focus_within(&self, id: ElementId) {
        let inside = {
            let d = self.0.borrow();
            d.focus.is_some_and(|f| d.is_ancestor_or_self(id, f))
        };
        if inside {
            self.blur();
        }
    }

    // ---------- stats ----------

    /// How many times [`Tree::mark`] was called, for tests.
    pub fn marks(&self) -> u64 {
        self.0.borrow().marks
    }

    fn edit(&self, id: ElementId, f: impl FnOnce(&mut Node)) -> Result<(), TreeError> {
        let mut d = self.0.borrow_mut();
        f(d.nodes.get_mut(id).ok_or(TreeError::NotFound)?);
        Ok(())
    }
}

impl Node {
    fn new(name: Cow<'static, str>, scope: Scope) -> Self {
        Node {
            name,
            parent: None,
            children: Vec::new(),
            dirty: Dirty::NONE,
            in_dirty_list: false,
            relayout_boundary: false,
            repaint_boundary: false,
            bounds: Rect::ZERO,
            clips: false,
            hit: HitMode::Normal,
            raised: false,
            focusable: false,
            skip_tab: false,
            focus_order: None,
            focus_scope: false,
            boundary: None,
            handlers: Vec::new(),
            scope,
            payload: None,
        }
    }
}

impl Data {
    /// No ancestor of `id` (or `id` itself) takes no input.
    fn takes_input(&self, id: ElementId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.nodes.get(c) {
                Some(n) if n.hit == HitMode::None => return false,
                Some(n) => cur = n.parent,
                None => return false,
            }
        }
        true
    }

    fn is_ancestor_or_self(&self, ancestor: ElementId, id: ElementId) -> bool {
        let mut cur = Some(id);
        while let Some(e) = cur {
            if e == ancestor {
                return true;
            }
            cur = self.nodes.get(e).and_then(|n| n.parent);
        }
        false
    }

    fn path(&self, id: ElementId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(e) = cur {
            let Some(n) = self.nodes.get(e) else { break };
            parts.push(n.name.to_string());
            cur = n.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    fn add_flags(&mut self, id: ElementId, flags: Dirty) {
        let Some(n) = self.nodes.get_mut(id) else { return };
        n.dirty |= flags;
        if !n.in_dirty_list {
            n.in_dirty_list = true;
            self.dirty_list.push(id);
        }
    }

    fn climb(&mut self, from: ElementId, flag: Dirty, stops: impl Fn(&Node) -> bool) {
        let start = self.nodes.get(from);
        if start.is_some_and(&stops) {
            return;
        }
        let mut cur = start.and_then(|n| n.parent);
        while let Some(e) = cur {
            let Some(n) = self.nodes.get(e) else { break };
            if n.dirty.contains(flag) {
                break;
            }
            let stop = stops(n);
            let parent = n.parent;
            self.add_flags(e, flag);
            if stop {
                break;
            }
            cur = parent;
        }
    }

    fn preorder(&self, from: ElementId) -> Vec<ElementId> {
        let mut out = Vec::new();
        let mut stack = vec![from];
        while let Some(e) = stack.pop() {
            let Some(n) = self.nodes.get(e) else { continue };
            out.push(e);
            stack.extend(n.children.iter().rev().copied());
        }
        out
    }

    fn order_index(&self) -> std::collections::HashMap<ElementId, usize> {
        self.preorder(self.root).into_iter().enumerate().map(|(i, e)| (e, i)).collect()
    }

    /// The topmost element under `p` at or below `id`: children before their parent, the
    /// last child first; an element that takes no hits, or clips and misses, hides its
    /// subtree. An explicit stack, not recursion: a tree any number of levels deep is hit
    /// tested on any thread's stack.
    fn hit(&self, id: ElementId, p: Point) -> Option<ElementId> {
        // The node, if `p` can hit it or something under it.
        let open = |id: ElementId| {
            self.nodes.get(id).filter(|n| n.hit != HitMode::None && (!n.clips || n.bounds.contains(p)))
        };
        // The elements being searched, outermost first, each with how many of its children
        // (from the first) are still to try: twice, the raised ones in the first pass (the
        // pass counts down from 1) and the rest in the second (0).
        let mut stack = vec![(id, open(id)?.children.len(), 1u8)];
        while let Some((at, left, pass)) = stack.last_mut() {
            let Some(n) = self.nodes.get(*at) else {
                stack.pop();
                continue;
            };
            if *left > 0 {
                *left -= 1;
                if let Some(&c) = n.children.get(*left)
                    && let Some(child) = open(c)
                    && child.raised == (*pass == 1)
                {
                    stack.push((c, child.children.len(), 1));
                }
                continue;
            }
            if *pass == 1 {
                *pass = 0;
                *left = n.children.len();
                continue;
            }
            // Nothing under it was hit: the element itself, if it takes hits there.
            let at = *at;
            if n.hit == HitMode::Normal && n.bounds.contains(p) {
                return Some(at);
            }
            stack.pop();
        }
        None
    }

    fn route(&self, target: ElementId, event: TypeId) -> Vec<(ElementId, Phase, HandlerFn)> {
        let mut path = Vec::new();
        let mut cur = Some(target);
        while let Some(e) = cur {
            path.push(e);
            cur = self.nodes.get(e).and_then(|n| n.parent);
        }
        path.reverse(); // root .. target
        let handlers = |e: ElementId, phase: Phase| -> Vec<(ElementId, Phase, HandlerFn)> {
            self.nodes
                .get(e)
                .map(|n| {
                    n.handlers
                        .iter()
                        .filter(|h| h.event == event && h.phase == phase)
                        .map(|h| (e, phase, h.f.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut route = Vec::new();
        let ancestors = path.get(..path.len().saturating_sub(1)).unwrap_or(&[]);
        for &e in ancestors {
            route.extend(handlers(e, Phase::Capture));
        }
        route.extend(handlers(target, Phase::Capture));
        route.extend(handlers(target, Phase::Bubble));
        for &e in ancestors.iter().rev() {
            route.extend(handlers(e, Phase::Bubble));
        }
        route
    }

    /// The nearest focus scope containing `id` (itself included), or the root.
    fn focus_scope_of(&self, id: ElementId) -> ElementId {
        let mut cur = Some(id);
        while let Some(e) = cur {
            let Some(n) = self.nodes.get(e) else { break };
            if n.focus_scope {
                return e;
            }
            cur = n.parent;
        }
        self.root
    }

    /// Focusable elements in `scope`, not entering nested focus scopes, in tab order.
    /// With `all`, elements out of the Tab order too, where they stand.
    fn tab_order(&self, scope: ElementId, all: bool) -> Vec<ElementId> {
        let mut ordered: Vec<(i32, usize, ElementId)> = Vec::new();
        let mut plain = Vec::new();
        let mut stack = vec![scope];
        let mut index = 0usize;
        while let Some(e) = stack.pop() {
            let Some(n) = self.nodes.get(e) else { continue };
            if e != scope && n.focus_scope {
                continue;
            }
            // Hidden and disabled subtrees are out of the tab order.
            if n.hit == HitMode::None {
                continue;
            }
            if n.focusable && (all || !n.skip_tab) {
                match n.focus_order {
                    Some(o) => ordered.push((o, index, e)),
                    None => plain.push(e),
                }
                index += 1;
            }
            stack.extend(n.children.iter().rev().copied());
        }
        ordered.sort_by_key(|&(o, i, _)| (o, i));
        ordered.into_iter().map(|(_, _, e)| e).chain(plain).collect()
    }
}
