//! Drag and drop: elements that can be dragged, elements that take drops, and one session
//! that works inside the app, out to other apps (handed to the platform when the pointer
//! leaves the window) and in from other apps.

use std::any::Any;
use std::rc::Rc;

use remedy_ui_core::{Point, Vec2};
use remedy_ui_core::host::{DragData, DragEvent, DropEffect};
use remedy_ui_core::input::{Buttons, PointerEvent, PointerPhase};
use remedy_ui_core::style::States;
use remedy_ui_core::tree::{Dirty, ElementId};

use crate::view::{El, Request, Ui};

/// Movement, in logical pixels, after which a press becomes a drag.
const DRAG_START: f32 = 6.0;

/// What is being dragged: data other apps understand, and, inside this app, any value.
#[derive(Clone)]
pub struct Drag {
    /// Files, text and links: what leaves the app when the drag goes to another one.
    pub data: DragData,
    /// What a drop may do; the target's choice must be one of these.
    pub allowed: Vec<DropEffect>,
    local: Option<Rc<dyn Any>>,
}

impl std::fmt::Debug for Drag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drag")
            .field("data", &self.data)
            .field("allowed", &self.allowed)
            .field("local", &self.local.is_some())
            .finish()
    }
}

impl Drag {
    /// Dragging `data`, to be copied or moved.
    pub fn new(data: DragData) -> Self {
        Drag { data, allowed: vec![DropEffect::Copy, DropEffect::Move], local: None }
    }

    /// Carries `value` to targets in this app (rows being moved, a widget being placed).
    pub fn with_local<T: 'static>(mut self, value: T) -> Self {
        self.local = Some(Rc::new(value));
        self
    }

    /// Allows only these effects.
    pub fn allow(mut self, effects: &[DropEffect]) -> Self {
        self.allowed = effects.to_vec();
        self
    }

    /// The value carried inside this app, if it is a `T`.
    pub fn local<T: 'static>(&self) -> Option<&T> {
        self.local.as_ref().and_then(|v| v.downcast_ref::<T>())
    }

    /// True when the drag started in this app.
    pub fn from_this_app(&self) -> bool {
        self.local.is_some()
    }
}

pub(crate) type DragSource = (Rc<dyn Fn() -> Option<Drag>>, Rc<dyn Fn(DropEffect)>);
pub(crate) type DropAccept = Rc<dyn Fn(&Drag, Point) -> DropEffect>;
pub(crate) type DropHandler = Rc<dyn Fn(&Drag, Point, DropEffect)>;
pub(crate) type DragHover = Rc<dyn Fn(Option<&Drag>)>;

/// The drag under way.
pub(crate) struct Session {
    drag: Drag,
    /// The element it came from, in this window.
    source: Option<ElementId>,
    /// The target under it, and what a drop there would do.
    target: Option<(ElementId, DropEffect)>,
    /// Handed to the platform: the pointer left the window.
    outside: bool,
    /// The elements with an `on_drag_hover` the drag is over, outermost last.
    hovered: Vec<ElementId>,
}

/// A press that may become a drag.
pub(crate) struct Pending {
    source: ElementId,
    at: Point,
}

impl El {
    /// Makes the element draggable: past a few pixels of movement with the primary button,
    /// `make` says what is dragged (`None`: nothing, the press stays a press), and
    /// `on_end` hears what the drop did (`Move`: let the item go; `None`: cancelled or
    /// refused). Dragged out of the window, the drag continues into other apps.
    ///
    /// Dragging needs a pointer: give keyboard users another way (cut and paste) and say so
    /// with [`El::keyboard_alternative`].
    pub fn draggable(
        self,
        make: impl Fn() -> Option<Drag> + 'static,
        on_end: impl Fn(DropEffect) + 'static,
    ) -> Self {
        let src: DragSource = (Rc::new(make), Rc::new(on_end));
        self.edit(Dirty::NONE, move |d| {
            d.drag_source = Some(src);
            d.pointer = true;
        });
        self
    }

