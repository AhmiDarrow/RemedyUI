//! Where boxes and text land on the device-pixel grid, as browsers place them: text boxes keep
//! fractional heights (no drift down a stack), box edges are snapped for crisp borders and
//! fills, and scroll containers hand their scroll bars to the compositor.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, LayerTree};
use remedy_ui_core::{Color, LineHeight, ScrollbarColor, ScrollbarVisibility, ScrollbarWidth, Size};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::layout::Length;
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_core::style::{States, Style, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

const SIZE: Size = Size { width: 200.0, height: 300.0 };

fn ui() -> Ui {
    let mut f = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = f.add_static(face.data);
    }
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::BLACK));
    Ui::new(Theme::dark_forest(), sheet, f)
}

fn draw(ui: &Ui) -> (LayerTree, Frame) {
    let f = ui.frame(SIZE, 1.0);
    let Some(tree) = f.layers else { unreachable!("a first frame has layers") };
    let mut comp = Compositor::new(200, 300, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(tree.clone());
    let Ok(frame) = CpuCompositor::new().present(&comp.frame(Duration::ZERO), 200, 300, Color::BLACK) else {
        unreachable!("draws")
    };
    (tree, frame)
}

#[test]
fn text_boxes_keep_fractional_line_heights_so_stacks_do_not_drift() {
    let ui = ui();
    let col = ui.el("column").column();
    let mut lines = Vec::new();
    for n in 0..10 {
        let t = ui.el("text").text(format!("Line {n}")).line_height(LineHeight::Absolute(21.12));
        lines.push(t.clone());
        col.clone().child(t);
    }
    ui.root().child(col);
    let _ = ui.frame(SIZE, 1.0);
    let heights: Vec<f32> = lines.iter().filter_map(|l| l.bounds()).map(|b| b.height()).collect();
    assert_eq!(heights.len(), 10);
    // The 1/64 px layout grid: 21.125, never rounded up to 22.
    assert!(heights.iter().all(|h| (*h - 21.125).abs() < 1e-4), "{heights:?}");
    let last = lines.last().and_then(remedy_ui_view::El::bounds).map_or(0.0, |b| b.max.y);
    assert!((last - 211.25).abs() < 1e-3, "ten lines end at 211.25, not 220: {last}");
}

#[test]
fn borders_and_fills_at_fractional_positions_are_crisp() {
    let ui = ui();
    let red = Color::rgb8(220, 30, 30);
    let white = Color::WHITE;
    // A box 30.4 px from the top: browsers draw its edge on pixel row 30, whole.
    let spacer = ui.el("spacer").layout(|l| l.height = Length::Px(30.4));
    let boxed =
        ui.el("panel").style(Style::new().background(red).border(white).border_width(1.0)).layout(|l| {
            l.width = Length::Px(50.6);
            l.height = Length::Px(20.3);
        });
    ui.root().child(ui.el("column").column().child(spacer).child(boxed));
    let (_, frame) = draw(&ui);
    let px = |x: u32, y: u32| frame.pixel(x, y).unwrap_or([0; 4]);
    assert_eq!(px(10, 29)[..3], [0, 0, 0], "nothing bleeds above the edge");
    assert_eq!(px(10, 30)[..3], [255, 255, 255], "the top border is one whole white row");
    assert_eq!(px(10, 31)[..3], [220, 30, 30], "then the fill, unblended");
    // Bottom edge at 50.7 snaps to 51: row 50 is the border, row 51 is outside.
    assert_eq!(px(10, 50)[..3], [255, 255, 255]);
    assert_eq!(px(10, 51)[..3], [0, 0, 0]);
    // Right edge at 50.6 snaps to 51: column 50 is the border.
    assert_eq!(px(50, 40)[..3], [255, 255, 255]);
    assert_eq!(px(51, 40)[..3], [0, 0, 0]);
}

#[test]
fn scroll_containers_give_the_compositor_their_scroll_bars() {
    let ui = ui();
    let thumb = Color::rgb8(90, 80, 60);
    let tall = ui.el("tall").layout(|l| {
        l.height = Length::Px(900.0);
        l.min_height = Length::Px(900.0);
    });
    let scroller = ui
        .el("scroll")
        .scroll()
        .column()
        .style(
            Style::new()
                .scrollbar_width(ScrollbarWidth::Thin)
                .scrollbar_color(ScrollbarColor::new(thumb, Color::TRANSPARENT))
                .scrollbar_visibility(ScrollbarVisibility::Always),
        )
        .layout(|l| l.height = Length::Px(200.0))
        .child(tall);
    let plain = ui.el("scroll").scroll().layout(|l| l.height = Length::Px(50.0));
    let hidden = ui
        .el("scroll")
        .scroll()
        .style(Style::new().scrollbar_width(ScrollbarWidth::None))
        .layout(|l| l.height = Length::Px(50.0));
    ui.root().child(ui.el("column").column().child(scroller).child(plain).child(hidden));
    let (tree, frame) = draw(&ui);
    let mut bars = Vec::new();
    tree.for_each(|l| {
        if l.scroll.is_some() {
            bars.push(l.scrollbars);
        }
    });
    assert_eq!(bars.len(), 3, "three scroll containers");
    let styled = bars.iter().flatten().find(|b| b.thumb == thumb);
    assert!(
        styled.is_some_and(|b| b.width == 8.0 && b.visibility == ScrollbarVisibility::Always),
        "{bars:?}"
    );
    assert!(
        bars.iter()
            .flatten()
            .any(|b| b.visibility == ScrollbarVisibility::Auto && b.width == ScrollbarWidth::AUTO_PX),
        "an unstyled container gets the platform's overlay bars: {bars:?}"
    );
    assert_eq!(bars.iter().filter(|b| b.is_none()).count(), 1, "`scrollbar-width: none` draws none");
    // The always-shown thumb is drawn at the right edge, near the top.
    assert_eq!(frame.pixel(196, 10).map(|p| [p[0], p[1], p[2]]), Some([90, 80, 60]));
}

#[test]
fn an_inline_code_chip_fills_pixels_behind_its_word() {
    use remedy_ui_text::{SpanBackground, TextStylePatch};
    let ui = ui();
    let green = Color::rgb8(0, 180, 40);
    let chip = SpanBackground::chip(green, 16.0);
    let line = remedy_ui_view::RichText::new()
        .text("use ")
        .styled("code", TextStylePatch::background(chip))
        .text(" here");
    ui.root().child(ui.el("text").text(line).style(Style::new().text_size(16.0).foreground(Color::WHITE)));
    let (_, frame) = draw(&ui);
    let mut green_px = 0;
    for y in 0..48 {
        for x in 0..200 {
            let p = frame.pixel(x, y).unwrap_or([0; 4]);
            if p[1] > 140 && p[0] < 40 && p[2] < 80 {
                green_px += 1;
            }
        }
    }
    assert!(green_px > 20, "the chip fills pixels behind the word: {green_px}");
}

#[test]
fn ch_lengths_are_advances_of_zero_in_the_elements_font() {
    let ui = ui();
    let zero = ui.el("text").text("0").nowrap().style(Style::new().text_size(20.0));
    let measure = ui.el("box").style(Style::new().text_size(20.0)).layout(|l| l.width = Length::Ch(10.0));
    let small = ui.el("box").style(Style::new().text_size(10.0)).layout(|l| l.max_width = Length::Ch(10.0));
    let row = ui.el("row").row().child(zero.clone());
    ui.root().child(ui.el("column").column().child(row).child(measure.clone()).child(small.clone()));
    let _ = ui.frame(SIZE, 1.0);
    let one = zero.bounds().map_or(0.0, |b| b.width());
    assert!(one > 5.0, "a digit is several pixels wide: {one}");
    let w = measure.bounds().map_or(0.0, |b| b.width());
    assert!((w - 10.0 * one).abs() < 0.1, "10ch is ten zeros: {w} vs {one}");
    let s = small.bounds().map_or(0.0, |b| b.width());
    assert!((s - 5.0 * one).abs() < 0.2, "at half the size, half as wide: {s}");
}

#[test]
fn a_rounded_image_element_rounds_its_picture() {
    let ui = ui();
    let white = remedy_ui_paint::Image::new(40, 40, vec![255u8; 40 * 40 * 4].into());
    ui.root().child(
        ui.el("image").image(white).width(40.0).height(40.0).style(StylePatch::default().radius(12.0)),
    );
    let (_, frame) = draw(&ui);
    let px = |x: u32, y: u32| frame.pixel(x, y).unwrap_or([0; 4]);
    assert_eq!(px(0, 0)[0], 0, "the corner is cut away");
    assert_eq!(px(20, 20)[0], 255, "the middle is the picture");
    assert_eq!(px(20, 0)[0], 255, "an edge away from the corners is the picture");
}
