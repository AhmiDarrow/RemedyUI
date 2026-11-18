//! What built elements are made of. The interpreter and the code the compiler writes both
//! call these, so an interpreted file and a compiled one build the same elements.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use remedy_ui_core::Insets;
pub use remedy_ui_layout as layout;
use remedy_ui_layout::{Align, Justify};
pub use remedy_ui_reactive as reactive;
use remedy_ui_reactive::Signal;
pub use remedy_ui_view as view;
use remedy_ui_view::{El, TextField, Ui};
pub use remedy_ui_widgets_core as w;

/// A space: grid units from the theme, or pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Space {
    /// `s3`: three of the theme's grid units.
    Units(u8),
    /// `12`: pixels.
    Px(f32),
}

impl Space {
    /// In pixels, by `ui`'s theme.
    pub fn px(self, ui: &Ui) -> f32 {
        match self {
            Space::Units(n) => ui.theme().spacing.s(n),
            Space::Px(p) => p,
        }
    }
}

/// Layout and style every element takes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Common {
    /// Space between children.
    pub gap: Option<Space>,
    /// Space inside the edges.
    pub padding: Option<Space>,
    /// How much spare room it takes.
    pub grow: Option<f32>,
    /// It fills its parent's width.
    pub fill_width: bool,
    /// Its style's variant.
    pub variant: Option<&'static str>,
    /// How children line up across.
    pub align: Option<Align>,
    /// How children spread along.
    pub justify: Option<Justify>,
    /// Shown only while this is true.
    pub visible: Option<Signal<bool>>,
}

/// A variant's name, held for the life of the program (each name once, however many files
/// and reloads use it).
pub fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = NAMES.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(n) = set.get(name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_owned().into_boxed_str());
    set.insert(n);
    n
}

/// `align: center` as an alignment.
pub fn align(word: &str) -> Option<Align> {
    Some(match word {
        "start" => Align::Start,
        "center" => Align::Center,
        "end" => Align::End,
        "stretch" => Align::Stretch,
        "baseline" => Align::Baseline,
        _ => return None,
    })
}

/// `justify: end` as a justification.
pub fn justify(word: &str) -> Option<Justify> {
    Some(match word {
        "start" => Justify::Start,
        "center" => Justify::Center,
        "end" => Justify::End,
        "space_between" => Justify::SpaceBetween,
        "space_around" => Justify::SpaceAround,
        _ => return None,
    })
}

/// Applies `c` to `el` and adds its `children`.
pub fn finish(ui: &Ui, el: El, c: &Common, children: Vec<El>) -> El {
    let mut el = el;
    if let Some(g) = c.gap {
        el = el.gap(g.px(ui));
    }
    if let Some(p) = c.padding {
        el = el.padding(Insets::all(p.px(ui)));
    }
    if let Some(g) = c.grow {
        el = el.grow(g);
    }
    if c.fill_width {
        el = el.fill_width();
    }
    if let Some(v) = c.variant {
        el = el.variant(v);
    }
    if let Some(a) = c.align {
        el = el.align(a);
    }
    if let Some(j) = c.justify {
        el = el.justify(j);
    }
    if let Some(shown) = c.visible {
        let e = el.clone();
        el = el.bind(move || {
            let _ = e.clone().hidden(!shown.get());
        });
    }
    for child in children {
        el = el.child(child);
    }
    el
}

/// Text that follows `text`, on screen and to screen readers.
pub fn live_text(el: El, text: Signal<String>) -> El {
    let e = el.clone();
    el.bind(move || {
        let now = text.get();
        let named = e.info().and_then(|i| i.semantics).filter(|s| s.name().is_some());
        let e = e.clone().text(remedy_ui_view::RichText::plain(now.clone()));
        if let Some(s) = named {
            let _ = e.semantics(s.label(now));
        }
    })
}

/// A field whose text and `value` follow each other.
pub fn bound_field(field: TextField, value: Signal<String>) -> El {
    if let Ok(v) = value.get_untracked() {
        field.set_value(&v);
    }
    let f = field.clone();
    field.observe_text(move || {
        let now = f.value();
        if value.get_untracked().is_ok_and(|v| v != now) {
            value.set(now);
        }
    });
    let f = field.clone();
    field.el().bind(move || {
        let v = value.get();
        if f.value() != v {
            f.set_value(&v);
        }
    })
}

/// An element built from a list of choices, built again whenever the list changes (the
/// chosen index lives in the app's signal, so it carries over). The element gets `c`'s style;
/// the slot holding it takes its place in the layout and its visibility.
pub fn with_items(
    ui: &Ui,
    c: &Common,
    items: Signal<Vec<String>>,
    build: impl Fn(&[String]) -> El + 'static,
) -> El {
    let inner = Common { visible: None, ..*c };
    let outer = Common { gap: None, padding: None, variant: None, align: None, justify: None, ..*c };
    let slot = finish(ui, ui.el("rui_items").column(), &outer, Vec::new());
    let (s, ui2) = (slot.clone(), ui.clone());
    slot.bind(move || {
        let list = items.get();
        s.clear();
        let _ = s.clone().child(finish(&ui2, build(&list), &inner, Vec::new()));
    })
}

/// `items` as the slices some widgets take.
pub fn strs(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

pub use crate::spec::default_step;

/// A heading level from markup: 1 to 6 (levels past 3 share the third level's style).
pub fn heading_level(level: f64) -> u8 {
    // The checker keeps numbers finite; the cast saturates.
    level.round().clamp(1.0, 6.0) as u8
}
