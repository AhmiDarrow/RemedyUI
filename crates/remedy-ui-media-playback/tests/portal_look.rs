//! The player in Portal's theme with Portal's own glyphs, drawn by the developer tools' preview
//! renderer, checked against the values Portal's page has (`web/src/styles.css`,
//! `web/src/themes.ts`, the SVGs in `web/index.html`), so its look cannot drift.
//! `REMEDY_SHOTS=<dir>` also writes the pictures.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_devtools::{Preview, Snapshot};
use remedy_ui_media_playback::*;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::Icon;

/// Portal's glyphs, as its page draws them: filled paths, and `.line` paths stroked 1.8 wide.
fn portal_svg(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Play => r#"<path d="M8 5.5v13l10.5-6.5z"/>"#,
        Icon::Pause => r#"<path d="M7.5 5.5h3v13h-3zM13.5 5.5h3v13h-3z"/>"#,
        Icon::Back10 => {
            r#"<path class="line" d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4.5v3.7h3.7"/><text x="12" y="15.6">10</text>"#
        }
        Icon::Forward10 => {
            r#"<path class="line" d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v3.7h-3.7"/><text x="12" y="15.6">10</text>"#
        }
        Icon::Volume => {
            r#"<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path class="line" d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>"#
        }
        Icon::VolumeMuted => {
            r#"<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path class="line" d="M16 9.5l5 5M21 9.5l-5 5"/>"#
        }
        Icon::Captions => {
            r#"<rect class="line" x="3" y="5.5" width="18" height="13" rx="2"/><path class="line" d="M10.5 10.3a2.2 2.2 0 1 0 0 3.4M17 10.3a2.2 2.2 0 1 0 0 3.4"/>"#
        }
        Icon::Theater => {
            r#"<rect class="line" x="2" y="6.5" width="20" height="11" rx="2"/><path class="line" d="M6 6.5v11M2 10h4"/>"#
        }
        Icon::Fullscreen => r#"<path class="line" d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5"/>"#,
        Icon::FullscreenExit => r#"<path class="line" d="M9 4v5H4M20 9h-5V4M15 20v-5h5M4 15h5v5"/>"#,
        _ => return None,
    })
}

/// What a Portal app hands the player: its SVGs, with the page's CSS for `.line` and `text`
/// written onto them.
fn portal_icons(ui: &Ui, icon: Icon, size: f32) -> Option<El> {
    let body = portal_svg(icon)?
        .replace(
            r#"class="line""#,
            r#"fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round""#,
        )
        .replace("<text ", r#"<text font-family="Remedy Sans" font-weight="700" font-size="6.5" text-anchor="middle" "#);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">{body}</svg>"#
    );
    Some(remedy_ui_media_svg::icon(ui, Arc::from(svg.into_bytes()), size, ""))
}

/// Portal's dock player: two rows, the skips, no speed button, a minute into three.
fn portal_player(ui: &Ui) -> El {
    let options = PlayerOptions::default().two_rows(true).skip_buttons(true).speed_button(false);
    let p = Player::empty_platform(ui, options);
    p.set_timeline(Some(Timeline::new(Duration::from_secs(60)).length(Duration::from_secs(180))));
    player_with(ui, &p, "RemedyUI cast test", PlayerParts::default().icons(portal_icons))
}

/// Renders the player at Portal's stage width in Portal's palette `id`; the picture, and the
/// player's element.
fn render(id: &str) -> (Snapshot, El) {
    let made: Rc<RefCell<Option<El>>> = Rc::default();
    let keep = made.clone();
    let preview = Preview::new(format!("portal-{id}"), move |ui| {
        let view = portal_player(ui);
        *keep.borrow_mut() = Some(view.clone());
        view
    })
    .themes(&[&format!("portal:{id}")])
    .sizes(&[(730.0, 480.0)]);
    let v = preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("one variation"));
    let shot = preview.render(&v);
    if let Ok(dir) = std::env::var("REMEDY_SHOTS") {
        let png = shot.to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
        std::fs::write(std::path::Path::new(&dir).join(format!("portal-{id}.png")), png)
            .unwrap_or_else(|e| unreachable!("{e}"));
    }
    let el = made.borrow().clone().unwrap_or_else(|| unreachable!("built"));
    (shot, el)
}

