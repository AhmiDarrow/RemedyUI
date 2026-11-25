//! Elements and the builder components use.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::paint::DisplayList;
use remedy_ui_core::a11y::{A11yTree, Semantics};
use remedy_ui_core::layout::{Available, LayoutNode, LayoutStyle, LayoutTree, MeasureInput};
use remedy_ui_core::reactive::{Effect, Reaction};
use remedy_ui_core::style::{ResolvedStyle, States, StyleKey, StylePatch, StyleSheet, Theme};
use remedy_ui_core::tree::{Dirty, ElementId, HitMode, Phase, Tree};
use remedy_ui_core::{Insets, Size, Vec2};
use remedy_ui_text::{FontCollection, Paragraph, ParagraphOptions, RichText, TextEngine, TextStyle};

use crate::view::Request;
use crate::view::events::Click;
use crate::view::field::Field;
use crate::view::prop::{IntoProp, Prop};

/// What a custom painter gets: the list to record into (local coordinates, origin at the
/// element's top-left), the element's size, its resolved style and the theme.
pub struct PaintCx<'a> {
    /// Record here.
    pub list: &'a mut DisplayList,
    /// The element's size in logical pixels.
    pub size: Size,
    /// The element's box with each edge on the nearest device pixel, in local logical
    /// coordinates (near `0, 0, size`): where its background and border are drawn. Pictures
    /// placed on it land on whole device pixels, as browsers place an `<img>`.
    pub pixel_box: remedy_ui_core::Rect,
    /// Its resolved style.
    pub style: &'a ResolvedStyle,
    /// The window's theme.
    pub theme: &'a Theme,
    /// The element's states (checked, selected, hover, …).
    pub states: States,
    /// Device pixels per logical pixel (painters re-record when it changes).
    pub scale: f32,
    /// The painter asked to run again in the next frame ([`PaintCx::request_next_frame`]).
    pub(crate) next_frame: bool,
    /// The element lays out right to left (mirror what is directional).
    pub rtl: bool,
    /// The frame's time: the predicted presentation time of the frame being drawn, on the
    /// window's frame clock ([`Ui::frame_time`]). A painter that animates samples it and calls
    /// [`PaintCx::request_next_frame`] while it moves, so it needs no timer.
    pub time: std::time::Duration,
    /// The window asks for reduced motion: a painter that animates draws a still frame
    /// (and does not ask for the next one).
    pub reduced_motion: bool,
    /// The window's text engine, for painters that place glyphs themselves.
    pub(crate) engine: &'a mut TextEngine,
    /// The window's ambient rate now (`None`: paused).
    pub(crate) ambient_rate: Option<crate::view::AmbientRate>,
    /// When this element last got an ambient tick.
    pub(crate) last_tick: Option<std::time::Duration>,
    /// What the painter asked of the frame clock.
    pub(crate) ask: crate::view::ambient::PaintAsk,
}

/// How [`PaintCx::text`] draws a line of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPaint {
    /// Font size in logical pixels.
    pub size: f32,
    /// Weight, 100 to 900.
    pub weight: f32,
    /// Fill color.
    pub color: remedy_ui_core::Color,
    /// Use the theme's families for numbers and codes (same-width figures).
    pub mono: bool,
}

/// Renders `list` (in logical pixels, `size` big) to a picture at `scale` device pixels per
/// logical pixel, on a transparent background, with the CPU renderer. For art that is
/// expensive to draw and rarely changes (a scene's backdrop, a texture, a frame): draw it
/// once, keep the picture, and push it as an image. `None` for an empty or too-large size
/// (the CPU renderer's axis limit is 65,535 device pixels). Text in `list` must have been
/// laid out with the window's fonts (see [`PaintCx::cached`], which does that for you).
pub fn rasterize(list: &DisplayList, size: Size, scale: f32) -> Option<crate::paint::Image> {
    thread_local! {
        static CPU: std::cell::RefCell<crate::render_cpu::CpuRenderer> = std::cell::RefCell::new(crate::render_cpu::CpuRenderer::new());
    }
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = (size.width * scale).ceil();
    let h = (size.height * scale).ceil();
    if !(w >= 1.0 && h >= 1.0 && w <= 65_535.0 && h <= 65_535.0) {
        return None;
    }
    let (w, h) = (w as u32, h as u32);
    let frame =
        CPU.with(|r| r.borrow_mut().render(list, w, h, scale, remedy_ui_core::Color::TRANSPARENT)).ok()?;
    Some(crate::paint::Image::new(frame.width, frame.height, std::sync::Arc::from(frame.pixels)))
}

thread_local! {
    /// [`PaintCx::cached`]'s pictures: by key, with the size and scale they were drawn at.
    static CACHED: std::cell::RefCell<HashMap<u64, (u32, u32, u32, crate::paint::Image)>> = std::cell::RefCell::new(HashMap::new());
}

/// Most pictures [`PaintCx::cached`] keeps unless the app says otherwise
/// ([`set_paint_cache_limit`]); past it the cache starts again.
pub const PAINT_CACHE_LIMIT: usize = 512;

static CACHED_MAX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(PAINT_CACHE_LIMIT);

/// Sets the most pictures [`PaintCx::cached`] keeps on each UI thread (default
/// [`PAINT_CACHE_LIMIT`], 512); past it the cache starts again. Each picture costs its
/// element's device pixels at 4 bytes a pixel, so a raised limit costs that memory for every
/// picture kept; 0 keeps none (every picture is drawn again each frame).
pub fn set_paint_cache_limit(pictures: usize) {
    CACHED_MAX.store(pictures, std::sync::atomic::Ordering::Relaxed);
}

/// Drops every picture [`PaintCx::cached`] keeps (on memory pressure, or a theme change
/// the keys do not cover).
pub fn clear_paint_cache() {
    CACHED.with(|c| c.borrow_mut().clear());
}

impl PaintCx<'_> {
    /// The element's corner radii from its style, physical (start and end swapped right to
    /// left): what its background is drawn with, for a painter's picture or clip to match.
    pub fn corners(&self) -> remedy_ui_core::CornerRadii {
        crate::view::frame::corners(self.style, self.rtl)
    }

    /// Draws what `f` records once as a picture and reuses it while `key`, the element's
    /// size and the scale stay the same; then it is drawn again. Put everything the drawing
    /// depends on (the theme's colours, a seed, a state) into `key`. The picture is drawn
    /// at device resolution, so it stays sharp. What `f` draws is clipped to the element.
    ///
    /// ```ignore
    /// el.paint(|cx| {
    ///     cx.cached(hash(("backdrop", night)), |cx| draw_forest(cx)); // once
    ///     draw_fireflies(cx);                                          // every frame
    /// })
    /// ```
    pub fn cached(&mut self, key: u64, f: impl FnOnce(&mut PaintCx<'_>)) {
        let size = self.size;
        let scale = if self.scale.is_finite() && self.scale > 0.0 { self.scale } else { 1.0 };
        let dims = ((size.width * 64.0).round() as u32, (size.height * 64.0).round() as u32, scale.to_bits());
        let hit = CACHED.with(|c| {
            c.borrow().get(&key).filter(|(w, h, s, _)| (*w, *h, *s) == dims).map(|(_, _, _, i)| i.clone())
        });
        let image = match hit {
            Some(i) => Some(i),
            None => {
                let mut list = DisplayList::new();
                {
                    let mut inner = PaintCx {
                        list: &mut list,
                        size,
                        pixel_box: remedy_ui_core::Rect::from_origin_size(remedy_ui_core::Point::ZERO, size),
                        style: self.style,
                        theme: self.theme,
                        states: self.states,
                        scale,
                        next_frame: false,
                        rtl: self.rtl,
                        time: self.time,
                        reduced_motion: self.reduced_motion,
                        engine: &mut *self.engine,
                        // Cached art is a still picture: it never animates.
                        ambient_rate: None,
                        last_tick: None,
                        ask: crate::view::ambient::PaintAsk::default(),
                    };
                    f(&mut inner);
                }
                let img = rasterize(&list, size, scale);
                if let Some(img) = &img {
                    CACHED.with(|c| {
                        let mut c = c.borrow_mut();
                        let max = CACHED_MAX.load(std::sync::atomic::Ordering::Relaxed);
                        if c.len() >= max {
                            c.clear();
                        }
                        if max > 0 {
                            c.insert(key, (dims.0, dims.1, dims.2, img.clone()));
                        }
                    });
                }
                img
            }
        };
        if let Some(image) = image {
            self.list.push(crate::paint::Item::Image {
                image,
                rect: remedy_ui_core::Rect::new(0.0, 0.0, size.width, size.height),
                radii: remedy_ui_core::CornerRadii::ZERO,
                sampling: crate::paint::Sampling::Linear,
            });
        }
    }

    /// Draws this element again in the next frame, and asks for that frame: content that
    /// changes every frame (a game view, a live 3D preview) calls it from its painter each
    /// frame while it moves, so it follows the display's rate and stops when it stops asking.
    pub fn request_next_frame(&mut self) {
        self.next_frame = true;
    }

    /// Lays out `text` with the window's fonts, for a painter that draws text itself (a
    /// terminal placing each character in its cell, a chart's labels, a note on a page).
    pub fn layout_text(
        &mut self,
        text: &RichText,
        style: &TextStyle,
        options: ParagraphOptions,
    ) -> Paragraph {
        self.engine.layout(text, style, options)
    }

    /// Draws a laid-out paragraph with its top-left at `origin`, in `color` where its spans
    /// set none.
    pub fn draw_paragraph(
        &mut self,
        paragraph: &Paragraph,
        origin: remedy_ui_core::Point,
        color: remedy_ui_core::Color,
    ) {
        crate::view::frame::span_backgrounds(self.list, paragraph, origin);
        crate::view::frame::glyphs(self.list, paragraph, origin, color);
    }

    fn line(&mut self, text: &str, p: TextPaint) -> Paragraph {
        let style = {
            let mut v_ = TextStyle::default();
            v_.families = self.theme.families(p.mono, false);
            v_.size = p.size;
            v_.weight = p.weight;
            v_.color = p.color;
            v_
        };
        // One line placed by its anchor, so alignment inside it does not matter.
        let options = {
            let mut v_ = ParagraphOptions::default();
            v_.max_width = None;
            v_.scale = self.scale;
            v_
        };
        self.layout_text(&RichText::plain(text), &style, options)
    }

    /// The size one line of `text` takes when drawn with `p`.
    pub fn measure_text(&mut self, text: &str, p: TextPaint) -> Size {
        let para = self.line(text, p);
        Size::new(para.width(), para.height())
    }

    /// Draws one line of `text` with `p`, placed so the point `at` falls on the text box's
    /// `anchor` (`(0, 0)` its top-left corner, `(0.5, 0.5)` its middle, `(1, 1)` its
    /// bottom-right). Returns the size of the text. This is the canvas's way to write
    /// labels, numbers and names inside a drawing; the text is data, so it is not
    /// pseudo-localized.
    pub fn text(&mut self, text: &str, at: remedy_ui_core::Point, anchor: Vec2, p: TextPaint) -> Size {
        let para = self.line(text, p);
        let size = Size::new(para.width(), para.height());
        let origin = remedy_ui_core::Point::new(at.x - size.width * anchor.x, at.y - size.height * anchor.y);
        self.draw_paragraph(&para, origin, p.color);
        size
    }
}

pub(crate) type Painter = Rc<dyn Fn(&mut PaintCx<'_>)>;

/// What a cached paragraph was laid out for: wrap width bits, right-to-left and the line limit.
type ParagraphKey = (Option<u32>, bool, u32);

/// A text element's content and its last layout.
pub(crate) struct TextSlot {
    pub(crate) text: RichText,
    pub(crate) style: TextStyle,
    pub(crate) wrap: bool,
    /// At most this many lines, the last cut short with "…" ([`El::max_lines`]); 0 for no
    /// limit.
    pub(crate) max_lines: u32,
    /// Where each line sits across the box ([`El::text_align`]).
    pub(crate) align: remedy_ui_text::Align,
    pub(crate) scale: f32,
    /// The element lays out right to left: start-aligned text goes to the right edge.
    pub(crate) rtl: bool,
    /// The pseudo-locale the text is shown in, if one is active.
    pub(crate) pseudo: Option<remedy_ui_text::Pseudo>,
    /// The text is data (a date, a name, what someone wrote): pseudo-locales leave it.
    pub(crate) data: bool,
    /// The paragraph at the last wrap width asked for.
    pub(crate) cache: Option<(ParagraphKey, Rc<Paragraph>)>,
    /// For a huge paragraph laid out lazily: the band of its lines last drawn, and its height
    /// then.
    pub(crate) painted: Option<(std::ops::Range<f32>, f32)>,
}

/// A picture inside a paragraph: drawn where the text's inline object with the same `id`
/// is laid out ([`remedy_ui_text::RichText::object`]), and read by screen readers, copied and
/// searched as `alt` (a custom emoji's `:name:`, say).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineImage {
    /// The inline object it fills.
    pub id: u64,
    /// The picture, scaled into the object's box.
    pub image: crate::paint::Image,
    /// What it stands for in words.
    pub alt: String,
}

impl InlineImage {
    /// The picture `image` for inline object `id`, read as `alt`.
    pub fn new(id: u64, image: crate::paint::Image, alt: impl Into<String>) -> Self {
        InlineImage { id, image, alt: alt.into() }
    }
}

/// `text` with each inline object's U+FFFC replaced by its picture's words, for readers,
/// copying and searching.
pub(crate) fn with_alts(
    text: &RichText,
    images: &[InlineImage],
    range: Option<std::ops::Range<usize>>,
) -> String {
    let s = text.as_str();
    let range = range.unwrap_or(0..s.len());
    let Some(part) = s.get(range.clone()) else { return String::new() };
    if images.is_empty() || text.objects().is_empty() {
        return part.to_owned();
    }
    let mut out = String::with_capacity(part.len());
    let mut at = range.start;
    for (offset, obj) in text.objects() {
        if *offset < range.start || *offset >= range.end {
            continue;
        }
        out.push_str(&s[at..*offset]);
        match images.iter().find(|i| i.id == obj.id) {
            Some(i) => out.push_str(&i.alt),
            None => out.push('\u{fffc}'),
        }
        at = offset + '\u{fffc}'.len_utf8();
    }
    out.push_str(&s[at.min(range.end)..range.end]);
    out
}

/// `v` rounded up to the 1/64 px grid layout keeps text boxes on, as browsers do (Chrome's
/// layout unit is 1/64 px): a 21.12 px line stays 21.125 px instead of becoming 22, so stacked
/// lines do not drift, and a box sized to its text's width never wraps that text when laid
/// out again. Float noise just above a grid line stays on it.
pub(crate) fn layout_unit_ceil(v: f32) -> f32 {
    if !v.is_finite() {
        return v;
    }
    ((v * 64.0 - 1.0 / 256.0).ceil() / 64.0).max(0.0)
}

/// The paragraph container direction for an element's layout direction.
pub(crate) fn container(rtl: bool) -> remedy_ui_text::Direction {
    if rtl { remedy_ui_text::Direction::Rtl } else { remedy_ui_text::Direction::Ltr }
}

impl TextSlot {
    /// The text as shown: pseudo-localized when a pseudo-locale is on and it is not data.
    pub(crate) fn shown(&self) -> RichText {
        match self.pseudo.filter(|_| !self.data) {
            Some(p) => p.apply(&self.text),
            None => self.text.clone(),
        }
    }

    pub(crate) fn paragraph(&mut self, engine: &mut TextEngine, max_width: Option<f32>) -> Rc<Paragraph> {
        // A line limit wraps, `nowrap` or not: it is what the limit counts.
        let max_width = if self.wrap || self.max_lines > 0 { max_width } else { None };
        let key = (max_width.map(f32::to_bits), self.rtl, self.max_lines);
        if let Some((k, p)) = &self.cache
            && *k == key
        {
            return p.clone();
        }
        let options = {
            let mut v_ = ParagraphOptions::default();
            // Half a pixel of slack: a box sized to its text's width (rounded up) must not
            // wrap it again when the advances add up to exactly that width, which text that
            // may break between any two characters (Chinese, Japanese) would.
            v_.max_width = max_width.map(|w| w + 0.5);
            v_.scale = self.scale;
            v_.container = container(self.rtl);
            v_.max_lines = (self.max_lines > 0).then_some(self.max_lines as usize);
            v_.align = self.align;
            v_
        };
        let shown;
        let text = match self.pseudo.filter(|_| !self.data) {
            Some(p) => {
                shown = p.apply(&self.text);
                &shown
            }
            None => &self.text,
        };
        let p = engine.layout(text, &self.style, options);
        let p = Rc::new(p);
        self.cache = Some((key, p.clone()));
        p
    }
}

/// A tool that sees pointer input first ([`Ui::set_pointer_interceptor`]).
pub type PointerInterceptor = Rc<dyn Fn(&remedy_ui_core::input::PointerEvent) -> bool>;

/// Told about each element marked for work ([`Ui::add_dirty_observer`]).
pub type DirtyObserver = Rc<dyn Fn(&DirtyRecord)>;

static NEXT_UI: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// An element marked for work, and why ([`Ui::add_dirty_observer`]): "row 42 repainted
/// because `message.edited` changed".
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DirtyRecord {
    /// The element.
    pub element: ElementId,
    /// Its style is resolved again.
    pub style: bool,
    /// It is laid out again.
    pub layout: bool,
    /// It is repainted.
    pub paint: bool,
    /// Its semantics are sent again.
    pub semantics: bool,
    /// The binding or painter run that marked it, with the signal writes that made it run,
    /// while reactive tracing is on ([`remedy_ui_core::reactive::debug::trace`]). `None` when the
    /// mark came from outside any recorded run: input, focus, or a direct call.
    pub cause: Option<remedy_ui_core::reactive::debug::Update>,
    /// The trace clock's time of the mark ([`remedy_ui_core::trace::now_ns`]).
    pub at_ns: u64,
}

