//! Pixel art written as code, and sprites with named states and frame animation.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size};
use remedy_ui_paint::{Image, Item, Sampling};
use remedy_ui_style::Theme;
use remedy_ui_view::{El, IntoProp, PaintCx, Ui};
use remedy_ui_viz::{Clock, Moved};

use crate::common::{Ink, Motion};

/// One picture: `width` by `height` cells, each 0 (clear) or a palette index plus one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Grid {
    width: u32,
    height: u32,
    cells: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq)]
struct ArtState {
    name: String,
    frames: Vec<Grid>,
    fps: f32,
}

/// Pixel art as code: a palette of characters and rows of them, so an app can ship its own
/// original art in its source (license-clean, diffable, restyled by the theme).
///
/// Each state (`idle`, `speaking`, `muted`, …) has one or more frames. A character the palette
/// does not name (`.` and space by convention) is clear. Palette entries are [`Ink`]s: a theme
/// role follows the skin, a fixed color stays.
///
/// ```
/// use remedy_ui_style::ColorRole;
/// use remedy_ui_widgets_stage::{Ink, PixelArt};
///
/// let lamp = PixelArt::new([('#', Ink::from(ColorRole::Line)), ('o', Ink::from(ColorRole::Warn))])
///     .state("off", &[".##.", "#..#", ".##."])
///     .animation("on", &[&[".##.", "#oo#", ".##."], &[".##.", "#o.#", ".##."]], 4.0);
/// assert_eq!(lamp.size(), (4, 3));
/// assert_eq!(lamp.frame_count("on"), 2);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct PixelArt {
    palette: Vec<(char, Ink)>,
    states: Vec<ArtState>,
}

impl PixelArt {
    /// Art with this palette and no states yet.
    pub fn new(palette: impl IntoIterator<Item = (char, Ink)>) -> Self {
        PixelArt { palette: palette.into_iter().collect(), states: Vec::new() }
    }

    fn grid(&self, rows: &[&str]) -> Grid {
        let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u32;
        let height = rows.len() as u32;
        let mut cells = Vec::with_capacity((width * height) as usize);
        for row in rows {
            let mut n = 0;
            for ch in row.chars() {
                let ix = self.palette.iter().position(|(c, _)| *c == ch).map_or(0, |i| i as u16 + 1);
                cells.push(ix);
                n += 1;
            }
            cells.extend(std::iter::repeat_n(0, (width - n) as usize));
        }
        Grid { width, height, cells }
    }

    /// Adds (or replaces) a still state.
    pub fn state(self, name: impl Into<String>, rows: &[&str]) -> Self {
        self.animation(name, &[rows], 0.0)
    }

    /// Adds (or replaces) a state with frames played at `fps` frames a second, looping.
    pub fn animation(mut self, name: impl Into<String>, frames: &[&[&str]], fps: f32) -> Self {
        let name = name.into();
        let frames: Vec<Grid> = frames.iter().map(|f| self.grid(f)).collect();
        let fps = if fps.is_finite() { fps.max(0.0) } else { 0.0 };
        let state = ArtState { name: name.clone(), frames, fps };
        match self.states.iter_mut().find(|s| s.name == name) {
            Some(s) => *s = state,
            None => self.states.push(state),
        }
        self
    }

    /// The largest frame's size in cells.
    pub fn size(&self) -> (u32, u32) {
        self.states.iter().flat_map(|s| &s.frames).fold((0, 0), |(w, h), g| (w.max(g.width), h.max(g.height)))
    }

    /// The states' names, in the order added.
    pub fn states(&self) -> Vec<&str> {
        self.states.iter().map(|s| s.name.as_str()).collect()
    }

    /// How many frames a state has (zero for one it does not have).
    pub fn frame_count(&self, state: &str) -> usize {
        self.states.iter().find(|s| s.name == state).map_or(0, |s| s.frames.len())
    }

    /// One frame as an image (one image pixel per cell, premultiplied RGBA8) with the palette
    /// resolved under `theme`, for drawing it elsewhere (a stage layer). `None` for a state or
    /// frame it does not have.
    pub fn render(&self, state: &str, frame: usize, theme: &Theme) -> Option<Image> {
        let s = self.states.iter().find(|s| s.name == state)?;
        let colors: Vec<Color> = self.palette.iter().map(|(_, i)| i.resolve(theme)).collect();
        s.frames.get(frame).map(|g| raster(g, &colors))
    }
}

fn raster(g: &Grid, colors: &[Color]) -> Image {
    let mut px = Vec::with_capacity(g.cells.len() * 4);
    for &c in &g.cells {
        let color = if c == 0 {
            Color::TRANSPARENT
        } else {
            colors.get(c as usize - 1).copied().unwrap_or(Color::TRANSPARENT)
        };
        let [r, gg, b, a] = color.premultiplied();
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        px.extend_from_slice(&[q(r), q(gg), q(b), q(a)]);
    }
    Image::new(g.width, g.height, Arc::from(px))
}