/// The first shown element named `widget` under `root`, depth first.
fn find(root: &El, widget: &str) -> El {
    let mut stack = vec![root.clone()];
    while let Some(e) = stack.pop() {
        let shown = e.bounds().is_some_and(|b| b.width() > 0.0);
        if shown && e.info().is_some_and(|i| i.widget == widget) {
            return e;
        }
        stack.extend(e.child_elements().into_iter().rev());
    }
    unreachable!("no {widget}")
}

/// The largest difference between two colors' channels (alpha left out).
fn off(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).take(3).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(u8::MAX)
}

fn hex(s: &str) -> [u8; 3] {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or_default();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The pixel at `(x, y)` is `want`, within 3 per channel (opaque, so premultiplied is plain).
fn assert_pixel(shot: &Snapshot, x: f32, y: f32, want: &str, what: &str) {
    let got =
        shot.pixel(x.round() as u32, y.round() as u32).unwrap_or_else(|| unreachable!("in the picture"));
    let want = hex(want);
    let close = off(&got, &want) <= 3;
    assert!(close, "{what} at ({x}, {y}): {got:?}, Portal's is {want:?}");
}

#[test]
fn player_wears_portals_look() {
    for (id, raised, line, accent, track) in [
        ("portal", "#211e18", "#3a3428", "#e2b15a", "#2c281f"),
        ("light", "#ffffff", "#a8a8b8", "#7c3aed", "#a8a8b8"),
        ("deutan", "#121821", "#34465a", "#56b4e9", "#34465a"),
    ] {
        let (shot, view) = render(id);
        let bounds = |w: &str| find(&view, w).bounds().unwrap_or_else(|| unreachable!("{w} laid out"));
        // `.chrome`: on the raised surface, under a 1 px line.
        let rule = bounds("player_rule");
        assert_eq!(rule.height(), 1.0, "{id}: the line over the bar");
        // Portal's bar is 67.8 px high (fractional, as its 0.15 rem and 0.4 rem padding make it),
        // so the line straddles two pixel rows: the brighter one is 80% the line.
        let want = hex(line);
        let row_off = |y: f32| {
            let p = shot.pixel(40, y as u32).unwrap_or_default();
            off(&p, &want)
        };
        let near = row_off(rule.min.y.floor()).min(row_off(rule.max.y.floor()));
        let far = off(&want, &hex(raised));
        assert!(near <= far / 4 + 3, "{id}: the line over the bar is {near} off");
        let gap = bounds("player_gap");
        assert_pixel(&shot, gap.center().x, gap.center().y, raised, "the bar");
        // `.icon-btn`: 2.25 rem round buttons, 1.3 rem glyphs.
        let play = bounds("player_button");
        assert_eq!((play.width(), play.height()), (36.0, 36.0), "{id}: Play's size");
        // `.seek-bar`: 1.35 rem high, a 4 px track, played in the accent, the rest the track.
        let seek = bounds("player_seek");
        assert_eq!(seek.height(), 22.0, "{id}: the seek bar's height");
        let mid = seek.center().y;
        assert_pixel(&shot, seek.min.x + seek.width() / 6.0, mid, accent, "the played part");
        assert_pixel(&shot, seek.min.x + seek.width() * 5.0 / 6.0, mid, track, "the track");
        assert_pixel(&shot, seek.min.x + seek.width() * 5.0 / 6.0, mid - 4.0, raised, "above the 4 px track");
        // The thumb at a third, in the accent.
        assert_pixel(&shot, seek.min.x + 5.5 + (seek.width() - 11.0) / 3.0, mid - 4.0, accent, "the thumb");
    }
}
