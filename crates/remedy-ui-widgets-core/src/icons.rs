//! Icons (vector, drawn as strokes on a 24-unit grid), images, avatars and canvases.

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Point;
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_view::{El, PaintCx, Ui};

/// The built-in icons. Each is drawn in the text color at any size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Icon {
    /// +
    Plus,
    /// −
    Minus,
    /// A window maximize box.
    Maximize,
    /// Two overlapping boxes: restore a maximized window.
    Restore,
    /// ×
    Close,
    /// ✓
    Check,
    /// A closed padlock.
    Lock,
    /// An open padlock.
    Unlock,
    /// A push pin.
    Pin,
    /// A bin.
    Trash,
    /// Two overlapping sheets.
    Copy,
    /// An eye.
    Eye,
    /// An eye struck through.
    EyeOff,
    /// A downward chevron.
    ChevronDown,
    /// A rightward chevron.
    ChevronRight,
    /// A leftward chevron.
    ChevronLeft,
    /// An upward chevron.
    ChevronUp,
    /// Three lines.
    Menu,
    /// A magnifying glass.
    Search,
    /// An i in a circle.
    Info,
    /// An exclamation mark in a triangle.
    Alert,
    /// A clock.
    Clock,
    /// A key.
    Key,
    /// A gear.
    Settings,
    /// A note with a folded corner.
    Note,
    /// An arrow out of a box.
    External,
    /// A circular arrow.
    Refresh,
    /// A folder.
    Folder,
    /// A page with a folded corner (a file).
    File,
    /// An arrow down onto a tray.
    Download,
    /// An arrow up out of a tray.
    Upload,
    /// A paper plane (send a message).
    Send,
    /// An arrow pointing left (back).
    ArrowLeft,
    /// An arrow pointing right (forward).
    ArrowRight,
    /// An arrow pointing up (up a level).
    ArrowUp,
    /// Two people (a group of PCs or people).
    Users,
    /// A screen on a stand (a computer).
    Monitor,
}

fn p(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn circle(path: &mut Path, c: Point, r: f32) {
    let steps = 32;
    for i in 0..=steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let pt = p(c.x + r * a.cos(), c.y + r * a.sin());
        if i == 0 {
            path.move_to(pt);
        } else {
            path.line_to(pt);
        }
    }
}

fn arc(path: &mut Path, c: Point, r: f32, from: f32, to: f32) {
    let steps = 24;
    for i in 0..=steps {
        let a = from + (to - from) * i as f32 / steps as f32;
        let pt = p(c.x + r * a.cos(), c.y + r * a.sin());
        if i == 0 {
            path.move_to(pt);
        } else {
            path.line_to(pt);
        }
    }
}

fn rect(path: &mut Path, x0: f32, y0: f32, x1: f32, y1: f32) {
    path.move_to(p(x0, y0)).line_to(p(x1, y0)).line_to(p(x1, y1)).line_to(p(x0, y1)).close();
}

