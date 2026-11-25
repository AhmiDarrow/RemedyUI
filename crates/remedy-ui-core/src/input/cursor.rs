//! The mouse pointer's shape.

/// The shape of the mouse pointer over an element: the standard set every desktop and the
/// web share (the names are CSS's). Touch platforms show none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Cursor {
    /// The platform's arrow.
    #[default]
    Default,
    /// A hand: a link.
    Pointer,
    /// An I-beam: text that can be selected or typed in.
    Text,
    /// An I-beam on its side, for vertical text.
    VerticalText,
    /// A cross, for picking a point or a region.
    Crosshair,
    /// Something that can be moved.
    Move,
    /// An open hand: something that can be grabbed and dragged.
    Grab,
    /// A closed hand: being dragged.
    Grabbing,
    /// The action will not be carried out.
    NotAllowed,
    /// What is dragged cannot be dropped here.
    NoDrop,
    /// Busy: nothing can be done until it is over.
    Wait,
    /// Busy in the background: the app still answers.
    Progress,
    /// Help is available.
    Help,
    /// A menu is available.
    ContextMenu,
    /// A table cell or cells can be selected.
    Cell,
    /// A link or shortcut will be made.
    Alias,
    /// A copy will be made.
    Copy,
    /// Panning in every direction.
    AllScroll,
    /// A press zooms in.
    ZoomIn,
    /// A press zooms out.
    ZoomOut,
    /// A column, or a divider between side-by-side panes, is resized.
    ColResize,
    /// A row, or a divider between stacked panes, is resized.
    RowResize,
    /// An edge moves left and right.
    EwResize,
    /// An edge moves up and down.
    NsResize,
    /// A corner moves along the rising diagonal.
    NeswResize,
    /// A corner moves along the falling diagonal.
    NwseResize,
    /// The top edge moves.
    NResize,
    /// The right edge moves.
    EResize,
    /// The bottom edge moves.
    SResize,
    /// The left edge moves.
    WResize,
    /// The top right corner moves.
    NeResize,
    /// The top left corner moves.
    NwResize,
    /// The bottom right corner moves.
    SeResize,
    /// The bottom left corner moves.
    SwResize,
}

impl Cursor {
    /// Every shape.
    pub const ALL: [Cursor; 34] = [
        Cursor::Default,
        Cursor::Pointer,
        Cursor::Text,
        Cursor::VerticalText,
        Cursor::Crosshair,
        Cursor::Move,
        Cursor::Grab,
        Cursor::Grabbing,
        Cursor::NotAllowed,
        Cursor::NoDrop,
        Cursor::Wait,
        Cursor::Progress,
        Cursor::Help,
        Cursor::ContextMenu,
        Cursor::Cell,
        Cursor::Alias,
        Cursor::Copy,
        Cursor::AllScroll,
        Cursor::ZoomIn,
        Cursor::ZoomOut,
        Cursor::ColResize,
        Cursor::RowResize,
        Cursor::EwResize,
        Cursor::NsResize,
        Cursor::NeswResize,
        Cursor::NwseResize,
        Cursor::NResize,
        Cursor::EResize,
        Cursor::SResize,
        Cursor::WResize,
        Cursor::NeResize,
        Cursor::NwResize,
        Cursor::SeResize,
        Cursor::SwResize,
    ];

    /// Its CSS name (`not-allowed`, `ew-resize`).
    pub fn css_name(self) -> &'static str {
        match self {
            Cursor::Default => "default",
            Cursor::Pointer => "pointer",
            Cursor::Text => "text",
            Cursor::VerticalText => "vertical-text",
            Cursor::Crosshair => "crosshair",
            Cursor::Move => "move",
            Cursor::Grab => "grab",
            Cursor::Grabbing => "grabbing",
            Cursor::NotAllowed => "not-allowed",
            Cursor::NoDrop => "no-drop",
            Cursor::Wait => "wait",
            Cursor::Progress => "progress",
            Cursor::Help => "help",
            Cursor::ContextMenu => "context-menu",
            Cursor::Cell => "cell",
            Cursor::Alias => "alias",
            Cursor::Copy => "copy",
            Cursor::AllScroll => "all-scroll",
            Cursor::ZoomIn => "zoom-in",
            Cursor::ZoomOut => "zoom-out",
            Cursor::ColResize => "col-resize",
            Cursor::RowResize => "row-resize",
            Cursor::EwResize => "ew-resize",
            Cursor::NsResize => "ns-resize",
            Cursor::NeswResize => "nesw-resize",
            Cursor::NwseResize => "nwse-resize",
            Cursor::NResize => "n-resize",
            Cursor::EResize => "e-resize",
            Cursor::SResize => "s-resize",
            Cursor::WResize => "w-resize",
            Cursor::NeResize => "ne-resize",
            Cursor::NwResize => "nw-resize",
            Cursor::SeResize => "se-resize",
            Cursor::SwResize => "sw-resize",
        }
    }

    /// The shape with this CSS name.
    pub fn from_css_name(name: &str) -> Option<Cursor> {
        Cursor::ALL.into_iter().find(|c| c.css_name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_has_its_own_css_name() {
        let mut names: Vec<&str> = Cursor::ALL.iter().map(|c| c.css_name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Cursor::ALL.len());
        assert_eq!(Cursor::from_css_name("nwse-resize"), Some(Cursor::NwseResize));
        assert_eq!(Cursor::default().css_name(), "default");
    }
}
