//! Previews: a piece of UI rendered in every variation asked for (themes, locales, sizes,
//! text scales), shown side by side, and checked against golden images.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_engine::compositor::Compositor;
use remedy_ui_core::{Color, Size};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::layout::Direction;
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_core::style::Theme;
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Ui};
use crate::base as w;

/// Builds the UI under preview into a [`Ui`] (theme, locale and fonts are already set).
pub type PreviewBuilder = Rc<dyn Fn(&Ui) -> El>;

/// A named piece of UI and the variations to show it in.
#[derive(Clone)]
pub struct Preview {
    /// Its name (also the golden-image file name).
    pub name: String,
    build: PreviewBuilder,
    /// Palette ids from the theme pack (`forest`, …).
    pub themes: Vec<String>,
    /// BCP 47 locales; right-to-left ones lay out mirrored.
    pub locales: Vec<String>,
    /// Window sizes in logical pixels.
    pub sizes: Vec<(f32, f32)>,
    /// Text scales (1.0 is the theme's own).
    pub text_scales: Vec<f32>,
    /// Device pixels per logical pixel.
    pub scale: f32,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Preview({})", self.name)
    }
}

/// A preview declared with `#[preview]` (from `remedy-ui-macros`): what to build and the
/// variations to show it in, as static data. Empty lists mean the defaults of
/// [`Preview::new`].
// Exhaustive on purpose: `#[preview]` writes it as a literal in the app's crate; a new
// setting comes with a macro version that fills it.
#[derive(Clone, Copy, Debug)]
pub struct PreviewSpec {
    /// Its name (also the golden-image file name).
    pub name: &'static str,
    /// Palette ids.
    pub themes: &'static [&'static str],
    /// Locales.
    pub locales: &'static [&'static str],
    /// Window sizes in logical pixels.
    pub sizes: &'static [(f32, f32)],
    /// Text scales.
    pub text_scales: &'static [f32],
    /// Builds the UI under preview.
    pub build: fn(&Ui) -> El,
}

impl PreviewSpec {
    /// The preview it declares.
    pub fn preview(&self) -> Preview {
        let mut p = Preview::new(self.name, self.build);
        if !self.themes.is_empty() {
            p = p.themes(self.themes);
        }
        if !self.locales.is_empty() {
            p = p.locales(self.locales);
        }
        if !self.sizes.is_empty() {
            p = p.sizes(self.sizes);
        }
        if !self.text_scales.is_empty() {
            p = p.text_scales(self.text_scales);
        }
        p
    }
}

impl From<&PreviewSpec> for Preview {
    fn from(s: &PreviewSpec) -> Preview {
        s.preview()
    }
}

