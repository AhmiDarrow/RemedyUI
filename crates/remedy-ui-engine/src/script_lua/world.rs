//! A widget's world, kept in Rust: its retained elements, reactive values, computations,
//! timers and hooks. Lua holds handles to them; the Lua values they need (a signal's value,
//! a callback, an element's properties) live in the widget's `refs` table under integer keys
//! ([`Ref`]), so the collector keeps them alive and the memory limit counts them.

use std::collections::VecDeque;
use std::time::Duration;

use remedy_ui_core::motion::Animation;

/// A key of the widget's `refs` table.
pub(crate) type Ref = i64;

/// A slot of an [`Arena`] and the generation it had when the handle was made: a handle to a
/// freed slot (a removed element, a stopped effect) is recognized as gone, never confused
/// with what took the slot next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Id {
    pub idx: u32,
    pub generation: u32,
}

/// Slots by index and generation, with a cap on how many may be live.
pub(crate) struct Arena<T> {
    slots: Vec<(u32, Option<T>)>,
    free: Vec<u32>,
    live: usize,
    cap: usize,
}

impl<T> Arena<T> {
    pub fn new(cap: usize) -> Self {
        Arena { slots: Vec::new(), free: Vec::new(), live: 0, cap }
    }

    /// Stores `v`; None when the arena is full.
    pub fn insert(&mut self, v: T) -> Option<Id> {
        if self.live >= self.cap {
            return None;
        }
        self.live += 1;
        if let Some(idx) = self.free.pop()
            && let Some(slot) = self.slots.get_mut(idx as usize)
        {
            slot.1 = Some(v);
            return Some(Id { idx, generation: slot.0 });
        }
        let idx = u32::try_from(self.slots.len()).ok()?;
        self.slots.push((0, Some(v)));
        Some(Id { idx, generation: 0 })
    }

    pub fn get(&self, id: Id) -> Option<&T> {
        self.slots.get(id.idx as usize).filter(|s| s.0 == id.generation).and_then(|s| s.1.as_ref())
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut T> {
        self.slots.get_mut(id.idx as usize).filter(|s| s.0 == id.generation).and_then(|s| s.1.as_mut())
    }

    pub fn contains(&self, id: Id) -> bool {
        self.get(id).is_some()
    }

    /// Frees `id`'s slot (its handles become stale) and returns what it held.
    pub fn remove(&mut self, id: Id) -> Option<T> {
        let slot = self.slots.get_mut(id.idx as usize).filter(|s| s.0 == id.generation)?;
        let v = slot.1.take()?;
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(id.idx);
        self.live -= 1;
        Some(v)
    }

    /// The live slot `idx` whose generation ends in `low` (30 bits), if any.
    pub fn at(&self, idx: u32, low: u32) -> Option<Id> {
        let slot = self.slots.get(idx as usize)?;
        (slot.1.is_some() && slot.0 & 0x3fff_ffff == low).then_some(Id { idx, generation: slot.0 })
    }

    /// Every live slot's id, in slot order.
    pub fn ids(&self) -> Vec<Id> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.1.is_some())
            .filter_map(|(i, s)| u32::try_from(i).ok().map(|idx| Id { idx, generation: s.0 }))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.live
    }
}

/// A retained element.
pub(crate) struct Element {
    /// Its kind (`column`, `text`, or one of the app's own).
    pub kind: String,
    /// Its properties: a Lua table.
    pub props: Ref,
    pub children: Vec<Id>,
    pub parent: Option<Id>,
    /// Made by a `render` function: released when the next render starts.
    pub transient: bool,
}

/// What kind of reactive value a signal is.
pub(crate) enum SignalKind {
    /// Set by the script.
    Plain,
    /// Worked out by computation `comp` when read after its inputs changed.
    Computed { comp: Id },
    /// A number moving to a target over time; its value is sampled at the widget's clock.
    Animated(Box<Animation<f32>>),
}

/// A reactive value.
pub(crate) struct Signal {
    /// Its current value (a `refs` key; for an animated value, unused).
    pub value: Ref,
    /// The computations that read it last time they ran.
    pub subs: Vec<Id>,
    pub kind: SignalKind,
}

/// Whether a computation is a computed value or an effect.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompKind {
    /// Works out signal `sig`.
    Computed { sig: Id },
    /// Runs for what it does, again whenever what it read changes.
    Effect,
}

