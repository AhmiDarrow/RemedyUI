//! The browser's events and values in RemedyUI's terms: keys, wheel units, pinches, pointer
//! buttons, ARIA roles for the accessibility mirror, pixels for a 2D canvas, the page's
//! settings, and text offsets between JavaScript's UTF-16 and Rust's UTF-8. Platform-neutral,
//! so it is tested on any machine.

use std::ops::Range;
use std::time::Duration;

use remedy_ui_core::{Color, Rect, Size, Vec2};
use remedy_ui_host::SystemSettings;
use remedy_ui_host::accesskit::{Node, Role, Toggled};
use remedy_ui_input::{Buttons, Key, Modifiers, Platform, PointerKind};

/// CSS pixels one DOM "line" scrolls (`WheelEvent.DOM_DELTA_LINE`): a line of 16 px
/// interface text. A wheel notch is usually three lines, 48 px, as on the desktop bridges.
pub const LINE_PIXELS: f32 = 16.0;

/// `WheelEvent.deltaMode` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaMode {
    /// `DOM_DELTA_PIXEL`.
    Pixel,
    /// `DOM_DELTA_LINE`.
    Line,
    /// `DOM_DELTA_PAGE`.
    Page,
}

impl DeltaMode {
    /// From `WheelEvent.deltaMode` (0, 1 or 2; anything else counts as pixels).
    pub fn from_dom(mode: u32) -> DeltaMode {
        match mode {
            1 => DeltaMode::Line,
            2 => DeltaMode::Page,
            _ => DeltaMode::Pixel,
        }
    }
}

/// A wheel event's movement in logical pixels, in the engine's direction (positive scrolls
/// toward the end, as the DOM's), and whether it came from a precise device (a touchpad).
/// `page` is the window's size, which a page-mode wheel scrolls by.
pub fn wheel_delta(dx: f64, dy: f64, mode: DeltaMode, page: Size) -> (Vec2, bool) {
    let (dx, dy) = (dx as f32, dy as f32);
    match mode {
        DeltaMode::Line => (Vec2::new(dx * LINE_PIXELS, dy * LINE_PIXELS), false),
        DeltaMode::Page => (Vec2::new(dx * page.width, dy * page.height), false),
        DeltaMode::Pixel => {
            // Browsers report a mouse wheel's notches in pixels too (100 or 120 a notch in
            // Chrome and Edge, 3 lines' worth elsewhere); a touchpad's are small and uneven.
            let notch = |d: f32| {
                let a = d.abs();
                a >= 40.0 && [100.0_f32, 120.0, 53.0, 48.0, 40.0].iter().any(|n| (a / n).fract() == 0.0)
            };
            let precise = !(notch(dx) || notch(dy)) || dx.fract() != 0.0 || dy.fract() != 0.0;
            (Vec2::new(dx, dy), precise)
        }
    }
}

/// The scale change of a touchpad pinch, which browsers report as a wheel event with Ctrl
/// set and the pinch in `deltaY` (negative spreads the fingers).
pub fn pinch_scale(delta_y: f64, mode: DeltaMode) -> f32 {
    let px = match mode {
        DeltaMode::Pixel => delta_y,
        DeltaMode::Line => delta_y * f64::from(LINE_PIXELS),
        DeltaMode::Page => delta_y * 800.0,
    };
    // Chrome, Firefox and Safari map one pixel of pinch to a 1% change, compounding.
    ((-px / 100.0).exp() as f32).clamp(0.01, 100.0)
}

/// Modifier keys from an event's flags.
pub fn modifiers(shift: bool, ctrl: bool, alt: bool, meta: bool) -> Modifiers {
    Modifiers { shift, ctrl, alt, meta }
}

/// Pointer buttons from `MouseEvent.buttons` (the same bit order: primary, secondary,
/// middle, back, forward).
pub fn buttons(dom: u16) -> Buttons {
    Buttons((dom & 0x1f) as u8)
}

/// What made a pointer event, from `PointerEvent.pointerType`.
pub fn pointer_kind(dom: &str) -> PointerKind {
    match dom {
        "touch" => PointerKind::Touch,
        "pen" => PointerKind::Pen,
        _ => PointerKind::Mouse,
    }
}

/// The engine's pointer id: the mouse is 0, every finger and pen its own nonzero id.
pub fn pointer_id(dom_id: i32, kind: PointerKind) -> u32 {
    match kind {
        PointerKind::Mouse => 0,
        // Fingers, pens and kinds newer than this bridge each have an id of their own.
        _ => u32::try_from(dom_id).unwrap_or(0).max(1),
    }
}