/// Frames cut from one picture laid out as a grid of equal cells, numbered row by row from
/// the top left (a sprite sheet).
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteSheet {
    image: Image,
    cell: (u32, u32),
    states: Vec<(String, Vec<u32>, f32)>,
}

impl SpriteSheet {
    /// A sheet of `cell_width` by `cell_height` pixel cells.
    pub fn new(image: Image, cell_width: u32, cell_height: u32) -> Self {
        SpriteSheet { image, cell: (cell_width.max(1), cell_height.max(1)), states: Vec::new() }
    }

    /// Adds (or replaces) a state showing these cells in turn at `fps` frames a second (a
    /// single cell is a still).
    pub fn state(mut self, name: impl Into<String>, cells: impl IntoIterator<Item = u32>, fps: f32) -> Self {
        let name = name.into();
        let cells: Vec<u32> = cells.into_iter().collect();
        let fps = if fps.is_finite() { fps.max(0.0) } else { 0.0 };
        match self.states.iter_mut().find(|s| s.0 == name) {
            Some(s) => *s = (name, cells, fps),
            None => self.states.push((name, cells, fps)),
        }
        self
    }

    fn columns(&self) -> u32 {
        (self.image.width / self.cell.0).max(1)
    }
}

/// What a [`sprite`] draws: pixel art written as code, or a sprite sheet.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SpriteSource {
    /// Pixel art written as code.
    Art(PixelArt),
    /// Cells of a picture.
    Sheet(SpriteSheet),
}

impl From<PixelArt> for SpriteSource {
    fn from(a: PixelArt) -> Self {
        SpriteSource::Art(a)
    }
}

impl From<SpriteSheet> for SpriteSource {
    fn from(s: SpriteSheet) -> Self {
        SpriteSource::Sheet(s)
    }
}

impl SpriteSource {
    /// The index of a state, falling back to the first.
    fn state_index(&self, name: &str) -> usize {
        match self {
            SpriteSource::Art(a) => a.states.iter().position(|s| s.name == name),
            SpriteSource::Sheet(s) => s.states.iter().position(|s| s.0 == name),
        }
        .unwrap_or(0)
    }

    fn frames_fps(&self, state: usize) -> (usize, f32) {
        match self {
            SpriteSource::Art(a) => a.states.get(state).map_or((0, 0.0), |s| (s.frames.len(), s.fps)),
            SpriteSource::Sheet(s) => s.states.get(state).map_or((0, 0.0), |s| (s.1.len(), s.2)),
        }
    }

    fn cell_size(&self) -> (u32, u32) {
        match self {
            SpriteSource::Art(a) => a.size(),
            SpriteSource::Sheet(s) => s.cell,
        }
    }
}

/// How big each art pixel is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelScale {
    /// Each art pixel is this many logical pixels; the element is sized to fit the art.
    Fixed(u32),
    /// The largest whole number that fits the box the app gives the element (smaller than one
    /// only when even one does not fit), centered.
    Fit,
}

/// How a [`sprite`] is drawn and animated.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SpriteOptions {
    /// How big each art pixel is drawn.
    pub scale: PixelScale,
    /// Where animation time comes from.
    pub clock: Clock,
}

impl Default for SpriteOptions {
    fn default() -> Self {
        SpriteOptions { scale: PixelScale::Fixed(4), clock: Clock::system() }
    }
}

impl SpriteOptions {
    /// Sets `scale`: how big each art pixel is drawn.
    pub fn scale(mut self, value: PixelScale) -> Self {
        self.scale = value;
        self
    }