/// The icon's strokes on a 24 × 24 grid.
fn strokes(icon: Icon) -> Path {
    use std::f32::consts::PI;
    let mut s = Path::new();
    match icon {
        Icon::Plus => {
            s.move_to(p(12.0, 5.0)).line_to(p(12.0, 19.0)).move_to(p(5.0, 12.0)).line_to(p(19.0, 12.0));
        }
        Icon::Minus => {
            s.move_to(p(5.0, 12.0)).line_to(p(19.0, 12.0));
        }
        Icon::Maximize => {
            s.move_to(p(6.0, 6.0)).line_to(p(18.0, 6.0)).line_to(p(18.0, 18.0)).line_to(p(6.0, 18.0)).close();
        }
        Icon::Restore => {
            s.move_to(p(6.0, 9.0)).line_to(p(15.0, 9.0)).line_to(p(15.0, 18.0)).line_to(p(6.0, 18.0)).close();
            s.move_to(p(9.0, 9.0))
                .line_to(p(9.0, 6.0))
                .line_to(p(18.0, 6.0))
                .line_to(p(18.0, 15.0))
                .line_to(p(15.0, 15.0));
        }
        Icon::Close => {
            s.move_to(p(6.0, 6.0)).line_to(p(18.0, 18.0)).move_to(p(18.0, 6.0)).line_to(p(6.0, 18.0));
        }
        Icon::Check => {
            s.move_to(p(5.0, 12.5)).line_to(p(10.0, 17.5)).line_to(p(19.0, 7.0));
        }
        Icon::Lock | Icon::Unlock => {
            rect(&mut s, 5.0, 11.0, 19.0, 21.0);
            if icon == Icon::Lock {
                arc(&mut s, p(12.0, 8.0), 4.0, PI, 2.0 * PI);
                s.move_to(p(8.0, 8.0)).line_to(p(8.0, 11.0)).move_to(p(16.0, 8.0)).line_to(p(16.0, 11.0));
            } else {
                arc(&mut s, p(12.0, 7.0), 4.0, PI, 2.0 * PI);
                s.move_to(p(8.0, 7.0)).line_to(p(8.0, 11.0));
            }
        }
        Icon::Pin => {
            s.move_to(p(9.0, 3.0))
                .line_to(p(15.0, 3.0))
                .move_to(p(10.0, 3.0))
                .line_to(p(10.0, 9.0))
                .line_to(p(6.0, 14.0));
            s.line_to(p(18.0, 14.0))
                .line_to(p(14.0, 9.0))
                .line_to(p(14.0, 3.0))
                .move_to(p(12.0, 14.0))
                .line_to(p(12.0, 21.0));
        }
        Icon::Trash => {
            s.move_to(p(4.0, 7.0))
                .line_to(p(20.0, 7.0))
                .move_to(p(9.0, 7.0))
                .line_to(p(9.0, 4.0))
                .line_to(p(15.0, 4.0))
                .line_to(p(15.0, 7.0));
            s.move_to(p(6.0, 7.0)).line_to(p(7.0, 20.0)).line_to(p(17.0, 20.0)).line_to(p(18.0, 7.0));
            s.move_to(p(10.0, 11.0)).line_to(p(10.0, 16.0)).move_to(p(14.0, 11.0)).line_to(p(14.0, 16.0));
        }
        Icon::Copy => {
            rect(&mut s, 9.0, 9.0, 20.0, 20.0);
            s.move_to(p(15.0, 9.0))
                .line_to(p(15.0, 4.0))
                .line_to(p(4.0, 4.0))
                .line_to(p(4.0, 15.0))
                .line_to(p(9.0, 15.0));
        }
        Icon::Eye | Icon::EyeOff => {
            arc(&mut s, p(12.0, 19.0), 11.0, -PI * 0.83, -PI * 0.17);
            arc(&mut s, p(12.0, 5.0), 11.0, PI * 0.17, PI * 0.83);
            circle(&mut s, p(12.0, 12.0), 3.0);
            if icon == Icon::EyeOff {
                s.move_to(p(4.0, 4.0)).line_to(p(20.0, 20.0));
            }
        }
        Icon::ChevronDown => {
            s.move_to(p(6.0, 9.0)).line_to(p(12.0, 15.0)).line_to(p(18.0, 9.0));
        }
        Icon::ChevronRight => {
            s.move_to(p(9.0, 6.0)).line_to(p(15.0, 12.0)).line_to(p(9.0, 18.0));
        }
        Icon::ChevronLeft => {
            s.move_to(p(15.0, 6.0)).line_to(p(9.0, 12.0)).line_to(p(15.0, 18.0));
        }
        Icon::ChevronUp => {
            s.move_to(p(6.0, 15.0)).line_to(p(12.0, 9.0)).line_to(p(18.0, 15.0));
        }
        Icon::Menu => {
            for y in [6.0, 12.0, 18.0] {
                s.move_to(p(4.0, y)).line_to(p(20.0, y));
            }
        }
        Icon::Search => {
            circle(&mut s, p(10.5, 10.5), 6.5);
            s.move_to(p(15.5, 15.5)).line_to(p(20.0, 20.0));
        }
        Icon::Info => {
            circle(&mut s, p(12.0, 12.0), 9.0);
            s.move_to(p(12.0, 11.0)).line_to(p(12.0, 16.5)).move_to(p(12.0, 7.5)).line_to(p(12.0, 7.6));
        }
        Icon::Alert => {
            s.move_to(p(12.0, 3.5)).line_to(p(21.5, 20.0)).line_to(p(2.5, 20.0)).close();
            s.move_to(p(12.0, 9.5)).line_to(p(12.0, 14.0)).move_to(p(12.0, 17.0)).line_to(p(12.0, 17.1));
        }
        Icon::Clock => {
            circle(&mut s, p(12.0, 12.0), 9.0);
            s.move_to(p(12.0, 7.0)).line_to(p(12.0, 12.0)).line_to(p(15.5, 14.0));
        }
        Icon::Key => {
            circle(&mut s, p(8.0, 14.0), 4.0);
            s.move_to(p(11.0, 11.0))
                .line_to(p(20.0, 4.0))
                .move_to(p(17.0, 6.5))
                .line_to(p(19.5, 9.0))
                .move_to(p(14.5, 8.5))
                .line_to(p(16.5, 10.5));
        }
        Icon::Settings => {
            circle(&mut s, p(12.0, 12.0), 3.0);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                s.move_to(p(12.0 + 6.0 * a.cos(), 12.0 + 6.0 * a.sin()))
                    .line_to(p(12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin()));
            }
            circle(&mut s, p(12.0, 12.0), 6.0);
        }
        Icon::Note => {
            s.move_to(p(5.0, 4.0))
                .line_to(p(19.0, 4.0))
                .line_to(p(19.0, 14.0))
                .line_to(p(13.0, 20.0))
                .line_to(p(5.0, 20.0))
                .close();
            s.move_to(p(19.0, 14.0)).line_to(p(13.0, 14.0)).line_to(p(13.0, 20.0));
        }
        Icon::External => {
            s.move_to(p(14.0, 4.0))
                .line_to(p(20.0, 4.0))
                .line_to(p(20.0, 10.0))
                .move_to(p(20.0, 4.0))
                .line_to(p(11.0, 13.0));
            s.move_to(p(17.0, 14.0))
                .line_to(p(17.0, 20.0))
                .line_to(p(4.0, 20.0))
                .line_to(p(4.0, 7.0))
                .line_to(p(10.0, 7.0));
        }
        Icon::Refresh => {
            arc(&mut s, p(12.0, 12.0), 8.0, -PI * 0.4, PI * 1.5);
            s.move_to(p(18.0, 3.5)).line_to(p(18.5, 8.5)).line_to(p(13.5, 8.0));
        }
        Icon::Folder => {
            s.move_to(p(3.0, 6.0))
                .line_to(p(9.0, 6.0))
                .line_to(p(11.0, 8.5))
                .line_to(p(21.0, 8.5))
                .line_to(p(21.0, 19.0))
                .line_to(p(3.0, 19.0))
                .close();
        }
        Icon::File => {
            s.move_to(p(6.0, 3.0))
                .line_to(p(14.0, 3.0))
                .line_to(p(19.0, 8.0))
                .line_to(p(19.0, 21.0))
                .line_to(p(6.0, 21.0))
                .close();
            s.move_to(p(14.0, 3.0)).line_to(p(14.0, 8.0)).line_to(p(19.0, 8.0));
        }
        Icon::Download | Icon::Upload => {
            s.move_to(p(4.0, 15.0)).line_to(p(4.0, 20.0)).line_to(p(20.0, 20.0)).line_to(p(20.0, 15.0));
            s.move_to(p(12.0, 4.0)).line_to(p(12.0, 15.0));
            if icon == Icon::Download {
                s.move_to(p(7.5, 10.5)).line_to(p(12.0, 15.0)).line_to(p(16.5, 10.5));
            } else {
                s.move_to(p(7.5, 8.5)).line_to(p(12.0, 4.0)).line_to(p(16.5, 8.5));
            }
        }
        Icon::Send => {
            s.move_to(p(3.0, 11.0))
                .line_to(p(21.0, 3.0))
                .line_to(p(13.0, 21.0))
                .line_to(p(11.0, 13.0))
                .close();
            s.move_to(p(11.0, 13.0)).line_to(p(21.0, 3.0));
        }
        Icon::ArrowLeft => {
            s.move_to(p(19.0, 12.0))
                .line_to(p(5.0, 12.0))
                .move_to(p(11.0, 6.0))
                .line_to(p(5.0, 12.0))
                .line_to(p(11.0, 18.0));
        }
        Icon::ArrowRight => {
            s.move_to(p(5.0, 12.0))
                .line_to(p(19.0, 12.0))
                .move_to(p(13.0, 6.0))
                .line_to(p(19.0, 12.0))
                .line_to(p(13.0, 18.0));
        }
        Icon::ArrowUp => {
            s.move_to(p(12.0, 19.0))
                .line_to(p(12.0, 5.0))
                .move_to(p(6.0, 11.0))
                .line_to(p(12.0, 5.0))
                .line_to(p(18.0, 11.0));
        }
        Icon::Users => {
            circle(&mut s, p(9.0, 8.0), 3.5);
            arc(&mut s, p(9.0, 20.0), 6.0, PI, 2.0 * PI);
            arc(&mut s, p(16.0, 8.0), 3.0, -PI * 0.5, PI * 0.5);
            arc(&mut s, p(16.5, 20.0), 5.0, -PI * 0.5, 0.0);
        }
        Icon::Monitor => {
            rect(&mut s, 3.0, 4.0, 21.0, 16.0);
            s.move_to(p(12.0, 16.0)).line_to(p(12.0, 20.0)).move_to(p(8.0, 20.0)).line_to(p(16.0, 20.0));
        }
    }
    s
}

