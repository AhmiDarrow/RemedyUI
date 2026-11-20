//! The reference scene for ambient-motion budgets: a detailed backdrop, `n` animated sprites
//! (fireflies wandering in their own boxes, each flickering) and a lighthouse beam sweeping the
//! whole window. Shared by `tests/ambient_budget.rs` (headless) and
//! `examples/ambient_bench.rs` (a real window).
#![allow(dead_code)]

use std::f32::consts::TAU;
use std::time::Duration;

use remedy_ui::core::{Color, CornerRadii, Insets, Point, Rect, Vec2};
use remedy_ui::layout::{Length, Position};
use remedy_ui::paint::{Blend, Brush, Item, Path, Stop};
use remedy_ui::view::AmbientLoop;
use remedy_ui::{El, PaintCx, Ui};

pub const WIDTH: f32 = 800.0;
pub const HEIGHT: f32 = 600.0;
pub const SPRITE: f32 = 32.0;

/// How the scene animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    /// The engine's ambient API: a cached backdrop, a small ambient painter per sprite, the
    /// beam as a compositor loop.
    Ambient,
    /// The naive way (before): one window-sized painter that draws everything and asks for
    /// the next frame every frame.
    EveryFrame,
}

fn absolute(el: El, x: f32, y: f32, w: f32, h: f32) -> El {
    el.layout(move |l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: y, start: x, bottom: 0.0, end: 0.0 };
        l.width = Length::Px(w);
        l.height = Length::Px(h);
    })
}

fn fill(el: El) -> El {
    el.layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets::ZERO;
        l.width = Length::Percent(1.0);
        l.height = Length::Percent(1.0);
    })
}

fn hash(i: u32) -> f32 {
    let x = i.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277_803_737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}

/// A night sky, 600 stars, three ranges of hills and a lighthouse: expensive enough that
/// drawing it every frame shows.
pub fn backdrop(cx: &mut PaintCx<'_>) {
    let (w, h) = (cx.size.width, cx.size.height);
    let sky = vec![
        Stop { offset: 0.0, color: Color::rgb8(8, 12, 40) },
        Stop { offset: 1.0, color: Color::rgb8(40, 60, 110) },
    ];
    cx.list.fill_rect(
        Rect::new(0.0, 0.0, w, h),
        Brush::Linear { start: Point::ZERO, end: Point::new(0.0, h), stops: sky },
    );
    for i in 0..600u32 {
        let (x, y) = (hash(i * 2) * w, hash(i * 2 + 1) * h * 0.7);
        let r = 0.5 + hash(i * 7) * 1.5;
        cx.list.fill_rounded(
            Rect::new(x - r, y - r, x + r, y + r),
            CornerRadii::all(r),
            Brush::Solid(Color::rgba8(255, 255, 230, 200)),
        );
    }
    for (k, shade) in [(0.0f32, 30u8), (1.0, 22), (2.0, 14)] {
        let mut p = Path::new();
        p.move_to(Point::new(0.0, h));
        for s in 0..=64 {
            let x = w * s as f32 / 64.0;
            let y = h * (0.62 + 0.1 * k) + 30.0 * ((x / w * TAU * (1.5 + k)) + k).sin();
            p.line_to(Point::new(x, y));
        }
        p.line_to(Point::new(w, h)).close();
        cx.list.push(Item::FillPath {
            path: p,
            brush: Brush::Solid(Color::rgb8(shade, shade + 10, shade + 20)),
            even_odd: false,
        });
    }
    cx.list.fill_rect(Rect::new(80.0, 300.0, 110.0, 520.0), Brush::Solid(Color::rgb8(220, 220, 210)));
}

