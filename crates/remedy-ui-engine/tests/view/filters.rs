//! Box shadows, filters and frosted glass from style: what an element's style asks for is in
//! what the window draws.

use std::time::Duration;

use remedy_ui_engine::compositor::{Compositor, LayerTree};
use remedy_ui_core::{Color, ColorMatrix, Size};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::layout::Length;
use remedy_ui_engine::paint::{Filter, Item};
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};
use remedy_ui_core::style::{Shadow, States, Style, StylePatch, StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::Ui;

fn ui() -> Ui {
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    Ui::new(Theme::dark_forest(), sheet, FontCollection::new())
}

fn boxed(ui: &Ui, style: Style) -> remedy_ui_engine::view::El {
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
        &|i| matches!(i, Item::FillRect { .. } if !matches!(i, Item::FillRect { brush: remedy_ui_engine::paint::Brush::Solid(c), .. } if *c == Color::WHITE)),
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
                remedy_ui_engine::paint::Brush::Solid(c),
            );
        }
    });
    let glass = boxed(&ui, Style::new().backdrop_blur(6.0).radius(8.0));
    ui.root().child(stripes.layout(|l| {
        l.width = Length::Px(200.0);
        l.height = Length::Px(120.0);
        l.position = remedy_ui_core::layout::Position::Absolute;
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

#[test]
fn a_style_can_round_some_corners_only() {
    use remedy_ui_core::CornerRadii;
    let ui = ui();
    let blue = Color::rgb8(40, 120, 220);
    // Round at the top only (CSS `border-radius: 16px 16px 0 0`), clipping its child.
    let top = CornerRadii { top_start: 16.0, top_end: 16.0, ..CornerRadii::ZERO };
    let fill = ui.el("fill").style(StylePatch::default().background(blue)).layout(|l| {
        l.width = Length::Px(80.0);
        l.height = Length::Px(40.0);
    });
    let card = boxed(&ui, Style::from(StylePatch::default().corners(top))).clip().child(fill);
    ui.root().padding(20.0).child(card);
    let (tree, frame) = draw(&ui);
    assert!(
        items(&tree).iter().any(|i| matches!(i, Item::PushClip { radii, .. } if *radii == top)),
        "the clip has the style's corners"
    );
    let at = |x, y| frame.pixel(x, y).unwrap_or([0; 4]);
    let is_blue = |p: [u8; 4]| p[0].abs_diff(40) <= 2 && p[1].abs_diff(120) <= 2 && p[2].abs_diff(220) <= 2;
    assert_eq!(at(21, 21), [255, 255, 255, 255], "the top start corner is round");
    assert_eq!(at(98, 21), [255, 255, 255, 255], "so is the top end");
    assert!(is_blue(at(21, 58)), "the bottom start corner is square: {:?}", at(21, 58));
    assert!(is_blue(at(98, 58)), "so is the bottom end: {:?}", at(98, 58));

    // A radius over corners rounds every corner again; tokens set corners too.
    let both = StylePatch::default().corners(top).merge(&StylePatch::default().radius(4.0));
    assert_eq!(both.corners, None);
    let theme = Theme::dark_forest();
    let tokens = Style::new().radius(2.0).corners(8.0, 8.0, 0.0, 0.0).resolve(&theme);
    assert_eq!(tokens.corners, Some(CornerRadii { top_start: 8.0, top_end: 8.0, ..CornerRadii::ZERO }));
    assert_eq!(tokens.radius, Some(8.0), "the radius is the largest corner");
}

#[test]
fn dashed_and_dotted_borders_follow_the_corners() {
    use remedy_ui_core::CornerRadii;
    use remedy_ui_core::style::BorderStyle;
    let ui = ui();
    let top = CornerRadii { top_start: 16.0, top_end: 16.0, ..CornerRadii::ZERO };
    let dashed =
        StylePatch::default().border(Color::BLACK).border_width(2.0).border_style(BorderStyle::Dashed);
    let card = boxed(&ui, Style::from(dashed.corners(top)));
    ui.root().padding(20.0).child(card);
    let (tree, frame) = draw(&ui);
    assert!(
        items(&tree)
            .iter()
            .any(|i| matches!(i, Item::StrokePath { stroke, .. } if stroke.dashes == [6.0, 6.0])),
        "a dashed border is a dashed stroke"
    );
    let at = |x, y| frame.pixel(x, y).unwrap_or([0; 4]);
    // The top edge starts after the round corner: a dash at 36..42, a gap at 42..48.
    assert!(at(39, 20)[0] < 40, "a dash: {:?}", at(39, 20));
    assert!(at(45, 20)[0] > 215, "a gap: {:?}", at(45, 20));
    assert_eq!(at(20, 20), [255, 255, 255, 255], "the top start corner is round");

    // Tokens carry the style; a later patch without one keeps it.
    let theme = Theme::dark_forest();
    let s = Style::new().border_style(BorderStyle::Dotted).resolve(&theme);
    assert_eq!(s.border_style, Some(BorderStyle::Dotted));
    let kept = StylePatch::default()
        .border_style(BorderStyle::Dotted)
        .merge(&StylePatch::default().border_width(4.0));
    assert_eq!(kept.border_style, Some(BorderStyle::Dotted));
}

#[test]
fn a_wide_dotted_border_is_round_dots() {
    use remedy_ui_core::style::BorderStyle;
    let ui = ui();
    let dotted =
        StylePatch::default().border(Color::BLACK).border_width(4.0).border_style(BorderStyle::Dotted);
    ui.root().padding(20.0).child(boxed(&ui, Style::from(dotted)));
    let (_, frame) = draw(&ui);
    let at = |x, y| frame.pixel(x, y).unwrap_or([0; 4]);
    // Dots 4 wide, 8 apart along the top edge's middle (y 22) from x 22: one at 30, a gap at 26.
    assert!(at(30, 22)[0] < 40, "a dot: {:?}", at(30, 22));
    assert!(at(26, 22)[0] > 215, "a gap: {:?}", at(26, 22));
}

#[test]
fn each_side_can_have_its_own_border() {
    use remedy_ui_core::CornerRadii;
    use remedy_ui_core::style::{BorderEdge, BorderSide, BorderStyle};
    let ui = ui();
    let (red, blue) = (Color::rgb8(220, 30, 30), Color::rgb8(30, 60, 220));
    // A thick red top, a dashed blue start, nothing at the end or bottom; round at the top start.
    let look = StylePatch::default()
        .border_side(BorderEdge::Top, BorderSide::new(6.0, red))
        .border_side(BorderEdge::Start, BorderSide::new(2.0, blue).style(BorderStyle::Dashed))
        .corners(CornerRadii { top_start: 12.0, ..CornerRadii::ZERO });
    let fill = ui.el("fill").style(StylePatch::default().background(Color::rgb8(0, 160, 0))).layout(|l| {
        l.width = Length::Px(20.0);
        l.height = Length::Px(10.0);
    });
    let card = boxed(&ui, Style::from(look)).child(fill.clone());
    ui.root().padding(20.0).child(card);
    let (tree, frame) = draw(&ui);
    let at = |x, y| frame.pixel(x, y).unwrap_or([0; 4]);
    let is = |p: [u8; 4], c: Color| {
        let w = c.to_rgba8();
        p[0].abs_diff(w[0]) <= 3 && p[1].abs_diff(w[1]) <= 3 && p[2].abs_diff(w[2]) <= 3
    };
    assert!(is(at(60, 22), red), "the top: {:?}", at(60, 22));
    assert!(is(at(99, 22), red), "the top runs to the square end corner: {:?}", at(99, 22));
    assert_eq!(at(20, 20), [255, 255, 255, 255], "the top start corner is round");
    assert_eq!(at(98, 40), [255, 255, 255, 255], "no border at the end");
    assert_eq!(at(60, 58), [255, 255, 255, 255], "nor at the bottom");
    assert!(
        items(&tree)
            .iter()
            .any(|i| matches!(i, Item::StrokePath { stroke, brush: remedy_ui_engine::paint::Brush::Solid(c), .. }
            if *c == blue && stroke.dashes == [6.0, 6.0])),
        "the start is a dashed blue stroke"
    );
    // The content sits inside each side's own width.
    let b = fill.bounds().unwrap_or_default();
    assert_eq!((b.min.x, b.min.y), (22.0, 26.0), "inside the start's 2 and the top's 6");

    // A side keeps its own border when a later patch sets the box's.
    let theme = Theme::dark_forest();
    let s = Style::new().border_side(BorderEdge::Bottom, BorderSide::new(1.0, red)).resolve(&theme);
    let merged = s.merge(&StylePatch::default().border_width(3.0));
    assert_eq!(merged.border_sides[2], Some(BorderSide::new(1.0, red)));
}

#[test]
fn text_lines_follow_the_elements_alignment_wrapped_or_not() {
    use remedy_ui_core::style::TextAlign;
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut sheet = StyleSheet::new();
    sheet.rule("window", None, States::NONE, StylePatch::default().background(Color::WHITE));
    let ui = Ui::new(Theme::dark_forest(), sheet, fonts);
    let dark = StylePatch::default().foreground(Color::BLACK);
    // Inherited: the box says end, its text follows over two lines.
    let label =
        ui.el("label").style(dark).text("Wide words wrap here").layout(|l| l.width = Length::Px(90.0));
    let host = ui.el("box").text_align(TextAlign::End).child(label.clone());
    ui.root().padding(10.0).child(host);
    let (_, frame) = draw(&ui);
    let b = label.bounds().unwrap_or_default();
    let ink = |y0: f32, y1: f32| {
        let mut xs = Vec::new();
        for y in y0 as u32..y1 as u32 {
            for x in b.min.x as u32..b.max.x as u32 {
                if frame.pixel(x, y).is_some_and(|p| p[0] < 128) {
                    xs.push(x);
                }
            }
        }
        (xs.iter().copied().min().unwrap_or(0), xs.iter().copied().max().unwrap_or(0))
    };
    let mid = b.min.y + b.height() / 2.0;
    let (first, second) = (ink(b.min.y, mid), ink(mid, b.max.y));
    assert!(b.height() > 24.0, "two lines: {b:?}");
    assert!(
        first.1 as f32 > b.max.x - 4.0 && second.1 as f32 > b.max.x - 4.0,
        "both end at the end: {first:?} {second:?}"
    );
    assert!(
        first.0 as f32 > b.min.x + 2.0 || second.0 as f32 > b.min.x + 2.0,
        "a shorter line starts in from the start"
    );
}

/// Black and white stripes across the window, behind everything.
fn stripes(ui: &Ui) -> remedy_ui_engine::view::El {
    ui.el("stripes")
        .paint(|cx| {
            for i in 0..20 {
                let c = if i % 2 == 0 { Color::BLACK } else { Color::WHITE };
                cx.list.fill_rect(
                    remedy_ui_core::Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 120.0),
                    remedy_ui_engine::paint::Brush::Solid(c),
                );
            }
        })
        .layout(|l| {
            l.width = Length::Px(200.0);
            l.height = Length::Px(120.0);
            l.position = remedy_ui_core::layout::Position::Absolute;
        })
}

/// How far the pixels in a row of the glass are from the average grey (sharp stripes: far).
fn contrast(frame: &Frame, y: u32) -> u32 {
    (40..100).map(|x| u32::from(frame.pixel(x, y).unwrap_or([0; 4])[0].abs_diff(128))).max().unwrap_or(0)
}

#[test]
fn glass_fades_with_its_element() {
    let ui = ui();
    ui.root().child(stripes(&ui));
    ui.root().padding(30.0).child(boxed(&ui, Style::new().backdrop_blur(6.0).opacity(0.0001)));
    let (tree, frame) = draw(&ui);
    let mut found = None;
    tree.for_each(|l| {
        if l.backdrop.is_some() {
            found = Some(l.opacity);
        }
    });
    assert!(found.is_some_and(|o| o < 0.01), "the glass's layer carries the opacity: {found:?}");
    // Nearly transparent glass: the stripes show through almost as sharp as without it.
    assert!(contrast(&frame, 50) > 120, "faded glass barely blurs: {}", contrast(&frame, 50));

    let ui = self::ui();
    ui.root().child(stripes(&ui));
    ui.root().padding(30.0).child(boxed(&ui, Style::new().backdrop_blur(6.0)));
    let (_, frame) = draw(&ui);
    assert!(contrast(&frame, 50) < 60, "opaque glass blurs: {}", contrast(&frame, 50));
}

#[test]
fn scroll_containers_can_be_frosted_glass() {
    let ui = ui();
    ui.root().child(stripes(&ui));
    let list = ui
        .el("list")
        .scroll()
        .style(Style::new().backdrop_blur(6.0).background(Color::rgba8(255, 0, 0, 40)))
        .layout(|l| {
            l.width = Length::Px(80.0);
            l.height = Length::Px(40.0);
        })
        .child(ui.el("tall").layout(|l| l.height = Length::Px(200.0)));
    ui.root().padding(30.0).child(list);
    let (tree, frame) = draw(&ui);
    let mut glass = None;
    tree.for_each(|l| {
        if l.backdrop.is_some() {
            glass = Some((l.scroll.is_some(), l.content.items().len()));
        }
    });
    let Some((scrolls, own)) = glass else { unreachable!("the scroll container's viewport is glass") };
    assert!(scrolls && own > 0, "the viewport layer holds the glass and the container's own drawing");
    assert!(contrast(&frame, 50) < 60, "blurred under it: {}", contrast(&frame, 50));
    let tint = frame.pixel(60, 50).unwrap_or([0; 4]);
    assert!(tint[0] > tint[2], "its background tints the glass: {tint:?}");
}

#[test]
fn a_filter_takes_children_that_are_layers_of_their_own() {
    let ui = ui();
    // A black square in a layer of its own (isolate_paint), inside a blurred panel.
    let square = ui
        .el("square")
        .style(Style::new().background(Color::BLACK))
        .layout(|l| {
            l.width = Length::Px(40.0);
            l.height = Length::Px(40.0);
        })
        .isolate_paint();
    let panel = ui.el("panel").style(Style::new().blur(4.0)).padding(20.0).child(square);
    ui.root().padding(20.0).child(panel);
    let (tree, frame) = draw(&ui);
    let mut filtered = None;
    tree.for_each(|l| {
        if !l.filters.is_empty() {
            filtered = Some((l.filters.clone(), l.clip.is_some()));
        }
    });
    let Some((filters, clipped)) = filtered else { unreachable!("the panel is a filtered layer") };
    assert_eq!(filters, vec![Filter::Blur { radius: 4.0 }]);
    assert!(!clipped, "a blur draws past the box: not clipped there");
    assert!(
        !items(&tree).iter().any(|i| matches!(i, Item::PushFilter(_))),
        "filtered once, by the compositor"
    );
    // The square spans 40..80; its edge is soft, and it is grey just outside.
    let edge = frame.pixel(39, 60).unwrap_or([255; 4]);
    assert!(edge[0] > 20 && edge[0] < 240, "the layer inside is blurred: {edge:?}");
    let inside = frame.pixel(60, 60).unwrap_or([255; 4]);
    assert!(inside[0] < 20, "its middle stays dark: {inside:?}");
}

#[test]
fn colour_filters_recolour_the_element() {
    let ui = ui();
    ui.root()
        .padding(20.0)
        .child(boxed(&ui, Style::new().background(Color::rgb8(200, 30, 30)).grayscale(1.0)))
        .child(boxed(&ui, Style::new().background(Color::rgb8(200, 30, 30)).invert(1.0).saturate(1.0)));
    let (tree, frame) = draw(&ui);
    let all = items(&tree);
    let matrices: Vec<_> = all
        .iter()
        .filter_map(|i| match i {
            Item::PushFilter(Filter::ColorMatrix { matrix }) => Some(*matrix),
            _ => None,
        })
        .collect();
    assert_eq!(matrices.len(), 2, "one matrix per element (chained presets are one matrix)");
    assert_eq!(matrices.first(), Some(&ColorMatrix::grayscale(1.0)));
    assert_eq!(matrices.get(1), Some(&ColorMatrix::invert(1.0).then(&ColorMatrix::saturate(1.0))));
    // The first box is grey, exactly as the matrix says.
    let want = ColorMatrix::grayscale(1.0).apply_premultiplied([200, 30, 30, 255]);
    assert_eq!(frame.pixel(60, 40), Some(want));
    let [r, g, b, _] = want;
    assert!(r == g && g == b, "grey: {want:?}");
    // The second is the negative of red.
    assert_eq!(frame.pixel(60, 80), Some([55, 225, 225, 255]));
    // The window around them is untouched.
    assert_eq!(frame.pixel(190, 110), Some([255, 255, 255, 255]));
}

#[test]
fn a_colour_filter_takes_children_that_are_layers_of_their_own() {
    let ui = ui();
    let square = ui
        .el("square")
        .style(Style::new().background(Color::rgb8(30, 200, 60)))
        .layout(|l| {
            l.width = Length::Px(40.0);
            l.height = Length::Px(40.0);
        })
        .isolate_paint();
    let panel = ui.el("panel").style(Style::new().hue_rotate(180.0)).padding(20.0).child(square);
    ui.root().padding(20.0).child(panel);
    let (tree, frame) = draw(&ui);
    let mut filtered = None;
    tree.for_each(|l| {
        if !l.filters.is_empty() {
            filtered = Some(l.filters.clone());
        }
    });
    assert_eq!(filtered, Some(vec![Filter::ColorMatrix { matrix: ColorMatrix::hue_rotate(180.0) }]));
    let want = ColorMatrix::hue_rotate(180.0).apply_premultiplied([30, 200, 60, 255]);
    assert_eq!(frame.pixel(60, 60), Some(want), "the layer inside is recoloured by the compositor");
    assert_eq!(frame.pixel(190, 110), Some([255, 255, 255, 255]));
}