/// A function that reads signals and is run again when they change.
pub(crate) struct Comp {
    pub func: Ref,
    pub deps: Vec<Id>,
    pub kind: CompKind,
    /// A computed value whose inputs changed since it last ran.
    pub dirty: bool,
    /// An effect waiting to run.
    pub queued: bool,
    /// Running now (a computed value that reads itself is an error).
    pub running: bool,
}

/// A timer on the widget's clock.
pub(crate) struct Timer {
    pub due: Duration,
    pub every: Option<Duration>,
    pub func: Ref,
}

/// What a render made besides elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owned {
    Signal,
    Comp,
    Timer,
    Frame,
}

/// How many of each thing one widget may hold: what a script cannot grow without bound.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caps {
    pub elements: usize,
    pub signals: usize,
    pub comps: usize,
    pub timers: usize,
    pub children: usize,
    /// How deeply elements may nest.
    pub nesting: usize,
}

impl Caps {
    /// Caps that fit `memory` bytes: an element or a signal costs at least a few hundred bytes
    /// of Lua memory, so these are never what stops a widget that keeps within its memory.
    pub fn for_memory(memory: usize, nesting: usize) -> Caps {
        let n = (memory / 256).clamp(1024, 1 << 20);
        Caps { elements: n, signals: n, comps: n / 4, timers: 1024, children: 4096, nesting }
    }
}

/// The widget's world.
pub(crate) struct World {
    pub elements: Arena<Element>,
    pub signals: Arena<Signal>,
    pub comps: Arena<Comp>,
    pub timers: Arena<Timer>,
    /// Per-frame callbacks.
    pub frames: Arena<Ref>,
    pub on_mount: Vec<Ref>,
    pub on_unmount: Vec<Ref>,
    /// Whether the mount hooks ran.
    pub mounted: bool,
    /// The computations running now, innermost last: what a signal read is tracked for.
    pub tracking: Vec<Id>,
    /// Effects waiting to run, in the order they were told.
    pub queue: VecDeque<Id>,
    /// The widget's clock: the frame loop's time, moved on by `advance`.
    pub now: Duration,
    /// Whether what the widget shows may have changed since it last rendered.
    pub changed: bool,
    pub next_ref: Ref,
    /// Keys of `refs` freed since the host last cleared them (to set to nil).
    pub dead_refs: Vec<Ref>,
    /// Cleared keys, used again first: the table stays as large as what the widget holds,
    /// not as large as all it ever made.
    pub free_refs: Vec<Ref>,
    /// The element the widget shows when it has no `render` function.
    pub root: Option<Id>,
    /// A `render` function is running: elements made now are transient.
    pub rendering: bool,
    /// The settings, host data and theme tokens, as signals.
    pub settings: Option<Id>,
    pub host: Option<Id>,
    pub theme: Option<Id>,
    /// The handlers of the last description, by action.
    pub handlers: Vec<Ref>,
    /// The element a render's call chose to show, for the widget to describe.
    pub root_shown: Option<Id>,
    /// Animated values jump to their targets (the person asked for reduced motion).
    pub reduced_motion: bool,
    /// Values, computations, timers and frame hooks a `render` function made: like its
    /// elements, they belong to that render and are released when the next one starts.
    pub render_owned: Vec<(Owned, Id)>,
    pub caps: Caps,
}

impl World {
    pub fn new(caps: Caps) -> World {
        World {
            elements: Arena::new(caps.elements),
            signals: Arena::new(caps.signals),
            comps: Arena::new(caps.comps),
            timers: Arena::new(caps.timers),
            frames: Arena::new(caps.timers),
            on_mount: Vec::new(),
            on_unmount: Vec::new(),
            mounted: false,
            tracking: Vec::new(),
            queue: VecDeque::new(),
            now: Duration::ZERO,
            changed: true,
            next_ref: 1,
            dead_refs: Vec::new(),
            free_refs: Vec::new(),
            root: None,
            rendering: false,
            settings: None,
            host: None,
            theme: None,
            handlers: Vec::new(),
            root_shown: None,
            reduced_motion: false,
            render_owned: Vec::new(),
            caps,
        }
    }

    /// A fresh `refs` key.
    pub fn new_ref(&mut self) -> Ref {
        if let Some(r) = self.free_refs.pop() {
            return r;
        }
        let r = self.next_ref;
        self.next_ref += 1;
        r
    }

