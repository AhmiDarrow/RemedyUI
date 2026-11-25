//! What an app can change about how a page view answers the wheel, the keyboard and taps.

use remedy_ui_core::Point;
use remedy_ui_input::{Chord, Key, Modifiers, Platform};

/// How pages follow one another in a [`PageView`](crate::PageView).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageMode {
    /// One long scroll through every page.
    #[default]
    Continuous,
    /// One row of pages (a page, or a spread) at a time: scrolling stays inside it, and a
    /// scroll past its edge, a page key or a swipe turns to the next or the previous one.
    Paged,
}

/// How far one step (a wheel notch, a key press) zooms.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ZoomStep {
    /// Multiplies the zoom: `Scale(1.1)` is 10% of the current zoom per step.
    Scale(f32),
    /// Adds to the zoom: `Add(0.01)` is one percentage point per step.
    Add(f32),
}

impl ZoomStep {
    /// The zoom `steps` steps in from `zoom` (negative: out).
    pub fn apply(self, zoom: f32, steps: f32) -> f32 {
        match self {
            ZoomStep::Scale(f) => zoom * f.max(f32::MIN_POSITIVE).powf(steps),
            ZoomStep::Add(a) => zoom + a * steps,
        }
    }
}

/// How a page view zooms from the wheel and the keyboard ([`PageView::set_input`](crate::PageView::set_input)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageInput {
    /// What one notch of the wheel does with the zoom key held (Ctrl, or Command on Apple
    /// platforms). `None`: the view leaves that wheel alone, for what is around it to answer.
    pub wheel_zoom: Option<ZoomStep>,
    /// The same with Shift held too. `None`: as [`PageInput::wheel_zoom`].
    pub wheel_zoom_shift: Option<ZoomStep>,
    /// What the zoom-in and zoom-out commands step by. `None`: the usual ladder of zooms
    /// (…, 75%, 100%, 125%, 150%, 200%, …).
    pub key_zoom: Option<ZoomStep>,
    /// The zoom the reset command goes to.
    pub reset_zoom: f32,
}

impl Default for PageInput {
    fn default() -> Self {
        PageInput { wheel_zoom: Some(ZoomStep::Scale(1.1)), wheel_zoom_shift: None, key_zoom: None, reset_zoom: 1.0 }
    }
}

/// Something a page view does from the keyboard. Each has an id and default shortcuts, and an
/// app can give it others ([`PageView::rebind`](crate::PageView::rebind)) or run it itself
/// ([`PageView::run`](crate::PageView::run)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PageCommand {
    /// Zooms in a step.
    ZoomIn,
    /// Zooms out a step.
    ZoomOut,
    /// Goes to [`PageInput::reset_zoom`].
    ResetZoom,
    /// Goes to the next page (in paged mode, the next row of pages). No default shortcut.
    NextPage,
    /// Goes to the previous page. No default shortcut.
    PreviousPage,
    /// Goes to the first page.
    FirstPage,
    /// Goes to the last page.
    LastPage,
    /// Scrolls up a line.
    ScrollUp,
    /// Scrolls down a line.
    ScrollDown,
    /// Scrolls left a line.
    ScrollLeft,
    /// Scrolls right a line.
    ScrollRight,
    /// Scrolls up most of a view.
    PageUp,
    /// Scrolls down most of a view.
    PageDown,
    /// Selects the text of the current page.
    SelectPage,
    /// Copies the selected text.
    Copy,
    /// Drops the text selection.
    ClearSelection,
}

impl PageCommand {
    /// Every command.
    pub const ALL: [PageCommand; 16] = [
        PageCommand::ZoomIn,
        PageCommand::ZoomOut,
        PageCommand::ResetZoom,
        PageCommand::NextPage,
        PageCommand::PreviousPage,
        PageCommand::FirstPage,
        PageCommand::LastPage,
        PageCommand::ScrollUp,
        PageCommand::ScrollDown,
        PageCommand::ScrollLeft,
        PageCommand::ScrollRight,
        PageCommand::PageUp,
        PageCommand::PageDown,
        PageCommand::SelectPage,
        PageCommand::Copy,
        PageCommand::ClearSelection,
    ];