    /// Makes the element take drops: `accept` says what a drop at a point (local to the
    /// element) would do (`DropEffect::None` refuses it, and the drop goes to an outer
    /// target), and `drop` receives it. While something it accepts is over it, the element
    /// is in [`States::DROP_TARGET`], which draws the theme's drop outline.
    pub fn on_drop(
        self,
        accept: impl Fn(&Drag, Point) -> DropEffect + 'static,
        drop: impl Fn(&Drag, Point, DropEffect) + 'static,
    ) -> Self {
        let a: DropAccept = Rc::new(accept);
        let h: DropHandler = Rc::new(drop);
        self.edit(Dirty::NONE, move |d| d.drop_target = Some((a, h)));
        self
    }

    /// Calls `f` with the drag when one comes over the element (or anything in it), whether
    /// or not anything there takes it, and with `None` when it leaves or ends: for hints such
    /// as "Drop files to add them" or "This folder is read-only".
    pub fn on_drag_hover(self, f: impl Fn(Option<&Drag>) + 'static) -> Self {
        let f: DragHover = Rc::new(f);
        self.edit(Dirty::NONE, move |d| d.drag_hover = Some(f));
        self
    }
}

impl Ui {
    /// The drag under way, if any (for previews that follow it).
    pub fn dragging(&self) -> Option<Drag> {
        self.0.borrow().drag.as_ref().map(|s| s.drag.clone())
    }

    fn local_point(&self, id: ElementId, p: Point) -> Point {
        let b = self.tree().bounds(id).unwrap_or_default();
        Point::new(p.x - b.min.x, p.y - b.min.y)
    }

    /// The target under `p` (or, with no position, the outermost target in the window) that
    /// accepts the drag, and what a drop there would do.
    fn find_target(&self, drag: &Drag, p: Option<Point>) -> Option<(ElementId, DropEffect)> {
        let tree = self.tree();
        let candidates: Vec<ElementId> = match p {
            Some(p) => {
                let mut out = Vec::new();
                let mut cur = tree.hit_test(p);
                while let Some(c) = cur {
                    out.push(c);
                    cur = tree.parent(c);
                }
                out
            }
            // The platform did not say where: the whole-window target, if there is one.
            None => tree.descendants(tree.root()),
        };
        for id in candidates {
            let target = self.0.borrow().els.get(&id).and_then(|d| d.drop_target.clone());
            if let Some((accept, _)) = target {
                let at = p.map_or(Point::ZERO, |p| self.local_point(id, p));
                let effect = accept(drag, at);
                if effect != DropEffect::None && drag.allowed.contains(&effect) {
                    return Some((id, effect));
                }
            }
        }
        None
    }

    /// Tells hover listeners the drag is now over the elements at `p` (none for `None`).
    fn rehover(&self, p: Option<Point>) {
        let Some(drag) = self.dragging() else { return };
        let now: Vec<ElementId> = match p {
            Some(p) => {
                let tree = self.tree();
                let mut out = Vec::new();
                let mut cur = tree.hit_test(p);
                while let Some(c) = cur {
                    if self.0.borrow().els.get(&c).is_some_and(|d| d.drag_hover.is_some()) {
                        out.push(c);
                    }
                    cur = tree.parent(c);
                }
                out
            }
            None => Vec::new(),
        };
        let before = match self.0.borrow_mut().drag.as_mut() {
            Some(s) => std::mem::replace(&mut s.hovered, now.clone()),
            None => return,
        };
        let listener = |id: &ElementId| self.0.borrow().els.get(id).and_then(|d| d.drag_hover.clone());
        let left: Vec<DragHover> =
            before.iter().filter(|id| !now.contains(id)).filter_map(listener).collect();
        let came: Vec<DragHover> =
            now.iter().filter(|id| !before.contains(id)).filter_map(listener).collect();
        for f in left {
            f(None);
        }
        for f in came {
            f(Some(&drag));
        }
    }

    fn retarget(&self, p: Option<Point>) -> DropEffect {
        let Some(drag) = self.dragging() else { return DropEffect::None };
        self.rehover(p);
        let next = self.find_target(&drag, p);
        let old = self.0.borrow_mut().drag.as_mut().and_then(|s| std::mem::replace(&mut s.target, next));
        if old.map(|(id, _)| id) != next.map(|(id, _)| id) {
            if let Some((id, _)) = old {
                self.set_state(id, States::DROP_TARGET, false);
            }
            if let Some((id, _)) = next {
                self.set_state(id, States::DROP_TARGET, true);
            }
        }
        next.map_or(DropEffect::None, |(_, e)| e)
    }