/// The golden-image test `#[preview]` generates: every variation of `spec` against its image
/// in `dir`. A missing image is written for review and passes (commit it); a differing one
/// fails, with the rendering written beside the golden image.
pub fn golden_test(spec: &PreviewSpec, dir: &Path) -> Result<(), GoldenError> {
    match check_golden(&spec.preview(), dir) {
        Ok(()) | Err(GoldenError::Missing(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// One rendering of a preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Variation {
    /// Palette id.
    pub theme: String,
    /// Locale.
    pub locale: String,
    /// Window size.
    pub size: (f32, f32),
    /// Text scale.
    pub text_scale: f32,
}

impl Variation {
    /// A file-name-safe tag: `forest-en-US-400x600-1.0`.
    pub fn tag(&self) -> String {
        format!(
            "{}-{}-{}x{}-{:.1}",
            self.theme, self.locale, self.size.0 as u32, self.size.1 as u32, self.text_scale
        )
    }

    /// A caption: `forest · en-US · 400×600 · 1.0×`.
    pub fn caption(&self) -> String {
        format!(
            "{} · {} · {}×{} · {:.1}×",
            self.theme, self.locale, self.size.0 as u32, self.size.1 as u32, self.text_scale
        )
    }
}

impl Preview {
    /// A preview of what `build` makes, in the first palette, en-US, 400×300, at 1×.
    pub fn new(name: impl Into<String>, build: impl Fn(&Ui) -> El + 'static) -> Self {
        let first =
            crate::themes::PALETTES.first().map_or_else(|| "forest".to_owned(), |p| p.id.to_owned());
        Preview {
            name: name.into(),
            build: Rc::new(build),
            themes: vec![first],
            locales: vec!["en-US".into()],
            sizes: vec![(400.0, 300.0)],
            text_scales: vec![1.0],
            scale: 1.0,
        }
    }

    /// Shows it in these palettes: the shared palettes' ids, or `portal:<id>` for Portal's.
    pub fn themes(mut self, ids: &[&str]) -> Self {
        self.themes = ids.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// Shows it in these locales.
    pub fn locales(mut self, tags: &[&str]) -> Self {
        self.locales = tags.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// Shows it at these window sizes.
    pub fn sizes(mut self, sizes: &[(f32, f32)]) -> Self {
        self.sizes = sizes.to_vec();
        self
    }

    /// Shows it at these text scales.
    pub fn text_scales(mut self, scales: &[f32]) -> Self {
        self.text_scales = scales.to_vec();
        self
    }

    /// Every combination of its themes, locales, sizes and text scales.
    pub fn variations(&self) -> Vec<Variation> {
        let mut out = Vec::new();
        for theme in &self.themes {
            for locale in &self.locales {
                for size in &self.sizes {
                    for text_scale in &self.text_scales {
                        out.push(Variation {
                            theme: theme.clone(),
                            locale: locale.clone(),
                            size: *size,
                            text_scale: *text_scale,
                        });
                    }
                }
            }
        }
        out
    }

    /// Renders one variation with the CPU renderer (deterministic pixels).
    pub fn render(&self, v: &Variation) -> Snapshot {
        let theme = theme_for(v);
        let fonts = bundled_fonts();
        // Portal's palettes come with Portal's sheet, as a Portal app sets them.
        let sheet = match v.theme.starts_with("portal:") {
            true => crate::themes::portal_sheet(&theme),
            false => crate::themes::sheet(&theme),
        };
        let ui = Ui::new(theme.clone(), sheet, fonts);
        ui.set_locale(v.locale.clone());
        ui.set_reduced_motion(true);
        let content = (self.build)(&ui);
        let root = ui.root();
        if is_rtl(&v.locale) {
            let _ = root.clone().layout(|l| l.direction = Direction::Rtl);
        }
        let _ = root.child(content);
        let scale = self.scale.max(0.25);
        let (lw, lh) = v.size;
        let (pw, ph) = ((lw * scale).round().max(1.0) as u32, (lh * scale).round().max(1.0) as u32);
        let mut comp = Compositor::new(pw, ph, scale, ScrollPhysics::CLAMPED);
        comp.set_reduced_motion(true);
        let mut cpu = CpuCompositor::new();
        let mut pixels = vec![0u8; (pw * ph * 4) as usize];
        // A few frames: layout settles, bindings that react to bounds run.
        for n in 0..3u64 {
            let f = ui.frame(Size::new(lw, lh), scale);
            if let Some(l) = f.layers {
                comp.commit(l);
            }
            let _ = ui.take_requests();
            let plan = comp.frame(Duration::from_millis(16 * (n + 1)));
            if let Ok(fr) = cpu.present(&plan, pw, ph, theme.colors.bg) {
                pixels = fr.pixels;
            }
        }
        Snapshot { width: pw, height: ph, rgba: pixels }
    }
}

/// The bundled faces with the script fallback faces, read once per process: previews in
/// every language draw the same on every machine.
fn bundled_fonts() -> FontCollection {
    use remedy_ui_text::FontFile;
    static FACES: std::sync::OnceLock<Vec<FontFile>> = std::sync::OnceLock::new();
    static CJK: std::sync::OnceLock<Vec<FontFile>> = std::sync::OnceLock::new();
    let faces = FACES.get_or_init(|| {
        remedy_ui_fonts::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
    });
    #[cfg(feature = "devtools-system-fonts")]
    let mut f = FontCollection::with_system_fonts();
    #[cfg(not(feature = "devtools-system-fonts"))]
    let mut f = FontCollection::new();
    for file in faces {
        let _ = f.add_file(file);
    }
    for fb in remedy_ui_fonts::script_fallbacks() {
        let _ = f.add_script_fallback(fb.script, fb.locale, fb.family);
    }
    let cjk = remedy_ui_fonts_cjk::script_fallbacks();
    f.add_on_demand(
        remedy_ui_fonts_cjk::needs,
        || {
            CJK.get_or_init(|| {
                remedy_ui_fonts_cjk::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
            })
            .to_vec()
        },
        cjk.iter().map(|fb| (fb.script, fb.locale, fb.family)),
    );
    f
}

fn theme_for(v: &Variation) -> Theme {
    // `portal:<id>` is Portal's own palette of that id (crate::themes::portal_theme).
    let portal = v.theme.strip_prefix("portal:").and_then(crate::themes::portal_theme);
    let shared =
        || crate::themes::palette(&v.theme).map_or_else(Theme::dark_forest, crate::themes::theme);
    let mut t = portal.unwrap_or_else(shared);
    let s = v.text_scale.clamp(0.5, 4.0);
    t.type_scale.small *= s;
    t.type_scale.body *= s;
    t.type_scale.title *= s;
    t
}

/// True for locales written right to left.
pub fn is_rtl(locale: &str) -> bool {
    remedy_ui_text::is_rtl_locale(locale)
}

/// Rendered pixels: premultiplied RGBA8, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Premultiplied RGBA8.
    pub rgba: Vec<u8>,
}

impl Snapshot {
    /// The pixel at `(x, y)`, premultiplied.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba.get(i..i + 4)?.try_into().ok()
    }

    /// As an image element source.
    pub fn image(&self) -> remedy_ui_engine::paint::Image {
        // FNV-1a over the pixels: the same pixels share a cache entry.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in &self.rgba {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
        remedy_ui_engine::paint::Image {
            id: h,
            width: self.width,
            height: self.height,
            pixels: Arc::from(self.rgba.as_slice()),
        }
    }

    /// Encodes as PNG (straight alpha, as PNG stores it).
    pub fn to_png(&self) -> Result<Vec<u8>, PngError> {
        let mut straight = self.rgba.clone();
        for p in straight.chunks_mut(4) {
            if let [r, g, b, a] = p
                && *a != 0
                && *a != 255
            {
                for c in [r, g, b] {
                    *c = ((u32::from(*c) * 255 + u32::from(*a) / 2) / u32::from(*a)).min(255) as u8;
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(PngError::codec)?;
            w.write_image_data(&straight).map_err(PngError::codec)?;
        }
        Ok(out)
    }

    /// Decodes a PNG written by [`Snapshot::to_png`].
    pub fn from_png(bytes: &[u8]) -> Result<Snapshot, PngError> {
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut r = dec.read_info().map_err(PngError::codec)?;
        let size = r.output_buffer_size().ok_or(PngError::TooLarge)?;
        let mut buf = vec![0u8; size];
        let info = r.next_frame(&mut buf).map_err(PngError::codec)?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(PngError::NotRgba8);
        }
        buf.truncate(info.buffer_size());
        for p in buf.chunks_mut(4) {
            if let [r, g, b, a] = p {
                for c in [r, g, b] {
                    *c = ((u32::from(*c) * u32::from(*a) + 127) / 255) as u8;
                }
            }
        }
        Ok(Snapshot { width: info.width, height: info.height, rgba: buf })
    }

    /// How many pixels differ by more than `tolerance` in any channel (all of them when the
    /// sizes differ).
    pub fn diff(&self, other: &Snapshot, tolerance: u8) -> usize {
        if (self.width, self.height) != (other.width, other.height) {
            return (self.width * self.height).max(other.width * other.height) as usize;
        }
        self.rgba
            .chunks(4)
            .zip(other.rgba.chunks(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > tolerance))
            .count()
    }
}

/// Why a snapshot could not be written or read as PNG.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PngError {
    /// The PNG encoder or decoder failed.
    Codec {
        /// The codec's own words.
        reason: String,
    },
    /// The image is larger than this machine can address.
    TooLarge,
    /// The PNG is not 8-bit RGBA, which every snapshot is.
    NotRgba8,
}

impl PngError {
    fn codec(e: impl std::fmt::Display) -> PngError {
        PngError::Codec { reason: e.to_string() }
    }
}

impl std::fmt::Display for PngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PngError::Codec { reason } => write!(f, "PNG coding failed: {reason}"),
            PngError::TooLarge => f.write_str("the PNG is too large to decode on this machine"),
            PngError::NotRgba8 => f.write_str("the PNG is not 8-bit RGBA, as snapshots are"),
        }
    }
}