impl DirtyRecord {
    /// What happens to the element, in a word: "repainted", "relaid out", "restyled" or
    /// "re-described" (semantics), the costliest first.
    pub fn verb(&self) -> &'static str {
        if self.layout {
            "relaid out"
        } else if self.style {
            "restyled"
        } else if self.paint {
            "repainted"
        } else {
            "re-described"
        }
    }

    /// Why, in words: the causes of the run that marked it, or how it was marked otherwise.
    pub fn because(&self) -> String {
        match &self.cause {
            Some(u) => {
                let who = match &u.name {
                    Some(n) => format!("its {n} binding"),
                    None => {
                        format!("the {} at {}:{}", u.kind.as_str(), u.created_at.file(), u.created_at.line())
                    }
                };
                format!("{who} re-ran: {}", u.because())
            }
            None => "changed directly (input, focus or a call)".to_owned(),
        }
    }
}

/// A snapshot of an element for inspectors and tests ([`El::info`]).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ElementInfo {
    /// Its id.
    pub id: ElementId,
    /// The widget name the style sheet styles it by.
    pub widget: &'static str,
    /// Its variant (empty for none).
    pub variant: &'static str,
    /// Hover, focus, pressed, disabled, …
    pub states: States,
    /// The style it is drawn with.
    pub style: ResolvedStyle,
    /// Its own style overrides.
    pub local: StylePatch,
    /// Its box in window logical pixels (after the last frame).
    pub bounds: Option<remedy_ui_core::Rect>,
    /// Its text, if it shows text.
    pub text: Option<String>,
    /// What it tells assistive technology (as the accessibility tree gets it).
    pub semantics: Option<Semantics>,
    /// Where in the source it was built.
    pub created_at: Option<&'static std::panic::Location<'static>>,
    /// Takes keyboard focus.
    pub focusable: bool,
    /// Has a click handler.
    pub clickable: bool,
    /// Has a custom painter.
    pub painter: bool,
    /// Is an editable text field.
    pub field: bool,
    /// Has a pointer handler of its own.
    pub pointer: bool,
    /// How keyboard users do what its pointer handling does, when it is not focusable.
    pub keyboard_alternative: Option<String>,
    /// Reactive bindings it owns.
    pub bindings: usize,
    /// Its bindings and painter in the reactive graph (see
    /// [`remedy_ui_core::reactive::debug`]): bindings first, then the painter's watch.
    pub binding_keys: Vec<remedy_ui_core::reactive::debug::NodeKey>,
    /// The signals and memos its bindings and painter read (each once, in first-seen order).
    pub reads: Vec<remedy_ui_core::reactive::Source>,
    /// Hidden.
    pub hidden: bool,
    /// Its children are cut to its box: a scroll container or a clipping element.
    pub clips: bool,
    /// A link inside a sentence of text ([`El::on_link`]): its size is the line's.
    pub inline_link: bool,
}

pub(crate) struct ElData {
    pub(crate) widget: &'static str,
    pub(crate) variant: &'static str,
    pub(crate) states: States,
    pub(crate) local: StylePatch,
    pub(crate) resolved: ResolvedStyle,
    pub(crate) layout: LayoutStyle,
    pub(crate) node: LayoutNode,
    pub(crate) text: Option<Rc<RefCell<TextSlot>>>,
    /// Pictures drawn in the text where its inline objects are ([`El::inline_images`]).
    pub(crate) inline_images: Rc<Vec<InlineImage>>,
    pub(crate) field: Option<Rc<RefCell<Field>>>,
    pub(crate) painter: Option<Painter>,
    /// Watches the signals the painter read, to repaint when they change.
    pub(crate) paint_watch: Option<Reaction>,
    pub(crate) semantics: Option<Semantics>,
    pub(crate) scroll: bool,
    pub(crate) clip: bool,
    pub(crate) click: Option<Rc<dyn Fn()>>,
    pub(crate) hover: Option<Rc<dyn Fn(bool)>>,
    pub(crate) focus: Option<Rc<dyn Fn(bool)>>,
    /// Told when focus comes onto it or inside it, and when it leaves all of it.
    pub(crate) focus_within: Option<Rc<dyn Fn(bool)>>,
    /// Takes text from input methods and the clipboard without being a field
    /// ([`El::on_text_input`]).
    pub(crate) text_client: Option<crate::view::client::TextClient>,
    /// Has a pointer handler of its own (`on_pointer`).
    pub(crate) pointer: bool,
    /// A primary press on it leaves keyboard focus where it is ([`El::keeps_focus_on_press`]).
    pub(crate) keeps_focus: bool,
    /// Arrows coming into it return to the child last focused inside ([`El::focus_group`]).
    pub(crate) focus_group: bool,
    /// Arrows wrap at its edges rather than leave it ([`El::focus_wrap`]).
    pub(crate) focus_wrap: bool,
    /// Draggable: what is dragged, and who hears how it ended.
    pub(crate) drag_source: Option<crate::view::dnd::DragSource>,
    /// Its text may be selected ([`El::selectable`]).
    pub(crate) selectable: bool,
    /// The bytes of its text selected now.
    pub(crate) sel: Option<std::ops::Range<usize>>,
    /// Parts of its text marked (search matches).
    pub(crate) marks: Vec<std::ops::Range<usize>>,
    /// Its text's links people can follow ([`El::on_link`]).
    pub(crate) links: Option<crate::view::links::TextLinks>,
    /// A link inside its parent's text: the link's id. It is placed over the link's words,
    /// not by layout.
    pub(crate) inline_link: Option<u64>,
    /// Takes drops.
    pub(crate) drop_target: Option<(crate::view::dnd::DropAccept, crate::view::dnd::DropHandler)>,
    /// Told when a drag comes over it (or its children) and when it goes.
    pub(crate) drag_hover: Option<crate::view::dnd::DragHover>,
    /// A scroll container that also scrolls sideways.
    pub(crate) scroll_x: bool,
    /// How far a right-to-left sideways scroll container's content overflows to the left:
    /// its scroll offset where the content's start (its right edge) shows.
    pub(crate) scroll_lead: f32,
    /// Told the element's new size after a frame where it changed.
    pub(crate) on_resize: Option<Rc<dyn Fn(Size)>>,
    pub(crate) last_size: Option<Size>,
    /// Told when the element becomes seen or unseen, and the last answer it was given.
    pub(crate) on_visibility: Option<Rc<dyn Fn(bool)>>,
    pub(crate) last_visible: Option<bool>,
    /// How near the window counts as seen for `on_visibility` ([`El::on_visibility_within`]).
    pub(crate) visibility_margin: f32,
    /// Told where the element is on the window after a frame where that changed.
    pub(crate) on_placed: Option<crate::view::frame::PlacedHandler>,
    pub(crate) last_placed: Option<Option<crate::view::Placement>>,
    /// A scroll container following another's offset: (leader, x, y).
    pub(crate) scroll_follow: Option<(ElementId, bool, bool)>,
    /// A scroll container that rests on its descendants' snap points ([`El::scroll_snap`]).
    pub(crate) scroll_snap: Option<Box<SnapSpec>>,
    /// Where this element's snap point is in its scroll container ([`El::snap_align`]).
    pub(crate) snap_align: Option<SnapAlign>,
    /// Space around this element's snap area ([`El::snap_margin`]).
    pub(crate) snap_margin: remedy_ui_core::Insets,
    /// Scrolling may not pass this element's snap point ([`El::snap_stop_always`]).
    pub(crate) snap_stop: bool,
    /// Painted into a compositor layer of its own ([`El::isolate_paint`]), with this
    /// transform the compositor applies.
    pub(crate) isolate: Option<crate::paint::Affine>,
    /// Compositor loops on its layer ([`El::ambient_loop`]).
    pub(crate) loops: Vec<crate::compositor::AmbientLoop>,
    /// Its painter animates with [`PaintCx::ambient`]: it gets a layer of its own.
    pub(crate) ambient_layer: bool,
    /// What is under the window shows through its box ([`El::set_see_through`]).
    pub(crate) see_through: bool,
    /// When its painter last got an ambient tick.
    pub(crate) ambient_last: Option<std::time::Duration>,
    /// Its text is data, not a translatable string ([`El::data_text`]).
    pub(crate) data_text: bool,
    /// How keyboard users do what this element's pointer handling does.
    pub(crate) keyboard_alternative: Option<String>,
    pub(crate) hidden: bool,
    /// Takes no input ([`El::disabled`]).
    pub(crate) inert: bool,
    /// Clicks go through it to what is below ([`El::pass_through`]).
    pub(crate) pass_through: bool,
    /// Playing its way out ([`crate::view::play_exit`]): takes no input until it is removed.
    pub(crate) leaving: bool,
    /// Lays out as nothing, like `hidden`, but set by the engine (a [`crate::view::show`] with
    /// nothing to show), so it never fights what the app sets with [`El::hidden`].
    pub(crate) collapsed: bool,
    pub(crate) removed: Vec<Box<dyn FnOnce()>>,
    pub(crate) on_scroll: Option<Rc<dyn Fn(Vec2)>>,
    /// The last recorded paint chunk, and the size and device-grid box it was recorded at.
    pub(crate) chunk: Option<((Size, remedy_ui_core::Rect), Rc<DisplayList>)>,
    /// Bindings owned by this element, disposed with it.
    pub(crate) effects: Vec<Effect>,
    /// Bindings of properties set by name ([`crate::view::IntoProp`]): setting the property again
    /// replaces its binding.
    pub(crate) prop_bindings: Vec<(&'static str, Effect)>,
    /// Its own style as given, tokens and all: constants merged in order, then the bound
    /// layer. `local` is this resolved under the theme.
    pub(crate) style_src: remedy_ui_core::style::Style,
    pub(crate) style_bound: Option<remedy_ui_core::style::Style>,
    /// Layout lengths given as tokens, resolved again when the theme changes.
    pub(crate) layout_tokens: Vec<(crate::view::tokens::LayoutField, remedy_ui_core::style::Dp)>,
    /// Values animations hold now, over its own style.
    pub(crate) anim_style: StylePatch,
    /// Animations running on it.
    pub(crate) anims: Vec<crate::view::motion::Running>,
    /// Its animated offset and scale (about its center), drawn without relayout.
    pub(crate) shift: (Vec2, f32),
    /// Where it was before a layout change it should glide through ([`El::flip`]), and how.
    pub(crate) flip: Option<(remedy_ui_core::Point, remedy_ui_core::motion::Motion)>,
    /// Does what assistive technology asks of it ([`El::on_accessibility_action`]).
    pub(crate) a11y_action: Option<Rc<dyn Fn(remedy_ui_core::a11y::Action) -> bool>>,
    /// Where the element was built (the first caller outside `#[track_caller]` builders).
    pub(crate) created_at: Option<&'static std::panic::Location<'static>>,
}

impl Inner {
    /// True for the content, overlay and trusted layer roots.
    pub(crate) fn layer_root(&self, id: ElementId) -> bool {
        id == self.content || id == self.overlays || id == self.trusted
    }
}

/// How a window draws text ([`Ui::set_text_rendering`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextRendering {
    /// Letters fitted to the pixel grid with the font's own hinting (crisper on low-density
    /// screens); false draws them exactly as designed, a little softer.
    pub sharp: bool,
    /// The edges of letters.
    pub antialias: TextAntialias,
}

impl Default for TextRendering {
    fn default() -> Self {
        TextRendering { sharp: true, antialias: TextAntialias::Grayscale }
    }
}

impl TextRendering {
    /// `sharp` letters with `antialias` edges.
    pub fn new(sharp: bool, antialias: TextAntialias) -> Self {
        TextRendering { sharp, antialias }
    }

    /// The hinting and coverage for a system that uses subpixel text as `system` says.
    pub fn resolve(self, system: Option<bool>) -> (remedy_ui_text::Hinting, remedy_ui_text::Antialias) {
        use remedy_ui_text::{Antialias, Hinting};
        let hinting = if self.sharp { Hinting::Full } else { Hinting::None };
        let aa = match self.antialias {
            TextAntialias::Grayscale => Antialias::Grayscale,
            TextAntialias::Subpixel => Antialias::Lcd { bgr: system.unwrap_or(false) },
            TextAntialias::System => system.map_or(Antialias::Grayscale, |bgr| Antialias::Lcd { bgr }),
        };
        (hinting, aa)
    }
}

/// The edges of letters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextAntialias {
    /// As the system draws its own text (ClearType on Windows when it is on), else grayscale.
    System,
    /// One shade per pixel: right for rotated or unstriped screens and screenshots.
    #[default]
    Grayscale,
    /// The red, green and blue stripes of each pixel for three times the detail across.
    Subpixel,
}

pub(crate) struct Inner {
    /// Every text is shaped again on the next frame: the fonts or font families changed.
    pub(crate) reshape_text: bool,
    pub(crate) tree: Tree,
    pub(crate) layout: LayoutTree,
    pub(crate) els: HashMap<ElementId, ElData>,
    pub(crate) sheet: StyleSheet,
    pub(crate) theme: Theme,
    /// The engine's starting style for `theme`, computed once per theme (elements are made
    /// often; deriving the theme's tokens is not cheap).
    pub(crate) base: StylePatch,
    pub(crate) engine: Rc<RefCell<TextEngine>>,
    pub(crate) size: Size,
    pub(crate) scale: f32,
    pub(crate) requests: Vec<Request>,
    pub(crate) scroll_offsets: HashMap<ElementId, Vec2>,
    pub(crate) hovered: Vec<ElementId>,
    /// Elements that hide the mouse pointer while it is over them ([`El::cursor_hidden`]).
    pub(crate) cursor_hiding: std::collections::HashSet<ElementId>,
    /// The pointer's shape over the elements that set one ([`El::cursor`]).
    pub(crate) cursors: HashMap<ElementId, remedy_ui_core::input::Cursor>,
    pub(crate) pressed: Option<ElementId>,
    pub(crate) focus_visible: bool,
    /// The keyboard came up: the next frame scrolls the focused element back into view.
    pub(crate) reveal_focus: bool,
    pub(crate) a11y: A11yTree,
    pub(crate) structure_changed: bool,
    /// A layer's transform changed: the next frame rebuilds layers without repainting.
    pub(crate) layers_changed: bool,
    pub(crate) frame: crate::view::frame::FrameState,
    pub(crate) now_ms: u64,
    pub(crate) field_session: Option<ElementId>,
    pub(crate) focus_shown: Option<ElementId>,
    pub(crate) platform: remedy_ui_core::input::Platform,
    /// The focused element and its ancestors, as when focus last moved ([`El::on_focus_within`]).
    pub(crate) focus_chain: Vec<ElementId>,
    /// Spatial navigation as the app chose it (`None` follows the system) and as the system
    /// suggests it ([`Ui::set_spatial_navigation`]).
    pub(crate) spatial: Option<bool>,
    /// How far an arrow key scrolls ([`Ui::set_scroll_step`]).
    pub(crate) scroll_step: f32,
    pub(crate) system_spatial: bool,
    /// The child each focus group last had focused ([`El::focus_group`]).
    pub(crate) group_memory: HashMap<ElementId, ElementId>,
    /// How spatial navigation weighs candidates ([`Ui::set_spatial_tuning`]).
    pub(crate) spatial_tuning: crate::view::SpatialTuning,
    /// How text is drawn ([`Ui::set_text_rendering`]) and whether the system uses subpixel
    /// text (`Some(bgr)`).
    pub(crate) text_rendering: TextRendering,
    pub(crate) system_subpixel: Option<bool>,
    /// The app's edition: which default behaviors this window follows.
    pub(crate) edition: remedy_ui_core::Edition,
    pub(crate) locale: remedy_ui_core::reactive::Signal<String>,
    /// The window's size in logical pixels, set as each frame starts.
    pub(crate) window_size: remedy_ui_core::reactive::Signal<Size>,
    pub(crate) pseudo: Option<remedy_ui_text::Pseudo>,
    /// The system's double-click time.
    pub(crate) double_click: std::time::Duration,
    /// The last primary press: element, place and time, and how many presses in a row.
    pub(crate) last_press: Option<(ElementId, remedy_ui_core::Point, std::time::Duration)>,
    pub(crate) click_count: u32,
    /// The text selection over non-editable text, and whether a drag is extending it.
    pub(crate) text_sel: Option<crate::view::select::TextSel>,
    pub(crate) selecting: bool,
    /// The last rich text copied from a field and the HTML put on the clipboard for it, so
    /// pasting that copy back keeps what HTML cannot carry (mentions).
    pub(crate) rich_clip: Option<(String, remedy_ui_text::RichDoc)>,
    /// The window's undo history (changes outside text fields).
    pub(crate) history: crate::view::history::History,
    pub(crate) local_time: remedy_ui_core::reactive::Signal<remedy_ui_core::host::LocalTime>,
    pub(crate) time_zone: remedy_ui_core::host::TimeZone,
    pub(crate) content: ElementId,
    pub(crate) overlays: ElementId,
    pub(crate) interceptor: Option<PointerInterceptor>,
    /// What hears the window's menu keys before anything focused ([`Ui::add_menu_keys`]),
    /// newest first.
    pub(crate) menu_keys: Vec<(u64, crate::view::events::MenuKeyHandler)>,
    /// Alt is down and nothing else happened since it went down: its release is a tap.
    pub(crate) alt_tap: bool,
    /// Alt is held ([`Ui::alt_held`]).
    pub(crate) alt_held: remedy_ui_core::reactive::Signal<bool>,
    /// A handler consumed the last pointer event (see [`Ui::pointer_consumed`]).
    pub(crate) consumed: bool,
    /// Called when a request arrives with none pending: the host's loop must come around.
    pub(crate) wake: Option<Rc<dyn Fn()>>,
    /// The element that took the current press: it gets the moves and the release, wherever
    /// the pointer goes, until the button comes up.
    pub(crate) captured: Option<ElementId>,
    /// Middle-button drag on a scroll container: the scroller and the last pointer position.
    pub(crate) middle_scroll: Option<(ElementId, remedy_ui_core::Point)>,
    /// Primary-button drag on a painted scrollbar thumb (or a jump on its track).
    /// `(scroller, vertical, grab)`: grab is the pointer's offset from the thumb start
    /// along that axis, so the thumb stays under the cursor while dragging.
    pub(crate) bar_scroll: Option<(ElementId, bool, f32)>,
    /// The element a touchpad pinch started on keeps it until the pinch ends.
    pub(crate) pinching: Option<ElementId>,
    /// The drag under way, and a press that may become one.
    pub(crate) drag: Option<crate::view::dnd::Session>,
    pub(crate) drag_pending: Option<crate::view::dnd::Pending>,
    pub(crate) trusted: ElementId,
    pub(crate) reduced_motion: remedy_ui_core::reactive::Signal<bool>,
    pub(crate) pointer_caps: remedy_ui_core::reactive::Signal<remedy_ui_core::host::PointerCaps>,
    /// The platform reports the pointer's caps: the last pointer used does not change them.
    pub(crate) pointer_reported: bool,
    /// The edges system interface covers (status and navigation bars, cutouts, the on-screen
    /// keyboard), in logical pixels.
    pub(crate) safe_area: remedy_ui_core::reactive::Signal<remedy_ui_core::host::SafeArea>,
    /// How often the window's display refreshes, where known.
    pub(crate) refresh: remedy_ui_core::reactive::Signal<Option<std::time::Duration>>,
    /// The window is minimized or fully covered: nothing in it is seen.
    pub(crate) occluded: bool,
    /// Where the window's layout direction comes from ([`Ui::set_direction`]).
    pub(crate) direction: WindowDirection,
    /// The window lays out right to left now ([`Ui::is_rtl`]).
    pub(crate) rtl: remedy_ui_core::reactive::Signal<bool>,
    pub(crate) timers: HashMap<u64, Box<dyn FnOnce()>>,
    pub(crate) next_timer: u64,
    /// The native window this view draws into, for content the platform draws itself (an
    /// embedded web view); `None` on hosts without one (headless).
    pub(crate) window_handles: Option<std::sync::Arc<dyn remedy_ui_core::host::WindowHandles>>,
    /// The window is drawn by the GPU renderer now.
    pub(crate) gpu: remedy_ui_core::reactive::Signal<bool>,
    /// This view's id, unique in the process ([`Ui::id`]).
    pub(crate) id: u64,
    /// Told about every element marked for work ([`Ui::add_dirty_observer`]), by id.
    pub(crate) dirty_observers: Vec<(u64, DirtyObserver)>,
    pub(crate) next_observer: u64,
    /// What the last frame did.
    pub(crate) last_stats: crate::view::frame::FrameStats,
    /// The frame clock: set by the runner each frame, or wall time since the view was made.
    pub(crate) clock: crate::view::motion::Clock,
    /// Error boundaries ([`crate::view::catch_errors`]) by element, with the failure each holds.
    pub(crate) boundaries:
        HashMap<ElementId, remedy_ui_core::reactive::Signal<Option<crate::view::CaughtError>>>,
    /// Failures caught in this window, oldest first, for the inspector.
    pub(crate) failures: Vec<crate::view::CaughtError>,
    /// Changes whenever the failure list does (readers of [`Ui::failures`] follow it).
    pub(crate) failures_rev: remedy_ui_core::reactive::Signal<u64>,
    /// The app's error hook ([`Ui::set_error_hook`]).
    pub(crate) error_hook: Option<crate::view::flow::ErrorHook>,
    /// This view's handler for effect panics in the reactive runtime.
    pub(crate) panic_handler: u64,
    /// Ambient motion: the policy, its rate and what waits on it.
    pub(crate) ambient: crate::view::ambient::AmbientState,
}

