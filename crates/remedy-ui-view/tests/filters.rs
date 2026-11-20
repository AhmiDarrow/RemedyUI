//! Box shadows, filters and frosted glass from style: what an element's style asks for is in
//! what the window draws.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, LayerTree};
use remedy_ui_core::{Color, Size};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_layout::Length;
use remedy_ui_paint::{Filter, Item};
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_style::{Shadow, States, Style, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    Ui::new(Theme::dark_forest(), sheet, FontCollection::new())
}

fn boxed(ui: &Ui, style: Style) -> remedy_ui_view::El {
    ui.el("panel").style(style).layout(|l| {
        l.width = Length::Px(80.0);
        l.height = Length::Px(40.0);
    })
}

fn items(tree: &LayerTree) -> Vec<Item> {
    let mut out = Vec::new();
    tree.for_each(|l| out.extend(l.content.items().iter().cloned()));
    out
}

fn draw(ui: &Ui) -> (LayerTree, Frame) {
    let f = ui.frame(Size::new(200.0, 120.0), 1.0);
    let Some(tree) = f.layers else { unreachable!("a first frame has layers") };
    let mut comp = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(tree.clone());
    let Ok(frame) = CpuCompositor::new().present(&comp.frame(Duration::ZERO), 200, 120, Color::BLACK) else {
        unreachable!("draws")
    };
    (tree, frame)
}

#[test]
fn box_shadows_draw_under_the_box() {
    let ui = ui();
    let shadow = Shadow::new(0.0, 6.0, 8.0, Color::rgba8(0, 0, 0, 200));
    ui.root()
        .padding(20.0)
        .child(boxed(&ui, Style::new().background(Color::rgb8(40, 120, 220)).radius(6.0).shadow(shadow)));
    let (tree, frame) = draw(&ui);
    let all = items(&tree);
    let at = |f: &dyn Fn(&Item) -> bool| all.iter().position(f);
    let s = at(&|i| matches!(i, Item::Shadow { .. }));
    let bg = at(
        &|i| matches!(i, Item::FillRect { .. } if !matches!(i, Item::FillRect { brush: remedy_ui_paint::Brush::Solid(c), .. } if *c == Color::WHITE)),
    );
    assert!(s.is_some() && s < bg, "the shadow is drawn before (under) the box: {s:?} {bg:?}");
    // Below the box (y 60..66) the shadow darkens the white window.
    let p = frame.pixel(60, 63).unwrap_or([255; 4]);
    assert!(p[0] < 200, "shadow under the box: {p:?}");
    let far = frame.pixel(190, 110).unwrap_or([0; 4]);
    assert_eq!(far, [255, 255, 255, 255], "nothing far from it");
}

#[test]
fn blur_and_drop_shadow_filter_the_element() {
    let ui = ui();
    let ds = Shadow::new(3.0, 3.0, 6.0, Color::rgba8(0, 0, 0, 160));
    ui.root()
        .padding(20.0)
        .child(boxed(&ui, Style::new().background(Color::BLACK).blur(4.0)))
        .child(boxed(&ui, Style::new().background(Color::rgb8(200, 30, 30)).drop_shadow(ds)));
    let (tree, frame) = draw(&ui);
    let all = items(&tree);
    assert!(all.iter().any(|i| matches!(i, Item::PushFilter(Filter::Blur { radius }) if *radius == 4.0)));
    assert!(all.iter().any(|i| matches!(i, Item::PushFilter(Filter::DropShadow { .. }))));
    let pushes = all.iter().filter(|i| matches!(i, Item::PushFilter(_))).count();
    assert_eq!(pushes, all.iter().filter(|i| matches!(i, Item::PopFilter)).count(), "balanced");
    // The blurred box's edge is soft: a pixel just outside it is grey, not white.
    let edge = frame.pixel(18, 40).unwrap_or([255; 4]);
    assert!(edge[0] > 20 && edge[0] < 240, "a soft edge: {edge:?}");
}

#[test]
fn backdrop_blur_makes_frosted_glass_on_a_layer_of_its_own() {
    let ui = ui();
    let stripes = ui.el("stripes").paint(|cx| {
        for i in 0..20 {
            let c = if i % 2 == 0 { Color::BLACK } else { Color::WHITE };
            cx.list.fill_rect(
                remedy_ui_core::Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 120.0),
                remedy_ui_paint::Brush::Solid(c),
            );
        }
    });
    let glass = boxed(&ui, Style::new().backdrop_blur(6.0).radius(8.0));
    ui.root().child(stripes.layout(|l| {
        l.width = Length::Px(200.0);
        l.height = Length::Px(120.0);
        l.position = remedy_ui_layout::Position::Absolute;
    }));
    ui.root().padding(30.0).child(glass);
    let (tree, frame) = draw(&ui);
    let mut found = None;
    tree.for_each(|l| {
        if let Some(b) = l.backdrop {
            found = Some((b.blur, l.bounds));
        }
    });
    let Some((blur, bounds)) = found else { unreachable!("the glass has a layer with a backdrop") };
    assert_eq!(blur, 6.0);
    assert_eq!((bounds.width(), bounds.height()), (80.0, 40.0));
    // Under the glass the stripes blur to grey; outside it they stay black and white.
    let under = frame.pixel(65, 50).unwrap_or([0; 4]);
    assert!(under[0] > 60 && under[0] < 200, "blurred under the glass: {under:?}");
    let outside = frame.pixel(65, 100).unwrap_or([0; 4]);
    assert!(outside[0] < 5 || outside[0] > 250, "sharp outside it: {outside:?}");
}
