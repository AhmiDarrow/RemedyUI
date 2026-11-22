//! The page's settings: media queries, the language, the user's text size and accent color,
//! followed as they change; and the screen's safe area.

use remedy_ui_host::{HostEvent, SafeArea};
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use super::dom::{self, Listener};
use crate::convert::{self, PageSettings};

/// Media queries the settings come from.
const QUERIES: [&str; 10] = [
    "(prefers-color-scheme: dark)",
    "(prefers-color-scheme: light)",
    "(prefers-reduced-motion: reduce)",
    "(prefers-contrast: more)",
    "(forced-colors: active)",
    "(prefers-reduced-transparency: reduce)",
    "(any-pointer: coarse)",
    "(any-pointer: fine)",
    "(hover: hover)",
    "(pointer: fine)",
];

fn matches(win: &web_sys::Window, q: &str) -> bool {
    win.match_media(q).ok().flatten().is_some_and(|m| m.matches())
}

/// The system accent color, read through CSS `AccentColor` where the browser supports it.
fn accent(win: &web_sys::Window) -> Option<remedy_ui_core::Color> {
    let doc = win.document()?;
    let body = doc.body()?;
    let probe: HtmlElement = dom::element("span")?;
    // A sentinel first: a browser without `AccentColor` keeps it.
    dom::style(&probe, &[("display", "none"), ("color", "rgb(1, 2, 3)")]);
    let _ = probe.style().set_property("color", "AccentColor");
    let _ = body.append_child(&probe);
    let css = win.get_computed_style(&probe).ok().flatten().and_then(|s| s.get_property_value("color").ok());
    probe.remove();
    css.and_then(|c| convert::parse_css_color(&c))
        .filter(|c| *c != remedy_ui_core::Color::new(1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 1.0))
}

fn platform_name(win: &web_sys::Window) -> String {
    let nav = win.navigator();
    let data = dom::get(&nav, "userAgentData");
    let from_data = (!data.is_undefined()).then(|| dom::get(&data, "platform").as_string()).flatten();
    from_data.or_else(|| nav.platform().ok()).unwrap_or_default()
}

/// What the page says now.
pub(crate) fn read(win: &web_sys::Window) -> PageSettings {
    let root_font_px = win
        .document()
        .and_then(|d| d.document_element())
        .and_then(|e| win.get_computed_style(&e).ok().flatten())
        .and_then(|s| s.get_property_value("font-size").ok())
        .map_or(16.0, |v| convert::parse_px(&v));
    PageSettings {
        prefers_dark: matches(win, QUERIES[0]),
        prefers_light: matches(win, QUERIES[1]),
        reduced_motion: matches(win, QUERIES[2]),
        high_contrast: matches(win, QUERIES[3]) || matches(win, QUERIES[4]),
        reduced_transparency: matches(win, QUERIES[5]),
        coarse_pointer_only: matches(win, QUERIES[6]) && !matches(win, QUERIES[7]),
        tv: convert::is_tv(&win.navigator().user_agent().unwrap_or_default()),
        language: win.navigator().language().unwrap_or_default(),
        apple: convert::is_apple(&platform_name(win)),
        hover: matches(win, QUERIES[8]),
        fine_pointer: matches(win, QUERIES[9]),
        root_font_px: if root_font_px > 0.0 { root_font_px } else { 16.0 },
        accent: accent(win),
    }
}

/// The listeners that keep the settings current; dropping it stops following them.
pub(crate) struct Watch {
    _listeners: Vec<Listener>,
}

fn changed() {
    let Some(d) = super::driver() else { return };
    let Ok(win) = super::web_window() else { return };
    let now = convert::settings(&read(&win));
    let differs = {
        let mut s = d.state.borrow_mut();
        let differs = s.settings != now;
        s.settings = now.clone();
        differs
    };
    if differs {
        d.post(HostEvent::Settings(now));
    }
}

pub(crate) fn watch(win: &web_sys::Window) -> Watch {
    let mut ls = Vec::new();
    for q in QUERIES {
        if let Ok(Some(m)) = win.match_media(q) {
            ls.push(Listener::new(&m, "change", |_| changed()));
        }
    }
    ls.push(Listener::new(win, "languagechange", |_| changed()));
    Watch { _listeners: ls }
}

/// The screen's insets in CSS pixels: cutouts and rounded corners (`env(safe-area-inset-*)`,
/// with `viewport-fit=cover`), and the on-screen keyboard at the bottom.
pub(crate) fn safe_insets(win: &web_sys::Window) -> SafeArea {
    let mut area = SafeArea::default();
    if let Some(body) = win.document().and_then(|d| d.body())
        && let Some(probe) = dom::element::<HtmlElement>("div")
    {
        dom::style(
            &probe,
            &[
                ("position", "fixed"),
                ("visibility", "hidden"),
                ("pointer-events", "none"),
                ("padding-top", "env(safe-area-inset-top, 0px)"),
                ("padding-right", "env(safe-area-inset-right, 0px)"),
                ("padding-bottom", "env(safe-area-inset-bottom, 0px)"),
                ("padding-left", "env(safe-area-inset-left, 0px)"),
            ],
        );
        let _ = body.append_child(&probe);
        if let Ok(Some(s)) = win.get_computed_style(&probe) {
            let px = |k: &str| s.get_property_value(k).map_or(0.0, |v| convert::parse_px(&v));
            area = SafeArea {
                top: px("padding-top"),
                right: px("padding-right"),
                bottom: px("padding-bottom"),
                left: px("padding-left"),
            };
        }
        probe.remove();
    }
    if let Some(vv) = win.visual_viewport() {
        let inner = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
        let keyboard = (inner - (vv.offset_top() + vv.height())).max(0.0) as f32;
        area.bottom = area.bottom.max(keyboard);
    }
    area
}

/// The device's memory in bytes, as `navigator.deviceMemory` rounds it (browsers report a
/// coarse value, at most 8 GiB in Chrome, so pages cannot fingerprint the machine).
pub(crate) fn device_memory(win: &web_sys::Window) -> Option<u64> {
    let gib = dom::get(win.navigator().unchecked_ref(), "deviceMemory").as_f64()?;
    (gib.is_finite() && gib > 0.0).then_some((gib * 1024.0 * 1024.0 * 1024.0) as u64)
}