/// A key's meaning from `KeyboardEvent.key`.
pub fn key(dom: &str) -> Key {
    match dom {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Escape" | "Esc" => Key::Escape,
        "Backspace" => Key::Backspace,
        "Delete" | "Del" => Key::Delete,
        " " | "Spacebar" => Key::Space,
        "ArrowLeft" | "Left" => Key::ArrowLeft,
        "ArrowRight" | "Right" => Key::ArrowRight,
        "ArrowUp" | "Up" => Key::ArrowUp,
        "ArrowDown" | "Down" => Key::ArrowDown,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "BrowserBack" | "GoBack" => Key::Back,
        other => {
            if let Some(n) = other.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
                && (1..=24).contains(&n)
            {
                return Key::F(n);
            }
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::char(c),
                // One user-perceived character spelled with several code points (an emoji
                // with a modifier, a letter with a combining mark).
                _ if !other.is_empty() && other.chars().all(|c| !c.is_ascii_alphanumeric()) => {
                    Key::Character(other.to_lowercase())
                }
                _ => Key::Named(other.to_owned()),
            }
        }
    }
}

/// The text a key press types: the key's own characters, unless Ctrl or Command (without
/// AltGr, which browsers report as Ctrl+Alt on Windows) makes it a shortcut, or it is a
/// named key.
pub fn typed_text(dom_key: &str, mods: Modifiers) -> Option<String> {
    let shortcut = (mods.ctrl && !mods.alt) || mods.meta;
    let single = {
        let mut c = dom_key.chars();
        c.next().is_some() && (dom_key.chars().count() == 1 || matches!(key(dom_key), Key::Character(_)))
    };
    let printable = single && !dom_key.chars().any(char::is_control);
    (printable && !shortcut).then(|| dom_key.to_owned())
}

/// True for key presses an input method is handling (`isComposing`, or the key the browser
/// reports for keys an input method took: `Process`, code 229).
pub fn is_ime_key(dom_key: &str, key_code: u32, composing: bool) -> bool {
    composing || dom_key == "Process" || key_code == 229
}

/// Clipboard shortcuts, which must reach the browser so it raises `copy`, `cut` and
/// `paste` events (the only time a page may write or read the clipboard without asking).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardKey {
    /// Copy.
    Copy,
    /// Cut.
    Cut,
    /// Paste.
    Paste,
}

/// Which clipboard shortcut a key press is, on `platform`.
pub fn clipboard_key(k: &Key, mods: Modifiers, platform: Platform) -> Option<ClipboardKey> {
    let primary = match platform {
        Platform::Apple => mods.meta && !mods.ctrl,
        // Every platform but Apple's uses Ctrl.
        _ => mods.ctrl && !mods.meta,
    };
    match k {
        Key::Character(c) if primary && !mods.alt => match c.as_str() {
            "c" => Some(ClipboardKey::Copy),
            "x" => Some(ClipboardKey::Cut),
            "v" => Some(ClipboardKey::Paste),
            _ => None,
        },
        Key::Named(n) if n == "Insert" && mods.shift && !mods.ctrl => Some(ClipboardKey::Paste),
        Key::Named(n) if n == "Insert" && mods.ctrl && !mods.shift => Some(ClipboardKey::Copy),
        Key::Delete if mods.shift && !mods.ctrl => Some(ClipboardKey::Cut),
        Key::Named(n) if n == "Copy" => Some(ClipboardKey::Copy),
        Key::Named(n) if n == "Cut" => Some(ClipboardKey::Cut),
        Key::Named(n) if n == "Paste" => Some(ClipboardKey::Paste),
        _ => None,
    }
}

/// Keys the browser keeps for itself even while an app has the keyboard: reloading, the
/// developer tools, full screen, and tab and window management. The app still hears them,
/// but their browser action is not cancelled.
pub fn browser_keeps(k: &Key, mods: Modifiers, platform: Platform) -> bool {
    let primary = match platform {
        Platform::Apple => mods.meta,
        _ => mods.ctrl,
    };
    match k {
        Key::F(5 | 11 | 12) => true,
        Key::Tab if primary => true,
        Key::PageUp | Key::PageDown if primary => true,
        Key::Character(c) if primary => {
            matches!(c.as_str(), "r" | "t" | "w" | "n" | "l" | "q")
                || (mods.shift && matches!(c.as_str(), "i" | "j" | "c" | "k"))
        }
        _ => false,
    }
}

