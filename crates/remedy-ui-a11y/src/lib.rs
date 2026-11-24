//! RemedyUI accessibility.
//!
//! Elements declare [`Semantics`]; an [`A11yTree`] keeps them and produces AccessKit tree
//! updates for the platform adapters (UI Automation, NSAccessibility, AT-SPI, Android) and for
//! consenting agents. Nothing is built until a client connects; after that, only changed
//! nodes are sent. See `docs/design/11-accessibility-and-automation.md`.
//!
//! - **Merging.** A compound widget (a chat row: avatar, name, time, text) can merge its
//!   descendants into one node with a composed label, so a screen reader moves row by row.
//! - **Protected content.** Password inputs report their role and label but never their text.
//! - **Live regions.** New messages and status changes are announced politely or assertively.
//!
//! ```
//! use remedy_ui_a11y::*;
//!
//! let mut tree = A11yTree::new(1);
//! tree.set(1, Semantics::new(Role::Window).label("FreeSpeak"));
//! tree.set(2, Semantics::new(Role::Button).label("Send").action(Action::Click));
//! tree.set_children(1, &[2]);
//! assert!(tree.take_update().is_none(), "nothing is built until a client connects");
//! tree.connect();
//! let update = tree.take_update().map(|u| u.nodes.len());
//! assert_eq!(update, Some(2));
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};

use accesskit::{Node, NodeId, Rect as AkRect, TreeId, TreeInfo, TreeUpdate};
use remedy_ui_core::Rect;

/// AccessKit's tree update, which platform adapters consume.
pub use accesskit::TreeUpdate as AccessKitUpdate;

/// What an element is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// A window.
    Window,
    /// A group of related elements.
    Group,
    /// A button.
    Button,
    /// A check box.
    Check,
    /// An on/off switch.
    Switch,
    /// One of a set of choices.
    Radio,
    /// A slider.
    Slider,
    /// A text input.
    TextInput,
    /// A text input for search terms (ARIA `searchbox`): a field with
    /// `InputPurpose::Search`.
    SearchInput,
    /// A text input with several lines.
    MultilineTextInput,
    /// A password input: its text is never exposed.
    PasswordInput,
    /// A link.
    Link,
    /// A list.
    List,
    /// An item in a list.
    ListItem,
    /// A list of choices to pick from, one or (when
    /// [`multiselectable`](Semantics::multiselectable)) several, as ARIA's `listbox`: a theme
    /// picker, a font list. Its choices are [`Role::Option`]s, each [`selected`](Semantics::selected)
    /// or not.
    ListBox,
    /// One choice in a [`Role::ListBox`], as ARIA's `option`.
    Option,
    /// A tab.
    Tab,
    /// A row of tabs.
    TabList,
    /// A menu.
    Menu,
    /// An item in a menu.
    MenuItem,
    /// A row of menus along the top of a window (File, Edit, View).
    MenuBar,
    /// A row of buttons for one thing's commands (a message's Reply, Copy, Delete; an
    /// editor's Bold, Italic). Screen readers say "toolbar" and its label as they enter it.
    Toolbar,
    /// A menu item that is checked or not (View ▸ Full Screen).
    MenuItemCheckBox,
    /// A menu item that is one of a set of choices, one of them checked (View ▸ Fit Width).
    MenuItemRadio,
    /// A line dividing groups of items (in a menu, a toolbar or a list).
    Separator,
    /// A dialog.
    Dialog,
    /// An urgent message.
    Alert,
    /// A status message.
    Status,
    /// A heading, with its level (1 to 6).
    Heading(u8),
    /// An image.
    Image,
    /// A progress indicator.
    Progress,
    /// Plain text.
    Label,
    /// A number field stepped with arrows.
    SpinButton,
    /// A drop-down choice (ARIA's `combobox`, as an HTML `<select>`): it shows the chosen
    /// option as its [`value`](Semantics::value) and opens a list of the others, saying
    /// whether it is [`expanded`](Semantics::expanded).
    ComboBox,
    /// A color chooser.
    ColorWell,
    /// A date chooser.
    DateInput,
    /// A time chooser.
    TimeInput,
    /// A grid of cells (a month calendar).
    Grid,
    /// One cell in a grid.
    GridCell,
    /// A form: fields submitted together.
    Form,
    /// A table of rows and columns.
    Table,
    /// A row of a table.
    Row,
    /// A cell of a table row.
    Cell,
    /// A column heading of a table.
    ColumnHeader,
    /// A tree of items (folders).
    Tree,
    /// An item of a tree.
    TreeItem,
    /// A document read page by page (a PDF, a book).
    Document,
    /// One page of a document.
    Page,
    /// A landmark: the main ways around the app or site (a sidebar of sections).
    Navigation,
    /// A landmark: the main content.
    Main,
    /// A landmark: the header across the top (logo, title, account).
    Banner,
    /// A landmark: a labelled section people should be able to jump to.
    Region,
    /// A landmark: the search area.
    Search,
    /// A self-contained piece of content (a post, a card in a feed).
    Article,
    /// A landmark: the footer (copyright, links).
    ContentInfo,
    /// A landmark: content beside the main content (related items).
    Complementary,
}

