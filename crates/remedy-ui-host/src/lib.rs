//! RemedyUI's host model.
//!
//! The engine's own idea of what a platform provides: windows, a frame clock, input, text
//! input, the clipboard, timers, a wake handle for other threads, and system settings. Every
//! platform is a bridge that implements [`Host`] and feeds [`HostEvent`]s to an [`App`]:
//! Windows, macOS, Wayland and X11 (sharing a winit core), Android, RemedyOS, the web, iOS, and
//! the headless bridge used by tests. Nothing platform-shaped appears here, so adding a
//! platform means writing a bridge and nothing else. A capability a platform cannot provide
//! returns [`Unsupported`] with the reason; it is never faked. See
//! `docs/design/10-platform-bridges.md`.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::{Color, Point, Rect, Size};
use remedy_ui_input::{GamepadTuning, KeyEvent, PinchEvent, Platform, PointerEvent, WheelEvent};

mod badge;
pub use badge::Badge;

pub use accesskit;
pub use raw_window_handle;

/// The wall clock and the local time zone at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalTime {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub unix_seconds: i64,
    /// Seconds east of UTC at that instant (daylight saving included). 0 where the platform
    /// cannot say.
    pub utc_offset: i32,
}

impl LocalTime {
    /// Seconds since 1970-01-01T00:00:00 on the local wall clock.
    pub fn local_seconds(self) -> i64 {
        self.unix_seconds + i64::from(self.utc_offset)
    }

    /// Days since 1970-01-01 on the local calendar.
    pub fn local_days(self) -> i64 {
        self.local_seconds().div_euclid(86_400)
    }

    /// Seconds until the next local midnight (at least 1).
    pub fn until_midnight(self) -> u64 {
        (86_400 - self.local_seconds().rem_euclid(86_400)).max(1) as u64
    }
}

/// A screen reader or other assistive technology talking to a window.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum AccessibilityEvent {
    /// It connected and wants the whole tree; send it with [`Host::update_accessibility`].
    Requested,
    /// It asks an element to do something (click, focus, increment, …).
    Action(accesskit::ActionRequest),
    /// It went away; stop sending updates.
    Deactivated,
}

/// A window's own icon: what the taskbar, the title bar and window switchers show for it
/// (Windows; X11; Wayland compositors with the `xdg-toplevel-icon` protocol; a browser page's
/// icon). Square, premultiplied RGBA8; give the largest size the artwork has (256 pixels is
/// plenty) and each platform scales it to the sizes it shows.
///
/// Artwork drawn at several sizes (pixel art, or small sizes simplified so they stay
/// legible) adds them with [`WindowIcon::with_size`]: a platform showing one of those sizes
/// shows that drawing as it is, and scales the nearest larger one for any other size.
/// [`WindowIcon::pixel_art`] scales by repeating pixels instead of blending them, so hard
/// pixel edges stay hard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowIcon {
    size: u32,
    rgba: Vec<u8>,
    /// Other drawn sizes, smallest first, each `(size, premultiplied RGBA8)`; never `size`.
    more: Vec<(u32, Vec<u8>)>,
    pixel_art: bool,
}

impl WindowIcon {
    /// An icon `size` pixels square from premultiplied RGBA8, row by row; `None` when `rgba`
    /// is not `size` × `size` pixels (or `size` is zero).
    pub fn new(size: u32, rgba: Vec<u8>) -> Option<Self> {
        let want = u64::from(size) * u64::from(size) * 4;
        (size > 0 && rgba.len() as u64 == want).then_some(WindowIcon {
            size,
            rgba,
            more: Vec::new(),
            pixel_art: false,
        })
    }

    /// Adds the artwork as drawn at another size (with `other`'s own drawn sizes). The
    /// largest drawing becomes [`WindowIcon::size`] and [`WindowIcon::rgba`]; a size given
    /// twice keeps the later drawing.
    #[must_use]
    pub fn with_size(mut self, other: WindowIcon) -> Self {
        let pixel_art = self.pixel_art || other.pixel_art;
        let mut all = self.drawn_pixels();
        for (size, rgba) in other.drawn_pixels() {
            all.retain(|(s, _)| *s != size);
            all.push((size, rgba));
        }
        all.sort_by_key(|(s, _)| *s);
        if let Some((size, rgba)) = all.pop() {
            self.size = size;
            self.rgba = rgba;
        }
        self.more = all;
        self.pixel_art = pixel_art;
        self
    }

    /// Scales by repeating the nearest pixel instead of blending (for pixel art).
    #[must_use]
    pub fn pixel_art(mut self) -> Self {
        self.pixel_art = true;
        self
    }

    /// True when it scales by repeating pixels ([`WindowIcon::pixel_art`]).
    pub fn is_pixel_art(&self) -> bool {
        self.pixel_art
    }

    /// Width and height in pixels of the largest drawing.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Premultiplied RGBA8 of the largest drawing, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Every drawn size, smallest first, each as an icon of its own (for platforms that take
    /// a list of sizes and choose: X11's `_NET_WM_ICON`, a status notifier's pixmaps).
    pub fn drawn(&self) -> Vec<WindowIcon> {
        self.drawn_pixels()
            .into_iter()
            .map(|(size, rgba)| WindowIcon { size, rgba, more: Vec::new(), pixel_art: self.pixel_art })
            .collect()
    }

    /// The sizes drawn, smallest first.
    pub fn drawn_sizes(&self) -> Vec<u32> {
        self.more.iter().map(|(s, _)| *s).chain([self.size]).collect()
    }

    fn drawn_pixels(&self) -> Vec<(u32, Vec<u8>)> {
        let mut all = self.more.clone();
        all.push((self.size, self.rgba.clone()));
        all
    }

    /// Straight (not premultiplied) RGBA8 of the largest drawing, as most platforms' icon
    /// formats take it.
    pub fn straight_rgba(&self) -> Vec<u8> {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, a]| {
                let un = |c: u8| match (u16::from(c) * 255 + u16::from(a) / 2).checked_div(u16::from(a)) {
                    Some(v) => v.min(255) as u8,
                    None => 0,
                };
                [un(r), un(g), un(b), a]
            })
            .collect()
    }

    /// The icon at `size` pixels square, for the exact sizes a platform shows: the drawing of
    /// that size when there is one; otherwise the smallest larger drawing (the largest when
    /// none is larger) resampled, each new pixel the area-weighted average of the pixels it
    /// covers (in premultiplied color, so edges keep no dark fringe), or the nearest pixel for
    /// [`WindowIcon::pixel_art`].
    pub fn scaled(&self, size: u32) -> WindowIcon {
        if size == 0 {
            return self.clone();
        }
        let one =
            |s: u32, rgba: Vec<u8>| WindowIcon { size: s, rgba, more: Vec::new(), pixel_art: self.pixel_art };
        let (from, src) = self
            .more
            .iter()
            .find(|(s, _)| *s >= size)
            .map_or((self.size, self.rgba.as_slice()), |(s, px)| (*s, px.as_slice()));
        if from == size {
            return one(size, src.to_vec());
        }
        let rgba = if self.pixel_art { nearest(src, from, size) } else { area_average(src, from, size) };
        one(size, rgba)
    }
}

/// Square premultiplied RGBA8 `from` pixels wide, resampled to `to` by the nearest pixel
/// (the source pixel under each new pixel's center).
fn nearest(src: &[u8], from: u32, to: u32) -> Vec<u8> {
    let (from, to) = (from as usize, to as usize);
    let mut rgba = Vec::with_capacity(to * to * 4);
    for y in 0..to {
        let sy = ((2 * y + 1) * from / (2 * to)).min(from - 1);
        for x in 0..to {
            let sx = ((2 * x + 1) * from / (2 * to)).min(from - 1);
            let at = (sy * from + sx) * 4;
            rgba.extend_from_slice(src.get(at..at + 4).unwrap_or(&[0; 4]));
        }
    }
    rgba
}