/// The ARIA role that presents an AccessKit role in a browser, or `None` for nodes that are
/// plain containers or text.
pub fn aria_role(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Window => "application",
        Role::Group | Role::Pane => "group",
        Role::Button | Role::DefaultButton | Role::ColorWell => "button",
        Role::CheckBox => "checkbox",
        Role::Switch => "switch",
        Role::RadioButton => "radio",
        Role::RadioGroup => "radiogroup",
        Role::Slider => "slider",
        Role::SpinButton => "spinbutton",
        Role::TextInput
        | Role::MultilineTextInput
        | Role::PasswordInput
        | Role::SearchInput
        | Role::EmailInput
        | Role::NumberInput
        | Role::PhoneNumberInput
        | Role::UrlInput
        | Role::DateInput
        | Role::DateTimeInput
        | Role::TimeInput
        | Role::WeekInput
        | Role::MonthInput => "textbox",
        Role::ComboBox | Role::EditableComboBox => "combobox",
        Role::Link => "link",
        Role::List => "list",
        Role::ListItem => "listitem",
        Role::ListBox => "listbox",
        Role::ListBoxOption => "option",
        Role::Tab => "tab",
        Role::TabList => "tablist",
        Role::TabPanel => "tabpanel",
        Role::Menu => "menu",
        Role::MenuBar => "menubar",
        Role::MenuItem => "menuitem",
        Role::MenuItemCheckBox => "menuitemcheckbox",
        Role::MenuItemRadio => "menuitemradio",
        Role::Dialog => "dialog",
        Role::AlertDialog => "alertdialog",
        Role::Alert => "alert",
        Role::Status => "status",
        Role::Log => "log",
        Role::Timer => "timer",
        Role::Tooltip => "tooltip",
        Role::Heading => "heading",
        Role::Image => "img",
        Role::ProgressIndicator => "progressbar",
        Role::Meter => "meter",
        Role::ScrollBar => "scrollbar",
        Role::Splitter => "separator",
        Role::Toolbar => "toolbar",
        Role::Grid => "grid",
        Role::GridCell => "gridcell",
        Role::Table => "table",
        Role::Row => "row",
        Role::Cell => "cell",
        Role::ColumnHeader => "columnheader",
        Role::RowHeader => "rowheader",
        Role::Tree => "tree",
        Role::TreeItem => "treeitem",
        Role::TreeGrid => "treegrid",
        Role::Form => "form",
        Role::Article => "article",
        Role::Navigation => "navigation",
        Role::Main => "main",
        Role::Banner => "banner",
        Role::Complementary => "complementary",
        Role::ContentInfo => "contentinfo",
        Role::Search => "search",
        Role::Region => "region",
        Role::Document => "document",
        Role::Figure => "figure",
        Role::Note => "note",
        Role::Math => "math",
        Role::Term => "term",
        Role::Definition => "definition",
        Role::Code => "code",
        Role::Paragraph => "paragraph",
        Role::Caption => "caption",
        Role::Blockquote => "blockquote",
        _ => return None,
    })
}

/// True for text-entry roles, whose value is their content.
pub fn is_text_entry(role: Role) -> bool {
    aria_role(role) == Some("textbox")
}

/// True for roles whose value is a number in a range.
pub fn is_range(role: Role) -> bool {
    matches!(role, Role::Slider | Role::SpinButton | Role::ProgressIndicator | Role::Meter | Role::ScrollBar)
}