    /// Ends the session: the target loses its outline, the source its dragging state, and
    /// the source hears what happened.
    fn end_drag(&self, effect: DropEffect) {
        self.rehover(None);
        let session = self.0.borrow_mut().drag.take();
        let Some(s) = session else { return };
        if let Some((id, _)) = s.target {
            self.set_state(id, States::DROP_TARGET, false);
        }
        if let Some(src) = s.source {
            self.set_state(src, States::DRAGGING, false);
            let end = self
                .0
                .borrow()
                .els
                .get(&src)
                .and_then(|d| d.drag_source.as_ref().map(|(_, end)| end.clone()));
            if let Some(end) = end {
                end(effect);
            }
        }
        self.request(Request::Frame);
    }

    fn drop_here(&self, p: Option<Point>) -> DropEffect {
        let effect = self.retarget(p);
        let target = self.0.borrow().drag.as_ref().and_then(|s| s.target.map(|(id, _)| id));
        let drag = self.dragging();
        if let (Some(id), Some(drag)) = (target, drag) {
            let handler =
                self.0.borrow().els.get(&id).and_then(|d| d.drop_target.as_ref().map(|(_, h)| h.clone()));
            if let Some(h) = handler {
                let at = p.map_or(Point::ZERO, |p| self.local_point(id, p));
                h(&drag, at, effect);
            }
        }
        effect
    }

    /// A drag from another app over this window (or a drop). Returns what a drop would do
    /// now, for the platform to show.
    pub fn drag_event(&self, e: &DragEvent) -> DropEffect {
        let _events = remedy_ui_core::phase!("events", ui = self.id()).entered();
        match e {
            DragEvent::Enter { position, data } => {
                let ours = self.0.borrow().drag.as_ref().is_some_and(|s| s.outside);
                if !ours {
                    self.end_drag(DropEffect::None);
                    self.0.borrow_mut().drag = Some(Session {
                        drag: Drag::new(data.clone()),
                        source: None,
                        target: None,
                        outside: false,
                        hovered: Vec::new(),
                    });
                } else if let Some(s) = self.0.borrow_mut().drag.as_mut() {
                    // Our own drag came back in.
                    s.outside = false;
                }
                self.retarget(*position)
            }
            DragEvent::Over { position } => self.retarget(Some(*position)),
            DragEvent::Leave => {
                let ours = self.0.borrow().drag.as_ref().is_some_and(|s| s.source.is_some());
                if ours {
                    // Our own drag left again: it is the platform's until it ends.
                    if let Some(s) = self.0.borrow_mut().drag.as_mut() {
                        s.outside = true;
                    }
                    let _ = self.retarget_none();
                } else {
                    self.end_drag(DropEffect::None);
                }
                DropEffect::None
            }
            DragEvent::Drop { position, data } => {
                if self.0.borrow().drag.is_none() {
                    self.0.borrow_mut().drag = Some(Session {
                        drag: Drag::new(data.clone()),
                        source: None,
                        target: None,
                        outside: false,
                        hovered: Vec::new(),
                    });
                }
                let effect = self.drop_here(*position);
                let external = self.0.borrow().drag.as_ref().is_some_and(|s| s.source.is_none());
                if external {
                    self.end_drag(effect);
                } else if let Some(s) = self.0.borrow_mut().drag.as_mut() {
                    // Our own drag dropped back on us through the platform: the platform's
                    // drag call returns next and ends the session.
                    s.outside = true;
                }
                effect
            }
        }
    }

    fn retarget_none(&self) -> DropEffect {
        self.rehover(None);
        let old = self.0.borrow_mut().drag.as_mut().and_then(|s| s.target.take());
        if let Some((id, _)) = old {
            self.set_state(id, States::DROP_TARGET, false);
        }
        DropEffect::None
    }

