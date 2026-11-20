//! The stage and sprites on a real window: placement as the box resizes, pixels, animation
//! that stops when it should, and the accessibility audit.

mod common;

use common::*;
use remedy_ui_core::{Color, Rect, Size};
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_style::ColorRole;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_stage::{
    Anchor, Fit, Ink, Layer, PixelArt, PixelScale, Place, SceneOptions, SpriteOptions, scene, sprite,
};

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
    let _ = frame(ui);
}

fn chip(ui: &Ui, w: f32, h: f32) -> El {
    ui.el("chip").width(w).height(h)
}

fn near_rect(a: Rect, b: Rect) -> bool {
    (a.min.x - b.min.x).abs() < 0.6
        && (a.min.y - b.min.y).abs() < 0.6
        && (a.max.x - b.max.x).abs() < 0.6
        && (a.max.y - b.max.y).abs() < 0.6
}

#[test]
fn children_stay_on_their_anchor_as_the_stage_resizes() {
    let ui = ui();
    let w = Signal::new(200.0f32);
    let point = chip(&ui, 20.0, 10.0);
    let filler = ui.el("chip");
    let stage = scene(&ui, "Harbor", Size::new(100.0, 50.0), SceneOptions::default())
        .anchor("pier", Anchor::point(50.0, 25.0))
        .anchor("dock", Anchor::rect(0.0, 40.0, 100.0, 10.0))
        .place("pier", point.clone())
        .place("dock", filler.clone());
    let el: El = stage.clone().into();
    let el = el.width(200.0).height(100.0);
    mount(&ui, &el);
    let b = el.bounds().unwrap_or_default();
    // Scale 2: the pier is at (100, 50) in the box; the chip keeps its own size.
    assert!(near_rect(point.bounds().unwrap_or_default(), Rect::new(90.0, 45.0, 110.0, 55.0).translate(b.min.to_vec2())));
    // The dock fills its scaled rectangle.
    assert!(near_rect(filler.bounds().unwrap_or_default(), Rect::new(0.0, 80.0, 200.0, 100.0).translate(b.min.to_vec2())));

    // A wider box letterboxes: scale stays 2, the design is centered.
    let _ = el.clone().width(300.0);
    let _ = (frame(&ui), frame(&ui));
    assert_eq!(stage.transform().offset.x, 50.0);
    assert!(near_rect(point.bounds().unwrap_or_default(), Rect::new(140.0, 45.0, 160.0, 55.0).translate(b.min.to_vec2())));
    let _ = w;

    // Moving the anchor moves the child.
    stage.set_anchor("pier", Anchor::point(0.0, 0.0).place(Place::Start, Place::Start));
    let _ = (frame(&ui), frame(&ui));
    assert!(near_rect(point.bounds().unwrap_or_default(), Rect::new(50.0, 0.0, 70.0, 10.0).translate(b.min.to_vec2())));
    assert!(audit(&ui).is_empty(), "{:?}", audit(&ui));
}

#[test]
fn pixel_perfect_cover_and_stretch_fit() {
    use remedy_ui_widgets_stage::StageTransform;
    let d = Size::new(320.0, 180.0);
    let t = StageTransform::fit(d, Size::new(700.0, 500.0), Fit::Contain, true);
    assert_eq!(t.scale_x, 2.0);
    let t = StageTransform::fit(d, Size::new(700.0, 500.0), Fit::Cover, false);
    assert!((t.scale_x - 500.0 / 180.0).abs() < 1e-4);
    let t = StageTransform::fit(d, Size::new(640.0, 90.0), Fit::Stretch, false);
    assert_eq!((t.scale_x, t.scale_y), (2.0, 0.5));
    let p = t.to_design(t.to_screen(remedy_ui_core::Point::new(10.0, 20.0)));
    assert!((p.x - 10.0).abs() < 1e-4 && (p.y - 20.0).abs() < 1e-4);
    // Smaller than the design still shrinks smoothly.
    let t = StageTransform::fit(d, Size::new(160.0, 90.0), Fit::Contain, true);
    assert_eq!(t.scale_x, 0.5);
}

