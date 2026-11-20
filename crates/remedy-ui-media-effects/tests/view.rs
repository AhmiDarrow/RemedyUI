//! The effect element under the ambient policy, on both renderers.

use std::time::Duration;

use remedy_ui_core::Size;
use remedy_ui_effects::{Effect, Instance, library};
use remedy_ui_media_effects::{Backend, EffectView, effect_view};
use remedy_ui_paint::Item;
use remedy_ui_view::{AmbientMode, AmbientPolicy, Request, Ui};

fn ui() -> Ui {
    Ui::new(
        remedy_ui_style::Theme::dark_forest(),
        remedy_ui_style::StyleSheet::new(),
        remedy_ui_text::FontCollection::new(),
    )
}

thread_local! {
    static LAST: std::cell::RefCell<Option<remedy_ui_compositor::LayerTree>> = const { std::cell::RefCell::new(None) };
    /// Timers asked for, with when they are due (virtual time).
    static TIMERS: std::cell::RefCell<Vec<(u64, Duration)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `frames` frames 1/60 s apart, firing timers; returns the frames that drew anything.
fn run(ui: &Ui, from: &mut Duration, frames: u32) -> u32 {
    let mut drew = 0;
    for _ in 0..frames {
        *from += Duration::from_micros(16_667);
        ui.set_frame_time(*from);
        let now = *from;
        let due: Vec<u64> = TIMERS.with(|t| {
            let mut t = t.borrow_mut();
            let due = t.iter().filter(|(_, at)| *at <= now).map(|(id, _)| *id).collect();
            t.retain(|(_, at)| *at > now);
            due
        });
        for id in due {
            ui.fire_timer(id);
        }
        let f = ui.frame(Size::new(320.0, 200.0), 1.0);
        for r in ui.take_requests() {
            match r {
                Request::Timer { id, delay } => TIMERS.with(|t| t.borrow_mut().push((id, now + delay))),
                Request::CancelTimer(id) => TIMERS.with(|t| t.borrow_mut().retain(|(i, _)| *i != id)),
                _ => {}
            }
        }
        if let Some(l) = f.layers {
            drew += 1;
            LAST.with(|x| *x.borrow_mut() = Some(l));
        }
    }
    drew
}

fn items(_ui: &Ui) -> Vec<Item> {
    let mut out = Vec::new();
    LAST.with(|x| {
        if let Some(tree) = x.borrow().as_ref() {
            tree.for_each(|l| out.extend(l.content.items().iter().cloned()));
        }
    });
    out
}

/// Mounts library effect `name`. Its CPU budget is generous, so tests about frame rates hold
/// in unoptimized builds too (the ladder has its own test).
fn mount(ui: &Ui, name: &str) -> EffectView {
    let Some(e) = library(name) else { unreachable!("{name}") };
    let (el, view) = effect_view(ui, Instance::new(e));
    view.set_cpu_budget(Duration::from_secs(10));
    ui.root().child(el);
    view
}

#[test]
fn frames_follow_the_effects_rate_and_the_policy() {
    let ui = ui();
    // Starfield asks for 15 fps; the window allows 30.
    let view = mount(&ui, "starfield");
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 10);
    let before = view.stats().frames;
    run(&ui, &mut t, 60);
    let per_second = view.stats().frames - before;
    assert!((14..=16).contains(&per_second), "{per_second} frames in a second");
    assert_eq!(view.stats().backend, Backend::Cpu);

    // Off: one still frame, then nothing.
    ui.set_ambient_policy(AmbientPolicy::off());
    run(&ui, &mut t, 5);
    let stills = view.stats().stills;
    let frames = view.stats().frames;
    run(&ui, &mut t, 60);
    assert!(stills >= 1, "a still frame was drawn");
    assert_eq!(view.stats().frames, frames, "nothing more while still");

    // A parameter change draws the still frame again, once.
    assert!(view.set("twinkle", 0.2).is_ok());
    run(&ui, &mut t, 10);
    assert_eq!(view.stats().frames, frames + 1);
}

#[test]
fn reduced_motion_shows_the_still_frame() {
    let ui = ui();
    ui.set_reduced_motion(true);
    let view = mount(&ui, "fire");
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 90);
    assert_eq!(view.stats().frames, 1, "one still frame");
    assert_eq!(view.stats().stills, 1);
    assert!(items(&ui).iter().any(|i| matches!(i, Item::Image { .. })), "the picture is placed");
}