impl Drop for Inner {
    fn drop(&mut self) {
        remedy_ui_core::reactive::remove_panic_handler(self.panic_handler);
    }
}

/// One window's UI. Cloning gives another handle to the same UI.
#[derive(Clone)]
pub struct Ui(pub(crate) Rc<RefCell<Inner>>);

/// A handle to a window's view that does not keep it alive ([`Ui::downgrade`]): for state
/// that outlives a frame but must not hold the window (caches, players, timers).
#[derive(Clone, Debug)]
pub struct WeakUi(std::rc::Weak<RefCell<Inner>>);

impl WeakUi {
    /// The view, while its window is open.
    pub fn upgrade(&self) -> Option<Ui> {
        self.0.upgrade().map(Ui)
    }
}

impl Ui {
    /// A handle that does not keep this view alive.
    pub fn downgrade(&self) -> WeakUi {
        WeakUi(Rc::downgrade(&self.0))
    }

    /// Whether `other` is a handle to this same view (the same window).
    pub fn is_same(&self, other: &Ui) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Tears the view down when its window closes: every element is removed (its bindings
    /// disposed, its removal callbacks run, its handlers dropped) and pending timers are
    /// forgotten, so nothing they captured keeps the window alive.
    pub fn close(&self) {
        let root = self.0.borrow().tree.root();
        for child in self.tree().children(root) {
            El { ui: self.clone(), id: child }.remove();
        }
        let _ = self.tree().clear_handlers(root);
        self.tree().set_failure_handler(None);
        let handler = std::mem::take(&mut self.0.borrow_mut().panic_handler);
        remedy_ui_core::reactive::remove_panic_handler(handler);
        let (effects, removed, watch) = {
            let mut i = self.0.borrow_mut();
            i.timers.clear();
            i.ambient.parked.clear();
            i.ambient.parked_paint.clear();
            i.ambient.wakes.clear();
            i.ambient.timer = None;
            i.history = crate::view::history::History::default();
            i.interceptor = None;
            i.wake = None;
            // The native window is let go: the platform destroys it when its last handle
            // drops, and a Ui the app still holds (in a closure, a signal) must not keep a
            // closed window on screen, frozen, with nothing drawing it.
            i.window_handles = None;
            i.requests.clear();
            i.rich_clip = None;
            match i.els.get_mut(&root) {
                Some(d) => {
                    d.click = None;
                    d.hover = None;
                    d.focus = None;
                    d.focus_within = None;
                    d.painter = None;
                    d.on_resize = None;
                    d.on_scroll = None;
                    d.on_visibility = None;
                    d.on_placed = None;
                    d.links = None;
                    let mut removed = std::mem::take(&mut d.removed);
                    let anims = std::mem::take(&mut d.anims);
                    removed.extend(crate::view::motion::cut_short(anims, crate::view::motion::End::Removed));
                    (std::mem::take(&mut d.effects), removed, d.paint_watch.take())
                }
                None => (Vec::new(), Vec::new(), None),
            }
        };
        for e in effects {
            e.dispose();
        }
        if let Some(w) = watch {
            w.dispose();
        }
        for f in removed {
            f();
        }
    }
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.try_borrow() {
            Ok(i) => write!(f, "Ui({} elements)", i.els.len()),
            Err(_) => f.write_str("Ui(busy)"),
        }
    }
}

/// An element being built or bound. Cheap to clone.
#[derive(Clone, Debug)]
pub struct El {
    pub(crate) ui: Ui,
    pub(crate) id: ElementId,
}

/// Where a window's layout direction comes from ([`Ui::set_direction`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowDirection {
    /// From the window's locale: right to left for Arabic, Hebrew, Persian, Urdu and the
    /// other right-to-left languages, left to right otherwise.
    #[default]
    Locale,
    /// Always left to right.
    LeftToRight,
    /// Always right to left.
    RightToLeft,
}

fn root_style() -> LayoutStyle {
    {
        let mut v_ = LayoutStyle::column();
        v_.width = remedy_ui_core::layout::Length::Percent(1.0);
        v_.height = remedy_ui_core::layout::Length::Percent(1.0);
        v_
    }
}

pub(crate) fn defaults(theme: &Theme) -> StylePatch {
    StylePatch::defaults(theme)
}

impl Ui {
    /// A UI with `theme`, styled by `sheet`, drawing text with `fonts`.
    pub fn new(theme: Theme, sheet: StyleSheet, fonts: FontCollection) -> Ui {
        let tree = Tree::new();
        let mut layout = LayoutTree::new();
        let root = tree.root();
        let node = layout.node(&root_style()).unwrap_or_else(|_| unreachable_root());
        let mut sheet = sheet;
        let resolved = sheet.resolve(
            &StyleKey { widget: "window", variant: "", states: States::NONE },
            &defaults(&theme),
            &StylePatch::default(),
        );
        let mut els = HashMap::new();
        els.insert(root, ElData::new("window", root_style(), node, resolved));
        tree.mark(root, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT);
        let ui = Ui(Rc::new(RefCell::new(Inner {
            tree,
            layout,
            els,
            sheet,
            base: defaults(&theme),
            theme,
            reshape_text: false,
            engine: Rc::new(RefCell::new(TextEngine::new(fonts))),
            size: Size::new(1.0, 1.0),
            scale: 1.0,
            requests: Vec::new(),
            scroll_offsets: HashMap::new(),
            hovered: Vec::new(),
            cursor_hiding: std::collections::HashSet::new(),
            cursors: HashMap::new(),
            pressed: None,
            focus_visible: false,
            reveal_focus: false,
            a11y: A11yTree::new(root_key(root)),
            structure_changed: true,
            layers_changed: false,
            frame: crate::view::frame::FrameState::default(),
            now_ms: 0,
            field_session: None,
            focus_shown: None,
            platform: remedy_ui_core::input::Platform::Standard,
            focus_chain: Vec::new(),
            spatial: None,
            scroll_step: crate::view::events::SCROLL_STEP,
            system_spatial: false,
            group_memory: HashMap::new(),
            spatial_tuning: crate::view::SpatialTuning::default(),
            text_rendering: TextRendering::default(),
            system_subpixel: None,
            edition: remedy_ui_core::Edition::FIRST,
            locale: remedy_ui_core::reactive::Signal::new(String::from("en-US")),
            window_size: remedy_ui_core::reactive::Signal::new(Size::ZERO),
            pseudo: None,
            double_click: std::time::Duration::from_millis(500),
            last_press: None,
            click_count: 0,
            text_sel: None,
            selecting: false,
            rich_clip: None,
            history: crate::view::history::History::default(),
            local_time: remedy_ui_core::reactive::Signal::new(remedy_ui_core::host::LocalTime::default()),
            time_zone: remedy_ui_core::host::TimeZone::default(),
            content: root,
            overlays: root,
            interceptor: None,
            menu_keys: Vec::new(),
            alt_tap: false,
            alt_held: remedy_ui_core::reactive::Signal::new(false),
            consumed: false,
            wake: None,
            captured: None,
            middle_scroll: None,
            bar_scroll: None,
            pinching: None,
            drag: None,
            drag_pending: None,
            trusted: root,
            reduced_motion: remedy_ui_core::reactive::Signal::new(false),
            pointer_caps: remedy_ui_core::reactive::Signal::new(remedy_ui_core::host::PointerCaps::default()),
            pointer_reported: false,
            safe_area: remedy_ui_core::reactive::Signal::new(remedy_ui_core::host::SafeArea::default()),
            refresh: remedy_ui_core::reactive::Signal::new(None),
            occluded: false,
            direction: WindowDirection::Locale,
            rtl: remedy_ui_core::reactive::Signal::new(false),
            timers: HashMap::new(),
            next_timer: 0,
            window_handles: None,
            gpu: remedy_ui_core::reactive::Signal::new(false),
            id: NEXT_UI.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            dirty_observers: Vec::new(),
            next_observer: 1,
            last_stats: crate::view::frame::FrameStats::default(),
            clock: crate::view::motion::Clock::default(),
            boundaries: HashMap::new(),
            failures: Vec::new(),
            failures_rev: remedy_ui_core::reactive::Signal::new(0),
            error_hook: None,
            panic_handler: 0,
            ambient: crate::view::ambient::AmbientState::default(),
        })));
        crate::view::flow::install_failure_handling(&ui);
        // The window holds the app's content, then a click-through layer for overlays
        // (dialogs, menus, toasts) that always draws above it.
        let content = ui.el("content").layout(|l| {
            l.grow = 1.0;
            // Held to the window: taller content overflows into scroll containers rather than
            // stretching the window's content past its edge (flex items otherwise refuse to
            // shrink below their content).
            l.min_height = remedy_ui_core::layout::Length::Px(0.0);
            l.width = remedy_ui_core::layout::Length::Percent(1.0);
            l.display = remedy_ui_core::layout::Display::Flex;
            l.axis = remedy_ui_core::layout::Axis::Column;
        });
        let overlays = ui.el("overlays").layout(|l| {
            l.position = remedy_ui_core::layout::Position::Absolute;
            l.inset = Insets::ZERO;
            l.width = remedy_ui_core::layout::Length::Percent(1.0);
            l.height = remedy_ui_core::layout::Length::Percent(1.0);
        });
        let overlays = overlays.pass_through();
        // Trusted prompts sit above everything the app can draw.
        let trusted = ui.el("trusted").layout(|l| {
            l.position = remedy_ui_core::layout::Position::Absolute;
            l.inset = Insets::ZERO;
            l.width = remedy_ui_core::layout::Length::Percent(1.0);
            l.height = remedy_ui_core::layout::Length::Percent(1.0);
        });
        let trusted = trusted.pass_through();
        let window = El { ui: ui.clone(), id: root };
        let _ = window.child(content.clone()).child(overlays.clone()).child(trusted.clone());
        {
            let mut i = ui.0.borrow_mut();
            i.content = content.id;
            i.overlays = overlays.id;
            i.trusted = trusted.id;
        }
        ui
    }

    /// Shows `el` above the window's content until it is removed. Positions inside are
    /// absolute in window coordinates when the element asks for it.
    pub fn overlay(&self, el: El) -> El {
        let host = self.0.borrow().overlays;
        let _ = El { ui: self.clone(), id: host }.child(el.clone());
        el
    }

    /// Shows `el` in the trusted layer: above every overlay the app draws. Only the engine's
    /// trusted prompts belong here.
    pub fn trusted_overlay(&self, el: El) -> El {
        let host = self.0.borrow().trusted;
        let _ = El { ui: self.clone(), id: host }.child(el.clone());
        el
    }

    /// The user's locale (BCP 47, such as `de-DE`), tracked: widgets that format numbers or
    /// dates re-render when it changes. The runner sets it from the system.
    pub fn locale(&self) -> String {
        let l = self.0.borrow().locale;
        l.get()
    }

    /// The window's size in logical pixels (tracked): overlays that must stay inside the
    /// window place themselves again when it changes. Zero before the first frame.
    pub fn window_size(&self) -> Size {
        let s = self.0.borrow().window_size;
        s.get()
    }

    /// The wall clock and local UTC offset (tracked). The runner keeps it current: every
    /// frame and at each local midnight, so date displays roll over on their own.
    pub fn local_time(&self) -> remedy_ui_core::host::LocalTime {
        let t = self.0.borrow().local_time;
        t.get()
    }

    /// The local time at `unix_seconds`, any instant past or future, with that instant's UTC
    /// offset: across a daylight-saving change it is the other side's. Tracked as
    /// [`local_time`](Self::local_time) is; where the time zone cannot say, today's offset
    /// stands in.
    pub fn local_time_at(&self, unix_seconds: i64) -> remedy_ui_core::host::LocalTime {
        let now = self.local_time();
        let utc_offset = self.0.borrow().time_zone.offset_at(unix_seconds).unwrap_or(now.utc_offset);
        remedy_ui_core::host::LocalTime { unix_seconds, utc_offset }
    }

    /// Sets the time zone [`local_time_at`](Self::local_time_at) asks (the runner sets the
    /// system's; tests may set their own).
    pub fn set_time_zone(&self, zone: remedy_ui_core::host::TimeZone) {
        self.0.borrow_mut().time_zone = zone;
        let t = self.0.borrow().local_time;
        // Readers of other instants format again.
        t.update(|_| {});
    }

    /// Sets the wall clock and offset (the runner does; tests may).
    pub fn set_local_time(&self, t: remedy_ui_core::host::LocalTime) {
        let s = self.0.borrow().local_time;
        // Readers care about the day, the minute at most: change only then.
        let same = s
            .get_untracked()
            .is_ok_and(|o| o.utc_offset == t.utc_offset && o.local_seconds() / 60 == t.local_seconds() / 60);
        if !same {
            s.set(t);
        }
    }

