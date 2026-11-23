//! The `ui` library scripts see, and the operations behind it: elements, signals, computed
//! values, effects, animated values, timers, hooks and theme tokens, over the [`World`].

use std::time::Duration;

use remedy_lua::{Error, Lua, TableRef, Value};
use remedy_ui_motion::{Animation, Easing, Motion, Spring};

use crate::world::{Caps, Comp, CompKind, Element, Id, Owned, Ref, Signal, SignalKind, Timer, World};

/// How often the widget asks for a frame while something moves or a frame hook is set.
pub(crate) const FRAME: Duration = Duration::from_millis(16);

/// The most effect runs one flush may make: effects that keep setting what each other read
/// are stopped with an error instead of running until the fuel is gone.
const MAX_EFFECT_RUNS: usize = 10_000;

/// The longest element kind or property key.
const MAX_NAME: usize = 64;

/// What a handle points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Element,
    Signal,
    Effect,
    Timer,
    Frame,
}

/// The userdata a script holds for an element, a signal, an effect, a timer or a frame hook.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Handle {
    pub kind: Kind,
    pub id: Id,
}

/// Everything a host function needs to reach the widget's Lua side: the `refs` table, the
/// weak table of element and value handles, and the handles' metatables. (The world itself
/// is the state's app data.)
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub refs: TableRef,
    pub handles: TableRef,
    pub metas: [TableRef; 5],
}

pub(crate) fn err(msg: impl Into<String>) -> Error {
    Error::runtime(msg)
}

/// Runs `f` on the world.
pub(crate) fn with<R>(lua: &mut Lua, ctx: Ctx, f: impl FnOnce(&mut World) -> R) -> Result<R, Error> {
    let _ = ctx;
    lua.app_data_mut::<World>().map(f).ok_or_else(|| err("the widget's world is gone"))
}

pub(crate) fn get_ref(lua: &Lua, ctx: Ctx, r: Ref) -> Value {
    lua.table_get(ctx.refs, Value::Int(r))
}

pub(crate) fn set_ref(lua: &mut Lua, ctx: Ctx, r: Ref, v: Value) -> Result<(), Error> {
    lua.table_set(ctx.refs, Value::Int(r), v)
}

/// Keeps `v` under a new key.
pub(crate) fn keep(lua: &mut Lua, ctx: Ctx, v: Value) -> Result<Ref, Error> {
    let r = with(lua, ctx, World::new_ref)?;
    set_ref(lua, ctx, r, v)?;
    Ok(r)
}

/// Clears the keys the world freed.
pub(crate) fn sweep_refs(lua: &mut Lua, ctx: Ctx) -> Result<(), Error> {
    let dead = with(lua, ctx, |w| std::mem::take(&mut w.dead_refs))?;
    for r in &dead {
        set_ref(lua, ctx, *r, Value::Nil)?;
    }
    with(lua, ctx, |w| w.free_refs.extend(dead))
}

fn meta(ctx: Ctx, kind: Kind) -> TableRef {
    let i = match kind {
        Kind::Element => 0,
        Kind::Signal => 1,
        Kind::Effect => 2,
        Kind::Timer => 3,
        Kind::Frame => 4,
    };
    ctx.metas.get(i).copied().unwrap_or(ctx.refs)
}

/// The key of an element's or a value's handle in the weak handle table, and in the
/// collector's reports: its slot, part of its generation, and its kind.
pub(crate) fn handle_key(kind: Kind, id: Id) -> i64 {
    let tag = i64::from(kind == Kind::Signal);
    (i64::from(id.generation & 0x3fff_ffff) << 33) | (i64::from(id.idx) << 1) | tag
}

/// What a handle key names: its kind, slot and the low bits of its generation.
pub(crate) fn from_handle_key(key: i64) -> (Kind, u32, u32) {
    let kind = if key & 1 == 1 { Kind::Signal } else { Kind::Element };
    (kind, ((key >> 1) & 0xffff_ffff) as u32, ((key >> 33) & 0x3fff_ffff) as u32)
}

/// The live handle for element or value `id`, if a script still holds one.
pub(crate) fn live_handle(lua: &Lua, ctx: Ctx, kind: Kind, id: Id) -> Option<Value> {
    let v = lua.table_get(ctx.handles, Value::Int(handle_key(kind, id)));
    lua.userdata::<Handle>(v).filter(|h| h.kind == kind && h.id == id).map(|_| v)
}