#[test]
fn light_mode_slows_down() {
    let ui = ui();
    let mut p = AmbientPolicy::default();
    p.mode = AmbientMode::Light;
    ui.set_ambient_policy(p);
    let view = mount(&ui, "embers");
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 10);
    let before = view.stats().frames;
    run(&ui, &mut t, 60);
    let n = view.stats().frames - before;
    assert!((14..=16).contains(&n), "{n} frames in a second in Light mode");
}

#[test]
fn a_slow_cpu_steps_down_to_a_still_frame() {
    let ui = ui();
    let view = mount(&ui, "volumetric_clouds");
    view.set_cpu_budget(Duration::from_micros(1));
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 120);
    assert_eq!(view.stats().level, 3, "{:?}", view.stats());
    let frames = view.stats().frames;
    run(&ui, &mut t, 60);
    assert!(view.stats().frames <= frames + 1, "held still");
    // A new size measures again from the top.
    view.set_cpu_budget(Duration::from_secs(10));
    t += Duration::from_micros(16_667);
    ui.set_frame_time(t);
    let _ = ui.frame(Size::new(400.0, 260.0), 1.0);
    assert_eq!(view.stats().level, 0, "{:?}", view.stats());
}

#[test]
fn hostile_parameter_values_are_clamped() {
    let ui = ui();
    let view = mount(&ui, "glow");
    assert!(view.set("radius", f32::INFINITY).is_ok());
    assert!(view.set("radius", -5.0).is_ok());
    assert!(view.set("intensity", 1.0e30).is_ok());
    assert!(view.set("nope", 1.0).is_err());
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 5);
    assert!(view.stats().frames >= 1);
}

#[test]
fn compiled_from_source_with_untrusted_limits() {
    let ui = ui();
    let src =
        "effect stripes(color c = #ff0080) { pass p { out = vec4(c.rgb, step(0.5, fract(px.x / 8.0))); } }";
    let made = remedy_ui_media_effects::effect_from_source(&ui, src, &remedy_ui_effects::Limits::untrusted());
    assert!(made.is_ok());
    let bad = remedy_ui_media_effects::effect_from_source(
        &ui,
        "effect x { pass p { out = nope; } }",
        &remedy_ui_effects::Limits::untrusted(),
    );
    assert!(bad.is_err_and(|e| e.to_string().contains("`nope` is not defined")));
    let _ = Effect::compile("effect y { pass p { out = 1.0; } }");
}

#[test]
fn the_gpu_path_places_a_texture() {
    let ui = ui();
    ui.set_gpu_rendering(true);
    let view = mount(&ui, "nebula");
    let mut t = Duration::ZERO;
    run(&ui, &mut t, 5);
    let s = view.stats();
    // Every machine here has at least a software adapter; without one the CPU draws.
    if s.backend == Backend::Gpu {
        assert!(items(&ui).iter().any(|i| matches!(i, Item::Texture { .. })), "a texture is placed");
        assert!(s.frames >= 1);
    } else {
        assert_eq!(s.backend, Backend::Cpu);
    }
}

#[test]
fn a_slow_gpu_steps_down_resolution_but_keeps_moving() {
    let ui = ui();
    ui.set_gpu_rendering(true);
    let view = mount(&ui, "plasma");
    view.set_gpu_budget(Duration::from_nanos(1));
    let mut t = Duration::ZERO;
    let mut sizes = Vec::new();
    for _ in 0..40 {
        run(&ui, &mut t, 2);
        sizes.push(view.stats().pixels);
    }
    let s = view.stats();
    if s.backend != Backend::Gpu || s.gpu_time.is_none() {
        // No adapter, or one without timestamp queries (GL): nothing measured, nothing changes.
        eprintln!("not measured here: {s:?}");
        assert_eq!(s.level, 0);
        return;
    }
    eprintln!("measured: {s:?}");
    assert_eq!(s.level, 2, "over budget: a quarter of the resolution ({s:?})");
    let (first, last) = (sizes.first().copied().unwrap_or_default(), s.pixels);
    assert!(last.0 * 3 < first.0, "drawn smaller: {first:?} then {last:?}");
    let frames = s.frames;
    run(&ui, &mut t, 20);
    assert!(view.stats().frames > frames, "the GPU never holds an effect still");
    // A generous budget lets it climb back.
    view.set_gpu_budget(Duration::from_secs(10));
    run(&ui, &mut t, 400);
    assert!(view.stats().level < 2, "{:?}", view.stats());
}