    /// Sets the locale.
    /// The pseudo-locales `en-XA` and `ar-XB` rewrite the app's own text (labels, headings,
    /// placeholders; never what people type) to check layouts before translations exist.
    pub fn set_locale(&self, locale: impl Into<String>) {
        let locale = locale.into();
        let pseudo = remedy_ui_text::Pseudo::for_locale(&locale);
        let changed = {
            let mut i = self.0.borrow_mut();
            let changed = i.pseudo != pseudo;
            i.pseudo = pseudo;
            // The text's language picks between fonts for the characters languages share
            // (Japanese and Chinese forms) and language-specific shaping: text is laid out
            // again when it changes.
            let language = i.engine.borrow_mut().set_locale(&locale);
            changed || language
        };
        if changed {
            let ids: Vec<ElementId> = {
                let i = self.0.borrow();
                i.els
                    .iter()
                    .filter_map(|(id, d)| {
                        if let Some(t) = &d.text {
                            let mut t = t.borrow_mut();
                            t.pseudo = pseudo;
                            t.cache = None;
                        }
                        if let Some(f) = &d.field {
                            f.borrow_mut().set_pseudo(pseudo);
                        }
                        (d.text.is_some() || d.field.is_some()).then_some(*id)
                    })
                    .collect()
            };
            for id in ids {
                let node = self.0.borrow().els.get(&id).map(|d| d.node);
                if let Some(n) = node {
                    let _ = self.0.borrow_mut().layout.mark_dirty(n);
                }
                self.mark(id, Dirty::LAYOUT | Dirty::PAINT);
            }
            // Screen readers and the inspector get the pseudo-localized strings too.
            let ids: Vec<ElementId> = self.0.borrow().els.keys().copied().collect();
            for id in ids {
                self.mark(id, Dirty::SEMANTICS);
            }
        }
        let l = self.0.borrow().locale;
        l.set_if_changed(locale);
        self.apply_direction();
    }

    /// Chooses where the window's layout direction comes from: the locale (the default, so
    /// Arabic, Hebrew, Persian and Urdu windows lay out right to left), or a fixed direction
    /// a person chose (a setting that keeps an Arabic interface left to right, or tries a
    /// right-to-left layout in English). Rows, padding, alignment, icons that point
    /// somewhere, menus, overlays and scroll bars all follow it.
    pub fn set_direction(&self, direction: WindowDirection) {
        self.0.borrow_mut().direction = direction;
        self.apply_direction();
    }

    /// Where the window's layout direction comes from ([`Ui::set_direction`]).
    pub fn direction(&self) -> WindowDirection {
        self.0.borrow().direction
    }

    /// Whether the window lays out right to left (tracked): from its locale unless
    /// [`Ui::set_direction`] fixed it. Elements inside can still set their own direction.
    pub fn is_rtl(&self) -> bool {
        let s = self.0.borrow().rtl;
        s.get()
    }

    /// Sets the window root's direction from the setting and the locale.
    fn apply_direction(&self) {
        let (setting, locale, signal, root) = {
            let i = self.0.borrow();
            (i.direction, i.locale.get_untracked().unwrap_or_default(), i.rtl, i.tree.root())
        };
        let rtl = match setting {
            WindowDirection::Locale => remedy_ui_text::is_rtl_locale(&locale),
            WindowDirection::LeftToRight => false,
            WindowDirection::RightToLeft => true,
        };
        let want =
            if rtl { remedy_ui_core::layout::Direction::Rtl } else { remedy_ui_core::layout::Direction::Ltr };
        let same = signal.get_untracked().ok() == Some(rtl)
            && self.0.borrow().els.get(&root).is_some_and(|d| d.layout.direction == want);
        if same {
            return;
        }
        let window = El { ui: self.clone(), id: root };
        let _ = window.layout(|l| l.direction = want);
        // Painters that mirror (icons, scroll bars, sliders) draw again.
        let ids: Vec<ElementId> = self.0.borrow().els.keys().copied().collect();
        for id in ids {
            self.mark(id, Dirty::PAINT | Dirty::SEMANTICS);
        }
        signal.set_if_changed(rtl);
    }

    /// Uses the system's double-click time (the runner sets it from system settings).
    pub fn set_double_click_time(&self, t: std::time::Duration) {
        self.0.borrow_mut().double_click = t;
    }

    /// Whether the mouse pointer should be hidden: it is over an element that hides it
    /// ([`El::cursor_hidden`]). The runner passes it to the platform after each event.
    pub fn cursor_hidden(&self) -> bool {
        let i = self.0.borrow();
        i.hovered.iter().any(|h| i.cursor_hiding.contains(h))
    }

    /// The shape the mouse pointer should have: that of the innermost element under it that
    /// sets one ([`El::cursor`]); text that can be selected has the I-beam unless something
    /// inside it says otherwise. The runner passes it to the platform after each event.
    pub fn cursor(&self) -> remedy_ui_core::input::Cursor {
        let i = self.0.borrow();
        // The chain runs from the element under the pointer out to the root.
        i.hovered
            .iter()
            .find_map(|h| {
                i.cursors.get(h).copied().or_else(|| {
                    i.els.get(h).is_some_and(|d| d.selectable).then_some(remedy_ui_core::input::Cursor::Text)
                })
            })
            .unwrap_or_default()
    }

    /// Presses in a row on the same spot of the same element, counting the one being
    /// handled now (1 for a single click, 2 for a double click, …): read it in pointer and
    /// click handlers.
    pub fn click_count(&self) -> u32 {
        self.0.borrow().click_count
    }

    /// Device pixels per logical pixel in the last frame (images decode at this density).
    pub fn scale(&self) -> f32 {
        self.0.borrow().scale
    }

    /// The platform's conventions for shortcuts and modifiers.
    pub fn platform(&self) -> remedy_ui_core::input::Platform {
        self.0.borrow().platform
    }

    /// The app's edition: which default behaviors this window follows. Code whose default
    /// changed in a later edition asks [`Edition::at_least`](remedy_ui_core::Edition::at_least)
    /// on it. A view starts on [`Edition::FIRST`](remedy_ui_core::Edition::FIRST); the runner
    /// sets the app's edition from `AppConfig::edition` before the window is built.
    ///
    /// ```
    /// use remedy_ui_core::Edition;
    /// use remedy_ui_core::style::{StyleSheet, Theme};
    /// use remedy_ui_text::FontCollection;
    /// use remedy_ui_engine::view::Ui;
    ///
    /// let ui = Ui::new(Theme::dark_forest(), StyleSheet::new(), FontCollection::new());
    /// assert_eq!(ui.edition(), Edition::FIRST);
    /// ui.set_edition(Edition::LATEST);
    /// assert!(ui.edition().at_least(Edition::E2026));
    /// ```
    pub fn edition(&self) -> remedy_ui_core::Edition {
        self.0.borrow().edition
    }

    /// Sets the edition this window follows (the runner sets the app's before building it).
    pub fn set_edition(&self, edition: remedy_ui_core::Edition) {
        self.0.borrow_mut().edition = edition;
    }

    /// How this window draws text: fitted to the pixel grid or as designed, and with grayscale
    /// or subpixel edges (an app's "sharp text" and "text rendering" settings). Every text is
    /// drawn again.
    pub fn set_text_rendering(&self, rendering: TextRendering) {
        let mut i = self.0.borrow_mut();
        if i.text_rendering == rendering {
            return;
        }
        i.text_rendering = rendering;
        Self::repaint_all_text(&mut i);
        drop(i);
        self.request(Request::Frame);
    }

    /// How this window draws text.
    pub fn text_rendering(&self) -> TextRendering {
        self.0.borrow().text_rendering
    }

    /// Follows the system's subpixel text setting (`Some(bgr)`; the runner sets it), which
    /// [`TextAntialias::System`] uses.
    pub fn set_system_subpixel(&self, subpixel: Option<bool>) {
        let mut i = self.0.borrow_mut();
        if i.system_subpixel == subpixel {
            return;
        }
        i.system_subpixel = subpixel;
        if i.text_rendering.antialias != TextAntialias::Grayscale {
            Self::repaint_all_text(&mut i);
            drop(i);
            self.request(Request::Frame);
        }
    }

    fn repaint_all_text(i: &mut Inner) {
        let ids: Vec<ElementId> =
            i.els.iter().filter(|(_, d)| d.text.is_some() || d.field.is_some()).map(|(k, _)| *k).collect();
        for id in ids {
            i.tree.mark(id, Dirty::PAINT);
        }
    }

    /// The hinting and edges glyphs are drawn with now.
    pub(crate) fn glyph_style(&self) -> (remedy_ui_text::Hinting, remedy_ui_text::Antialias) {
        let i = self.0.borrow();
        i.text_rendering.resolve(i.system_subpixel)
    }

    /// What the pointer in use can do: whether it hovers, whether it points finely. Widgets
    /// show hover-only affordances where it hovers and keep them on screen where it does not.
    /// On the web it follows the `(hover: hover)` and `(pointer: fine)` media queries;
    /// natively it starts from the pointing hardware attached (Windows, Linux and Android
    /// read it, and again when devices come and go) and follows the pointer last used (a
    /// mouse, a finger, a pen), so a convertible switching to tablet changes it with the first
    /// touch.
    pub fn pointer_caps(&self) -> remedy_ui_core::reactive::Signal<remedy_ui_core::host::PointerCaps> {
        self.0.borrow().pointer_caps
    }

    /// Sets what the platform reports for the pointer (the runner sets it from the system
    /// settings); `None` goes by the pointer last used.
    pub fn set_pointer_caps(&self, caps: Option<remedy_ui_core::host::PointerCaps>) {
        let s = {
            let mut i = self.0.borrow_mut();
            i.pointer_reported = caps.is_some();
            i.pointer_caps
        };
        if let Some(c) = caps {
            s.set_if_changed(c);
        }
    }

    /// Starts the caps from the pointing hardware attached (the runner sets it at start and
    /// when devices come and go); the pointer last used still refines them. Does nothing while
    /// the platform reports the caps ([`set_pointer_caps`](Self::set_pointer_caps)).
    pub fn set_pointer_hardware(&self, caps: remedy_ui_core::host::PointerCaps) {
        let (reported, s) = {
            let i = self.0.borrow();
            (i.pointer_reported, i.pointer_caps)
        };
        if !reported {
            s.set_if_changed(caps);
        }
    }

    /// The pointer last used changes the caps unless the platform reports them.
    pub(crate) fn pointer_used(&self, kind: remedy_ui_core::input::PointerKind) {
        let (reported, s) = {
            let i = self.0.borrow();
            (i.pointer_reported, i.pointer_caps)
        };
        if !reported {
            s.set_if_changed(match kind {
                remedy_ui_core::input::PointerKind::Touch => remedy_ui_core::host::PointerCaps::TOUCH,
                remedy_ui_core::input::PointerKind::Pen => remedy_ui_core::host::PointerCaps::PEN,
                _ => remedy_ui_core::host::PointerCaps::MOUSE,
            });
        }
    }

    /// Follows the system's reduced-motion setting (the runner sets it).
    pub fn set_reduced_motion(&self, on: bool) {
        let s = self.0.borrow().reduced_motion;
        s.set_if_changed(on);
        self.refresh_ambient();
    }

    /// True when motion should be reduced: animations jump to their end, spinners hold still.
    /// Tracked: what reads it in a binding runs again when the setting changes.
    pub fn reduced_motion(&self) -> bool {
        let s = self.0.borrow().reduced_motion;
        s.get()
    }

    /// Keeps the window's content, overlays and trusted prompts inside `area`: the edges
    /// system interface covers (the runner sets it from the host). The window's background
    /// still runs under those edges.
    pub fn set_safe_area(&self, area: remedy_ui_core::host::SafeArea) {
        let (signal, overlays, trusted) = {
            let i = self.0.borrow();
            (i.safe_area, i.overlays, i.trusted)
        };
        let old = signal.get_untracked().ok();
        if old == Some(area) {
            return;
        }
        // A soft keyboard rising over the field being typed in.
        if area.bottom > old.map_or(0.0, |o| o.bottom) {
            self.0.borrow_mut().reveal_focus = true;
        }
        let root = self.tree().root();
        signal.set_if_changed(area);
        // The window root adds the area to its padding when laid out (see the frame). The
        // area is physical: a cutout on the left is on the left in any language.
        let insets =
            remedy_ui_core::Insets { top: area.top, end: area.right, bottom: area.bottom, start: area.left };
        self.mark(root, Dirty::LAYOUT);
        for layer in [overlays, trusted] {
            // Absolute layers are placed against the padding box, so they take the area as
            // their insets, stretched between them.
            let _ = El { ui: self.clone(), id: layer }.layout(|l| {
                l.inset = insets;
                l.inset_physical = true;
                l.width = remedy_ui_core::layout::Length::Auto;
                l.height = remedy_ui_core::layout::Length::Auto;
            });
        }
        self.request(Request::Frame);
    }

    /// Sets how often the window's display refreshes (the runner does, from the host, when
    /// the window opens and moves between displays).
    pub fn set_refresh_interval(&self, interval: Option<std::time::Duration>) {
        let s = self.0.borrow().refresh;
        s.set_if_changed(interval);
    }

    /// How often the window's display refreshes, where the platform says: the time a frame
    /// has before it misses a refresh. Tracked.
    pub fn refresh_interval(&self) -> Option<std::time::Duration> {
        let s = self.0.borrow().refresh;
        s.get()
    }

    /// The edges system interface covers, in logical pixels (zero where nothing does).
    /// Tracked: what reads it in a binding runs again when it changes.
    pub fn safe_area(&self) -> remedy_ui_core::host::SafeArea {
        let s = self.0.borrow().safe_area;
        s.get()
    }

    /// Tells the UI the window is minimized or fully covered (`true`) or seen again (the
    /// runner does when the system says so). Elements watching their visibility
    /// ([`El::on_visibility`]) hear about it in the next frame.
    pub fn set_occluded(&self, on: bool) {
        let changed = {
            let mut i = self.0.borrow_mut();
            let changed = i.occluded != on;
            i.occluded = on;
            changed
        };
        if changed {
            self.request(Request::Frame);
            self.refresh_ambient();
        }
    }

    /// True while the window is minimized or fully covered.
    pub fn is_occluded(&self) -> bool {
        self.0.borrow().occluded
    }

    /// Runs `f` once after `delay`. Returns an id for [`Ui::cancel_timer`].
    pub fn after(&self, delay: std::time::Duration, f: impl FnOnce() + 'static) -> u64 {
        let id = {
            let mut i = self.0.borrow_mut();
            i.next_timer += 1;
            let id = i.next_timer;
            i.timers.insert(id, Box::new(f));
            id
        };
        self.request(Request::Timer { id, delay });
        id
    }

    /// Cancels a timer that has not fired.
    pub fn cancel_timer(&self, id: u64) {
        if self.0.borrow_mut().timers.remove(&id).is_some() {
            self.request(Request::CancelTimer(id));
        }
    }

    /// Runs a due timer (the runner calls this).
    pub fn fire_timer(&self, id: u64) {
        let f = self.0.borrow_mut().timers.remove(&id);
        if let Some(f) = f {
            f();
        }
    }

    /// The window's content element (a column filling the window, below overlays).
    pub fn root(&self) -> El {
        let id = self.0.borrow().content;
        El { ui: self.clone(), id }
    }

    /// A new element styled as `widget`. It shows once added to a parent.
    #[track_caller]
    pub fn el(&self, widget: &'static str) -> El {
        self.el_at(widget, std::panic::Location::caller())
    }

    /// A new element styled as `widget`, built at `at` in the source (for builders that
    /// make an element later than the call that asked for it).
    pub fn el_at(&self, widget: &'static str, at: &'static std::panic::Location<'static>) -> El {
        let mut i = self.0.borrow_mut();
        let id = i.tree.create(widget);
        let style = LayoutStyle::default();
        let node = i.layout.node(&style).unwrap_or_else(|_| unreachable_root());
        let base = i.base.clone();
        let resolved = i.sheet.resolve(
            &StyleKey { widget, variant: "", states: States::NONE },
            &base,
            &StylePatch::default(),
        );
        let mut data = ElData::new(widget, style, node, resolved);
        data.created_at = Some(at);
        i.els.insert(id, data);
        i.tree.mark(id, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT);
        El { ui: self.clone(), id }
    }

    /// The topmost element under `p` (window logical pixels), as input would find it.
    pub fn element_at(&self, p: remedy_ui_core::Point) -> Option<El> {
        let hit = self.tree().hit_test(p)?;
        Some(El { ui: self.clone(), id: hit })
    }

    /// The window's three layer roots: content, overlays, and the trusted layer.
    pub fn layers(&self) -> [El; 3] {
        let i = self.0.borrow();
        [i.content, i.overlays, i.trusted].map(|id| El { ui: self.clone(), id })
    }

    /// Sees pointer input before any element: `f` returning true consumes the event. For
    /// tools such as the inspector's element picker; `None` removes it.
    pub fn set_pointer_interceptor(&self, f: Option<PointerInterceptor>) {
        self.0.borrow_mut().interceptor = f;
    }

    /// A handle to an existing element.
    pub fn element(&self, id: ElementId) -> El {
        El { ui: self.clone(), id }
    }

    /// The element tree, for events, focus and hit testing.
    pub fn tree(&self) -> Tree {
        self.0.borrow().tree.clone()
    }

    /// The theme.
    pub fn theme(&self) -> Theme {
        self.0.borrow().theme.clone()
    }