/// Something an assistive technology or agent can ask an element to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Activate it (click, press).
    Click,
    /// Move focus to it.
    Focus,
    /// Increase its value.
    Increment,
    /// Decrease its value.
    Decrement,
    /// Expand it.
    Expand,
    /// Collapse it.
    Collapse,
    /// Show its context menu.
    ShowContextMenu,
    /// Scroll it into view.
    ScrollIntoView,
    /// Replace its text (a text field; the new text comes with the request).
    SetValue,
}

impl Action {
    /// The action's stable name, as agents and logs see it (`click`, `show_context_menu`).
    ///
    /// ```
    /// use remedy_ui_a11y::Action;
    ///
    /// assert_eq!(Action::ShowContextMenu.name(), "show_context_menu");
    /// ```
    pub fn name(self) -> &'static str {
        match self {
            Action::Click => "click",
            Action::Focus => "focus",
            Action::Increment => "increment",
            Action::Decrement => "decrement",
            Action::Expand => "expand",
            Action::Collapse => "collapse",
            Action::ShowContextMenu => "show_context_menu",
            Action::ScrollIntoView => "scroll_into_view",
            Action::SetValue => "set_value",
        }
    }
}

/// How changes to an element are announced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Live {
    /// Not announced.
    #[default]
    Off,
    /// Announced when the user is idle (new chat messages).
    Polite,
    /// Announced at once (call dropped, errors).
    Assertive,
}

/// Which item of a set is the current one (`aria-current`): the page a nav item opens, the step
/// of a wizard, the day in a calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Current {
    /// The current page (a nav item for the page shown).
    Page,
    /// The current step of a process.
    Step,
    /// The current location (a node of a flow chart, a breadcrumb).
    Location,
    /// The current date (today in a calendar).
    Date,
    /// The current time (now in a timetable).
    Time,
    /// The current item of a set that is none of the above.
    True,
}

/// What kind of popup an element opens (ARIA `aria-haspopup`): screen readers say so on a
/// menu button or a button that opens a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Popup {
    /// A menu.
    Menu,
    /// A list box of options.
    ListBox,
    /// A tree.
    Tree,
    /// A grid.
    Grid,
    /// A dialog.
    Dialog,
}

/// What an element tells assistive technology about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Semantics {
    role: Role,
    label: Option<String>,
    description: Option<String>,
    value: Option<String>,
    numeric: Option<(f64, f64, f64)>,
    checked: Option<bool>,
    selected: Option<bool>,
    multiselectable: bool,
    modal: bool,
    has_popup: Option<Popup>,
    expanded: Option<bool>,
    invalid: bool,
    disabled: bool,
    read_only: bool,
    hidden: bool,
    live: Live,
    actions: Vec<Action>,
    bounds: Option<Rect>,
    set_position: Option<(usize, usize)>,
    level: Option<usize>,
    merge: bool,
    shortcut: Option<String>,
    access_key: Option<String>,
    current: Option<Current>,
    url: Option<String>,
}

impl Semantics {
    /// The same semantics with its name and description passed through `f` (what is said,
    /// not the value, which is data): pseudo-locales rewrite what readers hear as they
    /// rewrite what is drawn.
    pub fn map_text(mut self, f: impl Fn(&str) -> String) -> Self {
        self.label = self.label.map(|l| f(&l));
        self.description = self.description.map(|d| f(&d));
        self
    }

    /// Semantics with a role and nothing else.
    pub fn new(role: Role) -> Self {
        Semantics {
            role,
            label: None,
            description: None,
            value: None,
            numeric: None,
            checked: None,
            selected: None,
            multiselectable: false,
            modal: false,
            has_popup: None,
            expanded: None,
            invalid: false,
            disabled: false,
            read_only: false,
            hidden: false,
            live: Live::Off,
            actions: Vec::new(),
            bounds: None,
            set_position: None,
            level: None,
            merge: false,
            shortcut: None,
            access_key: None,
            current: None,
            url: None,
        }
    }