/// Square premultiplied RGBA8 `from` pixels wide, resampled to `to` by area-weighted averages.
fn area_average(src: &[u8], from: u32, to: u32) -> Vec<u8> {
    let (from, to) = (from as usize, to as usize);
    let ratio = from as f64 / to as f64;
    // For each output column (and row): the input pixels it covers, with their weights.
    let spans: Vec<Vec<(usize, f64)>> = (0..to)
        .map(|o| {
            let (start, end) = (o as f64 * ratio, (o + 1) as f64 * ratio);
            let mut cover = Vec::new();
            let mut i = start.floor() as usize;
            while (i as f64) < end && i < from {
                let w = (end.min(i as f64 + 1.0) - start.max(i as f64)).max(0.0);
                if w > 0.0 {
                    cover.push((i, w));
                }
                i += 1;
            }
            cover
        })
        .collect();
    let mut rgba = Vec::with_capacity(to * to * 4);
    for ys in &spans {
        for xs in &spans {
            let mut sum = [0.0f64; 4];
            let mut total = 0.0;
            for &(y, wy) in ys {
                for &(x, wx) in xs {
                    let at = (y * from + x) * 4;
                    let w = wx * wy;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += f64::from(src.get(at + c).copied().unwrap_or(0)) * w;
                    }
                    total += w;
                }
            }
            for s in sum {
                rgba.push(if total > 0.0 { (s / total).round().clamp(0.0, 255.0) as u8 } else { 0 });
            }
        }
    }
    rgba
}

/// An app's icon in the system tray (notification area, menu bar extras, status notifier).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrayIcon {
    /// What hovering shows.
    pub tooltip: String,
    /// The icon: premultiplied RGBA8, square (the platform scales it).
    pub icon: TrayImage,
    /// The menu, top to bottom.
    pub menu: Vec<TrayItem>,
    /// The icon as drawn at several sizes ([`WindowIcon::with_size`]); when set, the platform
    /// takes the size it shows from it (a drawn one as it is) instead of scaling `icon`.
    pub artwork: Option<WindowIcon>,
}

impl TrayIcon {
    /// `icon` with `tooltip` and `menu`.
    pub fn new(tooltip: impl Into<String>, icon: TrayImage, menu: Vec<TrayItem>) -> Self {
        TrayIcon { tooltip: tooltip.into(), icon, menu, artwork: None }
    }

    /// Shows `artwork` at the size the platform draws tray icons (see [`TrayIcon::artwork`]).
    #[must_use]
    pub fn with_artwork(mut self, artwork: WindowIcon) -> Self {
        self.artwork = Some(artwork);
        self
    }

    /// The icon at `size` pixels square: from [`TrayIcon::artwork`] when set (a drawn size as
    /// it is), else `icon` resampled.
    pub fn image_at(&self, size: u32) -> TrayImage {
        let art = match &self.artwork {
            Some(a) => a.scaled(size),
            None => match WindowIcon::new(self.icon.size, self.icon.rgba.clone()) {
                Some(i) => i.scaled(size),
                None => return self.icon.clone(),
            },
        };
        TrayImage { size: art.size, rgba: art.rgba }
    }

    /// Every size the icon is drawn at, smallest first (for platforms that take a list):
    /// [`TrayIcon::artwork`]'s drawings, else `icon` alone.
    pub fn images(&self) -> Vec<TrayImage> {
        match &self.artwork {
            Some(a) => a.drawn().into_iter().map(|i| TrayImage { size: i.size, rgba: i.rgba }).collect(),
            None => vec![self.icon.clone()],
        }
    }
}

/// Tray icon pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayImage {
    /// Width and height in pixels.
    pub size: u32,
    /// Premultiplied RGBA8, row by row.
    pub rgba: Vec<u8>,
}

/// One tray menu entry. In a label, `&` before a character makes it the access key (the key
/// that picks the entry while the menu is open, underlined on Windows and X11), as on Windows;
/// `&&` is a literal `&`.
#[derive(Clone, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge must show completely, so a new kind is a major change.
pub enum TrayItem {
    /// A command; its id comes back in [`TrayEvent::Menu`].
    Item {
        /// The app's id for it.
        id: u32,
        /// What it says.
        label: String,
        /// Whether it can be chosen.
        enabled: bool,
        /// The menu's default command (what activating the icon does), shown in bold where
        /// the platform has a default item (Windows, the X11 tray's menu).
        default: bool,
    },
    /// A command with a check mark; its id comes back in [`TrayEvent::Menu`] and the app
    /// flips `checked` itself.
    Check {
        /// The app's id for it.
        id: u32,
        /// What it says.
        label: String,
        /// Whether it can be chosen.
        enabled: bool,
        /// Whether the mark shows.
        checked: bool,
    },
    /// A nested menu.
    Submenu {
        /// What it says.
        label: String,
        /// Its entries.
        items: Vec<TrayItem>,
    },
    /// A separator line.
    Separator,
}

impl TrayItem {
    /// An enabled command that is not the default.
    pub fn item(id: u32, label: impl Into<String>) -> Self {
        TrayItem::Item { id, label: label.into(), enabled: true, default: false }
    }

    /// An enabled command with a check mark.
    pub fn check(id: u32, label: impl Into<String>, checked: bool) -> Self {
        TrayItem::Check { id, label: label.into(), enabled: true, checked }
    }

    /// A nested menu.
    pub fn submenu(label: impl Into<String>, items: Vec<TrayItem>) -> Self {
        TrayItem::Submenu { label: label.into(), items }
    }

    /// The id a choice of this entry reports, if it is an enabled command.
    pub fn chosen_id(&self) -> Option<u32> {
        match self {
            TrayItem::Item { id, enabled: true, .. } | TrayItem::Check { id, enabled: true, .. } => Some(*id),
            _ => None,
        }
    }

    /// What it says (empty for a separator).
    pub fn label(&self) -> &str {
        match self {
            TrayItem::Item { label, .. }
            | TrayItem::Check { label, .. }
            | TrayItem::Submenu { label, .. } => label,
            TrayItem::Separator => "",
        }
    }
}

/// What the user did with the tray icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrayEvent {
    /// The icon was activated (clicked, or the platform's primary action).
    Activate,
    /// A menu entry was chosen.
    Menu(u32),
}

/// A native open or save dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileDialog {
    /// Title.
    pub title: String,
    /// Save rather than open.
    pub save: bool,
    /// Choose folders rather than files.
    pub folders: bool,
    /// Allow choosing several.
    pub multiple: bool,
    /// Start here.
    pub directory: Option<std::path::PathBuf>,
    /// Suggested file name (save).
    pub file_name: Option<String>,
    /// Filters: a name and the extensions it allows (without dots).
    pub filters: Vec<(String, Vec<String>)>,
}

/// A file dialog's identity while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DialogId(pub u64);

/// Where a file dialog stands once [`Host::file_dialog`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DialogStarted {
    /// It already closed (platforms whose dialogs run inside the call): the chosen paths,
    /// empty when cancelled.
    Finished(Vec<std::path::PathBuf>),
    /// It is open; [`HostEvent::FileDialog`] reports the choice (platforms whose pickers are
    /// separate screens, such as Android's).
    Pending(DialogId),
}

/// Something the app may need the person's leave for, on platforms that ask at run time
/// (Android). Desktop platforms grant at install or in their privacy settings instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Permission {
    /// Hearing the microphone (`RECORD_AUDIO` on Android, which the app's manifest must also
    /// declare).
    Microphone,
    /// Showing system notifications (`POST_NOTIFICATIONS` on Android 13 and later, which the
    /// app's manifest must also declare; earlier versions grant it at install). A browser
    /// asks the person.
    Notifications,
}

/// A system notification: a message outside the app's windows (Windows' notification
/// center, the Linux desktop's notification server, Android's shade, the browser's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notification {
    /// The app's own number for it: showing another with the same number replaces it, and
    /// [`HostEvent::Notification`] names it.
    pub id: u64,
    /// The first line, in bold where the platform has bold.
    pub title: String,
    /// The text under the title (may be empty).
    pub body: String,
    /// Buttons, in order. Platforms show the first few they have room for (Windows five,
    /// Android three, most Linux desktops several, browsers two).
    pub actions: Vec<NotificationAction>,
    /// Shown without a sound.
    pub silent: bool,
    /// The app's name, which the platform shows as the sender. The engine fills it in from
    /// the app's configuration.
    pub app: String,
    /// The picture shown with it (the app's icon when unset): premultiplied RGBA8.
    pub icon: Option<TrayImage>,
    /// How much it matters: platforms mark warnings and errors (Linux's urgency and stock
    /// icon, Windows' toast priority, macOS's interruption level, Android's category, a
    /// browser keeping it until dismissed).
    pub urgency: Urgency,
}