#[test]
fn layers_draw_in_design_coordinates() {
    let ui = ui();
    let stage = scene(&ui, "", Size::new(100.0, 100.0), SceneOptions::default()).layer(Layer::paint(|cx| {
        let c = cx.theme.colors.accent;
        cx.list.fill_rect(Rect::new(50.0, 50.0, 100.0, 100.0), Brush::Solid(c));
    }));
    let el: El = stage.into();
    let el = el.width(200.0).height(200.0);
    let _ = ui.root().child(el.clone());
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    shot.take(&ui);
    let b = el.bounds().unwrap_or_default();
    let accent = rgb(ui.theme().colors.accent);
    assert!(near(shot.at(b.min.x as u32 + 150, b.min.y as u32 + 150), accent, 3));
    assert!(!near(shot.at(b.min.x as u32 + 50, b.min.y as u32 + 50), accent, 3));
}

fn blinker() -> PixelArt {
    PixelArt::new([('#', Ink::from(ColorRole::Accent)), ('r', Ink::Color(Color::rgb8(255, 0, 0)))])
        .state("idle", &["##", "##"])
        .animation("speaking", &[&["##", "##"], &["rr", "rr"]], 10.0)
}

#[test]
fn sprites_are_crisp_and_change_state() {
    let ui = ui();
    let state = Signal::new("idle".to_owned());
    let clock = remedy_ui_viz::Clock::manual();
    let el = sprite(&ui, "Alex", blinker(), state, SpriteOptions::default().scale(PixelScale::Fixed(8)).clock(clock.clone()));
    mount(&ui, &el);
    let b = el.bounds().unwrap_or_default();
    assert_eq!(b.size(), Size::new(16.0, 16.0));
    let mut shot = Shot::new(&ui);
    shot.take(&ui);
    let accent = rgb(ui.theme().colors.accent);
    let (x, y) = (b.min.x as u32, b.min.y as u32);
    assert_eq!(shot.at(x, y), accent, "edge pixel is the art's color: crisp");
    assert_eq!(shot.at(x + 15, y + 15), accent);
    assert_eq!(pending_timers(&ui, &clock), 0, "a still sprite schedules nothing");

    state.set("speaking".to_owned());
    let _ = frame(&ui);
    pump(&ui, &clock, 120);
    shot.take(&ui);
    let red = shot.at(x + 4, y + 4) == [255, 0, 0];
    pump(&ui, &clock, 100);
    shot.take(&ui);
    let red2 = shot.at(x + 4, y + 4) == [255, 0, 0];
    assert_ne!(red, red2, "frames alternate");

    // Reduced motion holds the first frame and stops the timer.
    ui.set_reduced_motion(true);
    pump(&ui, &clock, 200);
    shot.take(&ui);
    assert_eq!(shot.at(x + 4, y + 4), accent);
    assert_eq!(pending_timers(&ui, &clock), 0);
    let s = el.info().and_then(|i| i.semantics).map(|s| s.name().map(str::to_owned));
    assert_eq!(s, Some(Some("Alex".into())));
}

#[test]
fn a_hidden_sprite_does_not_animate() {
    let ui = ui();
    let clock = remedy_ui_viz::Clock::manual();
    let el = sprite(&ui, "", blinker(), "speaking".to_owned(), SpriteOptions::default().clock(clock.clone()));
    let holder = ui.el("holder").child(el.clone());
    mount(&ui, &holder);
    assert!(pump(&ui, &clock, 100) > 0, "animates while seen");
    let _ = holder.clone().hidden(true);
    let _ = frame(&ui);
    pump(&ui, &clock, 100);
    assert_eq!(pending_timers(&ui, &clock), 0, "stops when hidden");
}