    /// Changes the theme and its style sheet; every element restyles.
    pub fn set_theme(&self, theme: Theme, sheet: StyleSheet) {
        let mut i = self.0.borrow_mut();
        i.base = defaults(&theme);
        if theme.font_families != i.theme.font_families
            || theme.mono_families != i.theme.mono_families
            || theme.heading_families != i.theme.heading_families
            || theme.text_scale != i.theme.text_scale
        {
            i.reshape_text = true;
        }
        i.theme = theme;
        i.sheet = sheet;
        let ids: Vec<ElementId> = i.els.keys().copied().collect();
        // Tokens resolve again: an element given `Space::S3` gets three units of this theme.
        let theme = i.theme.clone();
        let mut relayout = Vec::new();
        for (id, d) in &mut i.els {
            if !d.style_src.is_plain() || d.style_bound.as_ref().is_some_and(|s| !s.is_plain()) {
                d.local = crate::view::tokens::resolve_local(d, &theme);
            }
            if !d.layout_tokens.is_empty() {
                crate::view::tokens::apply_layout_tokens(d, &theme);
                relayout.push(*id);
            }
        }
        for id in ids {
            i.tree.mark(id, Dirty::STYLE);
        }
        for id in relayout {
            i.tree.mark(id, Dirty::LAYOUT);
        }
        drop(i);
        // As every other change that restyles: the restyle needs a frame to show.
        self.request(Request::Frame);
    }

    /// Adds every face in a font file (a theme's font, a person's own) to this window's fonts,
    /// returning the family names it holds; every text is shaped again so a family that was
    /// missing shows. Bytes that are not a font this engine can read add nothing and return an
    /// error.
    pub fn add_font(&self, data: Vec<u8>) -> Result<Vec<String>, remedy_ui_text::FontError> {
        self.update_fonts(|fonts| fonts.add_owned(data))
    }

    /// Changes this window's fonts with `f` (adds faces, maps a generic family such as
    /// `system-ui`), then shapes every text again so the change shows.
    pub fn update_fonts<R>(&self, f: impl FnOnce(&mut remedy_ui_text::FontCollection) -> R) -> R {
        let engine = self.0.borrow().engine.clone();
        let out = f(engine.borrow_mut().fonts());
        self.fonts_changed();
        out
    }

    /// Shapes every text again with the window's fonts: after faces arrive that text was
    /// waiting for (a browser page's fallback faces, fetched when text first needed them;
    /// `FontCollection::add_on_demand`), so what showed without them is drawn with them.
    pub fn fonts_changed(&self) {
        let mut i = self.0.borrow_mut();
        i.reshape_text = true;
        let ids: Vec<ElementId> = i.els.keys().copied().collect();
        for id in ids {
            i.tree.mark(id, Dirty::STYLE);
        }
        drop(i);
        self.request(Request::Frame);
    }

    /// Every font family this window can draw with (its own and those added).
    pub fn font_families(&self) -> Vec<String> {
        let engine = self.0.borrow().engine.clone();
        let mut names = engine.borrow_mut().fonts().families();
        names.sort();
        names.dedup();
        names
    }

    /// Requests for the host since the last call.
    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.0.borrow_mut().requests)
    }

    /// Sets the clock edits read to group typing into undo steps (a pause of
    /// [`UNDO_PAUSE_MS`](remedy_ui_text::UNDO_PAUSE_MS) starts a new step): the host's time,
    /// which the runner sets before it delivers each event.
    pub fn set_clock(&self, now: std::time::Duration) {
        self.0.borrow_mut().now_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
    }

    /// The scroll offset the compositor reports for a scroll container, so hit testing
    /// matches what is shown.
    pub fn set_scroll_offset(&self, el: ElementId, offset: Vec2) {
        let cb = {
            let mut i = self.0.borrow_mut();
            if i.scroll_offsets.get(&el) == Some(&offset) {
                return;
            }
            i.scroll_offsets.insert(el, offset);
            i.frame.bounds_stale = true;
            i.els.get(&el).and_then(|d| d.on_scroll.clone())
        };
        // Window positions inside the container moved: the next frame recomputes them (no
        // repaint: the compositor already moved the pixels).
        self.request(Request::Frame);
        if let Some(cb) = cb {
            cb(offset);
        }
    }

    /// The scroll offset of a scroll container, as last reported.
    pub fn scroll_offset(&self, el: ElementId) -> Vec2 {
        self.0.borrow().scroll_offsets.get(&el).copied().unwrap_or(Vec2::ZERO)
    }

    /// Accessibility: start producing AccessKit updates (a screen reader connected).
    pub fn connect_accessibility(&self) {
        let mut i = self.0.borrow_mut();
        i.a11y.connect();
        i.structure_changed = true;
        drop(i);
        self.request(Request::Frame);
    }

    /// Accessibility: stop producing updates (the screen reader went away).
    pub fn disconnect_accessibility(&self) {
        self.0.borrow_mut().a11y.disconnect();
    }

    /// Sets a text field's whole text as assistive technology asks (a screen reader's "set
    /// text"): one undoable edit, reported to the app like typing. False for an element that
    /// is gone or not a field, and for secret fields, whose contents are only ever typed.
    pub fn accessibility_set_value(&self, target: u64, text: &str) -> bool {
        let Some(id) = self.element_by_node_id(target).map(|e| e.id) else { return false };
        let Some(field) = self.field_of(id) else { return false };
        let now = self.0.borrow().now_ms;
        let done = crate::view::field::replace_all(&mut field.borrow_mut(), text, now);
        if done {
            self.field_edited(id, &field, true);
        }
        done
    }

    /// Does what assistive technology asked of an element: click, focus, increment or
    /// decrement (as the arrow keys would), or scroll it into view. Returns false for an
    /// element that is gone or an action it does not offer.
    pub fn accessibility_action(&self, target: u64, action: remedy_ui_core::a11y::Action) -> bool {
        use remedy_ui_core::a11y::Action as A;
        let Some(id) = self.element_by_node_id(target).map(|e| e.id) else { return false };
        let el = El { ui: self.clone(), id };
        let own = self.0.borrow().els.get(&id).and_then(|d| d.a11y_action.clone());
        if let Some(f) = own
            && remedy_ui_core::reactive::batch(|| f(action))
        {
            return true;
        }
        let tree = self.tree();
        let arrow = |k: remedy_ui_core::input::Key| {
            let e = remedy_ui_core::input::KeyEvent::press(k);
            tree.dispatch(id, &e).is_ok_and(|d| d.handled)
        };
        match action {
            A::Click => {
                // A disabled control refuses assistive technology as it refuses the pointer
                // (a trusted prompt's buttons stay disabled for a moment for that reason).
                let clickable = self.clickable(id);
                if clickable {
                    let _ = tree.dispatch(id, &crate::view::events::Click { position: None });
                }
                clickable
            }
            A::Focus => {
                if !tree.is_focusable(id) {
                    return false;
                }
                el.focus();
                true
            }
            A::Increment => arrow(remedy_ui_core::input::Key::ArrowUp),
            A::Decrement => arrow(remedy_ui_core::input::Key::ArrowDown),
            A::ScrollIntoView => {
                el.scroll_into_view();
                true
            }
            // Setting text needs the text: `accessibility_set_value`.
            A::Expand | A::Collapse | A::ShowContextMenu | A::SetValue => false,
            // An action newer than this view is reported as not performed.
            _ => false,
        }
    }

    /// The next accessibility update, when connected and something changed.
    pub fn take_accessibility_update(&self) -> Option<remedy_ui_core::a11y::AccessKitUpdate> {
        self.0.borrow_mut().a11y.take_update()
    }

    /// Asks for a frame (the next frame redraws only what changed).
    pub fn request_frame(&self) {
        self.request(Request::Frame);
    }

    /// Puts `text` on the clipboard (not sensitive: it may go to clipboard history). For
    /// widgets with their own selections, such as a paged document's text.
    pub fn copy_text(&self, text: impl Into<String>) {
        self.request(Request::Copy { text: text.into(), sensitive: false, html: None });
    }

    pub(crate) fn request(&self, r: Request) {
        let wake = {
            let mut i = self.0.borrow_mut();
            if r == Request::Frame && i.requests.contains(&Request::Frame) {
                return;
            }
            let first = i.requests.is_empty();
            i.requests.push(r);
            first.then(|| i.wake.clone()).flatten()
        };
        // A request made outside the host's event handling (a timer-free callback, work
        // posted from another thread) still reaches the host.
        if let Some(w) = wake {
            w();
        }
    }

    /// Sets what wakes the host's loop when requests arrive (the runner does this).
    pub fn set_wake(&self, wake: Rc<dyn Fn()>) {
        self.0.borrow_mut().wake = Some(wake);
    }

    /// Marks `id` for restyling: it moved under a new parent. Only `id` itself: what its
    /// children inherit comes from its resolved style alone, so the style pass restyles them
    /// when that changes, level by level. Attaching a subtree costs the same at any size, so
    /// a tree built from the leaves up is built in linear time.
    pub(crate) fn restyle_moved(&self, id: ElementId) {
        self.0.borrow().tree.mark(id, Dirty::STYLE);
    }

    pub(crate) fn mark(&self, id: ElementId, flags: Dirty) {
        let i = self.0.borrow();
        i.tree.mark(id, flags);
        let observers: Vec<DirtyObserver> = if i.dirty_observers.is_empty() {
            Vec::new()
        } else {
            i.dirty_observers.iter().map(|(_, f)| f.clone()).collect()
        };
        drop(i);
        if !observers.is_empty() {
            let record = DirtyRecord {
                element: id,
                style: flags.contains(Dirty::STYLE),
                layout: flags.contains(Dirty::LAYOUT),
                paint: flags.contains(Dirty::PAINT),
                semantics: flags.contains(Dirty::SEMANTICS),
                cause: remedy_ui_core::reactive::debug::current(),
                at_ns: remedy_ui_core::trace::now_ns(),
            };
            for observe in observers {
                observe(&record);
            }
        }
        self.request(Request::Frame);
    }

    /// This view's id, unique in the process: trace spans name the window they belong to
    /// by it.
    pub fn id(&self) -> u64 {
        self.0.borrow().id
    }

    /// Calls `f` for every element marked for work (restyle, relayout, repaint, semantics),
    /// with what marked it, until [`Ui::remove_dirty_observer`] is called with the returned
    /// id. Devtools use it to say why each element updated; turn on
    /// [`remedy_ui_core::reactive::debug::trace`] for the causes. With no observer, marking costs
    /// nothing more.
    pub fn add_dirty_observer(&self, f: DirtyObserver) -> u64 {
        let mut i = self.0.borrow_mut();
        let id = i.next_observer;
        i.next_observer += 1;
        i.dirty_observers.push((id, f));
        id
    }

    /// Stops calling the observer [`Ui::add_dirty_observer`] returned `id` for.
    pub fn remove_dirty_observer(&self, id: u64) {
        if let Ok(mut i) = self.0.try_borrow_mut() {
            i.dirty_observers.retain(|(o, _)| *o != id);
        }
    }

    /// What the last frame did ([`FrameStats`](crate::view::FrameStats)).
    pub fn last_frame_stats(&self) -> crate::view::frame::FrameStats {
        self.0.borrow().last_stats.clone()
    }

    /// The element owning the binding or painter `node` (a key of the reactive graph), if it
    /// is one of this view's.
    pub fn element_of_binding(&self, node: remedy_ui_core::reactive::debug::NodeKey) -> Option<ElementId> {
        let i = self.0.borrow();
        i.els.iter().find_map(|(id, d)| {
            let mine = d.effects.iter().any(|e| e.key() == node)
                || d.paint_watch.as_ref().is_some_and(|w| w.key() == node);
            mine.then_some(*id)
        })
    }

    /// The element whose accessibility node id ([`El::node_id`]) is `node`, while it lives.
    pub fn element_by_node_id(&self, node: u64) -> Option<El> {
        let index = u32::try_from(node & 0xFFFF_FFFF).ok()?;
        let generation = u32::try_from(node >> 32).ok()?;
        let id = ElementId::from_parts(index, generation);
        self.0.borrow().els.contains_key(&id).then(|| El { ui: self.clone(), id })
    }

    /// The window as a screen reader meets it, in reading order (see
    /// [`remedy_ui_core::a11y::A11yTree::reader_view`]), as of the last frame.
    pub fn reader_view(&self) -> Vec<remedy_ui_core::a11y::ReaderNode> {
        self.0.borrow().a11y.reader_view()
    }

    /// True when a showing modal (`Semantics::modal`) makes `el` or one of its ancestors
    /// inert: it is behind the modal, out of a screen reader's reach on purpose.
    pub fn behind_modal(&self, el: ElementId) -> bool {
        let i = self.0.borrow();
        let mut cur = Some(el);
        while let Some(id) = cur {
            if i.a11y.is_inert(root_key(id)) {
                return true;
            }
            cur = i.tree.parent(id);
        }
        false
    }
}

impl El {
    /// The element's id in the accessibility tree (what screen readers and agents name it
    /// by, and what [`Ui::accessibility_action`] takes).
    pub fn node_id(&self) -> u64 {
        root_key(self.id)
    }
}

pub(crate) fn root_key(id: ElementId) -> u64 {
    (u64::from(id.generation()) << 32) | u64::from(id.index())
}

fn unreachable_root() -> LayoutNode {
    // Creating a plain node on a fresh taffy tree cannot fail; this keeps the signature total.
    unreachable!("taffy refused a plain node")
}

impl ElData {
    fn new(widget: &'static str, layout: LayoutStyle, node: LayoutNode, resolved: ResolvedStyle) -> Self {
        ElData {
            widget,
            variant: "",
            states: States::NONE,
            local: StylePatch::default(),
            resolved,
            layout,
            node,
            text: None,
            inline_images: Rc::default(),
            field: None,
            painter: None,
            paint_watch: None,
            semantics: None,
            scroll: false,
            clip: false,
            click: None,
            hover: None,
            focus: None,
            focus_within: None,
            text_client: None,
            pointer: false,
            keeps_focus: false,
            focus_group: false,
            focus_wrap: false,
            drag_source: None,
            selectable: false,
            sel: None,
            marks: Vec::new(),
            links: None,
            inline_link: None,
            drop_target: None,
            drag_hover: None,
            scroll_x: false,
            scroll_lead: 0.0,
            on_resize: None,
            last_size: None,
            on_visibility: None,
            last_visible: None,
            visibility_margin: 0.0,
            on_placed: None,
            last_placed: None,
            scroll_follow: None,
            scroll_snap: None,
            snap_align: None,
            snap_margin: remedy_ui_core::Insets::default(),
            snap_stop: false,
            isolate: None,
            loops: Vec::new(),
            ambient_layer: false,
            see_through: false,
            ambient_last: None,
            data_text: false,
            keyboard_alternative: None,
            hidden: false,
            inert: false,
            pass_through: false,
            leaving: false,
            collapsed: false,
            removed: Vec::new(),
            on_scroll: None,
            chunk: None,
            effects: Vec::new(),
            prop_bindings: Vec::new(),
            style_src: remedy_ui_core::style::Style::default(),
            style_bound: None,
            layout_tokens: Vec::new(),
            anim_style: StylePatch::default(),
            anims: Vec::new(),
            shift: (Vec2::ZERO, 1.0),
            flip: None,
            a11y_action: None,
            created_at: None,
        }
    }
}

impl El {
    /// The element's id in the tree.
    pub fn id(&self) -> ElementId {
        self.id
    }

    /// The UI it belongs to.
    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    pub(crate) fn edit(&self, flags: Dirty, f: impl FnOnce(&mut ElData)) {
        {
            let mut i = self.ui.0.borrow_mut();
            if let Some(d) = i.els.get_mut(&self.id) {
                f(d);
            }
        }
        self.ui.mark(self.id, flags);
    }

    /// Runs `f` now and again whenever the signals it reads change, for as long as the
    /// element lives.
    #[track_caller]
    pub fn bind(self, f: impl FnMut() + 'static) -> Self {
        self.bind_named("bind", f)
    }