/// The handle for element or value `id`: the one scripts hold, if any (so the same thing is
/// always the same value), else a new one. Handles are held weakly: when scripts drop one,
/// the collector reports it and the host decides whether what it named goes too.
pub(crate) fn handle_for(lua: &mut Lua, ctx: Ctx, kind: Kind, id: Id) -> Result<Value, Error> {
    if let Some(v) = live_handle(lua, ctx, kind, id) {
        return Ok(v);
    }
    let key = handle_key(kind, id);
    let v = lua.create_userdata_tracked(Handle { kind, id }, Some(meta(ctx, kind)), key as u64)?;
    lua.table_set(ctx.handles, Value::Int(key), v)?;
    Ok(v)
}

/// A handle only the script keeps (an effect's, a timer's, a frame hook's): it goes when the
/// script lets it go.
fn loose_handle(lua: &mut Lua, ctx: Ctx, kind: Kind, id: Id) -> Result<Value, Error> {
    lua.create_userdata(Handle { kind, id }, Some(meta(ctx, kind)))
}

/// The handle `v`, of `kind`, or an error naming what was expected.
fn handle(lua: &Lua, v: Value, kind: Kind, what: &str) -> Result<Id, Error> {
    match lua.userdata::<Handle>(v) {
        Some(h) if h.kind == kind => Ok(h.id),
        _ => Err(err(format!("expected {what}"))),
    }
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).copied().unwrap_or(Value::Nil)
}

fn text(lua: &Lua, v: Value) -> Option<String> {
    lua.to_string_lossy(v)
}

fn number(v: Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(i as f64),
        Value::Num(f) => Some(f),
        _ => None,
    }
}

fn millis(v: Value, what: &str) -> Result<Duration, Error> {
    let ms = number(v)
        .filter(|m| m.is_finite() && *m >= 0.0)
        .ok_or_else(|| err(format!("{what} must be milliseconds")))?;
    Ok(Duration::from_secs_f64(ms.min(1e9) / 1000.0))
}

/// Whether two values are the same for a signal (no change, no notification): raw
/// equality (strings by content), and a NaN the same as itself.
fn same(lua: &Lua, a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x.to_bits() == y.to_bits() || x == y,
        _ => lua.raw_equal(&a, &b),
    }
}

// ---- elements ----

/// A new element of `kind` with properties from `props` (string keys; integer keys are
/// children when `children_from_props`) and `children`.
pub(crate) fn make_element(
    lua: &mut Lua,
    ctx: Ctx,
    kind: &str,
    props: Option<TableRef>,
    children: Vec<Value>,
) -> Result<Value, Error> {
    if kind.is_empty() || kind.len() > MAX_NAME {
        return Err(err("an element kind is a short name"));
    }
    let own = lua.create_table()?;
    let mut kids = children;
    if let Some(p) = props {
        for (k, v) in lua.table_pairs(p) {
            match k {
                Value::Str(_) => lua.table_set(own, k, v)?,
                Value::Int(_) => kids.push(v),
                _ => {}
            }
        }
    }
    let props_ref = keep(lua, ctx, Value::Table(own))?;
    let transient = with(lua, ctx, |w| w.rendering)?;
    let id = with(lua, ctx, |w| {
        w.elements.insert(Element {
            kind: kind.to_owned(),
            props: props_ref,
            children: Vec::new(),
            parent: None,
            transient,
        })
    })?
    .ok_or_else(|| err("the widget made too many elements"))?;
    let v = handle_for(lua, ctx, Kind::Element, id)?;
    with(lua, ctx, |w| w.changed = true)?;
    for c in kids {
        append(lua, ctx, id, c, None)?;
    }
    Ok(v)
}

/// Puts element `child` into `parent`, at `at` (1-based) or last. A child that has a parent
/// moves; an element cannot go inside itself.
fn append(lua: &mut Lua, ctx: Ctx, parent: Id, child: Value, at: Option<usize>) -> Result<(), Error> {
    let child = handle(lua, child, Kind::Element, "an element (made with ui.*) as a child")?;
    with(lua, ctx, |w| -> Result<(), Error> {
        if !w.elements.contains(child) {
            return Err(err("that element was removed"));
        }
        if !w.elements.contains(parent) {
            return Err(err("that element was removed"));
        }
        if w.is_ancestor(child, parent) {
            return Err(err("an element cannot go inside itself"));
        }
        if w.depth(parent) + 1 + w.height(child) > w.caps.nesting {
            return Err(err("elements nest too deeply"));
        }
        let count = w.elements.get(parent).map_or(0, |p| p.children.len());
        if count >= w.caps.children {
            return Err(err("an element holds too many children"));
        }
        if let Some(old) = w.elements.get(child).and_then(|e| e.parent)
            && let Some(op) = w.elements.get_mut(old)
        {
            op.children.retain(|c| *c != child);
        }
        if let Some(p) = w.elements.get_mut(parent) {
            let i = at.map_or(p.children.len(), |i| i.saturating_sub(1).min(p.children.len()));
            p.children.insert(i, child);
        }
        if let Some(c) = w.elements.get_mut(child) {
            c.parent = Some(parent);
        }
        if w.root == Some(child) {
            w.root = None;
        }
        w.changed = true;
        Ok(())
    })?
}

