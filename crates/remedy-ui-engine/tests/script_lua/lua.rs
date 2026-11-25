//! Lua widgets: elements made, changed and removed from Lua; signals, computed values and
//! effects; motion, timers and frame hooks on the widget's clock; lifecycle hooks; theme
//! tokens; the app's granted functions and elements. And hostile scripts: each is stopped
//! with a clear error, nothing panics or hangs, and the widget keeps working.

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests fail by panicking and index what they just checked"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_engine::script::{
    Action, Engines, Limits, Node, Package, ScriptError, ScriptWidget, TextStyle,
};
use remedy_ui_engine::script_lua::{Grants, LuaEngine, LuaWidget};
use serde_json::{Map, Value, json};

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

const CLAPS: &str = include_str!("../../examples/script_lua/kept.lua");
const PULSE: &str = include_str!("../../examples/script_lua/pulse.lua");

/// Limits that stop at the same step on every machine (the time limit, a backstop, set far
/// out of the way).
fn limits() -> Limits {
    Limits::default().operations(Some(2_000_000)).time(Duration::from_secs(30))
}

fn load(src: &str) -> Result<LuaWidget, ScriptError> {
    LuaEngine::new().load_source(src, limits())
}

fn load_with(src: &str, grants: Grants) -> Result<LuaWidget, ScriptError> {
    LuaEngine::with_grants(grants).load_source(src, limits())
}

fn render(w: &mut dyn ScriptWidget) -> Result<Node, ScriptError> {
    w.render(&Map::new(), &json!({ "name": "Sam", "house": "the Den" }))
}

fn children(n: &Node) -> &[Node] {
    match n {
        Node::Column { children, .. } | Node::Row { children, .. } | Node::Element { children, .. } => {
            children
        }
        _ => &[],
    }
}

fn text_of(n: &Node) -> String {
    match n {
        Node::Text { text, .. } => text.clone(),
        other => format!("{other:?}"),
    }
}

/// The first button's action, depth first.
fn button(n: &Node, label: &str) -> Option<Action> {
    match n {
        Node::Button { label: l, action } if l == label => Some(*action),
        other => children(other).iter().find_map(|c| button(c, label)),
    }
}

#[test]
fn the_example_keeps_its_elements_and_follows_its_signals() {
    let engines = Engines::new().with(std::rc::Rc::new(LuaEngine::new()));
    let mut w = engines.load(&Package::lua(CLAPS), limits()).must("claps loads");
    assert_eq!(w.name(), "Claps");
    assert_eq!(w.settings().len(), 2);
    let mut settings = Map::new();
    settings.insert("title".into(), "Party".into());
    settings.insert("goal".into(), 4.into());
    let host = json!({ "house": "the Den" });
    let n = w.render(&settings, &host).must("renders");
    let kids = children(&n);
    assert_eq!(kids.first(), Some(&Node::Text { text: "Party".into(), style: TextStyle::Heading(2) }));
    assert_eq!(kids.get(1).map(text_of).as_deref(), Some("0 claps from the Den"));
    for _ in 0..2 {
        let a = button(&n, "Clap").must("a clap button");
        w.act(a).must("clap");
    }
    let n = w.render(&settings, &host).must("renders again");
    let kids = children(&n);
    assert_eq!(kids.get(1).map(text_of).as_deref(), Some("2 claps from the Den"));
    assert_eq!(
        kids.get(2),
        Some(&Node::Progress { label: "Claps".into(), value: 0.5 }),
        "the effect moved the bar"
    );
}

#[test]
fn a_render_function_describes_the_widget_afresh_each_time() {
    let src = r#"
        local n = 0
        return {
          name = "Fresh",
          render = function(settings, host)
            n = n + 1
            return ui.column{ ui.text("render " .. n), ui.text(host.name or "?"),
                              ui.button("Again", function() n = n + 10 end) }
          end,
        }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    assert_eq!(children(&n).first().map(text_of).as_deref(), Some("render 1"));
    assert_eq!(children(&n).get(1).map(text_of).as_deref(), Some("Sam"));
    w.act(button(&n, "Again").must("button")).must("acts");
    let n = render(&mut w).must("renders again");
    assert_eq!(children(&n).first().map(text_of).as_deref(), Some("render 12"));
    // Each render's elements are released by the next: the count stays flat.
    let before = w.elements();
    for _ in 0..20 {
        render(&mut w).must("renders");
    }
    assert_eq!(w.elements(), before);
}