/// How much a [`Notification`] matters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Urgency {
    /// News: the default.
    #[default]
    Info,
    /// Something the person should look at.
    Warning,
    /// Something went wrong.
    Error,
}

impl Notification {
    /// Sets how much it matters.
    pub fn urgency(mut self, value: Urgency) -> Self {
        self.urgency = value;
        self
    }

    /// A notification numbered `id` with a title and a body.
    pub fn new(id: u64, title: impl Into<String>, body: impl Into<String>) -> Self {
        Notification { id, title: title.into(), body: body.into(), ..Notification::default() }
    }

    /// Adds a button that reports `id` when pressed.
    pub fn action(mut self, id: impl Into<String>, label: impl Into<String>) -> Self {
        self.actions.push(NotificationAction { id: id.into(), label: label.into() });
        self
    }

    /// Sets `silent`: shown without a sound.
    pub fn silent(mut self, value: bool) -> Self {
        self.silent = value;
        self
    }

    /// Sets the picture shown with it.
    pub fn icon(mut self, value: TrayImage) -> Self {
        self.icon = Some(value);
        self
    }
}

/// A button on a notification.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotificationAction {
    /// What [`NotificationResponse::Action`] reports.
    pub id: String,
    /// The button's text.
    pub label: String,
}

/// What the person did with a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationResponse {
    /// They pressed it (the app should come forward and show what it is about).
    Activated,
    /// They pressed one of its buttons.
    Action(String),
    /// They dismissed it, or it timed out of view (where the platform says so).
    Dismissed,
}

/// What ongoing work keeps the app running while it is out of sight, on platforms that stop
/// background apps (Android's foreground service types). Each kind the app uses must also be
/// listed in `remedy.toml`'s `[android] foreground`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ForegroundKind {
    /// Playing sound or video the person listens to (`mediaPlayback`): music, a voice call's
    /// far side.
    MediaPlayback,
    /// Hearing the microphone (`microphone`): recording, a voice call. Android also needs
    /// [`Permission::Microphone`] granted, and the app on screen when the work starts.
    Microphone,
    /// Using the camera (`camera`): a video call. Android also needs the camera permission
    /// granted, and the app on screen when the work starts.
    Camera,
    /// Moving data the person asked for (`dataSync`): an upload, a backup, a download.
    /// Android 15 limits it to six hours a day.
    DataSync,
}

/// Ongoing work that must keep running while the app is out of sight (Android's foreground
/// service): the kinds of work and the notification that shows it is under way while it
/// runs. Desktop and web apps keep running anyway; there it changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ForegroundTask {
    /// What the work does (at least one).
    pub kinds: Vec<ForegroundKind>,
    /// Shown while the work runs; its buttons, pressing it and dismissing it report through
    /// [`HostEvent::Notification`] with its id.
    pub notification: Notification,
}

impl ForegroundTask {
    /// Work of `kind`, shown by `notification`.
    pub fn new(kind: ForegroundKind, notification: Notification) -> Self {
        ForegroundTask { kinds: vec![kind], notification }
    }

    /// Adds another kind of work (a voice call both plays and hears sound).
    pub fn kind(mut self, kind: ForegroundKind) -> Self {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
        self
    }
}

/// A permission request's identity while the platform asks the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PermissionId(pub u64);

/// Where a permission request stands once [`Host::request_permission`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum PermissionStarted {
    /// Already decided: true when the app may go ahead. Platforms with no run-time
    /// permissions answer true; their own privacy settings may still refuse the device,
    /// which then reports why when it is opened.
    Finished(bool),
    /// The platform is asking the person; [`HostEvent::Permission`] reports the answer.
    Pending(PermissionId),
}

/// What to hand to the platform's share sheet: a title, a text, and files made in memory.
///
/// Made with [`ShareOptions::new`] and its setters:
///
/// ```
/// use remedy_ui_host::{ShareFile, ShareOptions};
/// let recipe = br#"{"name":"Lavender"}"#.to_vec();
/// let share = ShareOptions::new()
///     .title("Lavender soap")
///     .text("Lavender soap: 500 g oils, 5% superfat")
///     .file(ShareFile::new("lavender.alex-soap.json", "application/json", recipe));
/// assert_eq!(share.files.len(), 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShareOptions {
    /// The share's subject: what a mail gets as its subject and a sheet shows as its
    /// heading. Empty for none.
    pub title: String,
    /// The text shared. Empty for none.
    pub text: String,
    /// Files, in the order given.
    pub files: Vec<ShareFile>,
    /// Where a platform can share text but not files (some browsers), refuse with
    /// [`ShareOutcome::Unsupported`] instead of sharing the title and text alone. False by
    /// default: the text goes, without the files.
    pub require_files: bool,
}

impl ShareOptions {
    /// Nothing to share yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the subject.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the text.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Adds a file.
    pub fn file(mut self, file: ShareFile) -> Self {
        self.files.push(file);
        self
    }

    /// Replaces the files.
    pub fn files(mut self, files: impl IntoIterator<Item = ShareFile>) -> Self {
        self.files = files.into_iter().collect();
        self
    }

    /// Refuses to share without the files where a platform cannot share files.
    pub fn require_files(mut self, on: bool) -> Self {
        self.require_files = on;
        self
    }

    /// True when there is nothing to share: no title, text or files.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.text.is_empty() && self.files.is_empty()
    }

    /// The MIME type a platform labels the whole share with: the files' type when they all
    /// have one, their common family (`image/*`) when they share it, `*/*` for a mix, and
    /// `text/plain` for text alone.
    pub fn mime_type(&self) -> String {
        let mut types = self.files.iter().map(ShareFile::mime_type);
        let Some(first) = types.next() else { return "text/plain".to_owned() };
        let family = |t: &str| t.split('/').next().unwrap_or_default().to_owned();
        let mut all_same = true;
        let mut same_family = true;
        for t in types {
            all_same &= t == first;
            same_family &= family(t) == family(first);
        }
        if all_same {
            first.to_owned()
        } else if same_family {
            format!("{}/*", family(first))
        } else {
            "*/*".to_owned()
        }
    }
}

/// A file to share, made in memory: the name the receiving app sees, its MIME type, and its
/// contents.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShareFile {
    /// The file's name (`notes.txt`), without folders.
    pub name: String,
    /// Its MIME type (`application/json`); empty means `application/octet-stream`.
    pub mime: String,
    /// Its contents.
    pub bytes: Vec<u8>,
}

impl ShareFile {
    /// A file named `name` of type `mime` holding `bytes`.
    pub fn new(name: impl Into<String>, mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        ShareFile { name: name.into(), mime: mime.into(), bytes: bytes.into() }
    }

    /// The MIME type, `application/octet-stream` when none was given.
    pub fn mime_type(&self) -> &str {
        let m = self.mime.trim();
        if m.is_empty() || !m.contains('/') { "application/octet-stream" } else { m }
    }

    /// The name as a single file name every file system takes, for bridges that write the
    /// file before handing it over: folders, separators, control characters and the
    /// characters Windows refuses become `_`, and an empty or dot-only name becomes `file`.
    pub fn safe_name(&self) -> String {
        let mut name: String = self
            .name
            .chars()
            .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
            .collect();
        // Windows drops trailing dots and spaces, which would change the name.
        while name.ends_with(['.', ' ']) {
            name.pop();
        }
        let name = name.trim_start().to_owned();
        let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.ends_with(|c: char| c.is_ascii_digit()));
        if name.is_empty() || name.chars().all(|c| c == '.') {
            "file".to_owned()
        } else if reserved {
            format!("_{name}")
        } else {
            name
        }
    }
}

