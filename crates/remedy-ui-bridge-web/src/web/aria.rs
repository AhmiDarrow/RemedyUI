//! The accessibility mirror: the engine's AccessKit tree as ARIA elements over the canvas.
//!
//! AccessKit has no web adapter, so the bridge keeps its own: one element per node, nested as
//! the tree is, laid over the window at each node's bounds (so touch exploration finds them
//! where they are drawn), invisible and letting the pointer through to the canvas. Each
//! element carries the node's role, name, value and states; live regions are announced by
//! the browser. The engine's focus is the element's focus, so screen readers follow it, and
//! keys pressed there still reach the window. A screen reader's click and focus come back to
//! the engine as AccessKit actions.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use remedy_ui_host::accesskit::{Action, ActionRequest, Node, NodeId, Rect, TreeId, TreeUpdate};
use remedy_ui_host::{AccessibilityEvent, HostEvent, WindowId};
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use super::dom::{self, Listener};
use super::post;
use crate::convert;

struct Entry {
    el: HtmlElement,
    node: Node,
}

/// One window's mirror.
pub(crate) struct Mirror {
    window: WindowId,
    root_el: HtmlElement,
    nodes: RefCell<HashMap<NodeId, Entry>>,
    root: Cell<Option<NodeId>>,
    tree: Cell<TreeId>,
    focus: Cell<Option<NodeId>>,
    /// The bridge is moving focus itself.
    pub(crate) moving_focus: Cell<bool>,
    listeners: RefCell<Vec<Listener>>,
}

impl Mirror {
    pub(crate) fn new(root_el: HtmlElement, window: WindowId) -> Mirror {
        dom::style(
            &root_el,
            &[
                ("position", "absolute"),
                ("left", "0"),
                ("top", "0"),
                ("width", "100%"),
                ("height", "100%"),
                ("overflow", "hidden"),
                ("pointer-events", "none"),
                ("color", "transparent"),
                ("background", "transparent"),
                ("font-size", "1px"),
                ("white-space", "pre"),
            ],
        );
        let _ = root_el.set_attribute("data-remedy-a11y", "");
        Mirror {
            window,
            root_el,
            nodes: RefCell::new(HashMap::new()),
            root: Cell::new(None),
            tree: Cell::new(TreeId::ROOT),
            focus: Cell::new(None),
            moving_focus: Cell::new(false),
            listeners: RefCell::new(Vec::new()),
        }
    }

    /// Listens for screen readers' clicks and focus moves on the mirror.
    pub(crate) fn attach(&self) {
        let window = self.window;
        let click = Listener::new(&self.root_el, "click", move |e| {
            // A modified click on a link is the browser's: a new tab or window.
            let modified = e
                .dyn_ref::<web_sys::MouseEvent>()
                .is_some_and(|m| m.ctrl_key() || m.meta_key() || m.shift_key());
            let on_link = e
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .and_then(|el| el.closest("a[href]").ok().flatten())
                .is_some();
            if modified && on_link {
                return;
            }
            if let Some(id) = node_of(e.target()) {
                e.prevent_default();
                action(window, id, Action::Click);
            }
        });
        let focus = Listener::new(&self.root_el, "focusin", move |e| {
            let Some(w) = super::driver().and_then(|d| d.window(window)) else { return };
            if w.aria.moving_focus.get() {
                return;
            }
            if let Some(id) = node_of(e.target())
                && w.aria.focus.get() != Some(id)
            {
                action(window, id, Action::Focus);
            }
        });
        *self.listeners.borrow_mut() = vec![click, focus];
    }

    /// The name of the node with focus, for the hidden text field that stands in for it.
    pub(crate) fn focused_label(&self) -> Option<String> {
        let id = self.focus.get()?;
        self.nodes.borrow().get(&id).and_then(|e| e.node.label().map(str::to_owned))
    }

    /// The element of the node with focus.
    pub(crate) fn focused_element(&self) -> Option<HtmlElement> {
        let id = self.focus.get()?;
        self.nodes.borrow().get(&id).map(|e| e.el.clone())
    }