fn element_methods(lua: &mut Lua, ctx: Ctx, t: TableRef) -> Result<(), Error> {
    let live = |lua: &Lua, v: Value| handle(lua, v, Kind::Element, "an element");
    method(lua, t, "set", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let key = text(lua, arg(a, 1)).filter(|k| !k.is_empty() && k.len() <= MAX_NAME);
        let key = key.ok_or_else(|| err("a property name is a short string"))?;
        let props = with(lua, ctx, |w| w.elements.get(id).map(|e| e.props))?
            .ok_or_else(|| err("that element was removed"))?;
        let Value::Table(t) = get_ref(lua, ctx, props) else { return Err(err("that element was removed")) };
        let k = lua.create_string(key)?;
        lua.table_set(t, k, arg(a, 2))?;
        with(lua, ctx, |w| w.changed = true)?;
        Ok(vec![arg(a, 0)])
    })?;
    method(lua, t, "get", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let props = with(lua, ctx, |w| w.elements.get(id).map(|e| e.props))?
            .ok_or_else(|| err("that element was removed"))?;
        let Value::Table(t) = get_ref(lua, ctx, props) else { return Ok(vec![Value::Nil]) };
        Ok(vec![lua.table_get(t, arg(a, 1))])
    })?;
    method(lua, t, "kind", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let k = with(lua, ctx, |w| w.elements.get(id).map(|e| e.kind.clone()))?
            .ok_or_else(|| err("that element was removed"))?;
        Ok(vec![lua.create_string(k)?])
    })?;
    method(lua, t, "alive", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        Ok(vec![Value::Bool(with(lua, ctx, |w| w.elements.contains(id))?)])
    })?;
    method(lua, t, "append", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        for c in a.iter().skip(1) {
            append(lua, ctx, id, *c, None)?;
        }
        Ok(vec![arg(a, 0)])
    })?;
    method(lua, t, "insert", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let at = number(arg(a, 1)).filter(|n| *n >= 1.0).ok_or_else(|| err("insert(position, child)"))?;
        append(lua, ctx, id, arg(a, 2), Some(at as usize))?;
        Ok(vec![arg(a, 0)])
    })?;
    method(lua, t, "remove", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        with(lua, ctx, |w| w.free_element(id))?;
        sweep_refs(lua, ctx)?;
        Ok(vec![])
    })?;
    method(lua, t, "clear", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        with(lua, ctx, |w| {
            let kids = w.elements.get(id).map(|e| e.children.clone()).unwrap_or_default();
            for k in kids {
                w.free_element(k);
            }
        })?;
        sweep_refs(lua, ctx)?;
        Ok(vec![arg(a, 0)])
    })?;
    method(lua, t, "children", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let kids: Vec<Id> =
            with(lua, ctx, |w| w.elements.get(id).map(|e| e.children.clone()).unwrap_or_default())?;
        let list = lua.create_table()?;
        for (i, c) in kids.into_iter().enumerate() {
            let h = handle_for(lua, ctx, Kind::Element, c)?;
            lua.table_set(list, Value::Int(i as i64 + 1), h)?;
        }
        Ok(vec![Value::Table(list)])
    })?;
    method(lua, t, "parent", move |lua, a| {
        let id = live(lua, arg(a, 0))?;
        let p = with(lua, ctx, |w| w.elements.get(id).and_then(|e| e.parent))?;
        Ok(vec![match p {
            Some(p) => handle_for(lua, ctx, Kind::Element, p)?,
            None => Value::Nil,
        }])
    })?;
    Ok(())
}

// ---- signals and computations ----

/// What reading a signal involves, worked out with the world borrowed.
enum Read {
    Plain(Ref),
    Computed { comp: Id, value: Ref, dirty: bool, running: bool },
    Animated(f32),
}