impl std::fmt::Debug for ShareFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShareFile")
            .field("name", &self.name)
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// How a share ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShareOutcome {
    /// The person chose where it goes. Android and Windows cannot tell whether the receiving
    /// app then finished (a mail may still be discarded); a browser says so when the target
    /// took it.
    Shared,
    /// The person closed the share sheet without choosing.
    Cancelled,
    /// This platform cannot share it, and why; the app falls back (a save dialog, the
    /// clipboard).
    Unsupported(Unsupported),
}

/// A share sheet's identity while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShareId(pub u64);

/// Where a share stands once [`Host::share`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum ShareStarted {
    /// It already ended.
    Finished(ShareOutcome),
    /// The share sheet is open; [`HostEvent::Share`] reports how it ended.
    Pending(ShareId),
}

/// The edge or corner a resize drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: a window has four edges and four corners.
pub enum ResizeEdge {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top-left corner.
    TopLeft,
    /// Top-right corner.
    TopRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom-right corner.
    BottomRight,
}

/// A window's identity for the life of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// A timer's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

/// A capability this platform does not have, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unsupported {
    /// What was asked for.
    pub what: &'static str,
    /// Why this platform cannot do it.
    pub reason: String,
}

impl Unsupported {
    /// `what` is not available because of `reason`.
    pub fn new(what: &'static str, reason: impl Into<String>) -> Self {
        Unsupported { what, reason: reason.into() }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not available here: {}", self.what, self.reason)
    }
}

impl std::error::Error for Unsupported {}

/// How to open a window. Sizes are logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowOptions {
    /// Title (also what assistive technology and the task switcher show).
    pub title: String,
    /// Content size.
    pub size: Size,
    /// Smallest content size the user may resize to.
    pub min_size: Option<Size>,
    /// Position on the desktop; `None` lets the platform place it.
    pub position: Option<Point>,
    /// The user may resize it.
    pub resizable: bool,
    /// The platform draws a frame and title bar; false for frameless windows that draw their
    /// own chrome.
    pub decorated: bool,
    /// Stays above other windows.
    pub always_on_top: bool,
    /// Has a transparent background (for rounded or shaped windows).
    pub transparent: bool,
    /// Shown at creation.
    pub visible: bool,
    /// Kept out of screenshots, screen sharing and recordings where the platform allows.
    pub capture_protected: bool,
    /// The window's own icon; `None` shows the app's (from its executable or desktop entry).
    pub icon: Option<WindowIcon>,
    /// Showing it gives it keyboard focus and brings it forward (true by default); false
    /// shows it behind the active window without taking focus (Windows; X11 window managers
    /// that follow `_NET_WM_USER_TIME`; macOS; ignored where windows always come forward).
    pub activate: bool,
    /// It has a taskbar button (true by default); false keeps it off the taskbar and the
    /// window switcher's list (Windows; X11 window managers that follow `_NET_WM_STATE`).
    pub taskbar: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: String::new(),
            size: Size::new(800.0, 600.0),
            min_size: None,
            position: None,
            resizable: true,
            decorated: true,
            always_on_top: false,
            transparent: false,
            visible: true,
            capture_protected: false,
            icon: None,
            activate: true,
            taskbar: true,
        }
    }
}

/// System settings the engine follows, delivered again whenever one changes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SystemSettings {
    /// Shortcut conventions.
    pub platform: Platform,
    /// The system prefers dark; `None` when the platform does not say.
    pub dark: Option<bool>,
    /// The user's accent color, where the platform has one.
    pub accent: Option<Color>,
    /// High contrast is on.
    pub high_contrast: bool,
    /// Text scale chosen by the user (1 is the default size).
    pub text_scale: f32,
    /// Reduce motion.
    pub reduced_motion: bool,
    /// Reduce transparency.
    pub reduced_transparency: bool,
    /// BCP 47 locale, such as `en-US`.
    pub locale: String,
    /// The user's preferred languages (BCP 47), most preferred first: `navigator.languages`
    /// on the web, the Windows display-language list, `LANGUAGE` then `LC_ALL`,
    /// `LC_MESSAGES` or `LANG` on Linux, `NSLocale.preferredLanguages` on macOS, the
    /// configuration's locale list on Android. Never empty: `[locale]` where the platform
    /// gives no list. For choosing among content languages (subtitles, audio tracks).
    pub locales: Vec<String>,
    /// Longest gap between the clicks of a double click.
    pub double_click: Duration,
    /// Caret blink interval; `None` when the user turned blinking off.
    pub caret_blink: Option<Duration>,
    /// How long a key is held before it repeats (gamepad directions follow it unless the app
    /// tunes them: `GamepadTuning`).
    pub key_repeat_delay: Duration,
    /// The time between a held key's repeats.
    pub key_repeat_interval: Duration,
    /// The system draws text with subpixel antialiasing (ClearType on Windows): `Some(bgr)`,
    /// true where the panel's subpixels run blue, green, red; `None` for grayscale or unknown.
    pub subpixel_text: Option<bool>,
    /// The system saves power: on battery, or in a battery or energy saver mode. Ambient
    /// motion slows down or stops (the view's `AmbientPolicy`). False where the platform
    /// does not say.
    pub power_saving: bool,
    /// The device is driven by arrow keys or a d-pad rather than a pointer: a TV, or a
    /// browser with only a coarse (touch) pointer. Windows turn spatial navigation on unless
    /// the app chose (the view's `Ui::set_spatial_navigation`). False on desktops.
    pub spatial_navigation: bool,
    /// What the primary pointer can do, where the platform says (the web's `(hover: hover)`
    /// and `(pointer: fine)` media queries). `None` where the engine goes by the pointer
    /// last used.
    pub pointer: Option<PointerCaps>,
    /// What the pointing hardware attached can do ([`PointerCaps::of_devices`]), where the
    /// platform can tell, read again when devices come and go: the caps start from it and the
    /// pointer last used refines them. Ignored while [`pointer`](Self::pointer) is set.
    pub pointer_devices: Option<PointerCaps>,
}

/// What the pointer in use can do: widgets show hover-only affordances where it hovers, and
/// larger targets where it is coarse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PointerCaps {
    /// It can hover over things without pressing them (a mouse, a touchpad).
    pub hover: bool,
    /// It points precisely (a mouse or a pen, not a finger).
    pub fine: bool,
}

impl PointerCaps {
    /// A mouse or touchpad: hovers and points finely.
    pub const MOUSE: PointerCaps = PointerCaps { hover: true, fine: true };
    /// A finger: neither.
    pub const TOUCH: PointerCaps = PointerCaps { hover: false, fine: false };
    /// A stylus: fine, but most do not hover in a way pages can count on.
    pub const PEN: PointerCaps = PointerCaps { hover: false, fine: true };

    /// The caps from what the pointer does.
    pub fn new(hover: bool, fine: bool) -> PointerCaps {
        PointerCaps { hover, fine }
    }

    /// The caps of the pointing hardware attached: a mouse or touchpad hovers and points
    /// finely; without one, a touch screen is a finger (pens are usually beside one); a pen
    /// alone is a pen. `None` with none of them.
    pub fn of_devices(mouse: bool, touch: bool, pen: bool) -> Option<PointerCaps> {
        if mouse {
            Some(PointerCaps::MOUSE)
        } else if touch {
            Some(PointerCaps::TOUCH)
        } else if pen {
            Some(PointerCaps::PEN)
        } else {
            None
        }
    }
}

impl Default for PointerCaps {
    /// What a device starts with before any pointer is used: a finger on phones and
    /// tablets, a mouse elsewhere.
    fn default() -> Self {
        if cfg!(any(target_os = "android", target_os = "ios")) {
            PointerCaps::TOUCH
        } else {
            PointerCaps::MOUSE
        }
    }
}

