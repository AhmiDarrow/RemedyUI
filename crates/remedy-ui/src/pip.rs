//! A player's picture-in-picture window on the desktop ([`Player::request_pip`]): an
//! always-on-top frameless window showing the player's picture, which the person moves by
//! dragging it, resizes from its edges and corners, and sends back with a double click or
//! Escape. Installed for every app by [`run`](crate::run).
//!
//! [`Player::request_pip`]: remedy_ui_media_playback::Player::request_pip

use remedy_ui_core::Size;
use remedy_ui_core::host::ResizeEdge;
use remedy_ui_core::input::{Key, PointerPhase};
use remedy_ui_media_playback::{Player, player_picture, set_pip_window};

use crate::runner::{App, WindowSpec};

/// How wide the window opens, in logical pixels: a corner of the screen, as a browser's.
const WIDTH: f32 = 480.0;
/// The least height the window opens with or is resized to.
const MIN_HEIGHT: f32 = 90.0;
/// How near an edge a press resizes rather than moves.
const GRIP: f32 = 8.0;

/// The edge or corner a press at (`x`, `y`) in a window of `size` resizes from, if any.
fn edge(x: f32, y: f32, size: Size) -> Option<ResizeEdge> {
    let (l, r) = (x < GRIP, x > size.width - GRIP);
    let (t, b) = (y < GRIP, y > size.height - GRIP);
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (_, true, true, _) => ResizeEdge::TopRight,
        (true, _, _, true) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Left,
        (_, true, ..) => ResizeEdge::Right,
        (_, _, true, _) => ResizeEdge::Top,
        (_, _, _, true) => ResizeEdge::Bottom,
        _ => return None,
    })
}

/// Lets players on this thread open their picture-in-picture window through `app`.
pub(crate) fn install(app: &App) {
    let app = app.clone();
    set_pip_window(Some(Box::new(move |player: &Player| {
        let height = player
            .info()
            .and_then(|i| i.video)
            .map_or(WIDTH * 9.0 / 16.0, |v| WIDTH * v.height.max(1) as f32 / v.width.max(1) as f32)
            // The size is the file's word: a sliver of a video must not ask for a window miles tall.
            .clamp(MIN_HEIGHT, WIDTH * 3.0);
        let title = player.title().unwrap_or_default();
        let spec = WindowSpec::new(title.clone())
            .size(WIDTH, height)
            .min_size(160.0, MIN_HEIGHT)
            .frameless()
            .always_on_top(true)
            .in_taskbar(false);
        let p = player.clone();
        let win = app.open(spec, move |ui, win| {
            let (mover, back, keys) = (win.clone(), p.clone(), p.clone());
            let u = ui.clone();
            let picture = player_picture(ui, &p, title)
                .grow(1.0)
                .on_pointer(move |e| {
                    if e.phase == PointerPhase::Down {
                        let at = e.position;
                        match edge(at.x, at.y, u.window_size()) {
                            Some(edge) => mover.begin_resize(edge),
                            None => mover.begin_move(),
                        }
                    }
                    false
                })
                .on_double_click(move || back.exit_pip());
            let root = ui.root().column().on_key(move |k| {
                let escape = k.key == Key::Escape && k.pressed;
                if escape {
                    keys.exit_pip();
                }
                escape
            });
            let _ = root.child(picture);
        });
        let p = player.clone();
        win.on_closed(move || p.pip_closed());
        Some(Box::new(move || win.close()) as Box<dyn FnOnce()>)
    })));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_near_the_edges_resize_and_elsewhere_move() {
        let size = Size::new(480.0, 270.0);
        assert_eq!(edge(2.0, 2.0, size), Some(ResizeEdge::TopLeft));
        assert_eq!(edge(478.0, 268.0, size), Some(ResizeEdge::BottomRight));
        assert_eq!(edge(240.0, 1.0, size), Some(ResizeEdge::Top));
        assert_eq!(edge(479.0, 100.0, size), Some(ResizeEdge::Right));
        assert_eq!(edge(240.0, 135.0, size), None, "the middle moves the window");
    }
}