#[test]
fn elements_are_created_updated_and_removed_from_lua() {
    let src = r#"
        local list = ui.column{}
        local items = {}
        local function add()
          local row = ui.row{ ui.text("item " .. (#items + 1)) }
          items[#items + 1] = row
          list:append(row)
        end
        local function drop_first()
          local first = table.remove(items, 1)
          if first then first:remove() end
        end
        local title = ui.text("List")
        ui.mount(ui.column{ title, list, ui.button("Add", add), ui.button("Drop", drop_first),
          ui.button("Rename", function() title:set("text", "Renamed") end),
          ui.button("Move", function() if #items > 1 then list:insert(1, items[#items]) end end) })
        return { name = "List" }"#;
    let mut w = load(src).must("loads");
    let mut n = render(&mut w).must("renders");
    for _ in 0..3 {
        w.act(button(&n, "Add").must("add")).must("adds");
        n = render(&mut w).must("renders");
    }
    let list = children(&n).get(1).must("the list");
    assert_eq!(children(list).len(), 3);
    w.act(button(&n, "Move").must("move")).must("moves");
    n = render(&mut w).must("renders");
    let first = children(children(&n).get(1).must("list")).first().must("first row");
    assert_eq!(children(first).first().map(text_of).as_deref(), Some("item 3"));
    w.act(button(&n, "Drop").must("drop")).must("drops");
    w.act(button(&n, "Rename").must("rename")).must("renames");
    n = render(&mut w).must("renders");
    assert_eq!(children(children(&n).get(1).must("list")).len(), 2);
    assert_eq!(children(&n).first().map(text_of).as_deref(), Some("Renamed"));
}

#[test]
fn computed_values_are_lazy_and_effects_follow_what_they_read() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = calls.clone();
    let grants = Grants::none().function("log", move |args| {
        if let Ok(mut l) = log.lock() {
            l.push(args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(","));
        }
        Ok(Value::Null)
    });
    let src = r#"
        local a, b = ui.signal(1), ui.signal(2)
        local runs = 0
        local sum = ui.computed(function() runs = runs + 1 return a:get() + b:get() end)
        ui.effect(function() app.log("sum", sum:get()) end)
        local untracked = ui.effect(function() app.log("peek", ui.untrack(function() return a:get() end)) end)
        ui.mount(ui.column{ ui.button("A", function() a:set(10) end),
                            ui.button("Same", function() a:set(10) end),
                            ui.button("Both", function() a:set(20) b:set(30) end),
                            ui.button("Runs", function() app.log("runs", runs) end) })
        return { name = "Reactive" }"#;
    let mut w = load_with(src, grants).must("loads");
    let n = render(&mut w).must("renders");
    for b in ["A", "Same", "Both", "Runs"] {
        w.act(button(&n, b).must(b)).must(b);
    }
    let got = calls.lock().map(|l| l.clone()).unwrap_or_default();
    assert_eq!(
        got,
        [
            "\"sum\",3",
            "\"peek\",1",
            "\"sum\",12",
            "\"sum\",50", // two sets in one handler: the effect runs once, after it
            "\"runs\",3",
        ]
    );
}

#[test]
fn motion_timers_and_frame_hooks_run_on_the_frame_loop() {
    let grants = Grants::none().element("dot").theme(json!({ "color": { "accent": "#5865f2" } }));
    let mut w = load_with(PULSE, grants).must("pulse loads");
    let dot = |n: &Node| -> (f64, String) {
        let Some(Node::Element { props, .. }) = children(n).first() else { unreachable!("{n:?}") };
        let size = props.get("size").and_then(Value::as_f64).must("a size");
        let color = props.get("color").and_then(Value::as_str).must("a color").to_owned();
        (size, color)
    };
    let n = render(&mut w).must("renders");
    assert_eq!(dot(&n), (8.0, "#5865f2".to_owned()));
    // A timer is waiting: the next frame is when it is due, not every frame.
    assert_eq!(w.next_frame(), Some(Duration::from_secs(1)));
    assert!(!w.advance(Duration::from_millis(500)).must("half a second"), "nothing changed yet");
    assert!(w.advance(Duration::from_millis(500)).must("a second"), "the clock ticked");
    // Now the dot moves: frames every 16 ms until it settles.
    assert_eq!(w.next_frame(), Some(Duration::from_millis(16)));
    let mut sizes = Vec::new();
    for _ in 0..10 {
        w.advance(Duration::from_millis(16)).must("a frame");
        sizes.push(dot(&render(&mut w).must("renders")).0);
    }
    assert!(
        sizes.windows(2).all(|p| matches!(p, [a, b] if b >= a)) && sizes.last() > Some(&8.0),
        "{sizes:?}"
    );
    let n = render(&mut w).must("renders");
    assert_eq!(children(&n).get(1).map(text_of).as_deref(), Some("1 s"));
    // The theme changes: what reads its tokens follows.
    w.set_theme(&json!({ "color": { "accent": "#ff0000" } })).must("new theme");
    assert_eq!(dot(&render(&mut w).must("renders")).1, "#ff0000");
    // Between ticks the spring settles, and then only the timer asks for frames.
    let mut frames = 0;
    while w.next_frame() == Some(Duration::from_millis(16)) && frames < 200 {
        w.advance(Duration::from_millis(16)).must("frames");
        frames += 1;
    }
    let next = w.next_frame().must("the timer still waits");
    assert!(next <= Duration::from_secs(1) && next > Duration::from_millis(16), "{next:?}");
}

#[test]
fn timers_stop_and_frame_hooks_get_the_time() {
    let src = r#"
        local log = ui.signal("")
        local function say(s) log:update(function(l) return l .. s end) end
        local once = ui.after(100, function() say("a") end)
        local cancelled = ui.after(100, function() say("x") end)
        cancelled:stop()
        local n = 0
        local tick
        tick = ui.every(50, function() n = n + 1 say("t") if n == 3 then tick:stop() end end)
        local frames = 0
        local hook
        hook = ui.on_frame(function(ms) frames = frames + ms if frames >= 48 then hook:stop() say("f") end end)
        ui.mount(ui.text(log))
        return { name = "Timers" }"#;
    let mut w = load(src).must("loads");
    render(&mut w).must("renders");
    for _ in 0..20 {
        w.advance(Duration::from_millis(16)).must("frame");
    }
    let n = render(&mut w).must("renders");
    let t = text_of(&n);
    assert_eq!(t.matches('t').count(), 3, "{t}");
    assert_eq!(t.matches('a').count(), 1, "{t}");
    assert_eq!(t.matches('f').count(), 1, "{t}");
    assert!(!t.contains('x'), "{t}");
    assert_eq!(w.next_frame(), None, "nothing waits any more");
}

#[test]
fn mount_and_unmount_hooks_run_once_each() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = calls.clone();
    let grants = Grants::none().function("log", move |args| {
        if let Ok(mut l) = log.lock() {
            l.push(args.first().and_then(Value::as_str).unwrap_or_default().to_owned());
        }
        Ok(Value::Null)
    });
    let src = r#"
        ui.on_mount(function() app.log("mounted") end)
        ui.on_unmount(function() app.log("unmounted") end)
        ui.mount(ui.text("hi"))
        return { name = "Hooks" }"#;
    let mut w = load_with(src, grants).must("loads");
    render(&mut w).must("renders");
    render(&mut w).must("renders again");
    drop(w);
    assert_eq!(calls.lock().map(|l| l.clone()).unwrap_or_default(), ["mounted", "unmounted"]);
}

#[test]
fn app_elements_and_values_from_handlers() {
    let grants = Grants::none().element("knob");
    let src = r#"
        local at = ui.signal(0)
        ui.mount(ui.column{ ui.element("knob", { turned = at, on_turn = function(v) at:set(v) end }) })
        return { name = "Knob" }"#;
    let mut w = load_with(src, grants).must("loads");
    let n = render(&mut w).must("renders");
    let Some(Node::Element { handlers, .. }) = children(&n).first() else { unreachable!("{n:?}") };
    let (name, action) = handlers.first().must("a handler");
    assert_eq!(name, "on_turn");
    w.act_with(*action, &json!(42)).must("turns");
    let n = render(&mut w).must("renders");
    let Some(Node::Element { props, .. }) = children(&n).first() else { unreachable!("{n:?}") };
    assert_eq!(props.get("turned"), Some(&json!(42)));
    // An element the app does not offer is refused.
    let r = load("ui.mount(ui.element('missile', {})) return { name = 'x' }");
    assert!(matches!(r, Err(ScriptError::Invalid(ref m)) if m.contains("missile")), "{r:?}");
}

// ---- hostile scripts ----

/// A widget whose render runs `body` first.
fn widget_running(body: &str) -> String {
    format!(
        "ui.mount(ui.text('ok')) return {{ name = 'probe', render = function() {body} return ui.text('ok') end }}"
    )
}

/// What a hostile script's stop must look like.
type Expect = fn(&ScriptError) -> bool;

#[test]
fn runaway_scripts_are_stopped_and_the_widget_keeps_working() {
    let cases: &[(&str, Expect)] = &[
        ("while true do end", |e| *e == ScriptError::OutOfFuel),
        ("local function f() return f() + 1 end f()", |e| *e == ScriptError::TooDeep),
        ("local t = {} while true do t[#t + 1] = ('x'):rep(1000) end", |e| {
            matches!(e, ScriptError::TooBig | ScriptError::OutOfFuel)
        }),
        ("local s = 'x' while true do s = s .. s end", |e| *e == ScriptError::TooBig),
        (
            "ui.effect(function() local x = ui.signal(0) ui.effect(function() x:set(x:get() + 1) end) end)",
            |e| matches!(e, ScriptError::OutOfFuel | ScriptError::Runtime(_) | ScriptError::TooBig),
        ),
        ("local s = ui.signal(0) ui.effect(function() s:set(s:get() + 1) end)", |e| {
            matches!(e, ScriptError::Runtime(m) if m.contains("kept changing"))
                || *e == ScriptError::OutOfFuel
        }),
        (
            "local c c = ui.computed(function() return c:get() end) c:get()",
            |e| matches!(e, ScriptError::Runtime(m) if m.contains("reads itself")),
        ),
        (
            "local prev = ui.signal(1) for i = 1, 5000 do local p = prev prev = ui.computed(function() return p:get() end) end prev:get()",
            |e| matches!(e, ScriptError::TooDeep | ScriptError::OutOfFuel),
        ),
        ("for i = 1, 100000 do ui.text('x') end", |e| {
            matches!(e, ScriptError::Runtime(m) if m.contains("too many"))
                || *e == ScriptError::TooBig
                || *e == ScriptError::OutOfFuel
        }),
        (
            "for i = 1, 5000 do ui.every(1, function() end) end",
            |e| matches!(e, ScriptError::Runtime(m) if m.contains("too many timers")),
        ),
        ("coroutine.wrap(function() while true do coroutine.yield() end end)() while true do end", |e| {
            *e == ScriptError::OutOfFuel
        }),
    ];
    for (body, ok) in cases {
        // On a thread with the stack a UI thread gets on Windows: nothing overflows it.
        let src = widget_running(body);
        let r = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let mut w = load(&src).must("loads");
                let first = render(&mut w);
                // After a stop, the same widget renders again (and stops again the same way).
                let again = render(&mut w);
                (first, again, w.memory_used())
            })
            .must("thread")
            .join()
            .must("no panic");
        assert!(r.0.as_ref().err().is_some_and(ok), "{body}: {:?}", r.0);
        assert!(r.1.as_ref().err().is_some_and(ok), "{body} again: {:?}", r.1);
        assert!(r.2 <= limits().memory, "{body}: {} bytes held", r.2);
    }
}

#[test]
fn scripts_reach_only_what_is_granted() {
    let denied = [
        ("io.open('/etc/passwd')", "files"),
        ("os.execute('calc')", "operating system"),
        ("require('socket')", "require"),
        ("load('return 1')()", "load"),
        ("debug.getinfo(1)", "debug"),
        ("print('hi')", "print"),
        ("collectgarbage()", "collector"),
        ("dofile('x.lua')", "dofile"),
        ("app.secret()", "app"),
    ];
    for (code, what) in denied {
        let mut w = load(&widget_running(code)).must("loads");
        let r = render(&mut w);
        let ok = match &r {
            Err(ScriptError::Denied(m)) => m.contains(what) || what == "app",
            Err(ScriptError::Runtime(m)) => what == "app" && m.contains("app"),
            _ => false,
        };
        assert!(ok, "{code}: {r:?}");
    }
    // The handles' metatables are locked, and the theme a widget reads is its own copy.
    let src = widget_running(
        "local t = ui.text('x') assert(getmetatable(t) == 'locked') \
         assert(not pcall(setmetatable, t, {})) assert(not pcall(string.dump, print or tostring))",
    );
    let mut w = load(&src).must("loads");
    render(&mut w).must("the probes pass");
    // A granted function is there, and only that one.
    let grants = Grants::none().function("now", |_| Ok(json!(7)));
    let mut w =
        load_with(&widget_running("assert(app.now() == 7) assert(app.other == nil)"), grants).must("loads");
    render(&mut w).must("the app's function works");
}

#[test]
fn handles_to_removed_things_fail_safely() {
    let src = r#"
        local kept, signal, eff = nil, ui.signal(1), nil
        local box = ui.column{}
        ui.mount(box)
        return {
          name = "Stale",
          render = function()
            local results = {}
            local t = ui.text("gone soon")
            box:append(t)
            t:remove()
            results[#results + 1] = tostring(pcall(t.set, t, "text", "x"))
            results[#results + 1] = tostring(pcall(box.append, box, t))
            results[#results + 1] = tostring(t:alive())
            -- An element cannot go inside itself or its own child.
            local outer = ui.column{} local inner = ui.column{} outer:append(inner)
            results[#results + 1] = tostring(pcall(inner.append, inner, outer))
            results[#results + 1] = tostring(pcall(outer.append, outer, outer))
            -- A handle of one kind is not another.
            results[#results + 1] = tostring(pcall(signal.set, t, 3))
            results[#results + 1] = tostring(pcall(box.append, box, signal))
            -- Elements kept from the last render are gone now.
            if kept then results[#results + 1] = "kept " .. tostring(kept:alive()) end
            kept = ui.text("transient")
            return ui.text(table.concat(results, " "))
          end,
        }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    assert_eq!(text_of(&n), "false false false false false false false");
    let n = render(&mut w).must("renders again");
    assert_eq!(text_of(&n), "false false false false false false false kept false");
    // A button from an old description that is gone is said to be gone.
    assert!(matches!(w.act(Action(99)), Err(ScriptError::Shape(_))));
}

#[test]
fn reentrant_callbacks_cannot_break_the_world() {
    let src = r#"
        local s = ui.signal(0)
        local log = {}
        -- An effect that makes effects, timers and elements, and stops itself.
        local e
        e = ui.effect(function()
          local v = s:get()
          log[#log + 1] = v
          if v > 0 then ui.after(0, function() s:set(v + 1) end) end
          if v >= 3 and e then e:stop() end
          ui.text("made in an effect")
        end)
        ui.mount(ui.column{ ui.text(ui.computed(function() return table.concat(log, ",") .. "|" .. s:get() end)),
                            ui.button("Go", function() s:set(1) ui.untrack(function() s:set(2) end) end) })
        return { name = "Reentrant" }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    w.act(button(&n, "Go").must("go")).must("goes");
    for _ in 0..10 {
        w.advance(Duration::from_millis(16)).must("frames");
    }
    let n = render(&mut w).must("renders");
    let t = children(&n).first().map(text_of).must("text");
    assert!(t.starts_with("0,2,3"), "{t}");
}

#[test]
fn bad_widgets_are_refused_plainly() {
    for (src, check) in [
        ("return 5", "manifest"),
        ("return { name = 'x' }", "shows nothing"),
        ("syntax error here", ""),
        ("return { name = 'x', render = 3 }", "render must be a function"),
        ("return { name = 'x', render = function() return {} end }", "element made with ui"),
    ] {
        let r = load(src).and_then(|mut w| render(&mut w).map(|_| ()));
        let msg = match &r {
            Err(ScriptError::Invalid(m) | ScriptError::Runtime(m) | ScriptError::Shape(m)) => m.clone(),
            other => format!("{other:?}"),
        };
        assert!(r.is_err() && msg.contains(check), "{src}: {r:?}");
    }
}

#[test]
fn callbacks_that_rearm_or_fail_do_not_loop_or_break_the_widget() {
    let src = r#"
        local n = ui.signal(0)
        local function again() n:update(function(x) return x + 1 end) ui.after(0, again) end
        ui.after(0, again)
        local fails = ui.signal(false)
        ui.on_frame(function() if fails:peek() then error("frame hook failed") end end)
        -- Elements nest only as deep as the host allows.
        local deep = ui.column{}
        local ok = pcall(function() for i = 1, 200 do local c = ui.column{} c:append(deep) deep = c end end)
        ui.mount(ui.column{ ui.text(n), ui.text(tostring(ok)),
                            ui.button("Break", function() fails:set(true) end),
                            ui.button("Fix", function() fails:set(false) end),
                            ui.button("Throw", function() error("handler failed") end) })
        return { name = "Rearm" }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    assert_eq!(children(&n).get(1).map(text_of).as_deref(), Some("false"), "nesting was refused");
    // A timer that sets itself again runs once a frame, never in a loop within one.
    for _ in 0..5 {
        w.advance(Duration::from_millis(16)).must("frame");
    }
    assert_eq!(children(&render(&mut w).must("renders")).first().map(text_of).as_deref(), Some("5"));
    // A failing handler or frame hook stops that call only.
    let r = w.act(button(&n, "Throw").must("throw"));
    assert!(matches!(r, Err(ScriptError::Runtime(ref m)) if m.contains("handler failed")), "{r:?}");
    w.act(button(&n, "Break").must("break")).must("breaks");
    let r = w.advance(Duration::from_millis(16));
    assert!(matches!(r, Err(ScriptError::Runtime(ref m)) if m.contains("frame hook failed")), "{r:?}");
    let n = render(&mut w).must("still renders");
    w.act(button(&n, "Fix").must("fix")).must("fixes");
    w.advance(Duration::from_millis(16)).must("frames again");
}

#[test]
fn reduced_motion_makes_animated_values_jump() {
    let src = r#"
        local x = ui.animated(0)
        ui.mount(ui.column{ ui.element("dot", { size = x }), ui.button("Go", function() x:to(10, { duration = 500 }) end) })
        return { name = "Jump" }"#;
    let size = |w: &mut LuaWidget| -> f64 {
        let n = render(w).must("renders");
        let Some(Node::Element { props, .. }) = children(&n).first() else { unreachable!("{n:?}") };
        props.get("size").and_then(Value::as_f64).must("size")
    };
    let mut calm = load_with(src, Grants::none().element("dot").reduced_motion(true)).must("loads");
    let n = render(&mut calm).must("renders");
    calm.act(button(&n, "Go").must("go")).must("goes");
    assert_eq!(size(&mut calm), 10.0);
    assert_eq!(calm.next_frame(), None);
    let mut moving = load_with(src, Grants::none().element("dot")).must("loads");
    let n = render(&mut moving).must("renders");
    moving.act(button(&n, "Go").must("go")).must("goes");
    moving.advance(Duration::from_millis(100)).must("a frame");
    let s = size(&mut moving);
    assert!(s > 0.0 && s < 10.0, "{s}");
}

/// With the default limits (a time budget, turned into fuel at this device's rate, or the
/// time limit alone until the rate is measured), an endless loop is still stopped.
#[test]
fn the_default_time_budget_stops_endless_loops() {
    let started = std::time::Instant::now();
    let mut w =
        LuaEngine::new().load_source(&widget_running("while true do end"), Limits::default()).must("loads");

    let r = render(&mut w);
    assert!(matches!(r, Err(ScriptError::OutOfFuel | ScriptError::TooSlow)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}

/// What a render function makes (elements, values, effects, timers, frame hooks) belongs to
/// that render: thousands of renders hold no more than one.
#[test]
fn what_a_render_makes_is_released_by_the_next() {
    let src = r#"
        return { name = "Churn", render = function()
          local v = ui.signal(1)
          local c = ui.computed(function() return v:get() + 1 end)
          ui.effect(function() local _ = c:get() end)
          ui.after(1000, function() end)
          ui.on_frame(function() end)
          return ui.column{ ui.text(c), ui.text("x") }
        end }"#;
    let limits = limits().memory(1 << 20);
    let mut w = LuaEngine::new().load_source(src, limits).must("loads");
    for i in 0..5000 {
        let n = render(&mut w).unwrap_or_else(|e| unreachable!("render {i}: {e}"));
        assert_eq!(children(&n).first().map(text_of).as_deref(), Some("2"));
    }
    assert!(w.elements() <= 3, "{}", w.elements());
    assert_eq!(w.next_frame(), Some(Duration::from_millis(16)), "one frame hook, the last render's");
}

/// A kept tree changed for a long time holds what it shows, not all it ever made: thousands
/// of elements added and removed under a 1 MiB cap.
#[test]
fn a_kept_tree_churned_for_long_stays_within_its_memory() {
    let src = r#"
        local list = ui.column{}
        local n = 0
        ui.mount(ui.column{ list, ui.button("Churn", function()
          for i = 1, 10 do
            n = n + 1
            local row = ui.row{ ui.text("item " .. n), ui.button("x", function() end) }
            list:append(row)
          end
          for _, c in ipairs(list:children()) do c:remove() end
        end) })
        return { name = "Churn" }"#;
    let mut w = LuaEngine::new().load_source(src, limits().memory(1 << 20)).must("loads");
    let n = render(&mut w).must("renders");
    let churn = button(&n, "Churn").must("churn");
    for i in 0..1000 {
        w.act(churn).unwrap_or_else(|e| unreachable!("round {i}: {e}"));
        let _ = render(&mut w).must("renders");
    }
    assert_eq!(w.elements(), 3, "the two columns and the button");
}

/// The time limit stops a call by the clock, whatever its fuel; a watchdog on another thread
/// can stop a running call with the widget's interrupt; and the next call runs normally.
#[test]
fn time_limits_and_interrupts_stop_a_call_and_only_that_call() {
    let src = r#"
        return { name = "Slow", render = function(settings, host)
          if host.spin then while true do end end
          return ui.text("ok")
        end }"#;
    let lots = Limits::default().operations(Some(u64::MAX >> 8)).time(Duration::from_millis(50));
    let mut w = LuaEngine::new().load_source(src, lots).must("loads");
    let started = std::time::Instant::now();
    assert_eq!(w.render(&Map::new(), &json!({ "spin": true })), Err(ScriptError::TooSlow));
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert!(w.render(&Map::new(), &json!({})).is_ok());
    let lots = Limits::default().operations(Some(u64::MAX >> 8)).time(Duration::from_secs(60));
    let mut w = LuaEngine::new().load_source(src, lots).must("loads");
    let stop = w.interrupt_handle();
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        stop.interrupt();
    });
    let started = std::time::Instant::now();
    assert_eq!(w.render(&Map::new(), &json!({ "spin": true })), Err(ScriptError::TooSlow));
    let _ = watchdog.join();
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert!(w.render(&Map::new(), &json!({})).is_ok(), "the next call runs");
}

/// A watchdog's stop set between calls is not swallowed: it stops the next call, and is spent.
#[test]
fn a_stop_set_between_calls_stops_the_next_call() {
    let src = r#"return { name = "Quick", render = function() return ui.text("ok") end }"#;
    let mut w = LuaEngine::new().load_source(src, limits()).must("loads");
    assert!(render(&mut w).is_ok());
    w.interrupt_handle().interrupt();
    assert_eq!(render(&mut w).err(), Some(ScriptError::TooSlow), "the stop is not cleared unseen");
    assert!(render(&mut w).is_ok(), "the stop was spent on the call it stopped");
}

/// A panic in a host function poisons the state: the widget is stopped for good, not called again.
#[test]
fn a_poisoned_state_stops_the_widget_for_good() {
    let grants = Grants::none().function("boom", |_| panic!("host bug"));
    let src = r#"return { name = "Boom", render = function(_, host)
        if host.boom then app.boom() end
        return ui.text("ok") end }"#;
    let mut w = load_with(src, grants).must("loads");
    assert!(render(&mut w).is_ok());
    let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        w.render(&Map::new(), &json!({ "boom": true }))
    }));
    assert!(hit.is_err(), "the host function's panic reaches the host");
    for _ in 0..2 {
        assert!(matches!(render(&mut w), Err(ScriptError::Runtime(m)) if m.contains("for good")));
    }
}

/// The source-derived hash seed survives the limits being set again before every call.
#[test]
fn the_hash_seed_is_kept_across_calls() {
    let src = r#"return { name = "Keys", render = function()
        local t, out = {}, {}
        for i = 1, 40 do t["k" .. i] = i end
        for k in pairs(t) do out[#out + 1] = k end
        return ui.text(table.concat(out, ","))
      end }"#;
    let order = |w: &mut LuaWidget| format!("{:?}", render(w));
    let mut a = LuaEngine::new().load_source(src, limits()).must("loads");
    let mut b = LuaEngine::new().load_source(src, limits()).must("loads");
    let first = order(&mut a);
    for _ in 0..3 {
        assert_eq!(order(&mut a), first, "same state, same seed, every call");
    }
    assert_eq!(order(&mut b), first, "the same source gets the same seed");
}

#[test]
fn unmount_runs_the_hooks_once_and_dropping_does_not_repeat_them() {
    let calls = Arc::new(Mutex::new(0u32));
    let count = calls.clone();
    let grants = Grants::none().function("bye", move |_| {
        if let Ok(mut c) = count.lock() {
            *c += 1;
        }
        Ok(Value::Null)
    });
    let src = "ui.on_unmount(function() app.bye() end) ui.mount(ui.text('x')) return { name = 'Bye' }";
    let mut w = load_with(src, grants).must("loads");
    render(&mut w).must("renders");
    w.unmount();
    w.unmount();
    drop(w);
    assert_eq!(calls.lock().map(|c| *c).unwrap_or_default(), 1);
}

/// Elements and values scripts let go of without removing them are released once the
/// collector frees their last handle (so a widget that makes and drops them forever stays
/// within its caps); elements shown, and ones a script still holds, are kept.
#[test]
fn what_scripts_let_go_of_is_released() {
    let src = r#"
        local shown = ui.column{ ui.text("kept") }
        local held
        local function churn()
          for i = 1, 300 do
            local loose = ui.column{ ui.text("a"), ui.text("b") }
            local v = ui.signal(i)
            if i == 300 then held = loose:children()[1] end
          end
        end
        shown:append(ui.button("Churn", churn), ui.button("Held", function()
          assert(held:alive() and held:kind() == "text")
        end))
        ui.mount(shown)
        return { name = "Drops" }"#;
    // A small widget: without releasing, the churn would pass its element cap many times over.
    let mut w = LuaEngine::new().load_source(src, limits().memory(1 << 20)).must("loads");
    let n = render(&mut w).must("renders");
    let (churn, held) = (button(&n, "Churn").must("churn"), button(&n, "Held").must("held"));
    for i in 0..100 {
        w.act(churn).unwrap_or_else(|e| unreachable!("round {i}: {e}"));
        w.act(held).unwrap_or_else(|e| unreachable!("held, round {i}: {e}"));
        render(&mut w).must("renders");
    }
    assert!(w.elements() < 4096, "{}", w.elements());
    let n = render(&mut w).must("renders");
    assert_eq!(children(&n).first().map(text_of).as_deref(), Some("kept"));
}

/// A computed value whose function failed worked nothing out: the next read runs it again
/// instead of handing back what it held before.
#[test]
fn a_computed_value_that_failed_is_worked_out_again() {
    let src = r#"
        local fail, runs = true, 0
        local c = ui.computed(function() runs = runs + 1 if fail then error("not yet") end return "ready" end)
        local out = ui.signal("")
        ui.mount(ui.column{ ui.text(out),
          ui.button("Read", function() out:set(tostring(pcall(c.get, c)) .. " " .. runs) end),
          ui.button("Fix", function() fail = false end) })
        return { name = "Retry" }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    let (read, fix) = (button(&n, "Read").must("read"), button(&n, "Fix").must("fix"));
    let mut shown = Vec::new();
    for b in [read, read, fix, read] {
        w.act(b).must("acts");
        shown.push(children(&render(&mut w).must("renders")).first().map(text_of).must("text"));
    }
    assert_eq!(shown, ["false 1", "false 2", "false 2", "true 3"]);
}

/// What a call returned (the manifest, the element a render made) outlives the effects and
/// hooks that run after it, however much they allocate.
#[test]
fn what_a_call_returned_survives_the_effects_and_hooks_after_it() {
    let src = r#"
        local function churn() local t = {} for i = 1, 20000 do t[i % 64] = { i } end end
        local s = ui.signal(0)
        ui.effect(function() local _ = s:get() churn() end)
        s:set(1)
        ui.on_mount(churn)
        return { name = "Kept", render = function() return ui.text("shown") end }"#;
    let mut w = LuaEngine::new().load_source(src, limits().memory(1 << 20)).must("loads");
    assert_eq!(w.name(), "Kept");
    assert_eq!(text_of(&render(&mut w).must("renders")), "shown");
    assert_eq!(text_of(&render(&mut w).must("renders again")), "shown");
}

#[test]
fn animated_values_take_only_finite_numbers() {
    let src = r#"
        local a = ui.animated(1)
        ui.mount(ui.text(tostring(pcall(a.set, a, 0/0)) .. " " .. tostring(pcall(a.to, a, 1e300))
          .. " " .. tostring(pcall(ui.animated, 1e300)) .. " " .. math.floor(a:get())))
        return { name = "Finite" }"#;
    let mut w = load(src).must("loads");
    assert_eq!(text_of(&render(&mut w).must("renders")), "false false false 1");
}

/// A computed value an element shows is worked out inside the render's call, not while the
/// host describes the tree afterwards: what it sets off (here an effect) runs with that call.
#[test]
fn computed_values_shown_are_worked_out_inside_the_render_call() {
    let src = r#"
        local a = ui.animated(0)
        this.runs = 0
        ui.effect(function() local _ = a:get() this.runs = this.runs + 1 end)
        local c = ui.computed(function() a:set(1) return "c" end)
        return { name = "Inside", render = function()
          return ui.column{ ui.text(c), ui.text(tostring(this.runs)) }
        end }"#;
    let mut w = load(src).must("loads");
    let n = render(&mut w).must("renders");
    assert_eq!(children(&n).first().map(text_of).as_deref(), Some("c"));
    let n = render(&mut w).must("renders again");
    assert_eq!(
        children(&n).get(1).map(text_of).as_deref(),
        Some("2"),
        "the effect ran with the first render"
    );
}