    /// Applies an update: new and changed nodes, their children's order, nodes no longer in
    /// the tree, and the focus.
    pub(crate) fn update(&self, update: TreeUpdate) {
        if let Some(info) = &update.tree {
            self.root.set(Some(info.root));
            self.tree.set(update.tree_id);
        }
        let mut changed: Vec<NodeId> = Vec::with_capacity(update.nodes.len());
        {
            let mut nodes = self.nodes.borrow_mut();
            for (id, node) in update.nodes {
                // A link is a real `<a href>`, so the browser opens it in a new tab.
                let tag = if node.url().is_some() { "A" } else { "DIV" };
                match nodes.get_mut(&id) {
                    Some(e) if e.el.tag_name() == tag => {
                        apply(&e.el, &node, id);
                        e.node = node;
                    }
                    old => {
                        let Some(el) = dom::element::<HtmlElement>(&tag.to_ascii_lowercase()) else {
                            continue;
                        };
                        dom::style(&el, &[("position", "absolute"), ("margin", "0"), ("padding", "0")]);
                        let _ = el.set_attribute("data-node", &id.0.to_string());
                        apply(&el, &node, id);
                        if let Some(old) = old {
                            // It became a link or stopped being one: same place, new tag.
                            let _ = old.el.replace_with_with_node_1(&el);
                        }
                        nodes.insert(id, Entry { el, node });
                    }
                }
                changed.push(id);
            }
        }
        self.arrange(&changed);
        self.prune();
        self.set_focus(update.focus);
    }

    /// Puts each changed node's children in its element, in order, and places them.
    fn arrange(&self, changed: &[NodeId]) {
        let nodes = self.nodes.borrow();
        if let Some(root) = self.root.get().and_then(|r| nodes.get(&r))
            && root.el.parent_node().as_ref() != Some(self.root_el.unchecked_ref())
        {
            let _ = self.root_el.append_child(&root.el);
        }
        for id in changed {
            let Some(parent) = nodes.get(id) else { continue };
            let children: Vec<&Entry> = parent.node.children().iter().filter_map(|c| nodes.get(c)).collect();
            let current = parent.el.children();
            let in_order = current.length() as usize == children.len()
                && children
                    .iter()
                    .enumerate()
                    .all(|(i, c)| current.item(i as u32).as_ref() == Some(c.el.unchecked_ref()));
            if !in_order {
                for c in &children {
                    let _ = parent.el.append_child(&c.el);
                }
            }
        }
        // Positions are relative to the nearest ancestor with bounds (nodes without bounds
        // are not positioned, so their children measure from further up).
        let mut parents: HashMap<NodeId, NodeId> = HashMap::new();
        for (id, e) in nodes.iter() {
            for c in e.node.children() {
                parents.insert(*c, *id);
            }
        }
        let origin = |id: NodeId| {
            let mut at = parents.get(&id).copied();
            let mut hops = 0usize;
            while let Some(p) = at {
                if let Some(b) = nodes.get(&p).and_then(|e| e.node.bounds()) {
                    return Some(b);
                }
                hops += 1;
                if hops > nodes.len() {
                    break;
                }
                at = parents.get(&p).copied();
            }
            None
        };
        for id in changed {
            let Some(e) = nodes.get(id) else { continue };
            place(&e.el, e.node.bounds(), origin(*id));
            for c in e.node.children() {
                if let Some(ce) = nodes.get(c) {
                    place(&ce.el, ce.node.bounds(), origin(*c));
                }
            }
        }
    }

