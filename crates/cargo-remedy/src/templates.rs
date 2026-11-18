//! The `document` and `android` templates.

/// A viewer for documents made of page images: the doc-pages view over a source the app owns.
pub const DOCUMENT: &str = r#"//! {{title}}: a viewer for documents made of page images (scans, exported slides, comics):
//! zoom from 1% to 6400%, spreads, rotation, thumbnails and the keyboard, with each page
//! decoded off the UI thread. The pages are the images in `pages/`, in file-name order;
//! replace them with your own, or put another engine (a PDF library) behind `PageSource`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use remedy_ui::core::Size;
use remedy_ui::pages::{Cancel, PageSource, PageView, RenderError, RenderRequest, page_view, thumbnails};
use remedy_ui::paint::Image;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

/// Page images are shown at this many dots per inch.
pub const DPI: f32 = 150.0;

/// Pages from image files.
pub struct ImagePages {
    pages: Vec<(Vec<u8>, (u32, u32))>,
    /// The page decoded last: tiles of one page come together.
    last: Mutex<Option<(usize, Arc<Image>)>>,
}

impl ImagePages {
    /// The images in `dir` (PNG, JPEG, WebP, ...), in file-name order. Files that are not
    /// images are left out.
    pub fn open(dir: &Path) -> std::io::Result<ImagePages> {
        let mut files: Vec<_> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
        files.sort();
        let mut images = Vec::new();
        for f in files {
            images.push(std::fs::read(f)?);
        }
        Ok(ImagePages::from_images(images))
    }

    /// Pages from encoded images, in order.
    pub fn from_images(images: Vec<Vec<u8>>) -> ImagePages {
        let pages = images
            .into_iter()
            .filter_map(|bytes| remedy_ui::image::dimensions(&bytes).ok().map(|size| (bytes, size)))
            .collect();
        ImagePages { pages, last: Mutex::new(None) }
    }

    fn decoded(&self, page: usize) -> Result<Arc<Image>, RenderError> {
        let mut last = self.last.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((p, img)) = last.as_ref()
            && *p == page
        {
            return Ok(img.clone());
        }
        let (bytes, _) = self.pages.get(page).ok_or_else(|| RenderError::Failed(format!("there is no page {page}")))?;
        let img = Arc::new(
            remedy_ui::image::decode(bytes, None)
                .map_err(|e| RenderError::Failed(e.to_string()))?
                .image,
        );
        *last = Some((page, img.clone()));
        Ok(img)
    }
}

impl PageSource for ImagePages {
    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_size(&self, page: usize) -> Size {
        let (w, h) = self.pages.get(page).map_or((0, 0), |(_, s)| *s);
        Size::new(w as f32 * 72.0 / DPI, h as f32 * 72.0 / DPI)
    }

    fn render(&self, request: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError> {
        let img = self.decoded(request.page)?;
        let (w, h) = (request.width as usize, request.height as usize);
        let mut out = vec![0u8; w * h * 4];
        // Each tile pixel takes the image pixel under its center.
        let px_per_point = DPI / 72.0;
        for y in 0..h {
            if y % 64 == 0 && cancel.is_cancelled() {
                return Err(RenderError::Cancelled);
            }
            let sy = ((request.region.min.y + (y as f32 + 0.5) / request.scale) * px_per_point) as usize;
            let sy = sy.min(img.height.saturating_sub(1) as usize);
            for x in 0..w {
                let sx = ((request.region.min.x + (x as f32 + 0.5) / request.scale) * px_per_point) as usize;
                let sx = sx.min(img.width.saturating_sub(1) as usize);
                let from = (sy * img.width as usize + sx) * 4;
                let to = (y * w + x) * 4;
                if let (Some(src), Some(dst)) = (img.pixels.get(from..from + 4), out.get_mut(to..to + 4)) {
                    dst.copy_from_slice(src);
                }
            }
        }
        Ok(Image::new(request.width, request.height, Arc::from(out)))
    }
}

/// Opens the document in `pages/` beside the app (or the folder given on the command line).
pub fn app(app: &App) {
    let dir = std::env::args().nth(1).map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("pages"),
        std::path::PathBuf::from,
    );
    app.open(WindowSpec::new("{{title}}").size(900.0, 700.0).persist_geometry("main"), move |ui, _| {
        let el = match ImagePages::open(&dir) {
            Ok(pages) if pages.page_count() > 0 => document(ui, Arc::new(pages)),
            Ok(_) => w::empty_state(ui, "No pages", "Put page images in the pages folder."),
            Err(e) => w::empty_state(ui, "The pages could not be read", &e.to_string()),
        };
        ui.root().child(el);
    });
}