/// The value of signal `id`, recorded as read by the running computation when `track`.
pub(crate) fn signal_get(lua: &mut Lua, ctx: Ctx, id: Id, track: bool) -> Result<Value, Error> {
    let read = with(lua, ctx, |w| {
        let now = w.now;
        w.signals.get(id).map(|s| match &s.kind {
            SignalKind::Plain => Read::Plain(s.value),
            SignalKind::Computed { comp } => {
                let c = w.comps.get(*comp);
                Read::Computed {
                    comp: *comp,
                    value: s.value,
                    dirty: c.is_some_and(|c| c.dirty),
                    running: c.is_some_and(|c| c.running),
                }
            }
            SignalKind::Animated(a) => Read::Animated(a.sample(now)),
        })
    })?
    .ok_or_else(|| err("that value was dropped"))?;
    let v = match read {
        Read::Plain(r) => get_ref(lua, ctx, r),
        Read::Computed { comp, value, dirty, running } => {
            if running {
                return Err(err("a computed value reads itself"));
            }
            if dirty {
                run_comp(lua, ctx, comp)?;
            }
            get_ref(lua, ctx, value)
        }
        Read::Animated(x) => Value::Num(f64::from(x)),
    };
    if track {
        with(lua, ctx, |w| w.track(id))?;
    }
    Ok(v)
}

/// Sets plain signal `id` to `v`; what read it runs again (effects at the end of the
/// handler, timer or frame that set it).
pub(crate) fn signal_set(lua: &mut Lua, ctx: Ctx, id: Id, v: Value) -> Result<(), Error> {
    let (value, plain, in_computed) = with(lua, ctx, |w| {
        let in_computed = w
            .tracking
            .last()
            .and_then(|c| w.comps.get(*c))
            .is_some_and(|c| matches!(c.kind, CompKind::Computed { .. }));
        w.signals.get(id).map(|s| (s.value, matches!(s.kind, SignalKind::Plain), in_computed))
    })?
    .ok_or_else(|| err("that value was dropped"))?;
    if in_computed {
        return Err(err("a computed value cannot change signals (use an effect)"));
    }
    if !plain {
        return Err(err("a computed value cannot be set"));
    }
    if same(lua, get_ref(lua, ctx, value), v) {
        return Ok(());
    }
    set_ref(lua, ctx, value, v)?;
    with(lua, ctx, |w| w.notify(id))
}

/// Runs computation `comp`, tracking what it reads; a computed value keeps its result.
pub(crate) fn run_comp(lua: &mut Lua, ctx: Ctx, comp: Id) -> Result<(), Error> {
    let started = with(lua, ctx, |w| {
        w.clear_deps(comp);
        let depth = w.tracking.len();
        let c = w.comps.get_mut(comp)?;
        c.running = true;
        let r = (c.func, c.kind, depth);
        w.tracking.push(comp);
        Some(r)
    })?;
    let Some((func, kind, depth)) = started else { return Ok(()) };
    let f = get_ref(lua, ctx, func);
    let out = lua.call(f, &[]);
    with(lua, ctx, |w| {
        w.tracking.truncate(depth);
        if let Some(c) = w.comps.get_mut(comp) {
            c.running = false;
            c.dirty = false;
        }
    })?;
    let out = out?;
    if let CompKind::Computed { sig } = kind {
        let value = with(lua, ctx, |w| w.signals.get(sig).map(|s| s.value))?;
        if let Some(value) = value {
            set_ref(lua, ctx, value, out.first().copied().unwrap_or(Value::Nil))?;
        }
    }
    Ok(())
}

/// Runs the effects waiting, until none is left.
pub(crate) fn flush(lua: &mut Lua, ctx: Ctx) -> Result<(), Error> {
    for _ in 0..MAX_EFFECT_RUNS {
        let next = with(lua, ctx, |w| {
            while let Some(c) = w.queue.pop_front() {
                if let Some(comp) = w.comps.get_mut(c)
                    && comp.queued
                {
                    comp.queued = false;
                    return Some(c);
                }
            }
            None
        })?;
        let Some(c) = next else { return Ok(()) };
        run_comp(lua, ctx, c)?;
    }
    Err(err("effects kept changing what each other read"))
}

/// A new signal of `kind` holding `v`.
fn new_signal(lua: &mut Lua, ctx: Ctx, v: Value, kind: SignalKind) -> Result<(Value, Id), Error> {
    let value = keep(lua, ctx, v)?;
    let id = with(lua, ctx, |w| w.signals.insert(Signal { value, subs: Vec::new(), kind }))?
        .ok_or_else(|| err("the widget made too many values"))?;
    let h = handle_for(lua, ctx, Kind::Signal, id)?;
    with(lua, ctx, |w| w.owned_by_render(Owned::Signal, id))?;
    Ok((h, id))
}

/// A signal for the host's data (settings, host data, theme), made once.
pub(crate) fn host_signal(lua: &mut Lua, ctx: Ctx, v: Value) -> Result<(Value, Id), Error> {
    new_signal(lua, ctx, v, SignalKind::Plain)
}