    /// The role.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The accessible name, if set.
    pub fn name(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The longer description, if set.
    pub fn description_text(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The value text, if set.
    pub fn value_text(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// The actions offered.
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// True when hidden from assistive technology.
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// True when marked invalid.
    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// Checked, unchecked, or not something that can be checked (`None`).
    pub fn checked_state(&self) -> Option<bool> {
        self.checked
    }

    /// Selected, not selected, or not something that can be selected (`None`).
    pub fn selected_state(&self) -> Option<bool> {
        self.selected
    }

    /// Expanded, collapsed, or not something that expands (`None`).
    pub fn expanded_state(&self) -> Option<bool> {
        self.expanded
    }

    /// The live region politeness: how changes to it are announced.
    pub fn live_state(&self) -> Live {
        self.live
    }

    /// Which kind of current item it is, or not the current one (`None`).
    pub fn current_state(&self) -> Option<Current> {
        self.current
    }

    /// Marks it the current item of its set: `Current::Page` for the nav item of the page shown.
    pub fn current(mut self, c: Current) -> Self {
        self.current = Some(c);
        self
    }

    /// The accessible name.
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.label = Some(s.into());
        self
    }

    /// A longer description.
    pub fn description(mut self, s: impl Into<String>) -> Self {
        self.description = Some(s.into());
        self
    }

    /// A text value (a text input's contents). Ignored for password inputs.
    pub fn value(mut self, s: impl Into<String>) -> Self {
        self.value = Some(s.into());
        self
    }

    /// A numeric value with its range (sliders, progress).
    pub fn numeric(mut self, value: f64, min: f64, max: f64) -> Self {
        self.numeric = Some((value, min, max));
        self
    }

    /// Checked or on.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    /// Selected.
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = Some(on);
        self
    }

    /// More than one of its choices may be selected at once (a [`Role::ListBox`], a
    /// [`Role::Grid`], a [`Role::Tree`]), as ARIA's `aria-multiselectable`.
    pub fn multiselectable(mut self, on: bool) -> Self {
        self.multiselectable = on;
        self
    }

    /// True when more than one of its choices may be selected at once.
    pub fn is_multiselectable(&self) -> bool {
        self.multiselectable
    }

    /// A modal dialog or overlay (ARIA `aria-modal`): while it shows, everything outside it is
    /// inert to assistive technology (hidden from it, and in a page not focusable or
    /// clickable), as the rest of a page is behind a modal `<dialog>`.
    pub fn modal(mut self, on: bool) -> Self {
        self.modal = on;
        self
    }

    /// True for a modal dialog or overlay.
    pub fn is_modal(&self) -> bool {
        self.modal
    }

    /// Opens a popup of this kind (ARIA `aria-haspopup`): a menu button, a button opening a
    /// dialog.
    pub fn has_popup(mut self, popup: Popup) -> Self {
        self.has_popup = Some(popup);
        self
    }

    /// The kind of popup it opens, if any.
    pub fn popup(&self) -> Option<Popup> {
        self.has_popup
    }

    /// Expanded or collapsed.
    pub fn expanded(mut self, on: bool) -> Self {
        self.expanded = Some(on);
        self
    }

    /// The value is not acceptable (a form field failing validation).
    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
        self
    }

    /// Disabled.
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }

    /// Read-only.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Hidden from assistive technology (under a lock screen, decorative).
    pub fn hidden(mut self, on: bool) -> Self {
        self.hidden = on;
        self
    }

    /// How changes are announced.
    pub fn live(mut self, live: Live) -> Self {
        self.live = live;
        self
    }

    /// Adds an action.
    pub fn action(mut self, a: Action) -> Self {
        self.actions.push(a);
        self
    }

    /// Bounds in window coordinates.
    pub fn bounds(mut self, r: Rect) -> Self {
        self.bounds = Some(r);
        self
    }

    /// The depth of a tree item (1 for a top-level item).
    pub fn level(mut self, level: usize) -> Self {
        self.level = Some(level);
        self
    }

    /// Position in a set, 1-based, and the set's size: "item 5,032 of 10,000" even when
    /// most items of a virtual list are not mounted.
    pub fn set_position(mut self, position: usize, size: usize) -> Self {
        self.set_position = Some((position, size));
        self
    }

    /// Merges descendants into this node: their labels and values join this one's label,
    /// and they are not exposed on their own.
    pub fn merge_descendants(mut self) -> Self {
        self.merge = true;
        self
    }

    /// The keyboard shortcut that does the same from anywhere in the window, as the person
    /// reads it ("Ctrl+S", "Strg+S", "⌘S"): menus show it beside an item, and screen readers
    /// say it after the name.
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into()).filter(|s: &String| !s.is_empty());
        self
    }

    /// The shortcut, if set ([`Semantics::shortcut`]).
    pub fn shortcut_text(&self) -> Option<&str> {
        self.shortcut.as_deref()
    }

    /// The access key (mnemonic) that activates it while its menu or menu bar is active:
    /// the underlined letter, "F" for File.
    pub fn access_key(mut self, key: impl Into<String>) -> Self {
        self.access_key = Some(key.into()).filter(|s: &String| !s.is_empty());
        self
    }

    /// The access key, if set ([`Semantics::access_key`]).
    pub fn access_key_text(&self) -> Option<&str> {
        self.access_key.as_deref()
    }

    /// Where a link goes (`#/settings`, `https://…`): screen readers announce it, and in a
    /// browser page the link is a real `<a href>` that opens in a new tab on middle or Ctrl
    /// click.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into()).filter(|s: &String| !s.is_empty());
        self
    }

    /// Where it goes, if set ([`Semantics::url`]).
    pub fn url_text(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// True when it folds its descendants into itself ([`Semantics::merge_descendants`]).
    pub fn merges_descendants(&self) -> bool {
        self.merge
    }
}

/// The accessibility tree of one window.
#[derive(Debug)]
pub struct A11yTree {
    root: u64,
    nodes: HashMap<u64, (Semantics, Vec<u64>)>,
    parents: HashMap<u64, u64>,
    focus: u64,
    dirty: BTreeSet<u64>,
    connected: bool,
    sent_once: bool,
    /// What a modal showing makes inert: the siblings of it and of each of its ancestors (so
    /// everything outside it), hidden from assistive technology while it shows.
    inert: HashSet<u64>,
    /// Device pixels per logical pixel: bounds are logical, assistive technology reads
    /// device pixels, so the root carries this scale.
    scale: f64,
}