impl Default for SystemSettings {
    fn default() -> Self {
        SystemSettings {
            platform: Platform::Standard,
            dark: None,
            accent: None,
            high_contrast: false,
            text_scale: 1.0,
            reduced_motion: false,
            reduced_transparency: false,
            locale: String::from("en-US"),
            locales: vec![String::from("en-US")],
            double_click: Duration::from_millis(500),
            caret_blink: Some(Duration::from_millis(530)),
            key_repeat_delay: Duration::from_millis(500),
            key_repeat_interval: Duration::from_millis(33),
            subpixel_text: None,
            power_saving: false,
            spatial_navigation: false,
            pointer: None,
            pointer_devices: None,
        }
    }
}

/// The edges of a window that system interface covers or may cover: status and navigation
/// bars, display cutouts and the on-screen keyboard, in logical pixels. Content belongs inside
/// them; the window's background may run under them. Zero on every side where nothing covers
/// the window (desktop windows).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeArea {
    /// Covered at the top.
    pub top: f32,
    /// Covered on the right.
    pub right: f32,
    /// Covered at the bottom.
    pub bottom: f32,
    /// Covered on the left.
    pub left: f32,
}

impl SafeArea {
    /// Device pixels to logical pixels at `scale` device pixels per logical pixel.
    pub fn from_device(top: f32, right: f32, bottom: f32, left: f32, scale: f32) -> SafeArea {
        let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let px = |v: f32| if v.is_finite() { v.max(0.0) / s } else { 0.0 };
        SafeArea { top: px(top), right: px(right), bottom: px(bottom), left: px(left) }
    }
}

/// How hard the system is asking processes to give memory back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum MemoryPressure {
    /// Plenty.
    Normal,
    /// Drop caches that are cheap to rebuild.
    Moderate,
    /// Drop everything not on screen.
    Critical,
}

/// What a focused text field tells the input method.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextInputState {
    /// Text around the caret, for reconversion and prediction (empty for private fields).
    pub surrounding: String,
    /// Selection within `surrounding`, in bytes.
    pub selection: Range<usize>,
    /// Caret rectangle in window logical pixels, where the candidate window goes.
    pub caret: Rect,
    /// A password or other secret: no learning, no prediction, no cloud input.
    pub private: bool,
    /// Several lines (Enter inserts a line break instead of submitting).
    pub multiline: bool,
    /// A number: input methods show their number pad (with a decimal separator). True for
    /// [`InputPurpose::Digits`], [`InputPurpose::Decimal`] and [`InputPurpose::OneTimeCode`].
    pub numeric: bool,
    /// What the field holds, so keyboards show the right keys and password managers and
    /// autofill know what to offer.
    pub purpose: InputPurpose,
    /// What the touch keyboard's Enter key says and does; `None` picks it from the field
    /// ([`TextInputState::enter_key`]).
    pub enter: Option<EnterKey>,
}

impl TextInputState {
    /// The Enter key this field shows: [`enter`](Self::enter) when set, otherwise a line
    /// break for several lines, "search" for a search, "go" for an address, else "done".
    pub fn enter_key(&self) -> EnterKey {
        match self.enter {
            Some(k) => k,
            None if self.multiline => EnterKey::Enter,
            None => match self.purpose {
                InputPurpose::Search => EnterKey::Search,
                InputPurpose::Url => EnterKey::Go,
                _ => EnterKey::Done,
            },
        }
    }
}

/// What a touch keyboard's Enter key says and does (the web's `enterkeyhint`, Android's IME
/// action). [`Next`](Self::Next) and [`Previous`](Self::Previous) move focus as Tab and
/// Shift+Tab do; the others press Enter, which submits a one-line field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EnterKey {
    /// A line break.
    Enter,
    /// Finished with this field.
    Done,
    /// Go to what was typed (an address).
    Go,
    /// On to the next field.
    Next,
    /// Back to the previous field.
    Previous,
    /// Search for what was typed.
    Search,
    /// Send what was typed (a message).
    Send,
}

/// What a text field holds: picks the touch keyboard's layout, the input method's mode, and
/// what password managers and autofill offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InputPurpose {
    /// Ordinary text.
    #[default]
    Text,
    /// Whole numbers: the number pad, no decimal separator.
    Digits,
    /// A number with a decimal separator.
    Decimal,
    /// A web address.
    Url,
    /// An email address.
    Email,
    /// A search query: Enter shows as "search".
    Search,
    /// An account name, for password managers.
    Username,
    /// The account's existing password, for password managers to fill.
    CurrentPassword,
    /// A new password, for password managers to suggest and save.
    NewPassword,
    /// A one-time code sent by text or email, for the keyboard to offer.
    OneTimeCode,
}

impl InputPurpose {
    /// Shows the number pad.
    pub fn is_numeric(self) -> bool {
        matches!(self, InputPurpose::Digits | InputPurpose::Decimal | InputPurpose::OneTimeCode)
    }

    /// A password: never learned or predicted.
    pub fn is_password(self) -> bool {
        matches!(self, InputPurpose::CurrentPassword | InputPurpose::NewPassword)
    }
}

/// What an input method did.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum ImeEvent {
    /// Composition started.
    Enabled,
    /// The text being composed, with the input method's caret or selection inside it.
    Preedit {
        /// The composition.
        text: String,
        /// Caret or selection within `text`, in bytes.
        cursor: Option<Range<usize>>,
    },
    /// Final text to insert.
    Commit(String),
    /// Delete text around the caret (bytes before and after), as some input methods ask.
    DeleteSurrounding {
        /// Bytes before the caret.
        before: usize,
        /// Bytes after.
        after: usize,
    },
    /// Composition ended.
    Disabled,
}

/// Everything a host tells the app.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum HostEvent {
    /// The app may create windows (at start, and again after a mobile resume).
    Resumed,
    /// The app should release surfaces (mobile background).
    Suspended,
    /// A window's surface size or scale changed (device pixels and device pixels per logical
    /// pixel). Also sent once when a window is created.
    Resized {
        /// The window.
        window: WindowId,
        /// Width in device pixels.
        width: u32,
        /// Height in device pixels.
        height: u32,
        /// Device pixels per logical pixel.
        scale: f32,
    },
    /// The user asked to close a window. The app decides.
    CloseRequested(WindowId),
    /// A window moved; its top-left is now at this desktop position (logical pixels).
    Moved(WindowId, Point),
    /// A window is gone.
    Destroyed(WindowId),
    /// Keyboard focus came or went.
    Focused(WindowId, bool),
    /// The part of a window that system interface leaves clear changed (also sent once when
    /// it is first known).
    SafeArea(WindowId, SafeArea),
    /// A window became fully covered or minimized (true), or visible again (false).
    Occluded(WindowId, bool),
    /// Pointer input.
    Pointer(WindowId, PointerEvent),
    /// Wheel or touchpad scroll.
    Wheel(WindowId, WheelEvent),
    /// A pinch to zoom the platform recognized (touchpad magnify gestures).
    Pinch(WindowId, PinchEvent),
    /// Key input.
    Key(WindowId, KeyEvent),
    /// Input method events for the focused text field.
    Ime(WindowId, ImeEvent),
    /// Time to draw a requested frame, for presentation at `at`.
    Frame {
        /// The window.
        window: WindowId,
        /// Predicted presentation time on the host clock.
        at: Duration,
    },
    /// System settings changed.
    Settings(SystemSettings),
    /// The system wants memory back.
    MemoryPressure(MemoryPressure),
    /// A timer is due.
    Timer(TimerId),
    /// Another thread called the [`Waker`].
    Wake,
    /// The tray icon was used.
    Tray(TrayEvent),
    /// Assistive technology on a window.
    Accessibility(WindowId, AccessibilityEvent),
    /// Something is dragged over a window, or dropped on it.
    Drag(WindowId, DragEvent),
    /// A drag out of a window that [`Host::start_drag`] left pending ended.
    DragEnded(WindowId, DropEffect),
    /// A file dialog [`Host::file_dialog`] left pending closed: the chosen paths, empty when
    /// cancelled.
    FileDialog(DialogId, Vec<std::path::PathBuf>),
    /// A permission request [`Host::request_permission`] left pending was answered: true when
    /// granted.
    Permission(PermissionId, bool),
    /// The person used a notification the app showed (by its [`Notification::id`]).
    Notification(u64, NotificationResponse),
    /// The platform ended the work [`Host::start_foreground`] started, or could not start it
    /// after all (the app was no longer on screen, the system's time limit for the kind ran
    /// out): why, in words.
    ForegroundStopped(String),
    /// A share sheet [`Host::share`] left pending closed.
    Share(ShareId, ShareOutcome),
    /// The app was launched again while running (single instance); these are the new
    /// launch's arguments.
    SecondInstance(Vec<String>),
}