impl std::error::Error for PngError {}

/// Why a golden-image check failed.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GoldenError {
    /// There was no golden image; one was written for review.
    Missing(PathBuf),
    /// The rendering differs from the golden image; the new one was written beside it.
    Differs {
        /// The golden image.
        golden: PathBuf,
        /// The rendering, written as `<name>.actual.png`.
        actual: PathBuf,
        /// Pixels that differ.
        pixels: usize,
    },
    /// Reading or writing failed.
    Io(String),
    /// A snapshot or golden image could not be encoded or decoded.
    Png(PngError),
}

impl std::fmt::Display for GoldenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoldenError::Missing(p) => write!(f, "no golden image; wrote {} for review", p.display()),
            GoldenError::Differs { golden, actual, pixels } => {
                write!(f, "{pixels} pixels differ from {} (see {})", golden.display(), actual.display())
            }
            GoldenError::Io(e) => write!(f, "{e}"),
            GoldenError::Png(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GoldenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GoldenError::Png(e) => Some(e),
            _ => None,
        }
    }
}

/// Pixel channels may differ by this much (antialiasing across platforms' float math).
pub const GOLDEN_TOLERANCE: u8 = 8;

/// Checks every variation of `preview` against `dir/<name>-<variation>.png`. Missing
/// images are written and reported (all of them in one run); with `REMEDY_UPDATE_GOLDEN=1` every image is rewritten
/// and the check passes.
pub fn check_golden(preview: &Preview, dir: &Path) -> Result<(), GoldenError> {
    let update = std::env::var_os("REMEDY_UPDATE_GOLDEN").is_some_and(|v| v == "1");
    std::fs::create_dir_all(dir).map_err(|e| GoldenError::Io(e.to_string()))?;
    let safe: String =
        preview.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let mut first: Option<GoldenError> = None;
    for v in preview.variations() {
        let shot = preview.render(&v);
        let path = dir.join(format!("{safe}-{}.png", v.tag()));
        let write = |p: &Path, s: &Snapshot| -> Result<(), GoldenError> {
            let bytes = s.to_png().map_err(GoldenError::Png)?;
            std::fs::write(p, bytes).map_err(|e| GoldenError::Io(e.to_string()))
        };
        if update {
            write(&path, &shot)?;
            continue;
        }
        // Every variation is checked, so one run writes every missing image and every
        // differing rendering; the first failure is reported.
        let Ok(bytes) = std::fs::read(&path) else {
            write(&path, &shot)?;
            first.get_or_insert(GoldenError::Missing(path));
            continue;
        };
        let golden = Snapshot::from_png(&bytes).map_err(GoldenError::Png)?;
        let pixels = shot.diff(&golden, GOLDEN_TOLERANCE);
        if pixels > 0 {
            let actual = path.with_extension("actual.png");
            write(&actual, &shot)?;
            let differs = GoldenError::Differs { golden: path, actual, pixels };
            // A difference matters more than a missing image.
            if !matches!(first, Some(GoldenError::Differs { .. })) {
                first = Some(differs);
            }
        }
    }
    first.map_or(Ok(()), Err)
}

/// Every preview's variations side by side, each captioned.
#[track_caller]
pub fn gallery(ui: &Ui, previews: &[Preview]) -> El {
    let unit = ui.theme().spacing.unit;
    let mut col = w::column(ui).gap(unit * 6.0).padding(remedy_ui_core::Insets::all(unit * 4.0)).fill_width();
    for p in previews {
        let mut row = w::wrap(ui).gap(unit * 4.0);
        for v in p.variations() {
            let shot = p.render(&v);
            let caption = v.caption();
            row = row.child(
                w::column(ui)
                    .gap(unit)
                    .child(
                        w::image(ui, shot.image(), format!("{} preview, {caption}", p.name))
                            .width(v.size.0)
                            .height(v.size.1),
                    )
                    .child(w::text(ui, caption).variant("muted")),
            );
        }
        col = col.child(w::section(ui, &p.name).child(row));
    }
    w::scroll(ui).grow(1.0).fill_width().child(col)
}

/// The background a variation draws on (its theme's).
pub fn background(v: &Variation) -> Color {
    theme_for(v).colors.bg
}
