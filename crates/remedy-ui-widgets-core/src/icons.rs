//! Icons (vector, drawn as strokes on a 24-unit grid), images, avatars and canvases.

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Point;
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_view::{El, PaintCx, Ui, words};

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
    /// A heavy B: bold text.
    Bold,
    /// A slanted I: italic text.
    Italic,
    /// An S with a line through it: struck-through text.
    Strike,
    /// Angle brackets: code.
    Code,
    /// Two chain links: a link.
    Link,
    /// Dots and lines: a bulleted list.
    ListBullet,
    /// Numbered lines: a numbered list.
    ListNumber,
    /// Boxes and lines: a task list.
    ListTask,
    /// A quotation mark and a bar: a quote.
    Quote,
    /// A box with angle brackets: a block of code.
    CodeBlock,
    /// An H: a heading.
    Heading,
    /// An at sign: mention someone.
    At,
    /// A triangle pointing right: play.
    Play,
    /// Two bars: pause.
    Pause,
    /// A speaker with sound waves: volume.
    Volume,
    /// A speaker crossed out: muted.
    VolumeMuted,
    /// Four corners pointing out: fill the screen.
    Fullscreen,
    /// Four corners pointing in: leave full screen.
    FullscreenExit,
    /// A box with lines of text: captions and subtitles.
    Captions,
    /// A house: the home screen or start page.
    Home,
    /// A framed picture with a mountain and a sun: an image or photo.
    Image,
    /// A dot: the mark of a chosen radio item in a menu.
    Dot,
}