/// The ARIA attributes a node shows besides its role, in a fixed order: every attribute the
/// mirror may set, with `None` for those this node leaves unset (so stale ones are removed).
pub fn aria_attributes(node: &Node) -> Vec<(&'static str, Option<String>)> {
    let role = node.role();
    let flag = |on: bool| on.then(|| "true".to_owned());
    let toggled = node.toggled().map(|t| match t {
        Toggled::True => "true".to_owned(),
        Toggled::False => "false".to_owned(),
        Toggled::Mixed => "mixed".to_owned(),
    });
    let checkable = matches!(
        role,
        Role::CheckBox | Role::Switch | Role::RadioButton | Role::MenuItemCheckBox | Role::MenuItemRadio
    );
    let password = role == Role::PasswordInput;
    let range = is_range(role);
    let label = node.label().map(str::to_owned);
    vec![
        ("aria-label", label),
        ("aria-description", node.description().map(str::to_owned)),
        ("aria-disabled", flag(node.is_disabled())),
        ("aria-checked", toggled.clone().filter(|_| checkable)),
        ("aria-pressed", toggled.filter(|_| !checkable && role == Role::Button)),
        ("aria-selected", node.is_selected().map(|s| s.to_string())),
        ("aria-expanded", node.is_expanded().map(|s| s.to_string())),
        ("aria-required", flag(node.is_required())),
        ("aria-readonly", flag(node.is_read_only())),
        ("aria-busy", flag(node.is_busy())),
        ("aria-invalid", node.invalid().map(|_| "true".to_owned())),
        ("aria-multiline", flag(role == Role::MultilineTextInput)),
        ("aria-level", node.level().map(|l| l.to_string())),
        ("aria-posinset", node.position_in_set().map(|p| p.to_string())),
        ("aria-setsize", node.size_of_set().map(|s| s.to_string())),
        ("aria-valuenow", node.numeric_value().filter(|_| range).map(number)),
        ("aria-valuemin", node.min_numeric_value().filter(|_| range).map(number)),
        ("aria-valuemax", node.max_numeric_value().filter(|_| range).map(number)),
        ("aria-valuetext", node.value().filter(|_| range && !password).map(str::to_owned)),
        (
            "aria-live",
            node.live().and_then(|l| match l {
                remedy_ui_host::accesskit::Live::Off => None,
                remedy_ui_host::accesskit::Live::Polite => Some("polite".to_owned()),
                remedy_ui_host::accesskit::Live::Assertive => Some("assertive".to_owned()),
            }),
        ),
        ("aria-hidden", flag(node.is_hidden())),
    ]
}