    /// Sets `clock`: where animation time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

/// A frame's image and the palette colors it was made with.
type Rendered = (Vec<Color>, Image);

struct Shown {
    state: Cell<usize>,
    frame: Cell<usize>,
    since: Cell<Duration>,
    cache: RefCell<HashMap<(usize, usize), Rendered>>,
}

/// Snaps a logical coordinate to the device pixel grid.
fn snap(v: f32, scale: f32) -> f32 {
    if scale > 0.0 { (v * scale).round() / scale } else { v }
}

/// A sprite: `source`'s current `state` (a constant, signal or closure naming one of its
/// states; an unknown name shows the first), drawn crisp at a whole-number scale.
///
/// A state with several frames plays them at its frame rate while the sprite is seen; it
/// holds its first frame when hidden, off screen or under reduced motion, and a still sprite
/// schedules nothing. Changing state starts the new one at its first frame.
///
/// `label` (constant, signal or closure) is what assistive technology reads: say what the
/// picture means ("Alex, speaking"). An empty label makes the sprite decoration, left out of
/// the accessibility tree.
#[track_caller]
pub fn sprite(
    ui: &Ui,
    label: impl IntoProp<String>,
    source: impl Into<SpriteSource>,
    state: impl IntoProp<String>,
    options: SpriteOptions,
) -> El {
    let source = Rc::new(source.into());
    let (cw, ch) = source.cell_size();
    let shown = Rc::new(Shown {
        state: Cell::new(0),
        frame: Cell::new(0),
        since: Cell::new(Duration::ZERO),
        cache: RefCell::new(HashMap::new()),
    });
    let mut el = ui.el("sprite");
    if let PixelScale::Fixed(n) = options.scale {
        let n = n.max(1) as f32;
        el = el.width(cw as f32 * n).height(ch as f32 * n);
    }
    let scale = options.scale;
    let (src, sh) = (source.clone(), shown.clone());
    let el = el.paint(move |cx| draw(cx, &src, &sh, scale));

    let (src, sh) = (source.clone(), shown.clone());
    let (el, motion) = Motion::attach(el, options.clock, move |dt| {
        let (n, fps) = src.frames_fps(sh.state.get());
        if n <= 1 || fps <= 0.0 {
            return Moved::default();
        }
        let per = Duration::from_secs_f32(1.0 / fps);
        let mut since = sh.since.get() + dt;
        let mut frame = sh.frame.get();
        while since >= per {
            since -= per;
            frame = (frame + 1) % n;
        }
        sh.since.set(since);
        let changed = sh.frame.replace(frame) != frame;
        Moved { changed, pending: true }
    });

    let (src, sh, me) = (source, shown, el.clone());
    el.prop("state", state, move |_, name: String| {
        let ix = src.state_index(&name);
        // A sprite drawn at 8 frames a second needs 8 steps a second, whatever the window's rate.
        let (_, fps) = src.frames_fps(ix);
        motion.set_min_interval(if fps > 0.0 { Duration::from_secs_f32(1.0 / fps) } else { Duration::ZERO });
        if sh.state.replace(ix) != ix {
            sh.frame.set(0);
            sh.since.set(Duration::ZERO);
            me.repaint();
        }
        motion.kick();
    });
    let me = el.clone();
    el.prop("label", label, move |_, l: String| {
        let s = if l.is_empty() {
            Semantics::new(Role::Image).hidden(true)
        } else {
            Semantics::new(Role::Image).label(l)
        };
        let _ = me.clone().semantics(s);
    });
    el
}

/// Pixel art showing its first state (see [`sprite`] for states and animation).
#[track_caller]
pub fn pixel_art(ui: &Ui, label: impl IntoProp<String>, art: PixelArt, options: SpriteOptions) -> El {
    let first = art.states.first().map(|s| s.name.clone()).unwrap_or_default();
    sprite(ui, label, art, first, options)
}

fn draw(cx: &mut PaintCx<'_>, source: &SpriteSource, shown: &Shown, scale: PixelScale) {
    let (cw, ch) = source.cell_size();
    if cw == 0 || ch == 0 {
        return;
    }
    let s = match scale {
        PixelScale::Fixed(n) => n.max(1) as f32,
        PixelScale::Fit => {
            let f = (cx.size.width / cw as f32).min(cx.size.height / ch as f32);
            if f >= 1.0 { f.floor() } else { f.max(0.0) }
        }
    };
    let size = Size::new(cw as f32 * s, ch as f32 * s);
    let at = Point::new(
        snap((cx.size.width - size.width) / 2.0, cx.scale),
        snap((cx.size.height - size.height) / 2.0, cx.scale),
    );
    let state = shown.state.get();
    let frame = if cx.reduced_motion { 0 } else { shown.frame.get() };
    match source {
        SpriteSource::Art(art) => {
            let Some(st) = art.states.get(state) else { return };
            let Some(grid) = st.frames.get(frame) else { return };
            let colors: Vec<Color> = art.palette.iter().map(|(_, i)| i.resolve(cx.theme)).collect();
            let mut cache = shown.cache.borrow_mut();
            let image = match cache.get(&(state, frame)) {
                Some((c, img)) if *c == colors => img.clone(),
                _ => {
                    let img = raster(grid, &colors);
                    cache.insert((state, frame), (colors, img.clone()));
                    img
                }
            };
            let rect = Rect::from_origin_size(at, Size::new(grid.width as f32 * s, grid.height as f32 * s));
            cx.list.push(Item::Image { image, rect, radii: CornerRadii::ZERO, sampling: Sampling::Nearest });
        }
        SpriteSource::Sheet(sheet) => {
            let Some((_, cells, _)) = sheet.states.get(state) else { return };
            let Some(&cell) = cells.get(frame) else { return };
            let cols = sheet.columns();
            let (col, row) = (cell % cols, cell / cols);
            let dest = Rect::from_origin_size(at, size);
            let whole = Rect::from_origin_size(
                Point::new(at.x - (col * cw) as f32 * s, at.y - (row * ch) as f32 * s),
                Size::new(sheet.image.width as f32 * s, sheet.image.height as f32 * s),
            );
            cx.list.push_clip(dest, CornerRadii::ZERO);
            cx.list.push(Item::Image {
                image: sheet.image.clone(),
                rect: whole,
                radii: CornerRadii::ZERO,
                sampling: Sampling::Nearest,
            });
            cx.list.pop_clip();
        }
    }
}