/// The document: thumbnails beside the pages.
pub fn document(ui: &Ui, source: Arc<dyn PageSource>) -> El {
    let view = PageView::new(source);
    w::row(ui).grow(1.0).fill_width().child(thumbnails(ui, &view)).child(page_view(ui, &view, "{{title}}").grow(1.0))
}
"#;

/// The document template's main: the app's window.
pub const DOCUMENT_TEST_PREVIEW: &str = "{{crate}}::document(ui, std::sync::Arc::new({{crate}}::ImagePages::open(std::path::Path::new(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/pages\"))).unwrap_or_else(|_| {{crate}}::ImagePages::from_images(Vec::new()))))";

/// One codebase for Android and the desktop.
pub const ANDROID: &str = r#"//! {{title}}: one app for Android and the desktop. `cargo remedy run` runs it here;
//! `cargo remedy package --target android` builds its APK (see remedy.toml's [android]).

use remedy_ui::core::Insets;
use remedy_ui::i18n::{Arg, Messages};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

/// Opens the app's window (on Android, the activity's).
pub fn app(app: &App) {
    app.open(WindowSpec::new("{{title}}"), |ui, _| {
        ui.root().child(home(ui));
    });
}

/// The main screen.
pub fn home(ui: &Ui) -> El {
    let taps = Signal::new(0u32);
    let unit = ui.theme().spacing.unit;
    let (u, u2) = (ui.clone(), ui.clone());
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong"))
        .child(w::text(ui, "").variant("muted").text_dyn(move || {
            messages().format(&u, "taps", &[("count", Arg::from(taps.get()))])
        }))
        .child(w::button_dyn(ui, move || messages().text(&u2, "tap-me"), move || taps.update(|c| *c += 1)).variant("primary"))
}

/// The app's strings, from `i18n/<locale>.ftl`, carried inside the app.
pub fn messages() -> Messages {
    thread_local!(static MESSAGES: Messages = Messages::new("en-US").embedded("en-US", include_str!("../i18n/en-US.ftl")));
    MESSAGES.with(Clone::clone)
}

/// A label showing message `id`, following the window's locale.
fn say(ui: &Ui, id: &'static str, variant: &'static str) -> El {
    let u = ui.clone();
    w::text(ui, "").variant(variant).text_dyn(move || messages().text(&u, id))
}

/// Android starts the app here, in the activity's thread.
#[cfg(target_os = "android")]
fn android_main() {
    let config = remedy_ui::AppConfig::new("{{title}}").edition(remedy_ui::Edition::E{{edition}});
    if let Err(e) = remedy_ui::run(config, app) {
        remedy_ui::android::log(&format!("{{title}}: {e}"));
    }
}

#[cfg(target_os = "android")]
remedy_ui::android::main!(android_main);
"#;

/// The android template's Cargo.toml lines: the library is what the APK loads.
pub const ANDROID_LIB: &str = "\n[lib]\n# The APK loads the app as a shared library; the desktop links it into src/main.rs.\ncrate-type = [\"cdylib\", \"rlib\"]\n";

/// The android template's remedy.toml section.
pub const ANDROID_TOML: &str = r#"
[android]
# The Rust target the APK is built for: aarch64-linux-android (phones and tablets) or
# x86_64-linux-android (emulators). Install it with `rustup target add`.
target = "aarch64-linux-android"
# The lowest Android version the app runs on (API level) and the one it is built against.
min_api = 26
target_api = 35
# Sign for release with your keystore (and REMEDY_ANDROID_KEYSTORE_PASSWORD); without one,
# packages are signed with a debug key made for this build.
# keystore = "release.keystore"
# key_alias = "release"
"#;

/// The android template's messages.
pub const ANDROID_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org).\n\napp-title = {{title}}\ntaps = { $count ->\n    [0] Not tapped yet\n    [one] Tapped once\n   *[other] Tapped { $count } times\n}\ntap-me = Tap me\n";

/// The document template's messages.
pub const DOCUMENT_FTL: &str =
    "# {{title}}'s text, in Fluent (https://projectfluent.org).\n\napp-title = {{title}}\n";