/// What a drag carries: files, text and links, in the forms other apps understand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragData {
    /// Files and folders.
    pub files: Vec<std::path::PathBuf>,
    /// Plain text.
    pub text: Option<String>,
    /// Links (`https:` and other URIs).
    pub uris: Vec<String>,
}

impl DragData {
    /// Nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.text.is_none() && self.uris.is_empty()
    }

    /// Some files.
    pub fn files(files: Vec<std::path::PathBuf>) -> Self {
        DragData { files, ..DragData::default() }
    }

    /// Some text.
    pub fn text(text: impl Into<String>) -> Self {
        DragData { text: Some(text.into()), ..DragData::default() }
    }
}

/// What a drop does to what was dragged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the four outcomes every platform's drag and drop knows.
pub enum DropEffect {
    /// Refused: nothing happens.
    #[default]
    None,
    /// Copied; the source keeps it.
    Copy,
    /// Moved; the source lets it go.
    Move,
    /// Linked to where it is.
    Link,
}

/// Where a drag out of a window stands once [`Host::start_drag`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DragStarted {
    /// It already ended (platforms whose drags run to the end inside the call): what the
    /// drop did.
    Finished(DropEffect),
    /// It is under way; [`HostEvent::DragEnded`] reports how it ended.
    Pending,
}

/// A drag from another app (or from this one, outside its own windows) over a window.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DragEvent {
    /// It came in. Platforms that do not say where report `None` until it moves.
    Enter {
        /// Where, in logical pixels.
        position: Option<Point>,
        /// What it carries.
        data: DragData,
    },
    /// It moved.
    Over {
        /// Where, in logical pixels.
        position: Point,
    },
    /// It left, or was cancelled.
    Leave,
    /// It was dropped.
    Drop {
        /// Where, in logical pixels (`None` when the platform does not say).
        position: Option<Point>,
        /// What it carries.
        data: DragData,
    },
}

/// True for links an app may hand to the system to open: web pages and mail. Anything else
/// (`file:`, custom protocols, programs) could start software the user did not mean to run.
pub fn is_openable_url(url: &str) -> bool {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    let scheme_ok = ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s));
    // No whitespace or control characters: one link, nothing smuggled after it.
    scheme_ok && u.len() == url.len() && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Premultiplied RGBA8 pixels to present.
#[derive(Clone, Copy, Debug)]
pub struct Pixels<'a> {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Row-major premultiplied RGBA8.
    pub data: &'a [u8],
}

/// Wakes the host's event loop from any thread; the app then receives [`HostEvent::Wake`].
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A window's native handles, for creating GPU surfaces.
pub trait WindowHandles:
    raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync
{
}

impl<T: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync> WindowHandles
    for T
{
}