    /// The platform's drag out of this window ended with `effect` (the runner calls this
    /// when [`Request::StartDrag`] returns).
    pub fn drag_finished(&self, effect: DropEffect) {
        self.end_drag(effect);
    }

    /// Escape cancels a drag under way.
    pub(crate) fn cancel_drag(&self) -> bool {
        let inside = self.0.borrow().drag.as_ref().is_some_and(|s| !s.outside && s.source.is_some());
        if inside {
            self.end_drag(DropEffect::None);
        }
        inside
    }

    /// Pointer input for drags; true when the drag took it.
    pub(crate) fn drag_pointer(&self, e: &PointerEvent, hit: Option<ElementId>) -> bool {
        match e.phase {
            PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                let tree = self.tree();
                let mut cur = hit;
                let mut source = None;
                while let Some(c) = cur {
                    if self.0.borrow().els.get(&c).is_some_and(|d| d.drag_source.is_some()) {
                        source = Some(c);
                        break;
                    }
                    cur = tree.parent(c);
                }
                self.0.borrow_mut().drag_pending = source.map(|s| Pending { source: s, at: e.position });
                false
            }
            PointerPhase::Move => {
                let active = self.0.borrow().drag.as_ref().map(|s| (s.outside, s.source.is_some()));
                match active {
                    Some((false, true)) => {
                        let size = self.0.borrow().size;
                        let p = e.position;
                        let outside = p.x < 0.0 || p.y < 0.0 || p.x >= size.width || p.y >= size.height;
                        // Only what other apps can read leaves the app; a drag of app values
                        // alone stays in the window (over nothing while the pointer is out).
                        let portable = self.0.borrow().drag.as_ref().is_some_and(|s| !s.drag.data.is_empty());
                        if outside && !portable {
                            let _ = self.retarget_none();
                        } else if outside {
                            // Out of the window: the platform carries it to other apps.
                            let _ = self.retarget_none();
                            let drag = {
                                let mut i = self.0.borrow_mut();
                                let s = i.drag.as_mut();
                                s.map(|s| {
                                    s.outside = true;
                                    s.drag.clone()
                                })
                            };
                            if let Some(d) = drag {
                                self.request(Request::StartDrag { data: d.data, allowed: d.allowed });
                            }
                        } else {
                            self.retarget(Some(p));
                        }
                        true
                    }
                    Some(_) => true,
                    None => {
                        if !e.buttons.contains(Buttons::PRIMARY) {
                            return false;
                        }
                        let pending = self.0.borrow().drag_pending.as_ref().map(|p| (p.source, p.at));
                        let Some((source, at)) = pending else { return false };
                        let d = Vec2::new(e.position.x - at.x, e.position.y - at.y);
                        if d.x.hypot(d.y) < DRAG_START {
                            return false;
                        }
                        self.0.borrow_mut().drag_pending = None;
                        let make = self
                            .0
                            .borrow()
                            .els
                            .get(&source)
                            .and_then(|d| d.drag_source.as_ref().map(|(m, _)| m.clone()));
                        let Some(drag) = make.and_then(|m| m()) else { return false };
                        // The press is now a drag: no click when it ends, no capture.
                        let pressed = self.0.borrow_mut().pressed.take();
                        if let Some(p) = pressed {
                            self.set_state(p, States::PRESSED, false);
                        }
                        self.0.borrow_mut().captured = None;
                        self.set_state(source, States::DRAGGING, true);
                        self.0.borrow_mut().drag = Some(Session {
                            drag,
                            source: Some(source),
                            target: None,
                            outside: false,
                            hovered: Vec::new(),
                        });
                        self.retarget(Some(e.position));
                        true
                    }
                }
            }
            PointerPhase::Up => {
                self.0.borrow_mut().drag_pending = None;
                let inside = self.0.borrow().drag.as_ref().is_some_and(|s| !s.outside && s.source.is_some());
                if inside {
                    let effect = self.drop_here(Some(e.position));
                    self.end_drag(effect);
                    return true;
                }
                false
            }
            PointerPhase::Cancel => {
                self.0.borrow_mut().drag_pending = None;
                self.cancel_drag()
            }
            _ => false,
        }
    }
}