/// A new computation running `f`.
fn new_comp(lua: &mut Lua, ctx: Ctx, f: Value, kind: CompKind) -> Result<Id, Error> {
    if !matches!(f, Value::Func(_)) {
        return Err(err("expected a function"));
    }
    let func = keep(lua, ctx, f)?;
    let id = with(lua, ctx, |w| {
        w.comps.insert(Comp { func, deps: Vec::new(), kind, dirty: true, queued: false, running: false })
    })?
    .ok_or_else(|| err("the widget made too many computed values and effects"))?;
    // A computed value goes with its signal; an effect on its own.
    if kind == CompKind::Effect {
        with(lua, ctx, |w| w.owned_by_render(Owned::Comp, id))?;
    }
    Ok(id)
}

fn motion(lua: &Lua, v: Value) -> Result<Motion, Error> {
    let Value::Table(t) = v else {
        return Ok(Motion::Spring(Spring::smooth(Duration::from_millis(300))));
    };
    let field = |name: &str| -> Value {
        let mut out = Value::Nil;
        for (k, v) in lua.table_pairs(t) {
            if lua.str_bytes(k) == Some(name.as_bytes()) {
                out = v;
            }
        }
        out
    };
    if let Some(ms) = number(field("spring")) {
        let bounce = number(field("bounce")).unwrap_or(0.0).clamp(0.0, 0.9) as f32;
        let d = millis(Value::Num(ms), "spring")?;
        return Ok(Motion::Spring(Spring::new(d.max(Duration::from_millis(1)), bounce)));
    }
    let duration = millis(field("duration"), "duration")?;
    if duration.is_zero() {
        return Ok(Motion::Instant);
    }
    let easing = match text(lua, field("easing")).as_deref() {
        None | Some("standard") => Easing::STANDARD,
        Some("linear") => Easing::Linear,
        Some("decelerate") => Easing::DECELERATE,
        Some("accelerate") => Easing::ACCELERATE,
        Some("ease_in_out") => Easing::EASE_IN_OUT,
        Some(other) => return Err(err(format!("unknown easing {other}"))),
    };
    Ok(Motion::Tween { duration, easing })
}

fn signal_methods(lua: &mut Lua, ctx: Ctx, t: TableRef) -> Result<(), Error> {
    let sig = |lua: &Lua, v: Value| {
        handle(lua, v, Kind::Signal, "a value made with ui.signal, ui.computed or ui.animated")
    };
    method(lua, t, "get", move |lua, a| Ok(vec![signal_get(lua, ctx, sig(lua, arg(a, 0))?, true)?]))?;
    method(lua, t, "peek", move |lua, a| Ok(vec![signal_get(lua, ctx, sig(lua, arg(a, 0))?, false)?]))?;
    method(lua, t, "set", move |lua, a| {
        let id = sig(lua, arg(a, 0))?;
        let animated =
            with(lua, ctx, |w| matches!(w.signals.get(id).map(|s| &s.kind), Some(SignalKind::Animated(_))))?;
        if animated {
            let x = number(arg(a, 1)).ok_or_else(|| err("an animated value is a number"))? as f32;
            with(lua, ctx, |w| {
                if let Some(SignalKind::Animated(an)) = w.signals.get_mut(id).map(|s| &mut s.kind) {
                    an.set(x);
                }
                w.notify(id);
            })?;
        } else {
            signal_set(lua, ctx, id, arg(a, 1))?;
        }
        Ok(vec![])
    })?;
    method(lua, t, "update", move |lua, a| {
        let id = sig(lua, arg(a, 0))?;
        let old = signal_get(lua, ctx, id, false)?;
        let new = lua.call(arg(a, 1), &[old])?.first().copied().unwrap_or(Value::Nil);
        signal_set(lua, ctx, id, new)?;
        Ok(vec![new])
    })?;
    method(lua, t, "to", move |lua, a| {
        let id = sig(lua, arg(a, 0))?;
        let target = number(arg(a, 1))
            .filter(|x| x.is_finite())
            .ok_or_else(|| err("to(target) takes a number"))? as f32;
        let m = motion(lua, arg(a, 2))?;
        let ok = with(lua, ctx, |w| {
            let now = w.now;
            let moved = match w.signals.get_mut(id).map(|s| &mut s.kind) {
                Some(SignalKind::Animated(an)) => {
                    an.animate_to(target, m, now);
                    true
                }
                _ => false,
            };
            if moved {
                w.notify(id);
            }
            moved
        })?;
        if !ok {
            return Err(err("only a value made with ui.animated moves with to()"));
        }
        Ok(vec![arg(a, 0)])
    })?;
    method(lua, t, "settled", move |lua, a| {
        let id = sig(lua, arg(a, 0))?;
        let s = with(lua, ctx, |w| match w.signals.get(id).map(|s| &s.kind) {
            Some(SignalKind::Animated(an)) => an.is_settled(w.now),
            _ => true,
        })?;
        Ok(vec![Value::Bool(s)])
    })?;
    Ok(())
}