impl A11yTree {
    /// Sets the window's scale (device pixels per logical pixel). Element bounds stay in
    /// logical pixels; the root node scales them to the device pixels platforms expect.
    pub fn set_scale(&mut self, scale: f32) {
        let s = f64::from(scale);
        if s.is_finite() && s > 0.0 && s != self.scale {
            self.scale = s;
            let root = self.root;
            self.mark(root);
        }
    }

    /// A tree whose root element has id `root`.
    pub fn new(root: u64) -> Self {
        A11yTree {
            root,
            nodes: HashMap::new(),
            parents: HashMap::new(),
            focus: root,
            dirty: BTreeSet::new(),
            connected: false,
            sent_once: false,
            inert: HashSet::new(),
            scale: 1.0,
        }
    }

    /// Sets an element's semantics.
    pub fn set(&mut self, id: u64, s: Semantics) {
        let children = self.nodes.remove(&id).map(|(_, c)| c).unwrap_or_default();
        self.nodes.insert(id, (s, children));
        self.mark(id);
    }

    /// Sets an element's children, in reading order.
    pub fn set_children(&mut self, id: u64, children: &[u64]) {
        if let Some((_, c)) = self.nodes.get_mut(&id) {
            *c = children.to_vec();
        }
        for &c in children {
            self.parents.insert(c, id);
        }
        self.mark(id);
    }

    /// Removes an element (its parent's child list is updated by `set_children`).
    pub fn remove(&mut self, id: u64) {
        if self.focus == id {
            // A focus id the consumer cannot look up panics in accesskit_consumer.
            self.focus =
                self.parents.get(&id).copied().filter(|p| self.nodes.contains_key(p)).unwrap_or(self.root);
            self.mark(self.focus);
        }
        self.nodes.remove(&id);
        self.parents.remove(&id);
        self.dirty.remove(&id);
    }

    /// Drops every node `keep` rejects, except the root. Used by the view to prune elements
    /// that left the tree so a later update cannot name them as focus or send them as orphans.
    pub fn retain(&mut self, mut keep: impl FnMut(u64) -> bool) {
        let drop: Vec<u64> = self.nodes.keys().copied().filter(|&id| id != self.root && !keep(id)).collect();
        for id in drop {
            self.remove(id);
        }
    }

    /// Moves accessibility focus. An id that is not in the tree falls back to the root,
    /// so a later update cannot point AccessKit at a missing node.
    pub fn set_focus(&mut self, id: u64) {
        let id = if self.nodes.contains_key(&id) { id } else { self.root };
        if self.focus == id {
            return;
        }
        self.focus = id;
        self.mark(id);
    }

    /// A client (screen reader, agent) connected: the next update is the whole tree.
    pub fn connect(&mut self) {
        self.connected = true;
        self.sent_once = false;
    }