    /// A stable id, for an app's own command table and settings.
    pub fn id(self) -> &'static str {
        match self {
            PageCommand::ZoomIn => "pages.zoom-in",
            PageCommand::ZoomOut => "pages.zoom-out",
            PageCommand::ResetZoom => "pages.reset-zoom",
            PageCommand::NextPage => "pages.next-page",
            PageCommand::PreviousPage => "pages.previous-page",
            PageCommand::FirstPage => "pages.first-page",
            PageCommand::LastPage => "pages.last-page",
            PageCommand::ScrollUp => "pages.scroll-up",
            PageCommand::ScrollDown => "pages.scroll-down",
            PageCommand::ScrollLeft => "pages.scroll-left",
            PageCommand::ScrollRight => "pages.scroll-right",
            PageCommand::PageUp => "pages.page-up",
            PageCommand::PageDown => "pages.page-down",
            PageCommand::SelectPage => "pages.select-page",
            PageCommand::Copy => "pages.copy",
            PageCommand::ClearSelection => "pages.clear-selection",
        }
    }

    /// The command with this id.
    pub fn from_id(id: &str) -> Option<PageCommand> {
        PageCommand::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The shortcuts the command has until it is rebound.
    pub fn default_chords(self, platform: Platform) -> Vec<Chord> {
        let cmd = Modifiers::primary(platform);
        let plain = |key: Key| Chord::new(Modifiers::NONE, key);
        let with = |c: char| Chord::new(cmd, Key::char(c));
        match self {
            // "+" is Shift+"=" on most keyboards and its own key on the number pad.
            PageCommand::ZoomIn => vec![with('='), with('+'), Chord::new(cmd.with_shift(), Key::char('+'))],
            PageCommand::ZoomOut => vec![with('-'), Chord::new(cmd.with_shift(), Key::char('_'))],
            PageCommand::ResetZoom => vec![with('0')],
            PageCommand::NextPage | PageCommand::PreviousPage => Vec::new(),
            PageCommand::FirstPage => vec![plain(Key::Home)],
            PageCommand::LastPage => vec![plain(Key::End)],
            PageCommand::ScrollUp => vec![plain(Key::ArrowUp)],
            PageCommand::ScrollDown => vec![plain(Key::ArrowDown)],
            PageCommand::ScrollLeft => vec![plain(Key::ArrowLeft)],
            PageCommand::ScrollRight => vec![plain(Key::ArrowRight)],
            PageCommand::PageUp => vec![plain(Key::PageUp), Chord::new(Modifiers::SHIFT, Key::Space)],
            PageCommand::PageDown => vec![plain(Key::PageDown), plain(Key::Space)],
            PageCommand::SelectPage => vec![with('a')],
            PageCommand::Copy => vec![with('c')],
            PageCommand::ClearSelection => vec![plain(Key::Escape)],
        }
    }
}

/// Where a tap, a double tap or a long press landed in a page view.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageHit {
    /// The place in the view, in its own logical pixels.
    pub at: Point,
    /// The page under it and the place on that page, in the page's points; `None` between
    /// and around pages.
    pub page: Option<(usize, Point)>,
}

/// A quick sideways flick of one finger across a page view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Swipe {
    /// Towards the left: in paged mode, on to the next page.
    Left,
    /// Towards the right: in paged mode, back to the previous page.
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_scale_or_add_and_ids_round_trip() {
        assert!((ZoomStep::Add(0.01).apply(1.0, 3.0) - 1.03).abs() < 1e-6);
        assert!((ZoomStep::Scale(1.1).apply(1.0, -1.0) - 1.0 / 1.1).abs() < 1e-6);
        for c in PageCommand::ALL {
            assert_eq!(PageCommand::from_id(c.id()), Some(c));
        }
        let apple = PageCommand::Copy.default_chords(Platform::Apple);
        assert_eq!(apple, vec![Chord::new(Modifiers::META, Key::char('c'))]);
    }
}