    /// Forgets a `refs` key (its value is cleared at the end of the call).
    pub fn drop_ref(&mut self, r: Ref) {
        if r != 0 {
            self.dead_refs.push(r);
        }
    }

    /// Records that the running computation, if any, read `sig`.
    pub fn track(&mut self, sig: Id) {
        let Some(&comp) = self.tracking.last() else { return };
        let Some(c) = self.comps.get_mut(comp) else { return };
        if c.deps.contains(&sig) {
            return;
        }
        c.deps.push(sig);
        if let Some(s) = self.signals.get_mut(sig)
            && !s.subs.contains(&comp)
        {
            s.subs.push(comp);
        }
    }

    /// `sig` changed: what read it is marked (computed values) or queued (effects).
    pub fn notify(&mut self, sig: Id) {
        self.changed = true;
        // Iterative: a chain of computed values notifies down without recursion.
        let mut todo: Vec<Id> = self.signals.get(sig).map(|s| s.subs.clone()).unwrap_or_default();
        while let Some(comp) = todo.pop() {
            let Some(c) = self.comps.get_mut(comp) else { continue };
            match c.kind {
                CompKind::Computed { sig } => {
                    if !c.dirty {
                        c.dirty = true;
                        if let Some(s) = self.signals.get(sig) {
                            todo.extend(s.subs.iter().copied());
                        }
                    }
                }
                CompKind::Effect => {
                    if !c.queued {
                        c.queued = true;
                        self.queue.push_back(comp);
                    }
                }
            }
        }
    }

    /// Forgets what `comp` read, before it runs again or when it stops.
    pub fn clear_deps(&mut self, comp: Id) {
        let deps = self.comps.get_mut(comp).map(|c| std::mem::take(&mut c.deps)).unwrap_or_default();
        for d in deps {
            if let Some(s) = self.signals.get_mut(d) {
                s.subs.retain(|c| *c != comp);
            }
        }
    }

    /// Stops computation `comp` for good.
    pub fn drop_comp(&mut self, comp: Id) {
        self.clear_deps(comp);
        if let Some(c) = self.comps.remove(comp) {
            self.drop_ref(c.func);
        }
    }

    /// Frees element `id` and everything inside it (detaching it from its parent first). Its
    /// handles, and its children's, become stale.
    pub fn free_element(&mut self, id: Id) {
        if let Some(p) = self.elements.get(id).and_then(|e| e.parent)
            && let Some(parent) = self.elements.get_mut(p)
        {
            parent.children.retain(|c| *c != id);
        }
        let mut todo = vec![id];
        while let Some(n) = todo.pop() {
            // The mounted element may be inside what goes.
            if self.root == Some(n) {
                self.root = None;
            }
            if let Some(e) = self.elements.remove(n) {
                todo.extend(e.children);
                self.drop_ref(e.props);
            }
        }
        self.changed = true;
    }

    /// Whether `ancestor` is `id` or one of its ancestors (so `id` must not hold it).
    pub fn is_ancestor(&self, ancestor: Id, id: Id) -> bool {
        let mut at = Some(id);
        // Bounded by the number of elements: parents never form a loop, but a bound costs nothing.
        for _ in 0..=self.elements.len() {
            match at {
                Some(n) if n == ancestor => return true,
                Some(n) => at = self.elements.get(n).and_then(|e| e.parent),
                None => return false,
            }
        }
        true
    }

    /// How deep `id` sits below its topmost ancestor (0 at the top).
    pub fn depth(&self, id: Id) -> usize {
        let mut d = 0;
        let mut at = self.elements.get(id).and_then(|e| e.parent);
        while let Some(n) = at {
            d += 1;
            if d > self.elements.len() {
                break;
            }
            at = self.elements.get(n).and_then(|e| e.parent);
        }
        d
    }

    /// How deep the tree under `id` goes (1 for an element without children).
    pub fn height(&self, id: Id) -> usize {
        let mut best = 0;
        let mut todo = vec![(id, 1usize)];
        while let Some((n, d)) = todo.pop() {
            best = best.max(d);
            if let Some(e) = self.elements.get(n) {
                todo.extend(e.children.iter().map(|c| (*c, d + 1)));
            }
        }
        best
    }

    /// Whether some animation is still moving at the widget's clock.
    pub fn animating(&self) -> bool {
        self.signals.ids().into_iter().any(|id| {
            matches!(self.signals.get(id).map(|s| &s.kind), Some(SignalKind::Animated(a)) if !a.is_settled(self.now))
        })
    }