/// Draws `icon` into `cx` (its full size) in the element's text color.
pub(crate) fn draw(icon: Icon, cx: &mut PaintCx<'_>) {
    let k = cx.size.width.min(cx.size.height) / 24.0;
    let (ox, oy) = ((cx.size.width - 24.0 * k) / 2.0, (cx.size.height - 24.0 * k) / 2.0);
    let path = strokes(icon);
    let mut scaled = Path::new();
    for el in path.elements() {
        let t = |q: Point| p(ox + q.x * k, oy + q.y * k);
        match *el {
            remedy_ui_paint::PathEl::MoveTo(a) => {
                scaled.move_to(t(a));
            }
            remedy_ui_paint::PathEl::LineTo(a) => {
                scaled.line_to(t(a));
            }
            remedy_ui_paint::PathEl::QuadTo(a, b) => {
                scaled.quad_to(t(a), t(b));
            }
            remedy_ui_paint::PathEl::CubicTo(a, b, c) => {
                scaled.cubic_to(t(a), t(b), t(c));
            }
            remedy_ui_paint::PathEl::Close => {
                scaled.close();
            }
        }
    }
    let stroke = Stroke {
        width: (2.0 * k).max(1.0),
        cap: StrokeCap::Round,
        join: StrokeJoin::Round,
        dashes: Vec::new(),
    };
    cx.list.push(remedy_ui_paint::Item::StrokePath {
        path: scaled,
        stroke,
        brush: Brush::Solid(cx.style.foreground),
    });
}