/// One firefly at loop time `t`: wanders in its box and flickers.
pub fn firefly(cx: &mut PaintCx<'_>, i: u32, t: Option<f32>, origin: Vec2) {
    let t = t.unwrap_or(0.0);
    let phase = hash(i) * TAU;
    let x = origin.x + SPRITE / 2.0 + 9.0 * (t * 0.9 + phase).sin();
    let y = origin.y + SPRITE / 2.0 + 9.0 * (t * 1.3 + phase * 1.7).cos();
    let glow = 0.55 + 0.45 * (t * 7.0 + phase).sin().abs();
    let c = Color::rgb8(255, 230, 120);
    let stops =
        vec![Stop { offset: 0.0, color: c.with_alpha(glow) }, Stop { offset: 1.0, color: c.with_alpha(0.0) }];
    let center = Point::new(x, y);
    cx.list.push(Item::PushLayer { opacity: 1.0, blend: Blend::Plus });
    cx.list.fill_rect(
        Rect::new(x - 7.0, y - 7.0, x + 7.0, y + 7.0),
        Brush::Radial { center, radius: 7.0, stops },
    );
    cx.list.push(Item::PopLayer);
}

fn sprite_at(i: u32) -> (f32, f32) {
    let cols = 10;
    (60.0 + (i % cols) as f32 * 70.0, 60.0 + (i / cols) as f32 * 80.0)
}

/// The beam: a soft wedge from the lamp, pointing right.
pub fn beam(cx: &mut PaintCx<'_>, pivot: Point) {
    let mut p = Path::new();
    p.move_to(pivot)
        .line_to(Point::new(pivot.x + 900.0, pivot.y - 120.0))
        .line_to(Point::new(pivot.x + 900.0, pivot.y + 120.0))
        .close();
    let c = Color::rgb8(255, 240, 190);
    let stops =
        vec![Stop { offset: 0.0, color: c.with_alpha(0.5) }, Stop { offset: 1.0, color: c.with_alpha(0.0) }];
    cx.list.push(Item::PushLayer { opacity: 1.0, blend: Blend::Plus });
    cx.list.push(Item::FillPath {
        path: p,
        brush: Brush::Linear { start: pivot, end: Point::new(pivot.x + 900.0, pivot.y), stops },
        even_odd: false,
    });
    cx.list.push(Item::PopLayer);
}

pub const PIVOT: Point = Point { x: 95.0, y: 300.0 };
pub const BEAM_PERIOD: Duration = Duration::from_secs(8);

/// Builds the scene with `sprites` fireflies (and a beam when `with_beam`) into `ui`.
pub fn scene(ui: &Ui, drive: Drive, sprites: u32, with_beam: bool) -> El {
    let stage = fill(ui.el("stage"));
    match drive {
        Drive::Ambient => {
            let back = fill(ui.el("backdrop")).paint(|cx| cx.cached(0xbac6_d0b5, backdrop));
            let mut stage = stage.child(back);
            for i in 0..sprites {
                let (x, y) = sprite_at(i);
                let fly = absolute(ui.el("firefly"), x, y, SPRITE, SPRITE).paint(move |cx| {
                    let t = cx.ambient().map(|a| a.t.as_secs_f32());
                    firefly(cx, i, t, Vec2::ZERO);
                });
                stage = stage.child(fly);
            }
            if with_beam {
                let b = fill(ui.el("beam"))
                    .pass_through()
                    .paint(|cx| beam(cx, PIVOT))
                    .ambient_loop(AmbientLoop::spin(PIVOT, BEAM_PERIOD));
                stage = stage.child(b);
            }
            stage
        }
        Drive::EveryFrame => {
            let all = fill(ui.el("everything")).paint(move |cx| {
                let t = if cx.reduced_motion { 0.0 } else { cx.time.as_secs_f32() };
                backdrop(cx);
                for i in 0..sprites {
                    let (x, y) = sprite_at(i);
                    firefly(cx, i, Some(t), Vec2::new(x, y));
                }
                if with_beam {
                    let turn = TAU * (t / BEAM_PERIOD.as_secs_f32()).fract();
                    let about = remedy_ui::paint::Affine::translate(Vec2::new(-PIVOT.x, -PIVOT.y))
                        .then(remedy_ui::paint::Affine::rotate(turn))
                        .then(remedy_ui::paint::Affine::translate(PIVOT.to_vec2()));
                    cx.list.push(Item::PushTransform(about));
                    beam(cx, PIVOT);
                    cx.list.push(Item::PopTransform);
                }
                if !cx.reduced_motion {
                    cx.request_next_frame();
                }
            });
            stage.child(all)
        }
    }
}