    /// The animated values moving now.
    pub fn moving(&self) -> Vec<Id> {
        self.signals
            .ids()
            .into_iter()
            .filter(|id| {
                matches!(self.signals.get(*id).map(|s| &s.kind), Some(SignalKind::Animated(a)) if !a.is_settled(self.now))
            })
            .collect()
    }

    /// Frees signal `id` (scripts dropped its last handle and nothing of the widget's names
    /// it); a computed value's computation goes with it.
    pub fn free_signal(&mut self, id: Id) {
        if [self.settings, self.host, self.theme].contains(&Some(id)) {
            return;
        }
        if let Some(s) = self.signals.remove(id) {
            self.drop_ref(s.value);
            if let SignalKind::Computed { comp } = s.kind {
                self.drop_comp(comp);
            }
        }
    }

    /// Records that a `render` function made `id`, when one is running.
    pub fn owned_by_render(&mut self, what: Owned, id: Id) {
        if self.rendering {
            self.render_owned.push((what, id));
        }
    }

    /// Releases what the last render made: its top elements (and what is inside them) and
    /// its values, computations, timers and frame hooks.
    pub fn release_render(&mut self) {
        let old: Vec<Id> = self
            .elements
            .ids()
            .into_iter()
            .filter(|id| self.elements.get(*id).is_some_and(|e| e.transient && e.parent.is_none()))
            .collect();
        for id in old {
            self.free_element(id);
        }
        for (what, id) in std::mem::take(&mut self.render_owned) {
            match what {
                Owned::Signal => {
                    if let Some(s) = self.signals.remove(id) {
                        self.drop_ref(s.value);
                        if let SignalKind::Computed { comp } = s.kind {
                            self.drop_comp(comp);
                        }
                    }
                }
                Owned::Comp => self.drop_comp(id),
                Owned::Timer => {
                    if let Some(t) = self.timers.remove(id) {
                        self.drop_ref(t.func);
                    }
                }
                Owned::Frame => {
                    if let Some(f) = self.frames.remove(id) {
                        self.drop_ref(f);
                    }
                }
            }
        }
    }

    /// What a call left half done when it was stopped: nothing is tracking, running or
    /// waiting any more.
    pub fn settle_after_stop(&mut self) {
        self.tracking.clear();
        for id in self.comps.ids() {
            if let Some(c) = self.comps.get_mut(id) {
                c.running = false;
                c.queued = false;
            }
        }
        self.queue.clear();
        self.rendering = false;
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "a failed assertion in a unit test stops that test")]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_never_reach_new_slots() {
        let mut a: Arena<u8> = Arena::new(2);
        let x = a.insert(1).unwrap_or(Id { idx: 9, generation: 9 });
        assert!(a.insert(2).is_some() && a.insert(3).is_none(), "capped at two");
        assert_eq!(a.remove(x), Some(1));
        let y = a.insert(4).unwrap_or(Id { idx: 9, generation: 9 });
        assert_eq!(y.idx, x.idx);
        assert!(a.get(x).is_none() && a.get(y) == Some(&4));
    }

    #[test]
    fn notifying_marks_chains_and_queues_effects_once() {
        let mut w = World::new(Caps::for_memory(1 << 20, 64));
        let s = w.signals.insert(Signal { value: 1, subs: Vec::new(), kind: SignalKind::Plain });
        let Some(s) = s else { unreachable!() };
        let c_sig = w.signals.insert(Signal { value: 2, subs: Vec::new(), kind: SignalKind::Plain });
        let Some(c_sig) = c_sig else { unreachable!() };
        let comp =
            |kind| Comp { func: 0, deps: Vec::new(), kind, dirty: false, queued: false, running: false };
        let Some(c) = w.comps.insert(comp(CompKind::Computed { sig: c_sig })) else { unreachable!() };
        let Some(e) = w.comps.insert(comp(CompKind::Effect)) else { unreachable!() };
        w.tracking.push(c);
        w.track(s);
        w.tracking.pop();
        w.tracking.push(e);
        w.track(c_sig);
        w.track(s);
        w.tracking.pop();
        w.notify(s);
        w.notify(s);
        assert!(w.comps.get(c).is_some_and(|c| c.dirty));
        assert_eq!(w.queue.len(), 1);
        w.clear_deps(e);
        assert!(w.signals.get(s).is_some_and(|x| x.subs == [c]));
    }
}