/// An icon `size` logical pixels square, decorative (its parent names it).
#[track_caller]
pub fn icon(ui: &Ui, icon: Icon, size: f32) -> El {
    ui.el("icon")
        .width(size)
        .height(size)
        .layout(|l| l.shrink = 0.0)
        .paint(move |cx| draw(icon, cx))
        .semantics(Semantics::new(Role::Image).hidden(true))
}

/// An image (premultiplied RGBA), fitted into the element and described by `alt`.
#[track_caller]
pub fn image(ui: &Ui, image: remedy_ui_paint::Image, alt: impl Into<String>) -> El {
    ui.el("image").image(image).semantics(Semantics::new(Role::Image).label(alt.into()))
}

/// A round badge with initials on a color: a person or an item without a picture.
#[track_caller]
pub fn avatar(ui: &Ui, name: &str, size: f32, color: remedy_ui_core::Color) -> El {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    let fg = remedy_ui_style::ensure_contrast(ui.theme().colors.text, color, 4.5);
    ui.el("avatar")
        .width(size)
        .height(size)
        .row()
        .align(remedy_ui_layout::Align::Center)
        .justify(remedy_ui_layout::Justify::Center)
        // The circle is the element's own background, so contrast checks see what the
        // initials sit on.
        .style(remedy_ui_style::StylePatch {
            background: Some(color),
            radius: Some(size / 2.0),
            ..Default::default()
        })
        .child(ui.el("avatar_initials").text(remedy_ui_view::RichText::plain(initials)).nowrap().style(
            remedy_ui_style::StylePatch {
                foreground: Some(fg),
                weight: Some(700.0),
                text_size: Some(size * 0.4),
                ..Default::default()
            },
        ))
        .semantics(Semantics::new(Role::Image).label(name.to_owned()))
}

/// A drawing surface: `paint` records into the element's display list each time it is
/// repainted ([`El::repaint`]).
#[track_caller]
pub fn canvas(ui: &Ui, paint: impl Fn(&mut PaintCx<'_>) + 'static) -> El {
    ui.el("canvas").paint(paint)
}

#[cfg(test)]
mod tests {
    use super::{Icon, strokes};

    const ALL: [Icon; 34] = [
        Icon::Plus,
        Icon::Minus,
        Icon::Maximize,
        Icon::Restore,
        Icon::Close,
        Icon::Check,
        Icon::Lock,
        Icon::Unlock,
        Icon::Pin,
        Icon::Trash,
        Icon::Copy,
        Icon::Eye,
        Icon::EyeOff,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronLeft,
        Icon::ChevronUp,
        Icon::Menu,
        Icon::Search,
        Icon::Info,
        Icon::Alert,
        Icon::Clock,
        Icon::Key,
        Icon::Settings,
        Icon::Note,
        Icon::External,
        Icon::Refresh,
        Icon::Folder,
        Icon::File,
        Icon::Download,
        Icon::Upload,
        Icon::Send,
        Icon::ArrowLeft,
        Icon::ArrowRight,
    ];

    #[test]
    fn every_icon_draws_inside_its_grid() {
        for icon in ALL.into_iter().chain([Icon::ArrowUp, Icon::Users, Icon::Monitor]) {
            let path = strokes(icon);
            assert!(path.elements().len() > 1, "{icon:?} draws nothing");
            for el in path.elements() {
                if let remedy_ui_paint::PathEl::MoveTo(q) | remedy_ui_paint::PathEl::LineTo(q) = *el {
                    assert!(
                        (0.0..=24.0).contains(&q.x) && (0.0..=24.0).contains(&q.y),
                        "{icon:?} leaves the grid at {q:?}"
                    );
                }
            }
        }
    }
}
