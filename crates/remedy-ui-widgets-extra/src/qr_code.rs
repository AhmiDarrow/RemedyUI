//! The `qr_code` widget: shows a QR code for text that can change.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, Point, Rect, Vec2};
use remedy_ui_layout::Length;
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, TextPaint, Ui};

use crate::qr::{EncodeOptions, QrCode, QrError};

/// How a [`qr_code`] looks.
#[derive(Clone, Debug, Default)]
pub struct QrOptions {
    /// How the data is encoded (error correction level, version range, mask).
    pub encode: EncodeOptions,
    /// Light modules around the code that scanners need; `None` is the standard 4.
    pub quiet_zone: Option<u32>,
    /// The size of one module in logical pixels when the widget decides its own size; `None`
    /// takes one and a half theme units. The code is drawn on whole device pixels whatever
    /// size it is given.
    pub module_size: Option<f32>,
    /// What screen readers hear about the code. `None` says it is a QR code with its version
    /// and error correction level; the data itself is never read out unless the app says so.
    pub description: Option<String>,
}

struct Cache {
    key: Option<String>,
    value: Rc<Result<QrCode, QrError>>,
}

fn code_for(cache: &RefCell<Cache>, text: &str, options: &QrOptions) -> Rc<Result<QrCode, QrError>> {
    if let Ok(mut c) = cache.try_borrow_mut() {
        if c.key.as_deref() != Some(text) {
            c.value = Rc::new(QrCode::encode_text_with(text, options.encode));
            c.key = Some(text.to_owned());
        }
        return c.value.clone();
    }
    Rc::new(QrCode::encode_text_with(text, options.encode))
}

/// The module and background colors: dark on light whatever the theme, so scanners read it.
/// They are the theme's text and background colors, the darker one for the modules.
fn colors(theme: &remedy_ui_style::Theme) -> (Color, Color) {
    let text = theme.derive().text;
    let bg = theme.colors.bg;
    if bg.luminance() >= text.luminance() { (text, bg) } else { (bg, text) }
}

/// A QR code for `data`, redrawn when the text changes. Pairing codes, share links and
/// one-time setup secrets are the usual uses.
///
/// The code is dark modules on a light square in the theme's text and background colors
/// (swapped as needed so it is always dark on light), with the standard quiet zone. It is
/// drawn in whole device pixels so it scans at any scale. Text that is too long for any
/// version shows an explanation instead and reports itself invalid to assistive technology.
#[track_caller]
pub fn qr_code(ui: &Ui, label: impl Into<String>, data: Signal<String>, options: QrOptions) -> El {
    let label = label.into();
    let quiet = options.quiet_zone.unwrap_or(4) as usize;
    let module = options.module_size.unwrap_or(ui.theme().spacing.unit * 1.5).max(1.0);
    let cache = Rc::new(RefCell::new(Cache { key: None, value: Rc::new(Err(QrError::TooLong)) }));
    let options = Rc::new(options);
    // Space for the message shown when the data does not fit.
    let fallback = ui.theme().spacing.unit * 40.0;

    let paint_cache = cache.clone();
    let paint_options = options.clone();
    let el = ui.el("qr_code").paint(move |cx| {
        let text = data.get();
        let (dark, light) = colors(cx.theme);
        let result = code_for(&paint_cache, &text, &paint_options);
        let whole = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
        match &*result {
            Ok(code) => {
                let total = code.size() + 2 * quiet;
                let side = cx.size.width.min(cx.size.height);
                // Whole device pixels per module, so edges stay sharp and scanners see equal modules.
                let scale = cx.scale.max(1.0);
                let m = ((side * scale) / total as f32).floor().max(1.0) / scale;
                let used = m * total as f32;
                let ox = (cx.size.width - used) / 2.0;
                let oy = (cx.size.height - used) / 2.0;
                // Light all the way to the edges, so any margin counts as quiet zone.
                cx.list.fill_rect(whole, Brush::Solid(light));
                for y in 0..code.size() {
                    let mut x = 0;
                    while x < code.size() {
                        if code.module(x, y) {
                            let start = x;
                            while x < code.size() && code.module(x, y) {
                                x += 1;
                            }
                            let left = ox + (start + quiet) as f32 * m;
                            let top = oy + (y + quiet) as f32 * m;
                            cx.list.fill_rect(
                                Rect::new(left, top, left + (x - start) as f32 * m, top + m),
                                Brush::Solid(dark),
                            );
                        } else {
                            x += 1;
                        }
                    }
                }
            }
            Err(e) => {
                cx.list.fill_rounded(
                    whole,
                    remedy_ui_core::CornerRadii::all(cx.theme.radii.medium),
                    Brush::Solid(cx.theme.colors.card),
                );
                cx.text(
                    &e.to_string(),
                    Point::new(cx.size.width / 2.0, cx.size.height / 2.0),
                    Vec2::new(0.5, 0.5),
                    TextPaint {
                        size: cx.theme.type_scale.small,
                        weight: 400.0,
                        color: cx.theme.derive().muted,
                        mono: false,
                    },
                );
            }
        }
    });

    let bound = el.clone();
    el.bind(move || {
        let text = data.get();
        let result = code_for(&cache, &text, &options);
        let (side, sem) = match &*result {
            Ok(code) => {
                let total = (code.size() + 2 * quiet) as f32;
                let said = options.description.clone().unwrap_or_else(|| {
                    format!("QR code, version {}, error correction {}", code.version(), code.ecc().letter())
                });
                (total * module, Semantics::new(Role::Image).label(label.clone()).description(said))
            }
            Err(e) => (
                fallback,
                Semantics::new(Role::Image).label(label.clone()).description(e.to_string()).invalid(true),
            ),
        };
        let _ = bound.clone().semantics(sem);
        let _ = bound.clone().layout(|l| {
            l.width = Length::Px(side);
            l.height = Length::Px(side);
        });
        bound.repaint();
    })
}