fn stop_methods(lua: &mut Lua, ctx: Ctx, t: TableRef, kind: Kind) -> Result<(), Error> {
    method(lua, t, "stop", move |lua, a| {
        let id = handle(lua, arg(a, 0), kind, "a handle from ui.effect, ui.after, ui.every or ui.on_frame")?;
        with(lua, ctx, |w| match kind {
            Kind::Effect => w.drop_comp(id),
            Kind::Timer => {
                if let Some(t) = w.timers.remove(id) {
                    w.drop_ref(t.func);
                }
            }
            Kind::Frame => {
                if let Some(f) = w.frames.remove(id) {
                    w.drop_ref(f);
                }
            }
            Kind::Element | Kind::Signal => {}
        })?;
        sweep_refs(lua, ctx)?;
        Ok(vec![])
    })
}

// ---- installing the library ----

fn method(
    lua: &mut Lua,
    t: TableRef,
    name: &str,
    f: impl Fn(&mut Lua, &[Value]) -> Result<Vec<Value>, Error> + Send + Sync + 'static,
) -> Result<(), Error> {
    let k = lua.create_string(name)?;
    let func = lua.create_function(name, f)?;
    lua.table_set(t, k, func)
}

fn field(lua: &mut Lua, t: TableRef, name: &str, v: Value) -> Result<(), Error> {
    let k = lua.create_string(name)?;
    lua.table_set(t, k, v)
}

/// Makes the handles' metatables (kept in `refs` under keys -1 to -5), the weak handle table
/// (under 0), and keeps the world as the state's app data.
pub(crate) fn make_ctx(lua: &mut Lua, world: World, refs: TableRef) -> Result<Ctx, Error> {
    lua.set_app_data(world);
    let handles = lua.create_table()?;
    let weak = lua.create_table()?;
    let v = lua.create_string("v")?;
    field(lua, weak, "__mode", v)?;
    lua.set_metatable(handles, Some(weak))?;
    lua.table_set(refs, Value::Int(0), Value::Table(handles))?;
    let mut metas = [refs; 5];
    for (i, (slot, name)) in
        metas.iter_mut().zip(["element", "value", "effect", "timer", "frame hook"]).enumerate()
    {
        let m = lua.create_table()?;
        let methods = lua.create_table()?;
        field(lua, m, "__index", Value::Table(methods))?;
        let n = lua.create_string(name)?;
        field(lua, m, "__name", n)?;
        // A script cannot reach or replace the metatable.
        let locked = lua.create_string("locked")?;
        field(lua, m, "__metatable", locked)?;
        lua.table_set(refs, Value::Int(-(i as i64) - 1), Value::Table(m))?;
        *slot = m;
    }
    let ctx = Ctx { refs, handles, metas };
    let methods = |lua: &Lua, m: TableRef| -> TableRef {
        let mut out = m;
        for (k, v) in lua.table_pairs(m) {
            if let (Some(b"__index"), Value::Table(t)) = (lua.str_bytes(k), v) {
                out = t;
            }
        }
        out
    };
    let [em, sm, fm, tm, frm] = metas;
    let (em, sm, fm, tm, frm) =
        (methods(lua, em), methods(lua, sm), methods(lua, fm), methods(lua, tm), methods(lua, frm));
    element_methods(lua, ctx, em)?;
    signal_methods(lua, ctx, sm)?;
    stop_methods(lua, ctx, fm, Kind::Effect)?;
    stop_methods(lua, ctx, tm, Kind::Timer)?;
    stop_methods(lua, ctx, frm, Kind::Frame)?;
    Ok(ctx)
}

/// Text arguments of an element constructor: `ui.text("hi")`, `ui.text(signal)`.
fn put(lua: &mut Lua, t: TableRef, name: &str, v: Value) -> Result<(), Error> {
    if !v.is_nil() {
        field(lua, t, name, v)?;
    }
    Ok(())
}