    /// Drops the elements of nodes no longer reachable from the root.
    fn prune(&self) {
        let Some(root) = self.root.get() else { return };
        let mut nodes = self.nodes.borrow_mut();
        let mut seen: HashSet<NodeId> = HashSet::with_capacity(nodes.len());
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if seen.insert(id)
                && let Some(e) = nodes.get(&id)
            {
                stack.extend(e.node.children().iter().copied());
            }
        }
        if seen.len() == nodes.len() {
            return;
        }
        nodes.retain(|id, e| {
            let keep = seen.contains(id);
            if !keep {
                e.el.remove();
            }
            keep
        });
    }

    fn set_focus(&self, id: NodeId) {
        if self.focus.replace(Some(id)) == Some(id) {
            return;
        }
        let Some(w) = super::driver().and_then(|d| d.window(self.window)) else { return };
        // A field that edits holds focus in the hidden text field instead, named after it.
        if w.text.active() {
            let label = self.nodes.borrow().get(&id).and_then(|e| e.node.label().map(str::to_owned));
            w.text.set_label(label.as_deref());
            return;
        }
        let el = self.nodes.borrow().get(&id).map(|e| e.el.clone());
        let Some(el) = el else { return };
        // Focus moves only within a window that has it: the engine's focus never pulls the
        // page's away from another window or the page's own content.
        let inside = dom::active_element().is_some_and(|a| w.frame.contains(Some(&a)));
        if inside {
            self.moving_focus.set(true);
            el.set_tab_index(-1);
            dom::focus(&el);
            self.moving_focus.set(false);
        }
    }

    /// The window lost its screen reader's interest (it closed): empty the mirror.
    pub(crate) fn clear(&self) {
        for (_, e) in self.nodes.borrow_mut().drain() {
            e.el.remove();
        }
        self.root.set(None);
        self.focus.set(None);
    }
}

/// Shows a node: its role and ARIA attributes, its text, and whether actions find it.
fn apply(el: &HtmlElement, node: &Node, _id: NodeId) {
    dom::attr(el, "role", convert::aria_role(node.role()));
    if el.tag_name() == "A" {
        dom::attr(el, "href", node.url());
    }
    for (name, value) in convert::aria_attributes(node) {
        dom::attr(el, name, value.as_deref());
    }
    if node.children().is_empty() {
        let text = convert::text_content(node);
        if el.text_content() != text {
            el.set_text_content(text.as_deref());
        }
    }
    // Elements a screen reader may move to: those the engine can focus or activate.
    let operable = node.supports_action(Action::Focus) || node.supports_action(Action::Click);
    if operable && el.tab_index() != -1 {
        el.set_tab_index(-1);
    } else if !operable && el.has_attribute("tabindex") {
        let _ = el.remove_attribute("tabindex");
    }
}

/// Lays an element over where its node is drawn, relative to its parent's box.
fn place(el: &HtmlElement, bounds: Option<Rect>, parent: Option<Rect>) {
    let Some(b) = bounds else {
        dom::style(el, &[("position", "static"), ("width", "auto"), ("height", "auto")]);
        return;
    };
    let (px, py) = parent.map_or((0.0, 0.0), |p| (p.x0, p.y0));
    dom::style(
        el,
        &[
            ("position", "absolute"),
            ("left", &format!("{}px", b.x0 - px)),
            ("top", &format!("{}px", b.y0 - py)),
            ("width", &format!("{}px", (b.x1 - b.x0).max(0.0))),
            ("height", &format!("{}px", (b.y1 - b.y0).max(0.0))),
        ],
    );
}

/// The node an event happened on (its element or one inside it).
fn node_of(target: Option<web_sys::EventTarget>) -> Option<NodeId> {
    let el = target?.dyn_into::<web_sys::Element>().ok()?;
    let node = el.closest("[data-node]").ok()??;
    node.get_attribute("data-node")?.parse::<u64>().ok().map(NodeId)
}

fn action(window: WindowId, target: NodeId, action: Action) {
    let tree = super::driver().and_then(|d| d.window(window)).map_or(TreeId::ROOT, |w| w.aria.tree.get());
    let request = ActionRequest { action, target_tree: tree, target_node: target, data: None };
    post(HostEvent::Accessibility(window, AccessibilityEvent::Action(request)));
}