impl Icon {
    /// What the icon shows, in English, for when the icon alone stands for something ("Home"
    /// on a toolbar button, "Image" for a file's kind). Screen readers read it where the
    /// icon carries meaning; [`icon`] itself is decorative, so whatever holds the icon names
    /// it (an [`icon_button`](crate::icon_button)'s label, or this name translated, as
    /// [`Icon::accessible_name_in`] gives it).
    pub fn accessible_name(self) -> &'static str {
        match self {
            Icon::Plus => words::en("Add"),
            Icon::Minus => words::en("Remove"),
            Icon::Maximize => words::en("Maximize"),
            Icon::Restore => words::en("Restore"),
            Icon::Close => words::en("Close"),
            Icon::Check => words::en("Done"),
            Icon::Lock => words::en("Locked"),
            Icon::Unlock => words::en("Unlocked"),
            Icon::Pin => words::en("Pin"),
            Icon::Trash => words::en("Delete"),
            Icon::Copy => words::en("Copy"),
            Icon::Eye => words::en("Show"),
            Icon::EyeOff => words::en("Hide"),
            Icon::ChevronDown => words::en("Expand"),
            Icon::ChevronRight => words::en("Next"),
            Icon::ChevronLeft => words::en("Previous"),
            Icon::ChevronUp => words::en("Collapse"),
            Icon::Menu => words::en("Menu"),
            Icon::Search => words::en("Search"),
            Icon::Info => words::en("Information"),
            Icon::Alert => words::en("Warning"),
            Icon::Clock => words::en("Time"),
            Icon::Key => words::en("Key"),
            Icon::Settings => words::en("Settings"),
            Icon::Note => words::en("Note"),
            Icon::External => words::en("Open outside the app"),
            Icon::Refresh => words::en("Refresh"),
            Icon::Folder => words::en("Folder"),
            Icon::File => words::en("File"),
            Icon::Download => words::en("Download"),
            Icon::Upload => words::en("Upload"),
            Icon::Send => words::en("Send"),
            Icon::ArrowLeft => words::en("Back"),
            Icon::ArrowRight => words::en("Forward"),
            Icon::ArrowUp => words::en("Up"),
            Icon::Users => words::en("People"),
            Icon::Monitor => words::en("Computer"),
            Icon::Bold => words::en("Bold"),
            Icon::Italic => words::en("Italic"),
            Icon::Strike => words::en("Strikethrough"),
            Icon::Code => words::en("Code"),
            Icon::Link => words::en("Link"),
            Icon::ListBullet => words::en("Bulleted list"),
            Icon::ListNumber => words::en("Numbered list"),
            Icon::ListTask => words::en("Task list"),
            Icon::Quote => words::en("Quote"),
            Icon::CodeBlock => words::en("Code block"),
            Icon::Heading => words::en("Heading"),
            Icon::At => words::en("Mention"),
            Icon::Play => words::en("Play"),
            Icon::Pause => words::en("Pause"),
            Icon::Volume => words::en("Volume"),
            Icon::VolumeMuted => words::en("Muted"),
            Icon::Fullscreen => words::en("Full screen"),
            Icon::FullscreenExit => words::en("Exit full screen"),
            Icon::Captions => words::en("Captions"),
            Icon::Home => words::en("Home"),
            Icon::Image => words::en("Image"),
            Icon::Dot => words::en("Selected"),
        }
    }

    /// [`Icon::accessible_name`] in the window's locale ([`Ui::word`]).
    pub fn accessible_name_in(self, ui: &Ui) -> String {
        ui.word(self.accessible_name())
    }
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
        Icon::Bold => {
            // A B drawn twice, a little apart, so its strokes read heavy.
            for dx in [0.0, 1.2] {
                s.move_to(p(7.0 + dx, 4.0)).line_to(p(7.0 + dx, 20.0));
                arc(&mut s, p(12.5 + dx, 8.0), 4.0, -PI * 0.5, PI * 0.5);
                arc(&mut s, p(13.0 + dx, 16.0), 4.0, -PI * 0.5, PI * 0.5);
                s.move_to(p(7.0 + dx, 4.0)).line_to(p(12.5 + dx, 4.0));
                s.move_to(p(7.0 + dx, 12.0)).line_to(p(13.0 + dx, 12.0));
                s.move_to(p(7.0 + dx, 20.0)).line_to(p(13.0 + dx, 20.0));
            }
        }
        Icon::Italic => {
            s.move_to(p(10.0, 4.0)).line_to(p(18.0, 4.0));
            s.move_to(p(6.0, 20.0)).line_to(p(14.0, 20.0));
            s.move_to(p(14.0, 4.0)).line_to(p(10.0, 20.0));
        }
        Icon::Strike => {
            arc(&mut s, p(12.0, 8.0), 4.5, PI * 0.15, PI * 1.5);
            arc(&mut s, p(12.0, 16.0), 4.5, -PI * 0.5, PI * 0.85);
            s.move_to(p(4.0, 12.0)).line_to(p(20.0, 12.0));
        }
        Icon::Code => {
            s.move_to(p(8.0, 6.0)).line_to(p(3.0, 12.0)).line_to(p(8.0, 18.0));
            s.move_to(p(16.0, 6.0)).line_to(p(21.0, 12.0)).line_to(p(16.0, 18.0));
        }
        Icon::Link => {
            arc(&mut s, p(8.0, 12.0), 4.0, PI * 0.5, PI * 1.5);
            s.move_to(p(8.0, 8.0)).line_to(p(11.0, 8.0)).move_to(p(8.0, 16.0)).line_to(p(11.0, 16.0));
            arc(&mut s, p(16.0, 12.0), 4.0, -PI * 0.5, PI * 0.5);
            s.move_to(p(13.0, 8.0)).line_to(p(16.0, 8.0)).move_to(p(13.0, 16.0)).line_to(p(16.0, 16.0));
            s.move_to(p(9.0, 12.0)).line_to(p(15.0, 12.0));
        }
        Icon::ListBullet => {
            for y in [6.0, 12.0, 18.0] {
                circle(&mut s, p(5.0, y), 1.0);
                s.move_to(p(9.0, y)).line_to(p(20.0, y));
            }
        }
        Icon::ListNumber => {
            s.move_to(p(4.0, 4.5)).line_to(p(5.5, 3.5)).line_to(p(5.5, 8.5));
            s.move_to(p(4.0, 14.0)).line_to(p(6.5, 14.0)).line_to(p(4.0, 19.0)).line_to(p(7.0, 19.0));
            for y in [6.0, 12.0, 18.0] {
                s.move_to(p(10.0, y)).line_to(p(20.0, y));
            }
        }
        Icon::ListTask => {
            rect(&mut s, 3.0, 4.0, 8.0, 9.0);
            s.move_to(p(4.0, 6.5)).line_to(p(5.5, 8.0)).line_to(p(8.5, 3.5));
            rect(&mut s, 3.0, 15.0, 8.0, 20.0);
            s.move_to(p(11.0, 6.5)).line_to(p(20.0, 6.5)).move_to(p(11.0, 17.5)).line_to(p(20.0, 17.5));
        }
        Icon::Quote => {
            s.move_to(p(4.0, 4.0)).line_to(p(4.0, 20.0));
            s.move_to(p(9.0, 7.0)).line_to(p(20.0, 7.0));
            s.move_to(p(9.0, 12.0)).line_to(p(20.0, 12.0));
            s.move_to(p(9.0, 17.0)).line_to(p(16.0, 17.0));
        }
        Icon::CodeBlock => {
            rect(&mut s, 3.0, 4.0, 21.0, 20.0);
            s.move_to(p(10.0, 9.0)).line_to(p(7.0, 12.0)).line_to(p(10.0, 15.0));
            s.move_to(p(14.0, 9.0)).line_to(p(17.0, 12.0)).line_to(p(14.0, 15.0));
        }
        Icon::Heading => {
            s.move_to(p(6.0, 4.0)).line_to(p(6.0, 20.0)).move_to(p(18.0, 4.0)).line_to(p(18.0, 20.0));
            s.move_to(p(6.0, 12.0)).line_to(p(18.0, 12.0));
        }
        Icon::At => {
            circle(&mut s, p(12.0, 12.0), 3.5);
            arc(&mut s, p(12.0, 12.0), 8.5, 0.0, PI * 1.8);
            s.move_to(p(15.5, 9.0)).line_to(p(15.5, 13.5));
            arc(&mut s, p(18.0, 13.5), 2.5, PI * 0.0, PI);
        }
        Icon::Play => {
            s.move_to(p(7.0, 4.5)).line_to(p(19.0, 12.0)).line_to(p(7.0, 19.5)).close();
        }
        Icon::Pause => {
            rect(&mut s, 6.0, 5.0, 10.0, 19.0);
            rect(&mut s, 14.0, 5.0, 18.0, 19.0);
        }
        Icon::Volume | Icon::VolumeMuted => {
            s.move_to(p(3.0, 9.0))
                .line_to(p(7.0, 9.0))
                .line_to(p(12.0, 4.5))
                .line_to(p(12.0, 19.5))
                .line_to(p(7.0, 15.0))
                .line_to(p(3.0, 15.0))
                .close();
            if icon == Icon::Volume {
                arc(&mut s, p(12.0, 12.0), 4.0, -PI * 0.3, PI * 0.3);
                arc(&mut s, p(12.0, 12.0), 8.0, -PI * 0.3, PI * 0.3);
            } else {
                s.move_to(p(15.5, 9.0)).line_to(p(21.5, 15.0)).move_to(p(21.5, 9.0)).line_to(p(15.5, 15.0));
            }
        }
        Icon::Fullscreen => {
            s.move_to(p(4.0, 9.0)).line_to(p(4.0, 4.0)).line_to(p(9.0, 4.0));
            s.move_to(p(15.0, 4.0)).line_to(p(20.0, 4.0)).line_to(p(20.0, 9.0));
            s.move_to(p(20.0, 15.0)).line_to(p(20.0, 20.0)).line_to(p(15.0, 20.0));
            s.move_to(p(9.0, 20.0)).line_to(p(4.0, 20.0)).line_to(p(4.0, 15.0));
        }
        Icon::FullscreenExit => {
            s.move_to(p(4.0, 9.0)).line_to(p(9.0, 9.0)).line_to(p(9.0, 4.0));
            s.move_to(p(15.0, 4.0)).line_to(p(15.0, 9.0)).line_to(p(20.0, 9.0));
            s.move_to(p(20.0, 15.0)).line_to(p(15.0, 15.0)).line_to(p(15.0, 20.0));
            s.move_to(p(9.0, 20.0)).line_to(p(9.0, 15.0)).line_to(p(4.0, 15.0));
        }
        Icon::Dot => {
            // Rings drawn close together read as a filled dot at menu sizes.
            for r in [1.0, 2.0, 3.0, 4.0] {
                circle(&mut s, p(12.0, 12.0), r);
            }
        }
        Icon::Captions => {
            rect(&mut s, 3.0, 5.0, 21.0, 19.0);
            s.move_to(p(6.5, 11.0)).line_to(p(11.0, 11.0)).move_to(p(13.0, 11.0)).line_to(p(17.5, 11.0));
            s.move_to(p(6.5, 15.0)).line_to(p(14.0, 15.0));
        }
        Icon::Home => {
            // A roof over a body with a door, on the grid the other outlines use.
            s.move_to(p(3.0, 11.0)).line_to(p(12.0, 3.5)).line_to(p(21.0, 11.0));
            s.move_to(p(5.5, 9.0)).line_to(p(5.5, 20.0)).line_to(p(18.5, 20.0)).line_to(p(18.5, 9.0));
            s.move_to(p(10.0, 20.0)).line_to(p(10.0, 14.0)).line_to(p(14.0, 14.0)).line_to(p(14.0, 20.0));
        }
        Icon::Image => {
            rect(&mut s, 3.0, 4.0, 21.0, 20.0);
            circle(&mut s, p(8.5, 9.5), 1.75);
            s.move_to(p(3.0, 17.0)).line_to(p(9.0, 11.5)).line_to(p(14.0, 16.0));
            s.move_to(p(12.0, 14.0)).line_to(p(15.5, 11.0)).line_to(p(21.0, 16.0));
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

/// [`image`] for pixel art: scaled up by repeating pixels, never blurred.
#[track_caller]
pub fn pixel_image(ui: &Ui, image: remedy_ui_paint::Image, alt: impl Into<String>) -> El {
    ui.el("image")
        .image_sampled(image, remedy_ui_paint::Sampling::Nearest)
        .semantics(Semantics::new(Role::Image).label(alt.into()))
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
        .style(remedy_ui_style::StylePatch::default().background(color).radius(size / 2.0))
        .child(
            ui.el("avatar_initials").text(remedy_ui_view::RichText::plain(initials)).nowrap().style(
                remedy_ui_style::StylePatch::default().foreground(fg).weight(700.0).text_size(size * 0.4),
            ),
        )
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
        let more = [
            Icon::ArrowUp,
            Icon::Users,
            Icon::Monitor,
            Icon::Bold,
            Icon::Italic,
            Icon::Strike,
            Icon::Code,
            Icon::Link,
            Icon::ListBullet,
            Icon::ListNumber,
            Icon::ListTask,
            Icon::Quote,
            Icon::CodeBlock,
            Icon::Heading,
            Icon::At,
            Icon::Play,
            Icon::Pause,
            Icon::Volume,
            Icon::VolumeMuted,
            Icon::Fullscreen,
            Icon::FullscreenExit,
            Icon::Captions,
            Icon::Home,
            Icon::Image,
            Icon::Dot,
        ];
        let mut names = Vec::new();
        for icon in ALL.into_iter().chain(more) {
            let name = icon.accessible_name();
            assert!(!name.is_empty() && !names.contains(&name), "{icon:?} needs a name of its own");
            names.push(name);
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