/// Installs the `ui` table.
pub(crate) fn install(lua: &mut Lua, ctx: Ctx, offered: &[String]) -> Result<TableRef, Error> {
    let ui = lua.create_table()?;
    // Elements.
    method(lua, ui, "node", move |lua, a| {
        let kind = text(lua, arg(a, 0)).ok_or_else(|| err("ui.node(kind, props, children)"))?;
        let props = match arg(a, 1) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let kids = match arg(a, 2) {
            Value::Table(t) => (1..=lua.table_len(t)).map(|i| lua.table_get(t, Value::Int(i))).collect(),
            _ => Vec::new(),
        };
        Ok(vec![make_element(lua, ctx, &kind, props, kids)?])
    })?;
    for kind in ["column", "row"] {
        method(lua, ui, kind, move |lua, a| {
            // ui.column{ gap = 8, child, child } or ui.column(child, child).
            let (props, kids) = match arg(a, 0) {
                Value::Table(t) => (Some(t), a.get(1..).unwrap_or_default().to_vec()),
                _ => (None, a.to_vec()),
            };
            Ok(vec![make_element(lua, ctx, kind, props, kids)?])
        })?;
    }
    for (name, style) in [("text", "body"), ("muted", "muted"), ("strong", "strong")] {
        method(lua, ui, name, move |lua, a| {
            let t = lua.create_table()?;
            put(lua, t, "text", arg(a, 0))?;
            let style = match (style, arg(a, 1)) {
                ("body", Value::Str(_)) => arg(a, 1),
                _ => lua.create_string(style)?,
            };
            field(lua, t, "style", style)?;
            Ok(vec![make_element(lua, ctx, "text", Some(t), Vec::new())?])
        })?;
    }
    method(lua, ui, "heading", move |lua, a| {
        let t = lua.create_table()?;
        put(lua, t, "text", arg(a, 0))?;
        let s = lua.create_string("heading")?;
        field(lua, t, "style", s)?;
        put(lua, t, "level", arg(a, 1))?;
        Ok(vec![make_element(lua, ctx, "text", Some(t), Vec::new())?])
    })?;
    method(lua, ui, "button", move |lua, a| {
        let t = lua.create_table()?;
        put(lua, t, "label", arg(a, 0))?;
        put(lua, t, "on", arg(a, 1))?;
        Ok(vec![make_element(lua, ctx, "button", Some(t), Vec::new())?])
    })?;
    method(lua, ui, "link", move |lua, a| {
        let t = lua.create_table()?;
        put(lua, t, "label", arg(a, 0))?;
        put(lua, t, "url", arg(a, 1))?;
        Ok(vec![make_element(lua, ctx, "link", Some(t), Vec::new())?])
    })?;
    method(lua, ui, "progress", move |lua, a| {
        let t = lua.create_table()?;
        put(lua, t, "label", arg(a, 0))?;
        put(lua, t, "value", arg(a, 1))?;
        Ok(vec![make_element(lua, ctx, "progress", Some(t), Vec::new())?])
    })?;
    method(lua, ui, "divider", move |lua, _| Ok(vec![make_element(lua, ctx, "divider", None, Vec::new())?]))?;
    method(lua, ui, "image", move |lua, a| {
        let t = lua.create_table()?;
        put(lua, t, "src", arg(a, 0))?;
        put(lua, t, "alt", arg(a, 1))?;
        put(lua, t, "width", arg(a, 2))?;
        put(lua, t, "height", arg(a, 3))?;
        Ok(vec![make_element(lua, ctx, "image", Some(t), Vec::new())?])
    })?;
    let offered: Vec<String> = offered.to_vec();
    method(lua, ui, "element", move |lua, a| {
        let kind = text(lua, arg(a, 0)).ok_or_else(|| err("ui.element(kind, props, children)"))?;
        if !offered.contains(&kind) {
            return Err(err(format!("this app offers no {kind} element")));
        }
        let props = match arg(a, 1) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let kids = match arg(a, 2) {
            Value::Table(t) => (1..=lua.table_len(t)).map(|i| lua.table_get(t, Value::Int(i))).collect(),
            _ => Vec::new(),
        };
        Ok(vec![make_element(lua, ctx, &kind, props, kids)?])
    })?;
    method(lua, ui, "mount", move |lua, a| {
        let id = handle(lua, arg(a, 0), Kind::Element, "an element to show")?;
        with(lua, ctx, |w| {
            if !w.elements.contains(id) {
                return Err(err("that element was removed"));
            }
            if let Some(e) = w.elements.get_mut(id) {
                // Mounted elements outlive the render that made them.
                e.transient = false;
            }
            w.root = Some(id);
            w.changed = true;
            Ok(())
        })??;
        Ok(vec![])
    })?;
    // Reactive values.
    method(lua, ui, "signal", move |lua, a| Ok(vec![new_signal(lua, ctx, arg(a, 0), SignalKind::Plain)?.0]))?;
    method(lua, ui, "computed", move |lua, a| {
        let value = keep(lua, ctx, Value::Nil)?;
        let sig = with(lua, ctx, |w| {
            w.signals.insert(Signal { value, subs: Vec::new(), kind: SignalKind::Plain })
        })?
        .ok_or_else(|| err("the widget made too many values"))?;
        let comp = new_comp(lua, ctx, arg(a, 0), CompKind::Computed { sig })?;
        with(lua, ctx, |w| {
            if let Some(s) = w.signals.get_mut(sig) {
                s.kind = SignalKind::Computed { comp };
            }
            w.owned_by_render(Owned::Signal, sig);
        })?;
        Ok(vec![handle_for(lua, ctx, Kind::Signal, sig)?])
    })?;
    method(lua, ui, "effect", move |lua, a| {
        let comp = new_comp(lua, ctx, arg(a, 0), CompKind::Effect)?;
        let h = loose_handle(lua, ctx, Kind::Effect, comp)?;
        // It runs now, reading what it reads.
        run_comp(lua, ctx, comp)?;
        Ok(vec![h])
    })?;
    method(lua, ui, "animated", move |lua, a| {
        let x = number(arg(a, 0)).filter(|x| x.is_finite()).ok_or_else(|| err("ui.animated(number)"))? as f32;
        let mut anim = Animation::new(x);
        anim.set_reduced_motion(with(lua, ctx, |w| w.reduced_motion)?);
        Ok(vec![new_signal(lua, ctx, Value::Nil, SignalKind::Animated(Box::new(anim)))?.0])
    })?;
    method(lua, ui, "untrack", move |lua, a| {
        let saved = with(lua, ctx, |w| std::mem::take(&mut w.tracking))?;
        let r = lua.call(arg(a, 0), &[]);
        with(lua, ctx, |w| w.tracking = saved)?;
        r
    })?;
    // Time: the widget's own clock, moved on by the frame loop.
    method(lua, ui, "now", move |lua, _| {
        let now = with(lua, ctx, |w| w.now)?;
        Ok(vec![Value::Num(now.as_secs_f64() * 1000.0)])
    })?;
    for (name, repeat) in [("after", false), ("every", true)] {
        method(lua, ui, name, move |lua, a| {
            let d = millis(arg(a, 0), "a timer's delay")?;
            let d = if repeat { d.max(Duration::from_millis(1)) } else { d };
            if !matches!(arg(a, 1), Value::Func(_)) {
                return Err(err(format!("ui.{name}(milliseconds, function)")));
            }
            let func = keep(lua, ctx, arg(a, 1))?;
            let id = with(lua, ctx, |w| {
                let id =
                    w.timers.insert(Timer { due: w.now.saturating_add(d), every: repeat.then_some(d), func });
                if let Some(id) = id {
                    w.owned_by_render(Owned::Timer, id);
                }
                id
            })?
            .ok_or_else(|| err("the widget set too many timers"))?;
            Ok(vec![loose_handle(lua, ctx, Kind::Timer, id)?])
        })?;
    }
    method(lua, ui, "on_frame", move |lua, a| {
        if !matches!(arg(a, 0), Value::Func(_)) {
            return Err(err("ui.on_frame(function(ms) ... end)"));
        }
        let func = keep(lua, ctx, arg(a, 0))?;
        let id = with(lua, ctx, |w| {
            let id = w.frames.insert(func);
            if let Some(id) = id {
                w.owned_by_render(Owned::Frame, id);
            }
            id
        })?
        .ok_or_else(|| err("the widget set too many frame hooks"))?;
        Ok(vec![loose_handle(lua, ctx, Kind::Frame, id)?])
    })?;
    for (name, mount) in [("on_mount", true), ("on_unmount", false)] {
        method(lua, ui, name, move |lua, a| {
            if !matches!(arg(a, 0), Value::Func(_)) {
                return Err(err(format!("ui.{name}(function)")));
            }
            let func = keep(lua, ctx, arg(a, 0))?;
            with(lua, ctx, |w| {
                let list = if mount { &mut w.on_mount } else { &mut w.on_unmount };
                if list.len() < 64 {
                    list.push(func);
                }
            })?;
            Ok(vec![])
        })?;
    }
    // Theme tokens: `ui.token("color.accent", fallback)`, tracked.
    method(lua, ui, "token", move |lua, a| {
        let path = text(lua, arg(a, 0)).ok_or_else(|| err("ui.token(\"group.name\")"))?;
        let Some(theme) = with(lua, ctx, |w| w.theme)? else { return Ok(vec![arg(a, 1)]) };
        let mut at = signal_get(lua, ctx, theme, true)?;
        for part in path.split('.') {
            let Value::Table(t) = at else { break };
            let k = lua.create_string(part)?;
            at = lua.table_get(t, k);
        }
        Ok(vec![if at.is_nil() { arg(a, 1) } else { at }])
    })?;
    Ok(ui)
}

/// Caps for a widget of `memory` bytes.
pub(crate) fn caps(memory: usize, nesting: usize) -> Caps {
    Caps::for_memory(memory, nesting)
}