    /// Like [`El::bind`], with a debug name devtools show for the binding ("text", "style").
    #[track_caller]
    pub fn bind_named(self, name: &'static str, f: impl FnMut() + 'static) -> Self {
        let effect = self.owned_effect(f).named(name);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.effects.push(effect);
        }
        self
    }

    /// An effect owned by the element's scope (so it lives exactly as long as the element,
    /// whatever was running when it was made), or by the current owner when the element has
    /// no scope.
    #[track_caller]
    fn owned_effect(&self, f: impl FnMut() + 'static) -> Effect {
        match self.scope() {
            Some(s) => Effect::new_in(s.into(), f),
            None => Effect::new(f),
        }
    }

    /// Adds a child at the end: an element, or a builder that becomes one ([`crate::view::show`]).
    pub fn child(self, child: impl Into<El>) -> Self {
        let child: El = child.into();
        {
            let mut i = self.ui.0.borrow_mut();
            let _ = i.tree.append(self.id, child.id);
            let nodes = (i.els.get(&self.id).map(|d| d.node), i.els.get(&child.id).map(|d| d.node));
            let placed = i.els.get(&child.id).is_some_and(|d| d.inline_link.is_some());
            if let (Some(p), Some(c), false) = (nodes.0, nodes.1, placed) {
                let _ = i.layout.add_child(p, c);
            }
            i.structure_changed = true;
        }
        self.ui.mark(self.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        self.ui.restyle_moved(child.id);
        self
    }

    /// Puts `child` at `index` among this element's children, moving it there if it is
    /// already a child (of this or another element).
    pub fn insert_child(&self, index: usize, child: &El) {
        {
            let mut i = self.ui.0.borrow_mut();
            let old_parent = i.tree.parent(child.id);
            if old_parent.is_some() {
                let _ = i.tree.detach(child.id);
            }
            let n = i.tree.children(self.id).len();
            let _ = i.tree.insert(self.id, index.min(n), child.id);
            // Layout children follow the tree's order.
            for p in [Some(self.id), old_parent].into_iter().flatten() {
                let kids: Vec<LayoutNode> = i
                    .tree
                    .children(p)
                    .iter()
                    .filter_map(|k| i.els.get(k).filter(|d| d.inline_link.is_none()).map(|d| d.node))
                    .collect();
                if let Some(node) = i.els.get(&p).map(|d| d.node) {
                    let _ = i.layout.set_children(node, &kids);
                }
            }
            i.structure_changed = true;
        }
        self.ui.mark(self.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        self.ui.restyle_moved(child.id);
    }

    /// The element's children, in order.
    pub fn child_elements(&self) -> Vec<El> {
        self.ui.tree().children(self.id).into_iter().map(|id| El { ui: self.ui.clone(), id }).collect()
    }

    /// Adds children in order.
    pub fn children(self, children: impl IntoIterator<Item = El>) -> Self {
        children.into_iter().fold(self, El::child)
    }

    /// Removes the element and everything under it.
    pub fn remove(&self) {
        let ids = {
            let i = self.ui.0.borrow();
            let mut ids = i.tree.descendants(self.id);
            ids.push(self.id);
            ids
        };
        let parent = self.ui.0.borrow().tree.parent(self.id);
        let mut effects = Vec::new();
        let mut removed = Vec::new();
        {
            let mut i = self.ui.0.borrow_mut();
            for id in &ids {
                if let Some(d) = i.els.remove(id) {
                    if d.focus_group {
                        i.group_memory.remove(id);
                    }
                    let _ = i.layout.remove(d.node);
                    effects.extend(d.effects);
                    if let Some(w) = d.paint_watch {
                        w.dispose();
                    }
                    removed.extend(d.removed);
                    // Who waited for a move of it to end hears that it was cut short.
                    removed
                        .extend(crate::view::motion::cut_short(d.anims, crate::view::motion::End::Removed));
                }
                i.scroll_offsets.remove(id);
                i.boundaries.remove(id);
                if i.text_sel.is_some_and(|s| s.anchor.0 == *id || s.focus.0 == *id) {
                    i.text_sel = None;
                    i.selecting = false;
                }
                if i.pressed == Some(*id) {
                    i.pressed = None;
                }
            }
            // Once for all of them: the hover chain can be as long as the tree is deep.
            let gone: std::collections::HashSet<ElementId> = ids.iter().copied().collect();
            i.hovered.retain(|h| !gone.contains(h));
            i.cursor_hiding.retain(|h| !gone.contains(h));
            i.cursors.retain(|h, _| !gone.contains(h));
            i.structure_changed = true;
        }
        // Removing from the tree disposes the elements' scopes, which runs the app's clean-ups
        // (a view closing the dialog it opened on the overlay): the view is not borrowed then.
        let _ = self.ui.tree().remove(self.id);
        // Focus went with it (to an element around it, or nowhere): the view follows.
        if self.ui.0.borrow().focus_shown != self.ui.tree().focused() {
            crate::view::events::focus_changed(&self.ui);
        }
        for e in effects {
            e.dispose();
        }
        for f in removed {
            f();
        }
        if let Some(p) = parent {
            self.ui.mark(p, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        }
    }

    /// Removes every child.
    pub fn clear(&self) {
        let kids = self.ui.0.borrow().tree.children(self.id);
        for k in kids {
            El { ui: self.ui.clone(), id: k }.remove();
        }
    }

    /// Sets the theme variant (`primary`, `danger`, …), or binds it to a signal or closure.
    #[track_caller]
    pub fn variant(self, variant: impl IntoProp<&'static str>) -> Self {
        self.prop("variant", variant, |me, v| me.edit(Dirty::STYLE, |d| d.variant = v));
        self
    }

    /// Overrides style properties on this element: a [`StylePatch`] of values or a
    /// [`remedy_ui_core::style::Style`] of tokens (`Style::new().radius(Radius::R3)`), which
    /// resolve through the theme and again when it changes. Constants merge, later over
    /// earlier; a signal or closure binds a layer over them that follows its signals.
    #[track_caller]
    pub fn style(self, style: impl IntoProp<remedy_ui_core::style::Style>) -> Self {
        match style.into_prop() {
            Prop::Value(s) => {
                let theme = self.ui.0.borrow().theme.clone();
                self.edit(Dirty::STYLE, |d| {
                    d.style_src = d.style_src.merge(&s);
                    d.local = crate::view::tokens::resolve_local(d, &theme);
                });
            }
            Prop::Dynamic(f) => {
                let me = self.clone();
                self.set_binding("style", move || {
                    let s = f();
                    let theme = me.ui.0.borrow().theme.clone();
                    me.edit(Dirty::STYLE, |d| {
                        d.style_bound = Some(s);
                        d.local = crate::view::tokens::resolve_local(d, &theme);
                    });
                });
            }
        }
        self
    }

    /// Binds a property: runs `f` now and whenever its signals change, replacing the
    /// binding this property had.
    #[track_caller]
    pub(crate) fn set_binding(&self, name: &'static str, f: impl FnMut() + 'static) {
        self.drop_binding(name);
        let effect = self.owned_effect(f).named(name);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.effects.push(effect);
            d.prop_bindings.push((name, effect));
        }
    }

    /// Sets a property from a value, signal or closure: `set` runs once for a value, and
    /// for a signal or closure now and whenever what it reads changes. Setting the property
    /// again replaces its binding. Widgets use it for their own properties.
    #[track_caller]
    pub fn prop<T: 'static>(
        &self,
        name: &'static str,
        value: impl IntoProp<T>,
        set: impl Fn(&El, T) + 'static,
    ) {
        match value.into_prop() {
            Prop::Value(v) => {
                self.drop_binding(name);
                set(self, v);
            }
            Prop::Dynamic(f) => {
                let me = self.clone();
                self.set_binding(name, move || set(&me, f()));
            }
        }
    }

    fn drop_binding(&self, name: &'static str) {
        let old = self.ui.0.borrow_mut().els.get_mut(&self.id).and_then(|d| {
            let at = d.prop_bindings.iter().position(|(n, _)| *n == name)?;
            let (_, e) = d.prop_bindings.remove(at);
            d.effects.retain(|x| x.key() != e.key());
            Some(e)
        });
        if let Some(e) = old {
            e.dispose();
        }
    }

    /// Edits the layout style.
    pub fn layout(self, f: impl FnOnce(&mut LayoutStyle)) -> Self {
        self.edit(Dirty::LAYOUT, |d| f(&mut d.layout));
        self
    }

    /// Sticks the element within its nearest scroll container as that scrolls (CSS
    /// `position: sticky`): laid out in the flow, it keeps the set sides' distances from the
    /// container's edges and never leaves its parent's content box. The compositor moves it,
    /// with no new layout; hit testing and the accessibility bounds follow. A style sheet sets
    /// the same with [`Style::sticky`](remedy_ui_core::style::Style::sticky).
    pub fn sticky(self, insets: remedy_ui_core::style::StickyInsets) -> Self {
        let side = |v: Option<f32>| v.unwrap_or(remedy_ui_core::layout::AUTO);
        self.layout(|l| {
            l.position = remedy_ui_core::layout::Position::Sticky;
            l.inset = Insets {
                top: side(insets.top),
                end: side(insets.end),
                bottom: side(insets.bottom),
                start: side(insets.start),
            };
        })
    }

    /// Lays children out in a row.
    pub fn row(self) -> Self {
        self.layout(|l| {
            l.display = remedy_ui_core::layout::Display::Flex;
            l.axis = remedy_ui_core::layout::Axis::Row;
        })
    }

    /// Lays children out in a column.
    pub fn column(self) -> Self {
        self.layout(|l| {
            l.display = remedy_ui_core::layout::Display::Flex;
            l.axis = remedy_ui_core::layout::Axis::Column;
        })
    }

    /// Space between children: logical pixels or a spacing token (`Space::S3`), which
    /// follows the theme.
    pub fn gap(self, gap: impl Into<remedy_ui_core::style::Dp>) -> Self {
        self.layout_token(crate::view::tokens::LayoutField::Gap, gap.into())
    }

    /// Fixed width: logical pixels or a token.
    pub fn width(self, w: impl Into<remedy_ui_core::style::Dp>) -> Self {
        self.layout_token(crate::view::tokens::LayoutField::Width, w.into())
    }

    /// Fixed height: logical pixels or a token.
    pub fn height(self, h: impl Into<remedy_ui_core::style::Dp>) -> Self {
        self.layout_token(crate::view::tokens::LayoutField::Height, h.into())
    }

    fn layout_token(self, field: crate::view::tokens::LayoutField, v: remedy_ui_core::style::Dp) -> Self {
        let theme = self.ui.0.borrow().theme.clone();
        self.edit(Dirty::LAYOUT, |d| {
            d.layout_tokens.retain(|(f, _)| *f != field);
            if v.is_token() {
                d.layout_tokens.push((field, v));
            }
            field.set(&mut d.layout, v.resolve(&theme));
        });
        self
    }

    /// Takes a share of the free space along the parent's axis. An element with children
    /// that grows takes the room there is, however wide its content: set `min_width` or
    /// `min_height` to hold it to more. Text and other measured content keeps to its
    /// smallest size by itself.
    pub fn grow(self, g: f32) -> Self {
        self.layout(|l| l.grow = g)
    }

    /// Fills the parent's width.
    pub fn fill_width(self) -> Self {
        self.layout(|l| l.width = remedy_ui_core::layout::Length::Percent(1.0))
    }

    /// How children sit across the axis.
    pub fn align(self, a: remedy_ui_core::layout::Align) -> Self {
        self.layout(|l| l.align_items = a)
    }

    /// How children spread along the axis.
    pub fn justify(self, j: remedy_ui_core::layout::Justify) -> Self {
        self.layout(|l| l.justify = j)
    }

    /// Calls `f` for key presses while something inside the element has focus, before the
    /// focused element sees them (the capture phase); `f` returns true when it handled the
    /// key, and then the focused element never gets it. For app-wide shortcuts that must
    /// work even while a text field has focus (a way back, a panic key); keep them to
    /// chords a text field would not use, since the field loses them.
    pub fn on_key_capture(self, f: impl Fn(&remedy_ui_core::input::KeyEvent) -> bool + 'static) -> Self {
        let _ =
            self.ui.tree().on::<remedy_ui_core::input::KeyEvent>(self.id, Phase::Capture, move |cx, e| {
                if f(e) {
                    cx.set_handled();
                    cx.stop_propagation();
                }
            });
        self
    }

    /// Calls `f` for key presses while the element or something inside it has focus; `f`
    /// returns true when it handled the key.
    pub fn on_key(self, f: impl Fn(&remedy_ui_core::input::KeyEvent) -> bool + 'static) -> Self {
        let _ = self.ui.tree().on::<remedy_ui_core::input::KeyEvent>(self.id, Phase::Bubble, move |cx, e| {
            if f(e) {
                cx.set_handled();
                cx.stop_propagation();
            }
        });
        self
    }

    /// Makes the element a drag handle for its window (custom title bars): a primary press on
    /// it, or on anything inside it that is not itself interactive, calls `start` (which
    /// begins the platform's window move).
    pub fn drag_region(self, start: impl Fn() + 'static) -> Self {
        let ui = self.ui.clone();
        let _ =
            self.ui.tree().on::<remedy_ui_core::input::PointerEvent>(self.id, Phase::Bubble, move |cx, e| {
                if e.phase != remedy_ui_core::input::PointerPhase::Down
                    || !e.buttons.contains(remedy_ui_core::input::Buttons::PRIMARY)
                {
                    return;
                }
                let tree = ui.tree();
                let interactive = |id: ElementId| {
                    tree.is_focusable(id)
                        || ui.0.borrow().els.get(&id).is_some_and(|d| d.click.is_some() || d.field.is_some())
                };
                // Anything between the press and the handle that takes input keeps the press.
                let mut cur = Some(cx.target());
                while let Some(id) = cur {
                    if id == cx.current() {
                        break;
                    }
                    if interactive(id) {
                        return;
                    }
                    cur = tree.parent(id);
                }
                start();
                cx.set_handled();
            });
        self
    }

    /// Calls `f` with wheel and touchpad scrolls over the element (or over something inside
    /// it that did not take them); `f` returns true when it used the scroll. Scrolls nobody
    /// takes move the scroll containers under the pointer on the compositor, so only
    /// elements that do their own scrolling or zooming (a paged document, Ctrl and the wheel)
    /// need this.
    pub fn on_wheel(self, f: impl Fn(&remedy_ui_core::input::WheelEvent) -> bool + 'static) -> Self {
        let _ =
            self.ui.tree().on::<remedy_ui_core::input::WheelEvent>(self.id, Phase::Bubble, move |cx, e| {
                if f(e) {
                    cx.set_handled();
                    cx.stop_propagation();
                }
            });
        self
    }

    /// Calls `f` with touchpad pinches over the element; `f` returns true when it zoomed.
    pub fn on_pinch(self, f: impl Fn(&remedy_ui_core::input::PinchEvent) -> bool + 'static) -> Self {
        let _ =
            self.ui.tree().on::<remedy_ui_core::input::PinchEvent>(self.id, Phase::Bubble, move |cx, e| {
                if f(e) {
                    cx.set_handled();
                    cx.stop_propagation();
                }
            });
        self
    }

    /// Calls `f` with pointer input on the element (window logical pixels).
    pub fn on_pointer(self, f: impl Fn(&remedy_ui_core::input::PointerEvent) -> bool + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.pointer = true;
        }
        let _ =
            self.ui.tree().on::<remedy_ui_core::input::PointerEvent>(self.id, Phase::Bubble, move |cx, e| {
                if f(e) {
                    cx.set_handled();
                    cx.stop_propagation();
                }
            });
        self
    }

    /// Moves focus to the first focusable element inside this one (a dialog opening).
    /// Returns false when there is none.
    pub fn focus_first(&self) -> bool {
        let tree = self.ui.tree();
        let first = tree.descendants(self.id).into_iter().find(|d| tree.can_focus(*d));
        match first {
            Some(f) => {
                El { ui: self.ui.clone(), id: f }.focus();
                true
            }
            None => false,
        }
    }

    /// Moves focus to the next (`forward`) or previous focusable element inside this one,
    /// from the focused one, wrapping around: arrow keys between the controls of a tile.
    /// Returns false when there is none.
    pub fn focus_step(&self, forward: bool) -> bool {
        let tree = self.ui.tree();
        let all: Vec<ElementId> =
            tree.descendants(self.id).into_iter().filter(|d| tree.can_focus(*d)).collect();
        let n = all.len();
        let at = tree.focused().and_then(|f| all.iter().position(|d| *d == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => n.saturating_sub(1),
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        let Some(id) = all.get(next).copied() else { return false };
        El { ui: self.ui.clone(), id }.focus();
        true
    }

    /// Calls `f` with the element's size (logical pixels) after each frame in which it
    /// changed, including the first: virtual lists rebuild for a taller viewport, charts
    /// re-scale. An element may have several: each is called, in the order they were set.
    pub fn on_resize(self, f: impl Fn(Size) + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            let all: Rc<dyn Fn(Size)> = match d.on_resize.take() {
                Some(first) => Rc::new(move |s: Size| {
                    first(s);
                    f(s);
                }),
                None => Rc::new(f),
            };
            d.on_resize = Some(all);
            d.last_size = None;
        }
        self.ui.mark(self.id, Dirty::LAYOUT);
        self
    }

    /// Calls `f` with whether the element is seen: once after the first frame, then each
    /// time that changes. It is seen when some of it is inside the window and inside every
    /// scroll container or clipping element around it, nothing around it is hidden or fully
    /// transparent, and the window is not minimized or covered ([`Ui::set_occluded`]).
    /// Animations pause while unseen; lazy content loads when first seen.
    pub fn on_visibility(self, f: impl Fn(bool) + 'static) -> Self {
        self.on_visibility_within(0.0, f)
    }

    /// Like [`on_visibility`](Self::on_visibility), counting the element as seen once it is
    /// within `margin` logical pixels of the window and of every scroll container or clipping
    /// element around it (CSS `IntersectionObserver`'s `rootMargin`): content near the
    /// viewport loads before it scrolls in, as `loading="lazy"` does on the web.
    pub fn on_visibility_within(self, margin: f32, f: impl Fn(bool) + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.on_visibility = Some(Rc::new(f));
            d.last_visible = None;
            d.visibility_margin = if margin.is_finite() { margin.max(0.0) } else { 0.0 };
        }
        self.ui.request(Request::Frame);
        self
    }

    /// Lets a scroll container scroll sideways too, when its content is wider than it.
    pub fn scroll_x(self) -> Self {
        self.edit(Dirty::LAYOUT | Dirty::PAINT, |d| d.scroll_x = true);
        self
    }

    /// Makes this scroll container move with `leader` on the chosen axes, in the same frame:
    /// a table's frozen columns follow its body vertically, its header horizontally.
    pub fn scroll_follows(self, leader: &El, x: bool, y: bool) -> Self {
        let lead = leader.id;
        self.edit(Dirty::PAINT, |d| d.scroll_follow = Some((lead, x, y)));
        self
    }

    fn snap_spec(self, f: impl FnOnce(&mut SnapSpec)) -> Self {
        self.edit(Dirty::PAINT, |d| f(d.scroll_snap.get_or_insert_with(Default::default)));
        self
    }

    /// Makes this scroll container come to rest on its descendants' snap points
    /// ([`snap_align`](Self::snap_align)), as CSS `scroll-snap-type` does: on `axis`, always
    /// ([`SnapStrictness::Mandatory`]) or only when one is near ([`SnapStrictness::Proximity`]).
    /// It settles after a fling (on the point for where the fling would end), a wheel gesture,
    /// a drag release, a key step or page, a focus move and [`scroll_to`](Self::scroll_to)
    /// ([`scroll_to_exact`](Self::scroll_to_exact) does not snap), and stays on the same
    /// point when the layout changes. It glides there, or jumps under reduced motion.
    pub fn scroll_snap(self, axis: SnapAxis, strictness: SnapStrictness) -> Self {
        self.snap_spec(|s| {
            s.axis = Some(axis);
            s.tuning.mandatory = strictness == SnapStrictness::Mandatory;
        })
    }

    /// How near a snap point must be for a proximity snap ([`scroll_snap`](Self::scroll_snap)),
    /// as a fraction of the viewport on that axis. Default 0.3.
    pub fn scroll_snap_proximity(self, fraction: f32) -> Self {
        self.snap_spec(|s| s.tuning.proximity = fraction.max(0.0))
    }

    /// How a snapping container moves onto its snap point. Default a 250 ms decelerating
    /// tween; under reduced motion it always jumps.
    pub fn scroll_snap_motion(self, motion: remedy_ui_core::motion::Motion) -> Self {
        self.snap_spec(|s| s.tuning.motion = motion)
    }

    /// How long the wheel must be still before a snapping container settles. Default 150 ms.
    pub fn scroll_snap_wheel_idle(self, idle: std::time::Duration) -> Self {
        self.snap_spec(|s| s.tuning.wheel_idle = idle)
    }

    /// Insets the viewport a snapping container lines snap areas up with, as CSS
    /// `scroll-padding` does (logical: `start` follows the text direction).
    pub fn scroll_padding(self, padding: remedy_ui_core::Insets) -> Self {
        self.snap_spec(|s| s.padding = padding)
    }

    /// Gives this descendant of a snapping scroll container ([`scroll_snap`](Self::scroll_snap))
    /// a snap point: its start, center or end lines up with the container's, as CSS
    /// `scroll-snap-align` does.
    pub fn snap_align(self, align: SnapAlign) -> Self {
        self.snap_child(|d| d.snap_align = Some(align))
    }

    /// Grows this element's snap area, as CSS `scroll-margin` does (logical insets).
    pub fn snap_margin(self, margin: remedy_ui_core::Insets) -> Self {
        self.snap_child(|d| d.snap_margin = margin)
    }

    /// A fling, wheel gesture or key step never scrolls past this element's snap point, as
    /// CSS `scroll-snap-stop: always` does.
    pub fn snap_stop_always(self) -> Self {
        self.snap_child(|d| d.snap_stop = true)
    }

    fn snap_child(self, f: impl FnOnce(&mut ElData)) -> Self {
        self.edit(Dirty::PAINT, f);
        // The scroll container lists the snap points: it repaints.
        let tree = self.ui.tree();
        let mut cur = tree.parent(self.id);
        while let Some(p) = cur {
            self.ui.mark(p, Dirty::PAINT);
            if self.ui.0.borrow().els.get(&p).is_some_and(|d| d.scroll) {
                break;
            }
            cur = tree.parent(p);
        }
        self
    }

    /// Scrolls this scroll container to `offset`; a snapping one
    /// ([`scroll_snap`](Self::scroll_snap)) goes to the snap point nearest it.
    pub fn scroll_to(&self, offset: remedy_ui_core::Vec2) {
        self.ui.request(Request::ScrollTo { scroller: self.id, offset });
    }

    /// Scrolls this scroll container to exactly `offset`, snapping or not.
    pub fn scroll_to_exact(&self, offset: remedy_ui_core::Vec2) {
        self.ui.request(Request::ScrollToExact { scroller: self.id, offset });
    }

    /// Scrolls the scroll containers around this element just enough to show it.
    pub fn scroll_into_view(&self) {
        let size = self.ui.tree().bounds(self.id).map(|b| b.size());
        if let Some(size) = size {
            self.scroll_rect_into_view(remedy_ui_core::Rect::from_origin_size(
                remedy_ui_core::Point::ZERO,
                size,
            ));
        }
    }

    /// Scrolls the scroll containers around this element just enough to show `rect` (in the
    /// element's own coordinates): a virtual grid keeps a cursor in view whose tile is not
    /// built yet.
    pub fn scroll_rect_into_view(&self, rect: remedy_ui_core::Rect) {
        let tree = self.ui.tree();
        let Some(b) = tree.bounds(self.id) else { return };
        let mut want = rect.translate(Vec2::new(b.min.x, b.min.y));
        let mut cur = tree.parent(self.id);
        while let Some(p) = cur {
            let is_scroll = self.ui.0.borrow().els.get(&p).is_some_and(|d| d.scroll);
            if is_scroll && let Some(view) = tree.bounds(p) {
                let off = self.ui.scroll_offset(p);
                let mut next = off;
                if want.min.y < view.min.y {
                    next.y -= view.min.y - want.min.y;
                } else if want.max.y > view.max.y {
                    next.y += (want.max.y - view.max.y).min(want.min.y - view.min.y);
                }
                if want.min.x < view.min.x {
                    next.x -= view.min.x - want.min.x;
                } else if want.max.x > view.max.x {
                    next.x += (want.max.x - view.max.x).min(want.min.x - view.min.x);
                }
                next.x = next.x.max(0.0);
                next.y = next.y.max(0.0);
                if next != off {
                    // A snapping container steps to the snap point that way, gliding.
                    let snaps = self.ui.0.borrow().els.get(&p).is_some_and(|d| d.scroll_snap.is_some());
                    let scroller = p;
                    let offset = next;
                    self.ui.request(if snaps {
                        Request::ScrollStep { scroller, offset }
                    } else {
                        Request::ScrollTo { scroller, offset }
                    });
                    // The outer containers must show where this one will be.
                    want = view;
                }
            }
            cur = tree.parent(p);
        }
    }

    /// Says how keyboard users reach what this element's pointer handling does, for an
    /// element that is deliberately not focusable (a window-resize grip: the system's window
    /// menu). The accessibility check accepts pointer-only elements that say this.
    /// A closure or signal keeps it current (a translated hint follows the window's locale).
    pub fn keyboard_alternative(self, how: impl IntoProp<String>) -> Self {
        match how.into_prop() {
            Prop::Value(how) => {
                if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
                    d.keyboard_alternative = Some(how);
                }
                self
            }
            dynamic => {
                let (ui, id) = (self.ui.clone(), self.id);
                self.bind_named("keyboard_alternative", move || {
                    let how = dynamic.get();
                    if let Some(d) = ui.0.borrow_mut().els.get_mut(&id) {
                        d.keyboard_alternative = Some(how);
                    }
                })
            }
        }
    }

    /// Calls `f` on a double click (two presses within the system's double-click time, on
    /// the same spot).
    pub fn on_double_click(self, f: impl Fn() + 'static) -> Self {
        let ui = self.ui.clone();
        self.on_pointer(move |e| {
            if e.phase == remedy_ui_core::input::PointerPhase::Down
                && e.buttons.contains(remedy_ui_core::input::Buttons::PRIMARY)
                && ui.click_count() == 2
            {
                f();
                return true;
            }
            false
        })
    }

    /// Calls `f(true)` when the element gains keyboard focus and `f(false)` when it loses it.
    pub fn on_focus_change(self, f: impl Fn(bool) + 'static) -> Self {
        let f: Rc<dyn Fn(bool)> = Rc::new(f);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.focus = Some(f);
        }
        self
    }

    /// Calls `f(true)` when keyboard focus comes onto the element or anything inside it, and
    /// `f(false)` when focus leaves it and everything inside (moving between its children
    /// says nothing). The element also has [`States::FOCUS_WITHIN`](remedy_ui_core::style::States)
    /// meanwhile, for styles. A row whose actions show on hover shows them on this too, so the
    /// keyboard reaches them.
    pub fn on_focus_within(self, f: impl Fn(bool) + 'static) -> Self {
        let f: Rc<dyn Fn(bool)> = Rc::new(f);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.focus_within = Some(f);
        }
        self
    }

    /// Calls `f(true)` when the pointer comes over the element and `f(false)` when it leaves.
    /// An element may have several: each is called, in the order they were set.
    pub fn on_hover(self, f: impl Fn(bool) + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            let all: Rc<dyn Fn(bool)> = match d.hover.take() {
                Some(first) => Rc::new(move |on: bool| {
                    first(on);
                    f(on);
                }),
                None => Rc::new(f),
            };
            d.hover = Some(all);
        }
        self
    }

    /// Calls `f` with the scroll offset whenever a scroll container scrolls.
    pub fn on_scroll(self, f: impl Fn(Vec2) + 'static) -> Self {
        let f: Rc<dyn Fn(Vec2)> = Rc::new(f);
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.on_scroll = Some(f);
        }
        self
    }

    /// Keeps Tab focus inside the element (modals, menus).
    pub fn focus_scope(self) -> Self {
        let _ = self.ui.tree().set_focus_scope(self.id, true);
        self
    }

    /// Draws `image` scaled into the element's content box, keeping its aspect ratio.
    pub fn image(self, image: crate::paint::Image) -> Self {
        self.image_sampled(image, crate::paint::Sampling::Linear)
    }

    /// [`El::image`] with a choice of sampling: `Sampling::Nearest` repeats pixels when it
    /// scales up, so pixel art keeps its hard edges.
    pub fn image_sampled(self, image: crate::paint::Image, sampling: crate::paint::Sampling) -> Self {
        let (w, h) = (image.width.max(1) as f32, image.height.max(1) as f32);
        self.layout(|l| {
            if l.aspect_ratio.is_none() {
                l.aspect_ratio = Some(w / h);
            }
        })
        .paint(move |cx| {
            let s = (cx.size.width / w).min(cx.size.height / h);
            let (dw, dh) = (w * s, h * s);
            let x = (cx.size.width - dw) / 2.0;
            let y = (cx.size.height - dh) / 2.0;
            cx.list.push(crate::paint::Item::Image {
                image: image.clone(),
                rect: remedy_ui_core::Rect::new(x, y, x + dw, y + dh),
                // A rounded element rounds its picture, as `border-radius` rounds an `<img>`.
                radii: crate::view::frame::corners(cx.style, cx.rtl),
                sampling,
            });
        })
    }

    /// Padding (a style property, so themes can change it): [`Insets`], a token on every
    /// side (`Space::S3`), or a [`remedy_ui_core::style::Pad`] by side.
    #[track_caller]
    pub fn padding(self, p: impl Into<remedy_ui_core::style::Pad>) -> Self {
        self.style(remedy_ui_core::style::Style::new().padding(p))
    }

    /// Sets a state flag (selected, checked, invalid, …).
    pub fn state(&self, s: States, on: bool) {
        self.edit(Dirty::STYLE | Dirty::PAINT | Dirty::SEMANTICS, |d| d.states = d.states.with(s, on));
    }

    /// The element's current states.
    pub fn states(&self) -> States {
        self.ui.0.borrow().els.get(&self.id).map_or(States::NONE, |d| d.states)
    }

    /// Disables the element: no pointer or keyboard interaction, and the disabled look. A
    /// signal or closure keeps it in step.
    #[track_caller]
    pub fn disabled(self, on: impl IntoProp<bool>) -> Self {
        self.prop("disabled", on, |me, on| {
            me.state(States::DISABLED, on);
            me.edit(Dirty::NONE, |d| d.inert = on);
            me.sync_hit_mode();
        });
        self
    }

    /// Makes the element a text element showing `text`: a string or rich text, or a signal
    /// or closure giving one, which keeps it current.
    #[track_caller]
    pub fn text(self, text: impl IntoProp<RichText>) -> Self {
        self.prop("text", text, |me, t| me.set_text(t));
        self
    }

    fn set_text(&self, text: RichText) {
        let engine_scale = self.ui.0.borrow().scale;
        let pseudo = self.ui.0.borrow().pseudo;
        let data_text = self.ui.0.borrow().els.get(&self.id).is_some_and(|d| d.data_text);
        let slot = {
            let i = self.ui.0.borrow();
            i.els.get(&self.id).and_then(|d| d.text.clone())
        };
        match slot {
            Some(s) => {
                let mut s = s.borrow_mut();
                s.text = text;
                s.cache = None;
            }
            None => {
                // The style the element has now: a frame restyles text only when its style
                // changes, which it may not after this.
                let (style, max_lines, align) = {
                    let i = self.ui.0.borrow();
                    i.els
                        .get(&self.id)
                        .map(|d| {
                            (
                                crate::view::frame::text_style_for(d, &i.theme),
                                d.resolved.max_lines,
                                d.resolved.text_align,
                            )
                        })
                        .unwrap_or_default()
                };
                let slot = Rc::new(RefCell::new(TextSlot {
                    text,
                    style,
                    wrap: true,
                    max_lines,
                    align,
                    scale: engine_scale,
                    rtl: false,
                    pseudo,
                    data: data_text,
                    cache: None,
                    painted: None,
                }));
                self.make_measured(slot.clone());
                let mut i = self.ui.0.borrow_mut();
                if let Some(d) = i.els.get_mut(&self.id) {
                    d.text = Some(slot);
                }
            }
        }
        let node = self.ui.0.borrow().els.get(&self.id).map(|d| d.node);
        if let Some(n) = node {
            let _ = self.ui.0.borrow_mut().layout.mark_dirty(n);
        }
        self.ui.mark(self.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        crate::view::links::sync(self);
    }

    /// Shows the text `f` returns, updated when its signals change.
    #[deprecated(note = "`text` takes a closure or signal itself: `.text(move || …)`")]
    #[track_caller]
    pub fn text_dyn(self, f: impl Fn() -> String + 'static) -> Self {
        self.text(f)
    }

    /// Pictures to draw inside the text, each where the inline object with its id is laid
    /// out (custom emoji in a chat message, an icon in a sentence). Screen readers read, and
    /// copying and selection give, each picture's `alt` in its place. Add the objects to the
    /// [`RichText`] with [`RichText::object`] at the picture's size; the text wraps around them
    /// as around any other character.
    pub fn inline_images(self, images: Vec<InlineImage>) -> Self {
        self.edit(Dirty::PAINT | Dirty::SEMANTICS, move |d| d.inline_images = Rc::new(images));
        self
    }

    /// Marks the element's text as data (a date or number the locale formatted, a name, a
    /// file name, what someone wrote) rather than a string the app translates. Pseudo-locales
    /// (`en-XA`, `ar-XB`) rewrite only translatable strings, so they leave it as it is.
    pub fn data_text(self) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.data_text = true;
        }
        if let Some(s) = self.ui.0.borrow().els.get(&self.id).and_then(|d| d.text.clone()) {
            let mut s = s.borrow_mut();
            s.data = true;
            s.cache = None;
        }
        self.edit(Dirty::LAYOUT | Dirty::PAINT, |_| {});
        self
    }

    /// Keeps the text on one line and, where it is wider than its box, cuts it short with
    /// "…" (a long name in a narrow list, a long error in a band): [`El::max_lines`] of 1.
    pub fn truncate(self) -> Self {
        self.max_lines(1)
    }

    /// Lets the text take at most `n` lines (CSS `line-clamp`): where it would take more, the
    /// last line ends in "…", cut at a grapheme boundary as the shaper measures it (letter
    /// spacing, alignment and right-to-left text included; the "…" lands on the line's visual
    /// end). Its minimum width is the "…" alone, so a row can shrink it. Screen readers,
    /// copying and searching still get the whole text; show it to sighted readers too with
    /// `remedy_ui_widgets::base::full_text_tooltip`. 0 lifts the limit. Style sheets set the same with
    /// [`Style::max_lines`](remedy_ui_core::style::Style::max_lines).
    pub fn max_lines(self, n: u32) -> Self {
        self.style(remedy_ui_core::style::Style::default().max_lines(n))
    }

    /// Sets where each line of the element's text sits across its box (CSS `text-align`):
    /// start, center, end, or justified to both edges, wrapped lines too; inherited by the
    /// text inside. Style sheets set the same with
    /// [`Style::text_align`](remedy_ui_core::style::Style::text_align).
    pub fn text_align(self, align: remedy_ui_core::style::TextAlign) -> Self {
        self.style(remedy_ui_core::style::Style::default().text_align(align))
    }

    /// Draws the text cased by `t` (CSS `text-transform`): capitals for a label above a title,
    /// say. Casing follows the window's locale (Turkish dotted İ, German ß as SS, Greek final
    /// sigma); screen readers, copying and selection get the text as written. Style sheets set
    /// the same with [`Style::text_transform`](remedy_ui_core::style::Style::text_transform).
    pub fn text_transform(self, t: remedy_ui_core::style::TextTransform) -> Self {
        self.style(remedy_ui_core::style::Style::default().text_transform(t))
    }

    /// Sets how tall each line of text is (CSS `line-height`), inherited by descendants:
    /// `LineHeight::Relative(1.5)` for CSS `line-height: 1.5`, `LineHeight::Absolute(24.0)` for
    /// `24px`. The default is `Relative(1.33)`. Style sheets set the same with
    /// [`Style::line_height`](remedy_ui_core::style::Style::line_height).
    pub fn line_height(self, h: remedy_ui_core::style::LineHeight) -> Self {
        self.style(remedy_ui_core::style::Style::default().line_height(h))
    }

    /// Keeps the text on one line instead of wrapping.
    pub fn nowrap(self) -> Self {
        {
            let slot = self.text_slot();
            let mut s = slot.borrow_mut();
            s.wrap = false;
            s.cache = None;
        }
        self.edit(Dirty::LAYOUT, |_| {});
        self
    }

    /// The element's text, made empty when it has none yet. A text bound to a signal or
    /// closure inside a running binding (a view built by `show`) gets its first value only
    /// when that binding is done: `nowrap` and `truncate` must outlast that wait.
    fn text_slot(&self) -> Rc<RefCell<TextSlot>> {
        let existing = |me: &El| me.ui.0.borrow().els.get(&me.id).and_then(|d| d.text.clone());
        if let Some(s) = existing(self) {
            return s;
        }
        self.set_text(RichText::plain(""));
        existing(self).unwrap_or_else(|| {
            // A removed element: its flags go nowhere, as its text would.
            Rc::new(RefCell::new(TextSlot {
                text: RichText::plain(""),
                style: TextStyle::default(),
                wrap: true,
                max_lines: 0,
                align: remedy_ui_text::Align::Start,
                scale: 1.0,
                rtl: false,
                pseudo: None,
                data: false,
                cache: None,
                painted: None,
            }))
        })
    }

    /// Replaces the layout node with one measured by `slot`'s paragraph.
    pub(crate) fn make_measured(&self, slot: Rc<RefCell<TextSlot>>) {
        let engine = self.ui.0.borrow().engine.clone();
        let mut i = self.ui.0.borrow_mut();
        let Some(style) = i.els.get(&self.id).map(|d| d.layout.clone()) else { return };
        let measure = move |m: MeasureInput| {
            let width = m.known_width.or(match m.available_width {
                Available::Px(w) => Some(w),
                Available::MinContent => Some(0.0),
                Available::MaxContent => None,
            });
            let p = slot.borrow_mut().paragraph(&mut engine.borrow_mut(), width);
            // A lazy paragraph's widest line is known only once every line is laid out: it
            // takes the width it wraps in.
            let w = match width {
                Some(w) if p.is_lazy() && !p.is_complete() => w.max(p.width()),
                _ => p.width(),
            };
            Size::new(layout_unit_ceil(w), layout_unit_ceil(p.height()))
        };
        let Ok(node) = i.layout.measured(&style, measure) else { return };
        let parent = i.tree.parent(self.id).and_then(|p| i.els.get(&p).map(|d| d.node));
        let old = i.els.get(&self.id).map(|d| d.node);
        if let (Some(p), Some(old)) = (parent, old) {
            let kids: Vec<LayoutNode> =
                i.layout.children(p).into_iter().map(|k| if k == old { node } else { k }).collect();
            let _ = i.layout.set_children(p, &kids);
        }
        if let Some(old) = old {
            // Children it already has (an icon in a label) stay laid out inside it.
            let kids = i.layout.children(old);
            if !kids.is_empty() {
                let _ = i.layout.set_children(node, &kids);
            }
            let _ = i.layout.remove(old);
        }
        if let Some(d) = i.els.get_mut(&self.id) {
            d.node = node;
        }
    }

    /// Draws with `f` after the element's background and border. Signals `f` reads are
    /// watched: changing one repaints the element.
    pub fn paint(self, f: impl Fn(&mut PaintCx<'_>) + 'static) -> Self {
        let f: Painter = Rc::new(f);
        let ui = Rc::downgrade(&self.ui.0);
        let id = self.id;
        let on_change = move || {
            if let Some(inner) = ui.upgrade() {
                Ui(inner).mark(id, Dirty::PAINT);
            }
        };
        // The watch belongs to the element: a binding running again does not dispose it.
        let watch = match self.scope() {
            Some(s) => Reaction::new_in(s.into(), on_change),
            None => Reaction::new(on_change),
        }
        .named("paint");
        let mut old = None;
        self.edit(Dirty::PAINT, |d| {
            d.painter = Some(f);
            old = d.paint_watch.replace(watch);
        });
        if let Some(w) = old {
            w.dispose();
        }
        self
    }

    /// Calls `f` on click: a primary press and release on the element, or Enter or Space
    /// while it has focus. Makes the element focusable.
    pub fn on_click(self, f: impl Fn() + 'static) -> Self {
        let f: Rc<dyn Fn()> = Rc::new(f);
        let tree = self.ui.tree();
        let _ = tree.set_focusable(self.id, true);
        let g = f.clone();
        let _ = tree.on::<Click>(self.id, Phase::Bubble, move |cx, _| {
            g();
            cx.stop_propagation();
        });
        self.edit(Dirty::SEMANTICS, |d| d.click = Some(f));
        self
    }

    /// A primary press on the element, or on anything inside it, leaves keyboard focus where
    /// it is instead of moving it (or clearing it): a menu bar's titles, so the menu opened
    /// from them hands focus back to the field it came from.
    pub fn keeps_focus_on_press(self, on: bool) -> Self {
        self.edit(Dirty::NONE, |d| d.keeps_focus = on);
        self
    }

    /// Lets the element take keyboard focus.
    pub fn focusable(self, on: bool) -> Self {
        let tree = self.ui.tree();
        let _ = tree.set_focusable(self.id, on);
        let _ = tree.set_tab_stop(self.id, true);
        self
    }

    /// Lets the element take focus from code ([`focus`](Self::focus)) and from a click,
    /// without joining the Tab order: HTML's `tabindex="-1"` (a heading a skip link moves
    /// to, a list whose rows move focus themselves). [`focusable`](Self::focusable) puts it
    /// in the Tab order again.
    pub fn focus_target(self) -> Self {
        let tree = self.ui.tree();
        let _ = tree.set_focusable(self.id, true);
        let _ = tree.set_tab_stop(self.id, false);
        self
    }

    /// Runs `f` once when the element is removed.
    pub fn on_removed(self, f: impl FnOnce() + 'static) -> Self {
        if let Some(d) = self.ui.0.borrow_mut().els.get_mut(&self.id) {
            d.removed.push(Box::new(f));
        }
        self
    }

    /// Moves keyboard focus here.
    pub fn focus(&self) {
        let tree = self.ui.tree();
        let _ = tree.focus(self.id);
        crate::view::events::focus_changed(&self.ui);
    }

    /// Moves keyboard focus here with its focus ring shown, as a Tab there would: a dialog's
    /// default button, which Enter presses, looks chosen before any key is pressed.
    pub fn focus_visibly(&self) {
        self.ui.0.borrow_mut().focus_visible = true;
        self.focus();
    }

    /// The style the element would have in `states`, from the window's style sheet (and its
    /// own properties, and what it inherits): audits use it to check that focus, selection
    /// and the like are visible.
    pub fn style_in(&self, states: States) -> Option<ResolvedStyle> {
        let mut guard = self.ui.0.borrow_mut();
        let i = &mut *guard;
        let d = i.els.get(&self.id)?;
        let base = i.base.clone();
        let inherited = match i.tree.parent(self.id).and_then(|p| i.els.get(&p)) {
            Some(parent) => parent.resolved.inherited_by_children(&base),
            None => base,
        };
        let key = StyleKey { widget: d.widget, variant: d.variant, states };
        let local = d.local.clone();
        Some(i.sheet.resolve(&key, &inherited, &local))
    }

    /// What the element is to assistive technology and agents: exactly the semantics the
    /// accessibility tree carries for it (a field's text and actions, a password field's role
    /// without its text).
    pub fn composed_semantics(&self) -> Option<remedy_ui_core::a11y::Semantics> {
        let tree = self.ui.tree();
        let i = self.ui.0.borrow();
        let d = i.els.get(&self.id)?;
        Some(crate::view::frame::composed_semantics(
            d,
            tree.root() == self.id,
            tree.is_focusable(self.id),
            i.pseudo,
        ))
    }

    /// Everything the element is, for inspectors and tests.
    pub fn info(&self) -> Option<ElementInfo> {
        let tree = self.ui.tree();
        let i = self.ui.0.borrow();
        let d = i.els.get(&self.id)?;
        Some(ElementInfo {
            id: self.id,
            widget: d.widget,
            variant: d.variant,
            states: d.states,
            style: d.resolved.clone(),
            local: d.local.clone(),
            bounds: tree.bounds(self.id),
            // What is shown: pseudo-locales rewrite it for people, readers and inspectors alike.
            text: d.text.as_ref().map(|t| with_alts(&t.borrow().shown(), &d.inline_images, None)),
            semantics: Some(crate::view::frame::composed_semantics(
                d,
                i.layer_root(self.id),
                tree.is_focusable(self.id),
                i.pseudo,
            )),
            created_at: d.created_at,
            focusable: tree.is_focusable(self.id),
            clickable: d.click.is_some(),
            painter: d.painter.is_some(),
            field: d.field.is_some(),
            pointer: d.pointer,
            keyboard_alternative: d.keyboard_alternative.clone(),
            bindings: d.effects.len(),
            binding_keys: d
                .effects
                .iter()
                .map(Effect::key)
                .chain(d.paint_watch.iter().map(Reaction::key))
                .collect(),
            reads: {
                let mut all: Vec<remedy_ui_core::reactive::Source> = Vec::new();
                let from_effects = d.effects.iter().flat_map(Effect::sources);
                for s in from_effects.chain(d.paint_watch.iter().flat_map(Reaction::sources)) {
                    if !all.contains(&s) {
                        all.push(s);
                    }
                }
                all
            },
            hidden: d.hidden,
            clips: d.clip || d.scroll,
            inline_link: d.inline_link.is_some(),
        })
    }

    /// The parent element, if any.
    pub fn parent(&self) -> Option<El> {
        let p = self.ui.tree().parent(self.id)?;
        Some(El { ui: self.ui.clone(), id: p })
    }

    /// Takes keyboard focus away if the element has it.
    pub fn blur(&self) {
        let tree = self.ui.tree();
        if tree.focused() == Some(self.id) {
            tree.blur();
            crate::view::events::focus_changed(&self.ui);
        }
    }

    /// Describes the element to assistive technology; a signal or closure keeps the
    /// description current.
    #[track_caller]
    pub fn semantics(self, s: impl IntoProp<Semantics>) -> Self {
        self.prop("semantics", s, |me, s| me.edit(Dirty::SEMANTICS, |d| d.semantics = Some(s)));
        self
    }

    /// Does what assistive technology (and agents) ask of the element: `f` gets the action
    /// and returns whether it did it. It is asked before the element's built-in handling,
    /// so an element can take Click, Expand, Collapse, Increment, Decrement or Show menu
    /// its own way (a tree row expands, a row with no click handler selects).
    pub fn on_accessibility_action(self, f: impl Fn(remedy_ui_core::a11y::Action) -> bool + 'static) -> Self {
        let f: Rc<dyn Fn(remedy_ui_core::a11y::Action) -> bool> = Rc::new(f);
        self.edit(Dirty::SEMANTICS, |d| d.a11y_action = Some(f));
        self
    }

    /// Makes the element a vertical scroll container: its content scrolls on the compositor.
    pub fn scroll(self) -> Self {
        self.edit(Dirty::LAYOUT | Dirty::PAINT, |d| {
            d.scroll = true;
            d.clip = true;
            d.layout.max_height = remedy_ui_core::layout::Length::Percent(1.0);
        });
        let _ = self.ui.tree().set_clips(self.id, true);
        self
    }

    /// Hides the element and everything inside it: not laid out (it takes no space), not
    /// drawn, not hit, not reachable by keyboard and not in the accessibility tree.
    pub fn hidden(self, on: bool) -> Self {
        if self.ui.0.borrow().els.get(&self.id).is_some_and(|d| d.hidden == on) {
            return self;
        }
        self.edit(Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS, |d| d.hidden = on);
        self.sync_hit_mode();
        self.ui.0.borrow_mut().structure_changed = true;
        self
    }

    /// Makes an engine-made wrapper take no space (and no gap) while it holds nothing, or
    /// space again once it does.
    pub(crate) fn set_collapsed(&self, on: bool) {
        if self.ui.0.borrow().els.get(&self.id).is_none_or(|d| d.collapsed == on) {
            return;
        }
        self.edit(Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS, |d| d.collapsed = on);
        self.ui.0.borrow_mut().structure_changed = true;
    }

    /// With `on`, the element's box is a window onto what is under the app's window: it is
    /// cleared to transparent (inside its corners) where its own painting would go, and
    /// what is drawn after it (its children, siblings over it) draws over. A page's own
    /// `<video>` placed under the canvas shows there, decoded and composited by the
    /// browser as on any page, with nothing redrawn per frame. Holes clear the window's
    /// picture only outside group opacity and filters around the element.
    pub fn set_see_through(&self, on: bool) {
        if self.ui.0.borrow().els.get(&self.id).is_none_or(|d| d.see_through == on) {
            return;
        }
        self.edit(Dirty::PAINT, |d| d.see_through = on);
        // Its children move into (or out of) a layer of their own, snapped there.
        self.ui.0.borrow_mut().frame.bounds_stale = true;
    }

    /// Paints the element and everything inside it into a compositor layer of its own,
    /// clipped to its box: expensive content that changes rarely (a page, a chart's axis), or
    /// content the compositor moves or zooms without repainting ([`El::set_layer_transform`]).
    /// Repainting it re-rasters only its own layer.
    pub fn isolate_paint(self) -> Self {
        self.edit(Dirty::PAINT, |d| {
            d.isolate.get_or_insert(crate::paint::Affine::IDENTITY);
        });
        self
    }

    /// Transforms an isolated element's layer on the compositor (in the element's own
    /// coordinates, inside its clip) without repainting it. When the transform scales, the
    /// compositor zooms progressively: tiles of the old scale stand in while the new scale
    /// rasters within its per-frame budget. Does nothing for an element without
    /// [`isolate_paint`](Self::isolate_paint).
    pub fn set_layer_transform(&self, transform: crate::paint::Affine) {
        let changed = {
            let mut i = self.ui.0.borrow_mut();
            let changed = match i.els.get_mut(&self.id).and_then(|d| d.isolate.as_mut()) {
                Some(t) if *t != transform => {
                    *t = transform;
                    true
                }
                _ => false,
            };
            i.layers_changed |= changed;
            changed
        };
        if changed {
            self.ui.request(Request::Frame);
        }
    }

    /// Hides the mouse pointer while it is over the element or its children, or shows it
    /// again: a video full screen once its controls have faded. The runner tells the
    /// platform ([`Ui::cursor_hidden`]); touch and pointerless platforms show none anyway.
    pub fn cursor_hidden(self, hidden: bool) -> Self {
        let changed = {
            let mut i = self.ui.0.borrow_mut();
            if hidden { i.cursor_hiding.insert(self.id) } else { i.cursor_hiding.remove(&self.id) }
        };
        // The runner looks again after the event or timer that changed it.
        if changed {
            self.ui.request(Request::Frame);
        }
        self
    }

    /// Gives the mouse pointer this shape while it is over the element or its children
    /// (until a child sets its own): a hand over a link, a resize arrow over a divider,
    /// `not-allowed` over what cannot be dropped on. [`Cursor::Default`](remedy_ui_core::input::Cursor::Default)
    /// is the arrow even inside a parent with another shape. The runner tells the platform
    /// ([`Ui::cursor`]).
    pub fn cursor(self, cursor: remedy_ui_core::input::Cursor) -> Self {
        let changed = self.ui.0.borrow_mut().cursors.insert(self.id, cursor) != Some(cursor);
        // The runner looks again after the event or timer that changed it.
        if changed {
            self.ui.request(Request::Frame);
        }
        self
    }

    /// Lets clicks through the element itself to what is below; its children still take
    /// them.
    pub fn pass_through(self) -> Self {
        self.edit(Dirty::NONE, |d| d.pass_through = true);
        self.sync_hit_mode();
        self
    }

    /// Sets how the element is hit from everything that has a say: hidden, disabled or
    /// leaving it takes no input; otherwise clicks pass through it when it was made so.
    pub(crate) fn sync_hit_mode(&self) {
        let mode = match self.ui.0.borrow().els.get(&self.id) {
            Some(d) if d.hidden || d.inert || d.leaving => HitMode::None,
            Some(d) if d.pass_through => HitMode::PassThrough,
            Some(_) => HitMode::Normal,
            None => return,
        };
        let _ = self.ui.tree().set_hit_mode(self.id, mode);
    }

    /// Re-records the element's paint (a custom painter reads state that changed).
    pub fn repaint(&self) {
        self.ui.mark(self.id, Dirty::PAINT);
    }

    /// Clips children to the element's box.
    pub fn clip(self) -> Self {
        let _ = self.ui.tree().set_clips(self.id, true);
        self.edit(Dirty::PAINT, |d| d.clip = true);
        self
    }

    /// The element's bounds in window logical pixels after the last frame.
    pub fn bounds(&self) -> Option<remedy_ui_core::Rect> {
        self.ui.tree().bounds(self.id)
    }

    /// Whether the element lays out right to left (as of the last frame).
    pub fn is_rtl(&self) -> bool {
        let i = self.ui.0.borrow();
        i.els.get(&self.id).is_some_and(|d| i.layout.is_rtl(d.node))
    }

    /// Whether the element has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.ui.tree().focused() == Some(self.id)
    }
}

/// A snapping scroll container's settings ([`El::scroll_snap`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct SnapSpec {
    /// Snaps only once an axis is chosen.
    pub(crate) axis: Option<SnapAxis>,
    /// Strictness and tuning; its points are filled in each paint.
    pub(crate) tuning: crate::compositor::ScrollSnap,
    pub(crate) padding: remedy_ui_core::Insets,
}

/// The axes a scroll container snaps on ([`El::scroll_snap`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapAxis {
    /// Horizontally.
    X,
    /// Vertically.
    Y,
    /// On both axes.
    Both,
}

/// How firmly a scroll container snaps ([`El::scroll_snap`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapStrictness {
    /// Only onto a snap point near where scrolling would rest.
    Proximity,
    /// Always onto a snap point.
    Mandatory,
}

/// Which edge of a child lines up with its snapping scroll container ([`El::snap_align`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapAlign {
    /// Its start edge with the container's.
    Start,
    /// Its center with the container's.
    Center,
    /// Its end edge with the container's.
    End,
}
