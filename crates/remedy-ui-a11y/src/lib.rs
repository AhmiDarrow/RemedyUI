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

use std::collections::{BTreeSet, HashMap};

use accesskit::{Node, NodeId, Rect as AkRect, TreeId, TreeInfo, TreeUpdate};
use remedy_ui_core::Rect;

/// AccessKit's tree update, which platform adapters consume.
pub use accesskit::TreeUpdate as AccessKitUpdate;

/// What an element is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// A tab.
    Tab,
    /// A row of tabs.
    TabList,
    /// A menu.
    Menu,
    /// An item in a menu.
    MenuItem,
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
}

/// Something an assistive technology or agent can ask an element to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// How changes to an element are announced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Live {
    /// Not announced.
    #[default]
    Off,
    /// Announced when the user is idle (new chat messages).
    Polite,
    /// Announced at once (call dropped, errors).
    Assertive,
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
    expanded: Option<bool>,
    disabled: bool,
    read_only: bool,
    hidden: bool,
    live: Live,
    actions: Vec<Action>,
    bounds: Option<Rect>,
    set_position: Option<(usize, usize)>,
    merge: bool,
}

impl Semantics {
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
            expanded: None,
            disabled: false,
            read_only: false,
            hidden: false,
            live: Live::Off,
            actions: Vec::new(),
            bounds: None,
            set_position: None,
            merge: false,
        }
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

    /// Expanded or collapsed.
    pub fn expanded(mut self, on: bool) -> Self {
        self.expanded = Some(on);
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
}

impl A11yTree {
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
        self.nodes.remove(&id);
        self.parents.remove(&id);
        self.dirty.remove(&id);
    }

    /// Moves accessibility focus.
    pub fn set_focus(&mut self, id: u64) {
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

    fn mark(&mut self, id: u64) {
        if !self.connected {
            return;
        }
        self.dirty.insert(id);
        // A change inside a merged node changes the merged label.
        let mut cur = id;
        while let Some(&p) = self.parents.get(&cur) {
            if self.nodes.get(&p).is_some_and(|(s, _)| s.merge) {
                self.dirty.insert(p);
            }
            cur = p;
        }
    }

    /// True when `id` is inside a merging ancestor (and so not exposed on its own).
    fn merged_away(&self, id: u64) -> bool {
        let mut cur = id;
        while let Some(&p) = self.parents.get(&cur) {
            if self.nodes.get(&p).is_some_and(|(s, _)| s.merge) {
                return true;
            }
            cur = p;
        }
        false
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
        if let Some(l) = self.composed_label(id) {
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
        if let Some(e) = s.expanded {
            n.set_expanded(e);
        }
        if s.disabled {
            n.set_disabled();
        }
        if s.read_only {
            n.set_read_only();
        }
        if s.hidden {
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

    /// The changes since the last call, or the whole tree after `connect`. `None` when no
    /// client is connected or nothing changed.
    pub fn take_update(&mut self) -> Option<TreeUpdate> {
        if !self.connected {
            return None;
        }
        let ids: Vec<u64> = if self.sent_once {
            std::mem::take(&mut self.dirty).into_iter().collect()
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
            all
        };
        let nodes: Vec<(NodeId, Node)> = ids
            .into_iter()
            .filter(|id| !self.merged_away(*id))
            .filter_map(|id| self.build(id).map(|n| (NodeId(id), n)))
            .collect();
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
        Role::MultilineTextInput => R::MultilineTextInput,
        Role::PasswordInput => R::PasswordInput,
        Role::Link => R::Link,
        Role::List => R::List,
        Role::ListItem => R::ListItem,
        Role::Tab => R::Tab,
        Role::TabList => R::TabList,
        Role::Menu => R::Menu,
        Role::MenuItem => R::MenuItem,
        Role::Dialog => R::Dialog,
        Role::Alert => R::Alert,
        Role::Status => R::Status,
        Role::Heading(_) => R::Heading,
        Role::Image => R::Image,
        Role::Progress => R::ProgressIndicator,
        Role::Label => R::Label,
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
    }
}