    /// The last client left: stop building updates.
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.dirty.clear();
    }

    /// Marks `id` changed. The merging nodes above it, whose labels it changes too, are
    /// found when the update is built, once for all that changed.
    fn mark(&mut self, id: u64) {
        if self.connected {
            self.dirty.insert(id);
        }
    }

    /// The merging elements above `ids`, whose labels change with them, and for `ids` and
    /// those: whether each is inside a merging ancestor (and so not exposed on its own). Each
    /// path up is walked once however many of `ids` share it, so this is linear in the
    /// tree's size at any depth.
    fn merge_context(&self, ids: &[u64]) -> (BTreeSet<u64>, HashMap<u64, bool>) {
        let merges = |id: u64| self.nodes.get(&id).is_some_and(|(s, _)| s.merge);
        // Up from each element, collecting mergers, to the first element already walked
        // past: everything above that one was collected by the walk that first got there.
        let mut walked: HashSet<u64> = HashSet::new();
        let mut mergers = BTreeSet::new();
        for &id in ids {
            if !walked.insert(id) {
                continue;
            }
            let mut cur = id;
            while let Some(&p) = self.parents.get(&cur) {
                if merges(p) {
                    mergers.insert(p);
                }
                if !walked.insert(p) {
                    break;
                }
                cur = p;
            }
        }
        // Inside a merger: the parent merges or is inside one. Up to the first element known,
        // then down, so each is worked out once.
        let mut inside: HashMap<u64, bool> = HashMap::new();
        let mut path = Vec::new();
        for &id in ids.iter().chain(&mergers) {
            path.clear();
            let mut cur = Some(id);
            while let Some(e) = cur.filter(|e| !inside.contains_key(e)) {
                path.push(e);
                cur = self.parents.get(&e).copied();
            }
            for &e in path.iter().rev() {
                let here = self
                    .parents
                    .get(&e)
                    .is_some_and(|&p| merges(p) || inside.get(&p).copied().unwrap_or(false));
                inside.insert(e, here);
            }
        }
        (mergers, inside)
    }

    /// The label a merging node exposes: its own, then its descendants' labels and values,
    /// in reading order, joined with commas.
    pub fn composed_label(&self, id: u64) -> Option<String> {
        let (s, children) = self.nodes.get(&id)?;
        let mut parts: Vec<String> = s.label.iter().cloned().collect();
        if s.merge {
            let mut stack: Vec<u64> = children.iter().rev().copied().collect();
            while let Some(c) = stack.pop() {
                let Some((cs, cc)) = self.nodes.get(&c) else { continue };
                if cs.hidden {
                    continue;
                }
                parts.extend(cs.label.iter().cloned());
                if cs.role != Role::PasswordInput {
                    parts.extend(cs.value.iter().cloned());
                }
                stack.extend(cc.iter().rev().copied());
            }
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    fn build(&self, id: u64) -> Option<Node> {
        let (s, children) = self.nodes.get(&id)?;
        let mut n = Node::new(role(s.role));
        if id == self.root && self.scale != 1.0 {
            n.set_transform(accesskit::Affine::scale(self.scale));
        }
        if let Some(l) = self.composed_label(id) {
            // AccessKit reads static text's words from its value (its label comes from its
            // value on every platform); a label alone leaves the text unread.
            if s.role == Role::Label && s.value.is_none() {
                n.set_value(l.clone());
            }
            n.set_label(l);
        }
        if let Some(d) = &s.description {
            n.set_description(d.clone());
        }
        if s.role != Role::PasswordInput
            && let Some(v) = &s.value
        {
            n.set_value(v.clone());
        }
        if let Some((v, min, max)) = s.numeric {
            n.set_numeric_value(v);
            n.set_min_numeric_value(min);
            n.set_max_numeric_value(max);
        }
        if let Some(c) = s.checked {
            n.set_toggled(if c { accesskit::Toggled::True } else { accesskit::Toggled::False });
        }
        if let Some(sel) = s.selected {
            n.set_selected(sel);
        }
        if s.multiselectable {
            n.set_multiselectable();
        }
        if s.modal {
            n.set_modal();
        }
        if let Some(p) = s.has_popup {
            n.set_has_popup(match p {
                Popup::Menu => accesskit::HasPopup::Menu,
                Popup::ListBox => accesskit::HasPopup::Listbox,
                Popup::Tree => accesskit::HasPopup::Tree,
                Popup::Grid => accesskit::HasPopup::Grid,
                Popup::Dialog => accesskit::HasPopup::Dialog,
            });
        }
        if let Some(e) = s.expanded {
            n.set_expanded(e);
        }
        if let Some(c) = s.current {
            use accesskit::AriaCurrent as A;
            n.set_aria_current(match c {
                Current::Page => A::Page,
                Current::Step => A::Step,
                Current::Location => A::Location,
                Current::Date => A::Date,
                Current::Time => A::Time,
                Current::True => A::True,
            });
        }
        if s.invalid {
            n.set_invalid(accesskit::Invalid::True);
        }
        if s.disabled {
            n.set_disabled();
        }
        if s.read_only {
            n.set_read_only();
        }
        if s.hidden || self.inert.contains(&id) {
            n.set_hidden();
        }
        match s.live {
            Live::Off => {}
            Live::Polite => n.set_live(accesskit::Live::Polite),
            Live::Assertive => n.set_live(accesskit::Live::Assertive),
        }
        for a in &s.actions {
            n.add_action(action(*a));
        }
        if let Some(b) = s.bounds {
            n.set_bounds(AkRect {
                x0: f64::from(b.min.x),
                y0: f64::from(b.min.y),
                x1: f64::from(b.max.x),
                y1: f64::from(b.max.y),
            });
        }
        if let Some((pos, size)) = s.set_position {
            n.set_position_in_set(pos);
            n.set_size_of_set(size);
        }
        if let Some(level) = s.level {
            n.set_level(level.max(1));
        }
        if let Some(k) = &s.shortcut {
            n.set_keyboard_shortcut(k.clone());
        }
        if let Some(k) = &s.access_key {
            n.set_access_key(k.clone());
        }
        if let Some(u) = &s.url {
            n.set_url(u.clone());
        }
        if let Role::Heading(level) = s.role {
            n.set_level(usize::from(level.clamp(1, 6)));
        }
        if !s.merge {
            let kids: Vec<NodeId> =
                children.iter().filter(|c| self.nodes.contains_key(c)).map(|c| NodeId(*c)).collect();
            n.set_children(kids);
        }
        Some(n)
    }

    /// The tree as a screen reader meets it, in reading order: what is hidden left out, what
    /// a merging node holds folded into its label, password text never shown. Kept whether
    /// or not a client is connected, so devtools can show it at any time.
    pub fn reader_view(&self) -> Vec<ReaderNode> {
        let inert = self.inert_now();
        let mut out = Vec::new();
        let mut stack = vec![(self.root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            let Some((s, children)) = self.nodes.get(&id) else { continue };
            if s.hidden || inert.contains(&id) {
                continue;
            }
            let mut states = Vec::new();
            if let Some(c) = s.checked {
                states.push(if c { "checked" } else { "not checked" });
            }
            if s.selected == Some(true) {
                states.push("selected");
            }
            if let Some(e) = s.expanded {
                states.push(if e { "expanded" } else { "collapsed" });
            }
            if let Some(c) = s.current {
                states.push(match c {
                    Current::Page => "current page",
                    Current::Step => "current step",
                    Current::Location => "current location",
                    Current::Date => "current date",
                    Current::Time => "current time",
                    Current::True => "current",
                });
            }
            if s.disabled {
                states.push("disabled");
            }
            if s.read_only {
                states.push("read only");
            }
            if s.invalid {
                states.push("invalid");
            }
            let value = match (s.role, &s.value, s.numeric) {
                (Role::PasswordInput, _, _) => None,
                (_, Some(v), _) => Some(v.clone()),
                (_, None, Some((v, _, _))) => Some(format!("{v}")),
                _ => None,
            };
            out.push(ReaderNode {
                id,
                depth,
                role: s.role,
                name: self.composed_label(id),
                value,
                description: s.description.clone(),
                states,
                actions: s.actions.clone(),
                bounds: s.bounds,
                focused: self.focus == id,
                live: s.live,
                position: s.set_position,
                checked: s.checked,
                selected: s.selected,
                expanded: s.expanded,
                disabled: s.disabled,
                shortcut: s.shortcut.clone(),
                access_key: s.access_key.clone(),
                url: s.url.clone(),
            });
            if !s.merge {
                stack.extend(children.iter().rev().map(|c| (*c, depth + 1)));
            }
        }
        out
    }

    /// What the showing modal makes inert ([`Semantics::modal`]): the siblings of it and of
    /// each of its ancestors. With several, the newest element's.
    fn inert_now(&self) -> HashSet<u64> {
        let mut out = HashSet::new();
        let modal =
            self.nodes.iter().filter(|(id, (s, _))| s.modal && self.showing(**id)).map(|(id, _)| *id).max();
        let Some(mut cur) = modal else { return out };
        while let Some(&p) = self.parents.get(&cur) {
            if let Some((_, kids)) = self.nodes.get(&p) {
                out.extend(kids.iter().copied().filter(|k| *k != cur));
            }
            cur = p;
        }
        out
    }

    /// True when `id` is in the tree (each parent up to the root lists it). Hidden nodes
    /// still count: they are sent to AccessKit as hidden, not dropped. `known` holds the
    /// answers found so far in one update, so a chain of ancestors is walked once however
    /// many of its nodes are asked about (a tree 20,000 levels deep stays linear work).
    fn attached(&self, id: u64, known: &mut HashMap<u64, bool>) -> bool {
        let mut path = Vec::new();
        let mut cur = id;
        let answer = loop {
            if let Some(&a) = known.get(&cur) {
                break a;
            }
            // Longer than the tree: the parent links loop, so nothing here reaches the root.
            if !self.nodes.contains_key(&cur) || path.len() > self.nodes.len() {
                break false;
            }
            if cur == self.root {
                break true;
            }
            path.push(cur);
            let Some(&p) = self.parents.get(&cur) else { break false };
            if !self.nodes.get(&p).is_some_and(|(_, kids)| kids.contains(&cur)) {
                break false;
            }
            cur = p;
        };
        known.extend(path.into_iter().map(|n| (n, answer)));
        answer
    }

    /// True when `id` is in the tree (each parent up to the root lists it) and neither it nor
    /// an ancestor is hidden.
    fn showing(&self, id: u64) -> bool {
        let mut cur = id;
        loop {
            let Some((s, _)) = self.nodes.get(&cur) else { return false };
            if s.hidden {
                return false;
            }
            if cur == self.root {
                return true;
            }
            let Some(&p) = self.parents.get(&cur) else { return false };
            if !self.nodes.get(&p).is_some_and(|(_, kids)| kids.contains(&cur)) {
                return false;
            }
            cur = p;
        }
    }

    /// True when a showing modal makes `id` inert to assistive technology.
    pub fn is_inert(&self, id: u64) -> bool {
        self.inert_now().contains(&id)
    }

    /// The changes since the last call, or the whole tree after `connect`. `None` when no
    /// client is connected or nothing changed.
    pub fn take_update(&mut self) -> Option<TreeUpdate> {
        if !self.connected {
            return None;
        }
        // A modal opening or closing changes what is inert: those nodes are sent again.
        let inert = self.inert_now();
        if inert != self.inert {
            let changed: Vec<u64> = inert.symmetric_difference(&self.inert).copied().collect();
            self.dirty.extend(changed);
            self.inert = inert;
        }
        let (ids, inside): (Vec<u64>, HashMap<u64, bool>) = if self.sent_once {
            let changed: Vec<u64> = std::mem::take(&mut self.dirty).into_iter().collect();
            // A change inside a merging element changes its label too.
            let (mergers, inside) = self.merge_context(&changed);
            let ids: BTreeSet<u64> = changed.into_iter().chain(mergers).collect();
            (ids.into_iter().collect(), inside)
        } else {
            self.dirty.clear();
            let mut all = Vec::new();
            let mut stack = vec![self.root];
            while let Some(id) = stack.pop() {
                all.push(id);
                if let Some((s, c)) = self.nodes.get(&id)
                    && !s.merge
                {
                    stack.extend(c.iter().rev().copied());
                }
            }
            // The walk never goes into a merging element: nothing in it is merged away.
            (all, HashMap::new())
        };
        let mut attached = HashMap::new();
        self.focus = self.resolved_focus(&mut attached);
        // Orphan dirty nodes must not go in the same update as a parent that dropped them:
        // accesskit_consumer then both "updates" and removes the id, and unwraps at tree.rs:647.
        let nodes: Vec<(NodeId, Node)> = ids
            .into_iter()
            .filter(|id| !inside.get(id).copied().unwrap_or(false))
            .filter(|id| self.attached(*id, &mut attached))
            .filter_map(|id| self.build(id).map(|n| (NodeId(id), n)))
            .collect();
        // The consumer panics if `focus` names a node that is in neither this update nor the
        // tree it already has. An attached, buildable node was sent earlier or is here; if
        // ours is not, fall back to the window. Do not re-send the focused node just because
        // it has focus: that would turn a no-op frame into a root-only update.
        if !self.attached(self.focus, &mut attached) || self.build(self.focus).is_none() {
            self.focus = self.root;
        }
        if nodes.is_empty() && self.sent_once {
            return None;
        }
        let tree = (!self.sent_once).then(|| TreeInfo {
            root: NodeId(self.root),
            toolkit_name: Some("RemedyUI".into()),
            toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
        });
        self.sent_once = true;
        Some(TreeUpdate { nodes, tree, tree_id: TreeId::ROOT, focus: NodeId(self.focus) })
    }

    /// A node the AccessKit consumer can look up: attached to the root and not merged away.
    /// Otherwise the nearest exposed ancestor, or the window.
    fn resolved_focus(&self, attached: &mut HashMap<u64, bool>) -> u64 {
        let mut cur = self.focus;
        // A cycle or a long parent chain must not loop; the root is always a safe landing.
        for _ in 0..self.nodes.len().saturating_add(1) {
            if cur == self.root {
                return self.root;
            }
            let merge = self.nodes.get(&cur).is_some_and(|(s, _)| s.merge);
            if self.attached(cur, attached) && !merge {
                return cur;
            }
            cur = self.parents.get(&cur).copied().unwrap_or(self.root);
        }
        self.root
    }
}

/// One node of the tree as a screen reader meets it ([`A11yTree::reader_view`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReaderNode {
    /// The element's id.
    pub id: u64,
    /// How deep it is (the window is 0).
    pub depth: usize,
    /// What it is.
    pub role: Role,
    /// Its accessible name (for a merging node, its own label and its descendants').
    pub name: Option<String>,
    /// Its value (never a password's).
    pub value: Option<String>,
    /// Its longer description.
    pub description: Option<String>,
    /// Its states, in words: "checked", "selected", "expanded", "disabled", ….
    pub states: Vec<&'static str>,
    /// What can be asked of it.
    pub actions: Vec<Action>,
    /// Its box in window logical pixels.
    pub bounds: Option<Rect>,
    /// It has accessibility focus.
    pub focused: bool,
    /// How its changes are announced.
    pub live: Live,
    /// Its place in a set: (position from 1, size).
    pub position: Option<(usize, usize)>,
    /// Checked, not checked, or not something that can be checked (`None`).
    pub checked: Option<bool>,
    /// Selected, not selected, or not something that can be selected (`None`).
    pub selected: Option<bool>,
    /// Expanded, collapsed, or not something that expands (`None`).
    pub expanded: Option<bool>,
    /// It cannot be used now (`states` says "disabled" too).
    pub disabled: bool,
    /// The keyboard shortcut that does the same ("Ctrl+S").
    pub shortcut: Option<String>,
    /// The access key (mnemonic) that activates it in its menu ("F").
    pub access_key: Option<String>,
    /// Where a link goes ([`Semantics::url`]).
    pub url: Option<String>,
}

impl ReaderNode {
    /// What a screen reader says on reaching it: role, name, value, states and place, in
    /// that order ("button, Send, disabled").
    pub fn announcement(&self) -> String {
        let mut parts: Vec<String> = vec![role_name(self.role)];
        parts.extend(self.name.iter().filter(|n| !n.trim().is_empty()).cloned());
        parts.extend(self.value.iter().filter(|v| !v.trim().is_empty()).cloned());
        parts.extend(self.states.iter().map(|s| (*s).to_owned()));
        parts.extend(self.shortcut.iter().cloned());
        if let Some((pos, size)) = self.position {
            parts.push(format!("{pos} of {size}"));
        }
        parts.join(", ")
    }

    /// True when it is something a person operates (it offers actions beyond focus and
    /// scrolling, or its role is a control).
    pub fn is_control(&self) -> bool {
        is_control_role(self.role)
            || self.actions.iter().any(|a| !matches!(a, Action::Focus | Action::ScrollIntoView))
    }
}

/// True for roles people operate (buttons, inputs, links, tabs, …).
pub fn is_control_role(r: Role) -> bool {
    matches!(
        r,
        Role::Button
            | Role::Check
            | Role::Switch
            | Role::Radio
            | Role::Slider
            | Role::TextInput
            | Role::SearchInput
            | Role::MultilineTextInput
            | Role::PasswordInput
            | Role::Link
            | Role::Tab
            | Role::Option
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::SpinButton
            | Role::ComboBox
            | Role::ColorWell
            | Role::DateInput
            | Role::TimeInput
    )
}

/// A role in the words screen readers use.
pub fn role_name(r: Role) -> String {
    match r {
        Role::Window => "window".into(),
        Role::Group => "group".into(),
        Role::Button => "button".into(),
        Role::Check => "check box".into(),
        Role::Switch => "switch".into(),
        Role::Radio => "radio button".into(),
        Role::Slider => "slider".into(),
        Role::TextInput => "edit text".into(),
        Role::SearchInput => "search, edit text".into(),
        Role::MultilineTextInput => "edit text, multi-line".into(),
        Role::PasswordInput => "password, edit text".into(),
        Role::Link => "link".into(),
        Role::List => "list".into(),
        Role::ListItem => "list item".into(),
        Role::ListBox => "list box".into(),
        Role::Option => "option".into(),
        Role::Tab => "tab".into(),
        Role::TabList => "tab list".into(),
        Role::Menu => "menu".into(),
        Role::MenuItem => "menu item".into(),
        Role::MenuBar => "menu bar".into(),
        Role::Toolbar => "toolbar".into(),
        Role::MenuItemCheckBox => "check menu item".into(),
        Role::MenuItemRadio => "radio menu item".into(),
        Role::Separator => "separator".into(),
        Role::Dialog => "dialog".into(),
        Role::Alert => "alert".into(),
        Role::Status => "status".into(),
        Role::Heading(level) => format!("heading level {}", level.clamp(1, 6)),
        Role::Image => "image".into(),
        Role::Progress => "progress bar".into(),
        Role::Label => "text".into(),
        Role::SpinButton => "spin button".into(),
        Role::ComboBox => "combo box".into(),
        Role::ColorWell => "color well".into(),
        Role::DateInput => "date picker".into(),
        Role::TimeInput => "time picker".into(),
        Role::Grid => "grid".into(),
        Role::GridCell => "cell".into(),
        Role::Form => "form".into(),
        Role::Table => "table".into(),
        Role::Row => "row".into(),
        Role::Cell => "cell".into(),
        Role::ColumnHeader => "column header".into(),
        Role::Tree => "tree".into(),
        Role::TreeItem => "tree item".into(),
        Role::Document => "document".into(),
        Role::Page => "page".into(),
        Role::Navigation => "navigation".into(),
        Role::Main => "main".into(),
        Role::Banner => "banner".into(),
        Role::Region => "region".into(),
        Role::Search => "search".into(),
        Role::Article => "article".into(),
        Role::ContentInfo => "content information".into(),
        Role::Complementary => "complementary".into(),
    }
}

fn role(r: Role) -> accesskit::Role {
    use accesskit::Role as R;
    match r {
        Role::Window => R::Window,
        Role::Group => R::Group,
        Role::Button => R::Button,
        Role::Check => R::CheckBox,
        Role::Switch => R::Switch,
        Role::Radio => R::RadioButton,
        Role::Slider => R::Slider,
        Role::TextInput => R::TextInput,
        Role::SearchInput => R::SearchInput,
        Role::MultilineTextInput => R::MultilineTextInput,
        Role::PasswordInput => R::PasswordInput,
        Role::Link => R::Link,
        Role::List => R::List,
        Role::ListItem => R::ListItem,
        Role::ListBox => R::ListBox,
        Role::Option => R::ListBoxOption,
        Role::Tab => R::Tab,
        Role::TabList => R::TabList,
        Role::Menu => R::Menu,
        Role::MenuItem => R::MenuItem,
        Role::MenuBar => R::MenuBar,
        Role::Toolbar => R::Toolbar,
        Role::MenuItemCheckBox => R::MenuItemCheckBox,
        Role::MenuItemRadio => R::MenuItemRadio,
        // AccessKit's separator: UI Automation's Separator, AT-SPI's separator, ARIA's
        // `separator`.
        Role::Separator => R::Splitter,
        Role::Dialog => R::Dialog,
        Role::Alert => R::Alert,
        Role::Status => R::Status,
        Role::Heading(_) => R::Heading,
        Role::Image => R::Image,
        Role::Progress => R::ProgressIndicator,
        Role::Label => R::Label,
        Role::SpinButton => R::SpinButton,
        Role::ComboBox => R::ComboBox,
        Role::ColorWell => R::ColorWell,
        Role::DateInput => R::DateInput,
        Role::TimeInput => R::TimeInput,
        Role::Grid => R::Grid,
        Role::GridCell => R::GridCell,
        Role::Form => R::Form,
        Role::Table => R::Table,
        Role::Row => R::Row,
        Role::Cell => R::Cell,
        Role::ColumnHeader => R::ColumnHeader,
        Role::Tree => R::Tree,
        Role::TreeItem => R::TreeItem,
        Role::Document => R::Document,
        Role::Page | Role::Article => R::Article,
        Role::Navigation => R::Navigation,
        Role::Main => R::Main,
        Role::Banner => R::Banner,
        Role::Region => R::Region,
        Role::Search => R::Search,
        Role::ContentInfo => R::ContentInfo,
        Role::Complementary => R::Complementary,
    }
}

fn action(a: Action) -> accesskit::Action {
    use accesskit::Action as A;
    match a {
        Action::Click => A::Click,
        Action::Focus => A::Focus,
        Action::Increment => A::Increment,
        Action::Decrement => A::Decrement,
        Action::Expand => A::Expand,
        Action::Collapse => A::Collapse,
        Action::ShowContextMenu => A::ShowContextMenu,
        Action::ScrollIntoView => A::ScrollIntoView,
        Action::SetValue => A::SetValue,
    }
}