/// What the app may ask of its host. Every bridge implements it.
pub trait Host {
    /// Shortcut conventions of this platform.
    fn platform(&self) -> Platform;
    /// The host clock (monotonic, the clock frame times use).
    fn now(&self) -> Duration;
    /// The wall clock and the local UTC offset now (for dates and times people read).
    fn local_time(&self) -> LocalTime;
    /// Current system settings.
    fn settings(&self) -> SystemSettings;
    /// The machine's physical memory in bytes, when the platform says (packs size caches
    /// from it).
    fn total_memory(&self) -> Option<u64> {
        None
    }
    /// Opens a window. A [`HostEvent::Resized`] follows with its real size and scale.
    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported>;
    /// Closes a window. [`HostEvent::Destroyed`] follows.
    fn close_window(&mut self, window: WindowId);
    /// A window's size in device pixels and its scale.
    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)>;
    /// The edges of a window that system interface covers (status and navigation bars,
    /// cutouts, the on-screen keyboard). [`HostEvent::SafeArea`] reports changes.
    fn safe_area(&self, window: WindowId) -> SafeArea {
        let _ = window;
        SafeArea::default()
    }
    /// How often the display the window is on refreshes (16.7 ms at 60 Hz), where the
    /// platform says. Frames that take longer miss a refresh.
    fn refresh_interval(&self, window: WindowId) -> Option<std::time::Duration> {
        let _ = window;
        None
    }
    /// Changes the title.
    fn set_title(&mut self, window: WindowId, title: &str);
    /// Resizes the content to `size` logical pixels.
    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported>;
    /// Shows or hides.
    fn set_visible(&mut self, window: WindowId, visible: bool);
    /// Whether [`Host::set_visible`]`(window, false)` takes the open window off the screen.
    /// False where the platform has no such request (Wayland: only the compositor unmaps a
    /// toplevel); the engine then minimizes instead of hiding.
    fn can_hide(&self, window: WindowId) -> bool {
        let _ = window;
        true
    }
    /// The window's top-left on the desktop in logical pixels, where the platform reports it
    /// (Wayland does not).
    fn window_position(&self, window: WindowId) -> Option<Point>;
    /// Moves the window's top-left to a desktop position in logical pixels.
    fn set_position(&mut self, window: WindowId, position: Point) -> Result<(), Unsupported>;
    /// Moves the window to the middle of the display it is on, as a message box opens.
    /// [`Unsupported`] where the app may not place windows (Wayland, a browser, Android): the
    /// platform places them itself there.
    fn center_window(&mut self, window: WindowId) -> Result<(), Unsupported> {
        let _ = window;
        Err(Unsupported::new("centering windows", "this platform places windows itself"))
    }
    /// Brings the window to the front and gives it keyboard focus.
    fn focus_window(&mut self, window: WindowId);
    /// Minimizes or restores.
    fn set_minimized(&mut self, window: WindowId, minimized: bool);
    /// Maximizes or restores.
    fn set_maximized(&mut self, window: WindowId, maximized: bool);
    /// True while the window is maximized (by the app or the user).
    fn is_maximized(&self, window: WindowId) -> bool;
    /// Makes the window fill its display with no frame (a player's full screen, a slide
    /// show), or brings it back.
    fn set_fullscreen(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        let _ = (window, on);
        Err(Unsupported::new("full-screen windows", "this host has no full-screen mode"))
    }
    /// True while the window fills its display (by the app or the user).
    fn is_fullscreen(&self, window: WindowId) -> bool {
        let _ = window;
        false
    }
    /// Starts a user move of the window, as dragging a title bar does (call on a pointer
    /// press in custom chrome).
    fn begin_move(&mut self, window: WindowId) -> Result<(), Unsupported>;
    /// Starts a user resize from `edge`, as dragging a window border does.
    fn begin_resize(&mut self, window: WindowId, edge: ResizeEdge) -> Result<(), Unsupported>;
    /// Keeps above other windows, or not.
    fn set_always_on_top(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported>;
    /// Keeps the window out of screenshots and screen sharing, or not.
    fn set_capture_protected(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported>;
    /// Gives the window its own icon (taskbar, title bar, window switchers), or back the
    /// app's with `None`. Platforms whose windows have no icons of their own (macOS, where
    /// the Dock shows the app's; Android) say `Unsupported`.
    fn set_window_icon(&mut self, window: WindowId, icon: Option<&WindowIcon>) -> Result<(), Unsupported> {
        let _ = (window, icon);
        Err(Unsupported::new("window icons", "this host shows no icons for windows"))
    }
    /// Hides the mouse pointer while it is over the window, or shows it again (a video
    /// playing full screen with its controls faded). Hosts without a pointer ignore it.
    fn set_cursor_hidden(&mut self, window: WindowId, hidden: bool) {
        let _ = (window, hidden);
    }
    /// Asks for a [`HostEvent::Frame`] at the next display refresh. Several requests before
    /// it arrives merge into one.
    fn request_frame(&mut self, window: WindowId);
    /// Shows CPU-rendered pixels. Only `damage` (device pixels) changed since the last
    /// present; hosts that can, copy and flush only that.
    fn present_pixels(
        &mut self,
        window: WindowId,
        pixels: Pixels<'_>,
        damage: &[Rect],
    ) -> Result<(), Unsupported>;
    /// The window's native handles, for a GPU surface.
    fn window_handles(&self, window: WindowId) -> Result<Arc<dyn WindowHandles>, Unsupported>;
    /// True when a window's surface keeps the renderer it was first presented with: once
    /// CPU pixels reached it, a GPU swapchain cannot take it over until the platform gives
    /// the window a new surface (Android). The engine then picks the renderer before a
    /// window's first present.
    fn renderer_fixed_per_surface(&self) -> bool {
        false
    }
    /// Which renderer draws a window now: the GPU (on a surface from
    /// [`Host::window_handles`]) or the CPU (through [`Host::present_pixels`]). Called before
    /// a window's first frame and whenever its renderer changes; a host that shows the two
    /// differently (a browser's GPU and 2D canvases) switches here.
    fn renderer_changed(&mut self, window: WindowId, gpu: bool) {
        let _ = (window, gpu);
    }
    /// Whether the app consumed the pointer event it was just given (an element's handler took
    /// it, such as a context menu opening on a secondary press). Called after every pointer
    /// event; a host whose platform acts on presses itself (a browser's own context menu)
    /// cancels that action only when the app took the press.
    fn pointer_consumed(&mut self, window: WindowId, consumed: bool) {
        let _ = (window, consumed);
    }
    /// Starts, updates (`Some`) or ends (`None`) text input on a window.
    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>);
    /// The clipboard's text.
    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported>;
    /// Puts text on the clipboard; `sensitive` keeps it out of clipboard history and cloud
    /// sync where the platform allows.
    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported>;
    /// Empties the clipboard (a copied secret's time is up).
    fn clear_clipboard(&mut self) -> Result<(), Unsupported>;
    /// The primary selection's text: what was last selected anywhere, pasted with the middle
    /// button (X11 and Wayland). Platforms without one say `Unsupported`.
    fn primary_selection_text(&mut self) -> Result<Option<String>, Unsupported> {
        Err(Unsupported::new("primary_selection_text", "this platform has no primary selection"))
    }
    /// Makes `text` the primary selection (what was just selected), where the platform has
    /// one.
    fn set_primary_selection(&mut self, text: &str) -> Result<(), Unsupported> {
        let _ = text;
        Err(Unsupported::new("set_primary_selection", "this platform has no primary selection"))
    }
    /// The clipboard's HTML (formatted text another app copied), if it holds any.
    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        Err(Unsupported::new("clipboard_html", "this host reads plain text only"))
    }
    /// Puts formatted text on the clipboard: `html` for apps that paste formatting, `text` for
    /// the rest. A host without HTML support puts the text alone.
    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        let _ = html;
        self.set_clipboard_text(text, false)
    }
    /// Sends a window's accessibility tree, or what changed in it, to assistive technology.
    fn update_accessibility(&mut self, window: WindowId, update: accesskit::TreeUpdate);
    /// Shows, updates (`Some`) or removes (`None`) the app's tray icon.
    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported>;
    /// Shows `badge` on the app's taskbar button, Dock tile or launcher icon, or takes it
    /// away (`None`). `app_id` is the app's reverse-domain id, which names its desktop entry
    /// on Linux (`<app_id>.desktop`, where docks look the badge up); empty when the app gave
    /// none, and other platforms ignore it. By default the platform shows no badges.
    fn set_badge(&mut self, app_id: &str, badge: Option<Badge>) -> Result<(), Unsupported> {
        let _ = (app_id, badge);
        Err(Unsupported::new("app badges", "this platform shows no badges"))
    }
    /// How gamepads' held directions repeat and how far their sticks must lean
    /// (`remedy_ui_input::GamepadTuning`). Hosts that read no gamepads (headless, or
    /// platforms whose gamepads arrive as the system's own keys) ignore it.
    fn set_gamepad_tuning(&mut self, tuning: GamepadTuning) {
        let _ = tuning;
    }
    /// Runs a native open or save dialog, modal to `window`. Desktop dialogs finish inside the
    /// call with the chosen paths (empty when cancelled); others return at once and report
    /// the choice with [`HostEvent::FileDialog`].
    fn file_dialog(
        &mut self,
        window: Option<WindowId>,
        dialog: &FileDialog,
    ) -> Result<DialogStarted, Unsupported>;
    /// Asks for the person's leave to use `permission`. Platforms without run-time
    /// permissions answer [`PermissionStarted::Finished`] with true at once; Android shows its
    /// system prompt (when the permission is not already granted) and reports the answer
    /// with [`HostEvent::Permission`].
    fn request_permission(&mut self, permission: Permission) -> Result<PermissionStarted, Unsupported> {
        let _ = permission;
        Ok(PermissionStarted::Finished(true))
    }
    /// Shows a system notification, or replaces the one with the same id. Hosts without
    /// notifications say [`Unsupported`].
    fn show_notification(&mut self, notification: &Notification) -> Result<(), Unsupported> {
        let _ = notification;
        Err(Unsupported::new("system notifications", "this host has none"))
    }
    /// Takes away a notification the app showed (it no longer applies), if still shown.
    fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
        let _ = id;
        Err(Unsupported::new("system notifications", "this host has none"))
    }
    /// Keeps the app running while it is out of sight for `task`, showing its notification
    /// (Android's foreground service); starting again while it runs changes the work and the
    /// notification. Hosts whose apps keep running anyway (desktops, browsers) answer `Ok`
    /// and do nothing. If the platform stops the work later, [`HostEvent::ForegroundStopped`]
    /// says why.
    fn start_foreground(&mut self, task: &ForegroundTask) -> Result<(), Unsupported> {
        let _ = task;
        Ok(())
    }
    /// Ends the work [`Host::start_foreground`] started and takes its notification away.
    fn stop_foreground(&mut self) -> Result<(), Unsupported> {
        Ok(())
    }
    /// Hands `share` to the platform's share sheet, over `window`. Sheets that run inside the
    /// call answer [`ShareStarted::Finished`]; the others return
    /// [`ShareStarted::Pending`] and report with [`HostEvent::Share`]. By default the
    /// platform has no share sheet.
    fn share(&mut self, window: Option<WindowId>, share: &ShareOptions) -> Result<ShareStarted, Unsupported> {
        let _ = (window, share);
        Err(Unsupported::new("sharing", "this platform has no share sheet"))
    }
    /// Prints `html` (a whole page, UTF-8) through the platform's print dialog, over
    /// `window`, under `title` (the job's name). The dialog opens and the call returns;
    /// printing or cancelling happens there. By default the platform cannot print.
    fn print_html(&mut self, window: Option<WindowId>, title: &str, html: &str) -> Result<(), Unsupported> {
        let _ = (window, title, html);
        Err(Unsupported::new("printing", "this platform has no print dialog"))
    }
    /// Starts dragging `data` out of `window` to other apps (call while the pointer button
    /// that began the drag is held). Some platforms run the drag to its end inside the call;
    /// others return at once and report the end with [`HostEvent::DragEnded`].
    fn start_drag(
        &mut self,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported>;
    /// What a drop on `window` would do now, as the app decided for the last
    /// [`DragEvent`]: platforms show it with the cursor and tell the source.
    fn set_drop_effect(&mut self, window: WindowId, effect: DropEffect);
    /// Opens `url` (an `http:`, `https:` or `mailto:` link) in the user's browser or mail app.
    fn open_url(&mut self, url: &str) -> Result<(), Unsupported>;
    /// A one-shot timer at host time `at`.
    fn set_timer(&mut self, at: Duration) -> TimerId;
    /// Cancels a timer that has not fired.
    fn cancel_timer(&mut self, timer: TimerId);
    /// A handle other threads use to wake the loop.
    fn waker(&self) -> Waker;
    /// Ends the event loop after the current event.
    fn exit(&mut self);
}