/// The text a node shows as its content (text and labels, and the text of a field); never a
/// password's.
pub fn text_content(node: &Node) -> Option<String> {
    let role = node.role();
    if role == Role::PasswordInput || !node.children().is_empty() {
        return None;
    }
    match role {
        Role::Label | Role::TextRun => node.value().or(node.label()).map(str::to_owned),
        r if is_text_entry(r) => node.value().map(str::to_owned),
        _ => None,
    }
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

/// Byte offset in `text` for a UTF-16 offset (a DOM selection), clamped to the text and kept
/// on a character boundary.
pub fn utf16_to_byte(text: &str, utf16: u32) -> usize {
    let mut units = 0u32;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

/// UTF-16 offset for a byte offset in `text` (rounded down to a character boundary).
pub fn byte_to_utf16(text: &str, byte: usize) -> u32 {
    text.char_indices().take_while(|(i, _)| *i < byte).map(|(_, c)| c.len_utf16() as u32).sum()
}

/// What an edit the input method or soft keyboard made to the hidden field means, by
/// comparing the field's text before and after: the bytes it deleted before and after the
/// old caret, and the text it inserted there. `caret` is the old caret's byte offset.
pub fn edit_between(old: &str, new: &str, caret: usize) -> (usize, usize, String) {
    let caret = caret.min(old.len());
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .take_while(|((_, a), b)| a == b)
        .last()
        .map_or(0, |((i, a), _)| i + a.len_utf8());
    let suffix = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>()
        .min(old.len() - prefix)
        .min(new.len() - prefix);
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    let inserted = new.get(prefix..new_end).unwrap_or_default().to_owned();
    let before = caret.saturating_sub(prefix);
    let after = old_end.saturating_sub(caret.max(prefix));
    (before, after, inserted)
}

/// A device-pixel rectangle: `(x, y, width, height)`.
pub type PixelRect = (u32, u32, u32, u32);

/// The damaged rectangles of a `width` × `height` frame, clamped to it and rounded out to
/// whole pixels; the whole frame when there is no damage list.
pub fn damage_rects(damage: &[Rect], width: u32, height: u32) -> Vec<PixelRect> {
    if damage.is_empty() {
        return vec![(0, 0, width, height)];
    }
    damage
        .iter()
        .filter_map(|r| {
            let x0 = r.min.x.floor().clamp(0.0, width as f32) as u32;
            let y0 = r.min.y.floor().clamp(0.0, height as f32) as u32;
            let x1 = r.max.x.ceil().clamp(0.0, width as f32) as u32;
            let y1 = r.max.y.ceil().clamp(0.0, height as f32) as u32;
            (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
        })
        .collect()
}

/// The pixels of `rect` from a premultiplied RGBA8 frame `stride` pixels wide, as the
/// straight (not premultiplied) RGBA a canvas's `ImageData` holds.
pub fn straight_rgba(frame: &[u8], stride: u32, rect: PixelRect) -> Vec<u8> {
    let (x, y, w, h) = rect;
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * stride as usize + x as usize) * 4;
        let Some(line) = frame.get(start..start + w as usize * 4) else { break };
        for &[r, g, b, a] in line.as_chunks::<4>().0 {
            if a == 255 || a == 0 {
                out.extend_from_slice(&[r, g, b, a]);
            } else {
                let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                out.extend_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
    }
    out
}

/// What the page says about the user's preferences, read from its media queries and
/// navigator.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSettings {
    /// `(prefers-color-scheme: dark)` matches.
    pub prefers_dark: bool,
    /// `(prefers-color-scheme: light)` matches (neither means the browser does not say).
    pub prefers_light: bool,
    /// `(prefers-reduced-motion: reduce)`.
    pub reduced_motion: bool,
    /// `(prefers-contrast: more)` or `(forced-colors: active)`.
    pub high_contrast: bool,
    /// `(prefers-reduced-transparency: reduce)`.
    pub reduced_transparency: bool,
    /// `navigator.language`.
    pub language: String,
    /// The browser runs on macOS, iOS or iPadOS.
    pub apple: bool,
    /// `(hover: hover)`: the primary pointer hovers.
    pub hover: bool,
    /// `(pointer: fine)`: the primary pointer points finely.
    pub fine_pointer: bool,
    /// The root element's font size in CSS pixels (the user's default text size).
    pub root_font_px: f32,
    /// The system accent color (CSS `AccentColor`), where the browser reveals it.
    pub accent: Option<Color>,
}

/// The engine's settings from the page's.
pub fn settings(page: &PageSettings) -> SystemSettings {
    // The caret blink keeps the engine's default.
    let mut s = SystemSettings::default();
    s.platform = if page.apple { Platform::Apple } else { Platform::Standard };
    s.dark = if page.prefers_dark {
        Some(true)
    } else if page.prefers_light {
        Some(false)
    } else {
        None
    };
    s.accent = page.accent;
    s.high_contrast = page.high_contrast;
    // Browsers start at 16 px; a larger default is the user asking for larger text.
    s.text_scale =
        if page.root_font_px.is_finite() && page.root_font_px > 0.0 { page.root_font_px / 16.0 } else { 1.0 };
    s.reduced_motion = page.reduced_motion;
    s.reduced_transparency = page.reduced_transparency;
    if !page.language.trim().is_empty() {
        s.locale = page.language.clone();
    }
    // Browsers expose no double-click time; this is the platforms' default pages inherit.
    s.double_click = Duration::from_millis(500);
    s.pointer = Some(remedy_ui_host::PointerCaps::new(page.hover, page.fine_pointer));
    s
}

/// True for a user agent on Apple's platforms (`navigator.platform` or the user agent data's
/// platform).
pub fn is_apple(platform: &str) -> bool {
    let p = platform.to_ascii_lowercase();
    p.starts_with("mac") || p.contains("iphone") || p.contains("ipad") || p.contains("ipod") || p == "ios"
}

/// A CSS color as the browser serializes a computed one: `rgb(r, g, b)` or
/// `rgba(r, g, b, a)`, with the alpha optional.
pub fn parse_css_color(css: &str) -> Option<Color> {
    let s = css.trim();
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<f32> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    let channel = |v: f32| v.clamp(0.0, 255.0) / 255.0;
    match parts.as_slice() {
        [r, g, b] => Some(Color::new(channel(*r), channel(*g), channel(*b), 1.0)),
        [r, g, b, a] => Some(Color::new(channel(*r), channel(*g), channel(*b), a.clamp(0.0, 1.0))),
        _ => None,
    }
}

/// A length the browser computed in pixels (`"12px"`, `"0px"`), as a number.
pub fn parse_px(css: &str) -> f32 {
    css.trim()
        .strip_suffix("px")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

/// An `<input type=file>` `accept` list from a dialog's filters (`.png,.jpg`); empty
/// accepts everything.
pub fn accept_list(filters: &[(String, Vec<String>)]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (_, exts) in filters {
        for e in exts {
            let e = e.trim().trim_start_matches('.');
            if e.is_empty() || e == "*" {
                // A filter that allows everything allows everything.
                return String::new();
            }
            let dotted = format!(".{e}");
            if !out.contains(&dotted) {
                out.push(dotted);
            }
        }
    }
    out.join(",")
}

/// Byte ranges kept inside `text` and on character boundaries.
pub fn clamp_range(text: &str, r: &Range<usize>) -> Range<usize> {
    let fit = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (a, b) = (fit(r.start), fit(r.end));
    a.min(b)..a.max(b)
}

/// What a floating window's close button is called for screen readers, in the page's
/// language (`navigator.language`, BCP 47). The bridge sits below the view and its message
/// catalogs, so it keeps these few words itself: the same as the core widgets' close icon
/// (`icon-close` in `remedy-ui-widgets-core`), for the languages the engine ships; English
/// for any other.
pub fn close_label(language: &str) -> &'static str {
    let primary = language.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
    match primary.as_str() {
        "es" => "Cerrar",
        "zh" => "关闭",
        "hi" => "बंद करें",
        "ar" => "إغلاق",
        "fr" => "Fermer",
        "pt" => "Fechar",
        "ru" => "Закрыть",
        "ja" => "閉じる",
        "de" => "Schließen",
        "bn" => "বন্ধ করুন",
        _ => "Close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_by_meaning() {
        assert_eq!(key("Enter"), Key::Enter);
        assert_eq!(key("a"), Key::Character("a".into()));
        assert_eq!(key("A"), Key::Character("a".into()), "characters are lower case");
        assert_eq!(key(" "), Key::Space);
        assert_eq!(key("F7"), Key::F(7));
        assert_eq!(key("F25"), Key::Named("F25".into()));
        assert_eq!(key("BrowserBack"), Key::Back);
        assert_eq!(key("Insert"), Key::Named("Insert".into()));
        assert_eq!(key("Dead"), Key::Named("Dead".into()));
        assert_eq!(key("👍🏽"), Key::Character("👍🏽".into()));
    }

    #[test]
    fn typed_text_skips_shortcuts_but_keeps_altgr() {
        let none = Modifiers::NONE;
        let ctrl = Modifiers { ctrl: true, ..none };
        let altgr = Modifiers { ctrl: true, alt: true, ..none };
        assert_eq!(typed_text("a", none).as_deref(), Some("a"));
        assert_eq!(typed_text("A", Modifiers { shift: true, ..none }).as_deref(), Some("A"));
        assert_eq!(typed_text("c", ctrl), None);
        assert_eq!(typed_text("@", altgr).as_deref(), Some("@"));
        assert_eq!(typed_text("Enter", none), None);
        assert_eq!(typed_text("ArrowLeft", none), None);
        assert_eq!(typed_text("é", none).as_deref(), Some("é"));
    }

    #[test]
    fn clipboard_and_browser_shortcuts() {
        let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
        let cmd = Modifiers { meta: true, ..Modifiers::NONE };
        let v = Key::Character("v".into());
        assert_eq!(clipboard_key(&v, ctrl, Platform::Standard), Some(ClipboardKey::Paste));
        assert_eq!(clipboard_key(&v, cmd, Platform::Apple), Some(ClipboardKey::Paste));
        assert_eq!(clipboard_key(&v, cmd, Platform::Standard), None);
        let shift_ins = Modifiers { shift: true, ..Modifiers::NONE };
        assert_eq!(
            clipboard_key(&Key::Named("Insert".into()), shift_ins, Platform::Standard),
            Some(ClipboardKey::Paste)
        );
        assert!(browser_keeps(&Key::F(5), Modifiers::NONE, Platform::Standard));
        assert!(browser_keeps(&Key::Character("r".into()), ctrl, Platform::Standard));
        assert!(!browser_keeps(&Key::Character("s".into()), ctrl, Platform::Standard));
        assert!(!browser_keeps(&Key::Tab, Modifiers::NONE, Platform::Standard), "Tab moves focus in the app");
    }

    #[test]
    fn wheel_units_become_pixels() {
        let page = Size::new(800.0, 600.0);
        let (d, precise) = wheel_delta(0.0, 3.0, DeltaMode::Line, page);
        assert_eq!(d, Vec2::new(0.0, 48.0), "a notch of three lines is 48 px");
        assert!(!precise);
        let (d, _) = wheel_delta(0.0, -1.0, DeltaMode::Page, page);
        assert_eq!(d, Vec2::new(0.0, -600.0));
        let (d, precise) = wheel_delta(0.0, 100.0, DeltaMode::Pixel, page);
        assert_eq!(d, Vec2::new(0.0, 100.0));
        assert!(!precise, "Chrome's 100 px notch is a mouse wheel");
        let (_, precise) = wheel_delta(1.5, 7.25, DeltaMode::Pixel, page);
        assert!(precise, "a touchpad's fractional deltas are precise");
        let (_, precise) = wheel_delta(0.0, 12.0, DeltaMode::Pixel, page);
        assert!(precise);
    }

    #[test]
    fn pinches_compound_by_one_percent_a_pixel() {
        assert!((pinch_scale(0.0, DeltaMode::Pixel) - 1.0).abs() < 1e-6);
        assert!(pinch_scale(-10.0, DeltaMode::Pixel) > 1.0, "spreading zooms in");
        assert!(pinch_scale(10.0, DeltaMode::Pixel) < 1.0);
        let there = pinch_scale(-10.0, DeltaMode::Pixel) * pinch_scale(10.0, DeltaMode::Pixel);
        assert!((there - 1.0).abs() < 1e-5, "in and out again is no change");
    }

    #[test]
    fn pointers_and_buttons() {
        assert_eq!(buttons(1 | 2 | 32), Buttons(3), "unknown bits are dropped");
        assert_eq!(pointer_kind("pen"), PointerKind::Pen);
        assert_eq!(pointer_id(1, PointerKind::Mouse), 0, "the mouse is pointer 0");
        assert_eq!(pointer_id(0, PointerKind::Touch), 1, "fingers never take the mouse's id");
        assert_eq!(pointer_id(17, PointerKind::Touch), 17);
    }

    #[test]
    fn aria_roles_and_attributes() {
        assert_eq!(aria_role(Role::Button), Some("button"));
        assert_eq!(aria_role(Role::PasswordInput), Some("textbox"));
        assert_eq!(aria_role(Role::GenericContainer), None);
        let mut n = Node::new(Role::CheckBox);
        n.set_label("Wi-Fi");
        n.set_toggled(Toggled::True);
        let a = aria_attributes(&n);
        let get = |k: &str| a.iter().find(|(n, _)| *n == k).and_then(|(_, v)| v.clone());
        assert_eq!(get("aria-label").as_deref(), Some("Wi-Fi"));
        assert_eq!(get("aria-checked").as_deref(), Some("true"));
        assert_eq!(get("aria-pressed"), None);
        let mut s = Node::new(Role::Slider);
        s.set_numeric_value(150.0);
        s.set_min_numeric_value(0.0);
        s.set_max_numeric_value(200.5);
        let a = aria_attributes(&s);
        let get = |k: &str| a.iter().find(|(n, _)| *n == k).and_then(|(_, v)| v.clone());
        assert_eq!(get("aria-valuenow").as_deref(), Some("150"));
        assert_eq!(get("aria-valuemax").as_deref(), Some("200.5"));
        // Menus: a menu bar, check and radio items with their state, separators, and each
        // item's place in its menu.
        assert_eq!(aria_role(Role::MenuBar), Some("menubar"));
        assert_eq!(aria_role(Role::MenuItemCheckBox), Some("menuitemcheckbox"));
        assert_eq!(aria_role(Role::Splitter), Some("separator"));
        let mut r = Node::new(Role::MenuItemRadio);
        r.set_toggled(Toggled::False);
        r.set_position_in_set(2);
        r.set_size_of_set(3);
        let a = aria_attributes(&r);
        let get = |k: &str| a.iter().find(|(n, _)| *n == k).and_then(|(_, v)| v.clone());
        assert_eq!(aria_role(Role::MenuItemRadio), Some("menuitemradio"));
        assert_eq!(get("aria-checked").as_deref(), Some("false"));
        assert_eq!((get("aria-posinset").as_deref(), get("aria-setsize").as_deref()), (Some("2"), Some("3")));
        let mut p = Node::new(Role::PasswordInput);
        p.set_value("••••");
        assert_eq!(text_content(&p), None, "a password's text never reaches the page");
        let mut l = Node::new(Role::Label);
        l.set_value("Hello");
        assert_eq!(text_content(&l).as_deref(), Some("Hello"));
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let t = "a😀b";
        assert_eq!(utf16_to_byte(t, 0), 0);
        assert_eq!(utf16_to_byte(t, 1), 1);
        assert_eq!(utf16_to_byte(t, 3), 5, "the emoji is two UTF-16 units and four bytes");
        assert_eq!(utf16_to_byte(t, 99), t.len());
        assert_eq!(byte_to_utf16(t, 5), 3);
        assert_eq!(byte_to_utf16(t, t.len()), 4);
    }

    #[test]
    fn edits_from_the_hidden_field() {
        assert_eq!(edit_between("helo", "hello", 3), (0, 0, "l".into()));
        assert_eq!(edit_between("hello", "hell", 5), (1, 0, String::new()), "backspace");
        assert_eq!(edit_between("teh cat", "the cat", 3), (2, 0, "he".into()), "an autocorrection");
        assert_eq!(edit_between("ab", "ab", 1), (0, 0, String::new()));
        assert_eq!(edit_between("", "日本", 0), (0, 0, "日本".into()));
        assert_eq!(edit_between("xyz", "xz", 1), (0, 1, String::new()), "forward delete");
    }

    #[test]
    fn damage_and_straight_pixels() {
        let r = damage_rects(&[Rect::new(-2.0, 1.5, 3.2, 9.0)], 4, 4);
        assert_eq!(r, vec![(0, 1, 4, 3)]);
        assert_eq!(damage_rects(&[], 2, 3), vec![(0, 0, 2, 3)]);
        // One opaque pixel and one half-transparent premultiplied pixel.
        let frame = [10, 20, 30, 255, 64, 32, 0, 128];
        assert_eq!(straight_rgba(&frame, 2, (0, 0, 2, 1)), vec![10, 20, 30, 255, 128, 64, 0, 128]);
        assert_eq!(straight_rgba(&frame, 2, (1, 0, 1, 1)), vec![128, 64, 0, 128]);
    }

    #[test]
    fn page_settings_become_system_settings() {
        let page = PageSettings {
            prefers_dark: true,
            prefers_light: false,
            reduced_motion: true,
            high_contrast: false,
            reduced_transparency: false,
            language: "de-DE".into(),
            apple: true,
            hover: false,
            fine_pointer: true,
            root_font_px: 20.0,
            accent: parse_css_color("rgb(255, 0, 0)"),
        };
        let s = settings(&page);
        assert_eq!(s.dark, Some(true));
        assert_eq!(s.platform, Platform::Apple);
        assert!((s.text_scale - 1.25).abs() < 1e-6);
        assert_eq!(s.locale, "de-DE");
        assert!(s.reduced_motion);
        assert_eq!(s.pointer, Some(remedy_ui_host::PointerCaps::PEN), "fine but not hovering");
        assert_eq!(s.accent, Some(Color::new(1.0, 0.0, 0.0, 1.0)));
        let quiet = settings(&PageSettings { prefers_dark: false, language: String::new(), ..page });
        assert_eq!(quiet.dark, None, "neither scheme matched: the browser does not say");
        assert_eq!(quiet.locale, "en-US");
        assert!(
            is_apple("MacIntel") && is_apple("iPhone") && !is_apple("Win32") && !is_apple("Linux x86_64")
        );
        assert_eq!(parse_css_color("rgba(0, 0, 0, 0.5)"), Some(Color::new(0.0, 0.0, 0.0, 0.5)));
        assert_eq!(parse_css_color("AccentColor"), None);
        assert!((parse_px("12.5px") - 12.5).abs() < 1e-6);
        assert_eq!(parse_px("auto"), 0.0);
    }

    #[test]
    fn accept_lists_and_ranges() {
        let f = vec![
            ("Images".to_owned(), vec!["png".to_owned(), ".jpg".to_owned()]),
            ("PNG".into(), vec!["png".into()]),
        ];
        assert_eq!(accept_list(&f), ".png,.jpg");
        assert_eq!(accept_list(&[("All".into(), vec!["*".into()])]), "");
        assert_eq!(clamp_range("añb", &(2..9)), 1..4, "inside the text, on character boundaries");
    }

    #[test]
    fn the_close_button_is_named_in_the_page_language() {
        assert_eq!(close_label("de-DE"), "Schließen");
        assert_eq!(close_label("pt_BR"), "Fechar");
        assert_eq!(close_label("ZH-hans-CN"), "关闭");
        assert_eq!(close_label("en-US"), "Close");
        assert_eq!(close_label(""), "Close");
        assert_eq!(close_label("tlh"), "Close", "English where the engine has no words");
    }
}