/// An application driven by a host.
pub trait App {
    /// Handles one event. `host` is how the app acts on the platform.
    fn event(&mut self, host: &mut dyn Host, event: HostEvent);
}

impl<T: App + ?Sized> App for &mut T {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        (**self).event(host, event);
    }
}

#[cfg(test)]
mod safe_area_tests {
    use super::SafeArea;

    #[test]
    fn device_pixels_become_logical_and_never_negative() {
        let a = SafeArea::from_device(96.0, 0.0, 144.0, -3.0, 2.0);
        assert_eq!(a, SafeArea { top: 48.0, right: 0.0, bottom: 72.0, left: 0.0 });
        let bad = SafeArea::from_device(f32::NAN, 10.0, 0.0, 0.0, 0.0);
        assert_eq!(
            bad,
            SafeArea { top: 0.0, right: 10.0, bottom: 0.0, left: 0.0 },
            "a bad scale counts as 1"
        );
    }
}

#[cfg(test)]
mod window_icon_tests {
    use super::WindowIcon;

    #[test]
    fn icons_are_square_rgba_and_scale_by_area() {
        assert!(WindowIcon::new(2, vec![0; 15]).is_none(), "not 2 × 2 pixels");
        assert!(WindowIcon::new(0, Vec::new()).is_none());
        // A 2 × 2 icon: opaque red, half-transparent green (premultiplied), clear, opaque blue.
        let px = [[255, 0, 0, 255], [0, 128, 0, 128], [0, 0, 0, 0], [0, 0, 255, 255]];
        let icon = WindowIcon::new(2, px.concat()).unwrap_or_else(|| unreachable!("2 × 2"));
        assert_eq!(icon.straight_rgba().get(4..8), Some(&[0, 255, 0, 128][..]), "unpremultiplied");
        let one = icon.scaled(1);
        assert_eq!(one.rgba(), &[64, 32, 64, 160], "the area average of all four");
        let four = icon.scaled(4);
        assert_eq!(four.size(), 4);
        assert_eq!(four.rgba().get(..4), Some(&[255, 0, 0, 255][..]), "each pixel covers one source pixel");
        assert_eq!(four.rgba().get(60..64), Some(&[0, 0, 255, 255][..]));
        // Thirds: every output pixel still lands inside the source.
        let odd = WindowIcon::new(3, vec![200; 36]).unwrap_or_else(|| unreachable!("3 × 3")).scaled(2);
        assert!(odd.rgba().iter().all(|&c| c == 200));
    }

    #[test]
    fn drawn_sizes_show_as_drawn_and_pixel_art_keeps_hard_edges() {
        let flat = |size: u32, v: u8| {
            WindowIcon::new(size, vec![v; (size * size * 4) as usize])
                .unwrap_or_else(|| unreachable!("square"))
        };
        let icon = flat(64, 64).with_size(flat(16, 16)).with_size(flat(32, 32));
        assert_eq!(icon.size(), 64, "the largest drawing leads");
        assert_eq!(icon.drawn_sizes(), [16, 32, 64]);
        assert_eq!(icon.drawn().iter().map(WindowIcon::size).collect::<Vec<_>>(), [16, 32, 64]);
        assert!(icon.scaled(16).rgba().iter().all(|&c| c == 16), "a drawn size as it is");
        assert!(icon.scaled(20).rgba().iter().all(|&c| c == 32), "the next larger drawing, scaled");
        assert!(icon.scaled(128).rgba().iter().all(|&c| c == 64), "the largest when none is larger");
        assert!(
            icon.with_size(flat(16, 99)).scaled(16).rgba().iter().all(|&c| c == 99),
            "the later drawing of a size wins"
        );
        // A 2 × 2 checkerboard: smooth scaling blends it, pixel art repeats its pixels.
        let px = [[255, 255, 255, 255], [0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255]];
        let board = WindowIcon::new(2, px.concat()).unwrap_or_else(|| unreachable!("2 × 2"));
        assert_eq!(board.scaled(1).rgba(), &[128, 128, 128, 255]);
        let sharp = board.pixel_art().scaled(4);
        assert!(sharp.is_pixel_art());
        assert_eq!(
            sharp.rgba().get(..8),
            Some(&[255, 255, 255, 255, 255, 255, 255, 255][..]),
            "the top-left pixel twice"
        );
        assert_eq!(sharp.rgba().get(8..12), Some(&[0, 0, 0, 255][..]));
        assert!(sharp.rgba().iter().all(|&c| c == 0 || c == 255), "no blended pixels");
    }

    #[test]
    fn tray_icons_take_their_size_from_the_artwork() {
        let flat = |size: u32, v: u8| {
            WindowIcon::new(size, vec![v; (size * size * 4) as usize])
                .unwrap_or_else(|| unreachable!("square"))
        };
        let plain = super::TrayIcon::new(
            "tip",
            super::TrayImage { size: 32, rgba: vec![7; 32 * 32 * 4] },
            Vec::new(),
        );
        assert_eq!(plain.image_at(16).size, 16);
        assert_eq!(plain.images().len(), 1);
        let art = plain.with_artwork(flat(16, 1).with_size(flat(24, 2)).with_size(flat(32, 3)));
        assert!(art.image_at(24).rgba.iter().all(|&c| c == 2));
        assert_eq!(art.images().iter().map(|i| i.size).collect::<Vec<_>>(), [16, 24, 32]);
    }
}

#[cfg(test)]
mod local_time_tests {
    use super::LocalTime;

    #[test]
    fn local_days_and_midnight_follow_the_offset() {
        // 2026-11-17T23:30:00Z.
        let t = LocalTime { unix_seconds: 1_794_958_200, utc_offset: 0 };
        assert_eq!(t.until_midnight(), 1800);
        let east = LocalTime { utc_offset: 3600, ..t };
        assert_eq!(east.local_days(), t.local_days() + 1, "already tomorrow in UTC+1");
        assert_eq!(east.until_midnight(), 86_400 - 1800);
        let west = LocalTime { utc_offset: -5 * 3600, ..t };
        assert_eq!(west.local_days(), t.local_days());
        let before_1970 = LocalTime { unix_seconds: -1, utc_offset: 0 };
        assert_eq!(before_1970.local_days(), -1);
    }
}

#[cfg(test)]
mod share_tests {
    use super::{ShareFile, ShareOptions};

    #[test]
    fn file_names_become_single_safe_names() {
        let name = |n: &str| ShareFile::new(n, "", Vec::new()).safe_name();
        assert_eq!(name("Lavender.alex-soap.json"), "Lavender.alex-soap.json");
        assert_eq!(name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(name("a\\b:c*?\"<>|\u{7}.txt"), "a_b_c_______.txt");
        assert_eq!(name("notes. . "), "notes", "Windows would drop the trailing dots and spaces");
        assert_eq!(name(""), "file");
        assert_eq!(name(".."), "file");
        assert_eq!(name("  "), "file");
        assert_eq!(name("con.txt"), "_con.txt", "a device name on Windows");
        assert_eq!(name("COM3"), "_COM3");
        assert_eq!(name("Combo.txt"), "Combo.txt");
        assert_eq!(name("Seife \u{1F9FC}.json"), "Seife \u{1F9FC}.json");
    }

    #[test]
    fn a_share_is_labelled_with_its_files_common_type() {
        let f = |t: &str| ShareFile::new("f", t, vec![1, 2]);
        assert_eq!(ShareOptions::new().text("hi").mime_type(), "text/plain");
        assert_eq!(ShareOptions::new().file(f("application/json")).mime_type(), "application/json");
        assert_eq!(ShareOptions::new().file(f("")).mime_type(), "application/octet-stream");
        assert_eq!(ShareOptions::new().files([f("image/png"), f("image/jpeg")]).mime_type(), "image/*");
        assert_eq!(ShareOptions::new().files([f("image/png"), f("text/plain")]).mime_type(), "*/*");
        assert!(ShareOptions::new().is_empty());
        assert!(!ShareOptions::new().title("t").is_empty());
        let debug = format!("{:?}", f("text/plain"));
        assert!(debug.contains("bytes: 2"), "contents are not dumped into logs: {debug}");
    }
}
