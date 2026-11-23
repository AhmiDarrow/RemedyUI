//! Semantics to AccessKit updates.

use accesskit::NodeId;
use remedy_ui_a11y::*;

fn node(u: &accesskit::TreeUpdate, id: u64) -> Option<&accesskit::Node> {
    u.nodes.iter().find(|(n, _)| *n == NodeId(id)).map(|(_, n)| n)
}

fn chat() -> A11yTree {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("FreeSpeak"));
    t.set(10, Semantics::new(Role::List).label("Messages"));
    t.set(
        11,
        Semantics::new(Role::ListItem).merge_descendants().set_position(5032, 10_000).action(Action::Click),
    );
    t.set(12, Semantics::new(Role::Label).label("Alex"));
    t.set(13, Semantics::new(Role::Label).label("10:42"));
    t.set(14, Semantics::new(Role::Label).label("Meet at 10:30"));
    t.set(20, Semantics::new(Role::PasswordInput).label("Vault password").value("hunter2"));
    t.set_children(1, &[10, 20]);
    t.set_children(10, &[11]);
    t.set_children(11, &[12, 13, 14]);
    t
}

#[test]
fn the_first_update_is_the_whole_tree_with_toolkit_info() {
    let mut t = chat();
    assert!(t.take_update().is_none());
    t.connect();
    let u = t.take_update();
    let u = u.as_ref();
    assert_eq!(u.and_then(|u| u.tree.as_ref()).map(|i| i.root), Some(NodeId(1)));
    assert_eq!(
        u.and_then(|u| u.tree.as_ref()).and_then(|i| i.toolkit_name.clone()),
        Some("RemedyUI".to_string())
    );
    assert_eq!(u.map(|u| u.nodes.len()), Some(4), "window, list, merged row, password field");
}

#[test]
fn merged_rows_read_as_one_sentence() {
    let mut t = chat();
    t.connect();
    let u = t.take_update();
    let row = u.as_ref().and_then(|u| node(u, 11));
    assert_eq!(row.and_then(|n| n.label()), Some("Alex, 10:42, Meet at 10:30"));
    assert_eq!(row.and_then(|n| n.position_in_set()), Some(5032));
    assert_eq!(row.and_then(|n| n.size_of_set()), Some(10_000));
    assert!(row.is_some_and(|n| n.children().is_empty()), "the parts are not exposed on their own");
    assert!(u.as_ref().and_then(|u| node(u, 12)).is_none());
}

#[test]
fn passwords_never_expose_their_text() {
    let mut t = chat();
    t.connect();
    let u = t.take_update();
    let pw = u.as_ref().and_then(|u| node(u, 20));
    assert_eq!(pw.map(|n| n.role()), Some(accesskit::Role::PasswordInput));
    assert_eq!(pw.and_then(|n| n.label()), Some("Vault password"));
    assert_eq!(pw.and_then(|n| n.value()), None);
    // Not even through a merged parent.
    t.set(30, Semantics::new(Role::Group).merge_descendants());
    t.set(31, Semantics::new(Role::PasswordInput).value("secret"));
    t.set_children(30, &[31]);
    assert!(t.composed_label(30).is_none_or(|l| !l.contains("secret")));
}

#[test]
fn later_updates_carry_only_what_changed() {
    let mut t = chat();
    t.connect();
    t.take_update();
    assert!(t.take_update().is_none(), "nothing changed");
    t.set(14, Semantics::new(Role::Label).label("Meet at 11:00"));
    let u = t.take_update();
    let ids: Vec<NodeId> =
        u.as_ref().map(|u| u.nodes.iter().map(|(id, _)| *id).collect()).unwrap_or_default();
    assert_eq!(ids, [NodeId(11)], "the merged row is rebuilt; its hidden part is not sent");
    assert_eq!(
        u.as_ref().and_then(|u| node(u, 11)).and_then(|n| n.label()),
        Some("Alex, 10:42, Meet at 11:00")
    );
    assert!(u.is_some_and(|u| u.tree.is_none()));
}

#[test]
fn live_regions_states_and_disconnecting() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window));
    t.set(2, Semantics::new(Role::Status).label("Connected").live(Live::Polite));
    t.set(3, Semantics::new(Role::Switch).label("Noise suppression").checked(true).action(Action::Click));
    t.set(4, Semantics::new(Role::Slider).label("Input gain").numeric(150.0, 0.0, 200.0));
    t.set_children(1, &[2, 3, 4]);
    t.connect();
    let u = t.take_update();
    let get = |id| u.as_ref().and_then(|u| node(u, id));
    assert_eq!(get(2).and_then(|n| n.live()), Some(accesskit::Live::Polite));
    assert_eq!(get(3).and_then(|n| n.toggled()), Some(accesskit::Toggled::True));
    assert_eq!(get(4).and_then(|n| n.numeric_value()), Some(150.0));
    t.disconnect();
    t.set(2, Semantics::new(Role::Status).label("Reconnecting").live(Live::Polite));
    assert!(t.take_update().is_none(), "no client, no work");
}

#[test]
fn the_root_scales_logical_bounds_to_device_pixels() {
    let mut t = chat();
    t.set_scale(2.5);
    t.connect();
    let u = t.take_update();
    let root = u.as_ref().and_then(|u| node(u, 1)).and_then(|n| n.transform().copied());
    assert_eq!(root, Some(accesskit::Affine::scale(2.5)));
    // A new scale reaches a connected client as a root change only.
    t.set_scale(3.0);
    let u = t.take_update();
    assert_eq!(u.as_ref().map(|u| u.nodes.len()), Some(1));
    let root = u.as_ref().and_then(|u| node(u, 1)).and_then(|n| n.transform().copied());
    assert_eq!(root, Some(accesskit::Affine::scale(3.0)));
    // At scale 1 nothing is transformed.
    let mut plain = chat();
    plain.connect();
    let u = plain.take_update();
    assert_eq!(u.as_ref().and_then(|u| node(u, 1)).and_then(|n| n.transform().copied()), None);
}

#[test]
fn static_text_carries_its_words_as_the_value_adapters_read() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Demo"));
    t.set(2, Semantics::new(Role::Label).label("Taps: 2"));
    t.set_children(1, &[2]);
    t.connect();
    let u = t.take_update();
    let label = u.as_ref().and_then(|u| node(u, 2));
    assert_eq!(label.and_then(|n| n.value()), Some("Taps: 2"));
    assert_eq!(label.and_then(|n| n.label()), Some("Taps: 2"));
    let window = u.as_ref().and_then(|u| node(u, 1));
    assert_eq!(window.and_then(|n| n.value()), None, "only static text gets a value");
}

#[test]
fn menus_reach_platforms_with_their_roles_states_and_keys() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Reader"));
    t.set(2, Semantics::new(Role::MenuBar));
    t.set(
        3,
        Semantics::new(Role::MenuItem).label("View").access_key("V").expanded(true).action(Action::Click),
    );
    t.set(4, Semantics::new(Role::Menu).label("View"));
    t.set(5, Semantics::new(Role::MenuItemCheckBox).label("Book Mode").checked(true).shortcut("Ctrl+B"));
    t.set(6, Semantics::new(Role::Separator));
    t.set(7, Semantics::new(Role::MenuItemRadio).label("Fit Page").checked(false).set_position(2, 3));
    t.set_children(1, &[2, 4]);
    t.set_children(2, &[3]);
    t.set_children(4, &[5, 6, 7]);
    t.connect();
    let u = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    let role = |id| node(&u, id).map(accesskit::Node::role);
    assert_eq!(role(2), Some(accesskit::Role::MenuBar));
    assert_eq!(role(5), Some(accesskit::Role::MenuItemCheckBox));
    assert_eq!(role(6), Some(accesskit::Role::Splitter), "AccessKit's separator");
    assert_eq!(role(7), Some(accesskit::Role::MenuItemRadio));
    assert_eq!(node(&u, 3).and_then(accesskit::Node::access_key), Some("V"));
    assert_eq!(node(&u, 5).and_then(accesskit::Node::keyboard_shortcut), Some("Ctrl+B"));
    assert_eq!(node(&u, 5).and_then(accesskit::Node::toggled), Some(accesskit::Toggled::True));
    assert_eq!(node(&u, 7).and_then(accesskit::Node::toggled), Some(accesskit::Toggled::False));
    let reader = t.reader_view();
    let book = reader.iter().find(|n| n.role == Role::MenuItemCheckBox).unwrap_or_else(|| unreachable!());
    assert_eq!(book.announcement(), "check menu item, Book Mode, checked, Ctrl+B");
    assert_eq!(book.shortcut.as_deref(), Some("Ctrl+B"));
    assert!(is_control_role(Role::MenuItemRadio) && !is_control_role(Role::Separator));
    assert_eq!(role_name(Role::MenuBar), "menu bar");
}

#[test]
fn a_toolbar_reaches_platforms_as_one() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Chat"));
    t.set(2, Semantics::new(Role::Toolbar).label("Message actions"));
    t.set(3, Semantics::new(Role::Button).label("Reply").action(Action::Click));
    t.set_children(1, &[2]);
    t.set_children(2, &[3]);
    t.connect();
    let u = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    assert_eq!(node(&u, 2).map(accesskit::Node::role), Some(accesskit::Role::Toolbar));
    assert_eq!(role_name(Role::Toolbar), "toolbar");
    assert!(!is_control_role(Role::Toolbar), "a container, not a control");
}

#[test]
fn landmarks_and_the_current_item_reach_accesskit() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Portal"));
    let landmarks = [
        (2, Role::Banner, accesskit::Role::Banner),
        (3, Role::Navigation, accesskit::Role::Navigation),
        (4, Role::Main, accesskit::Role::Main),
        (5, Role::Region, accesskit::Role::Region),
        (6, Role::Search, accesskit::Role::Search),
        (7, Role::Article, accesskit::Role::Article),
        (8, Role::ContentInfo, accesskit::Role::ContentInfo),
        (9, Role::Complementary, accesskit::Role::Complementary),
    ];
    for (id, r, _) in landmarks {
        t.set(id, Semantics::new(r).label("Section"));
    }
    t.set(10, Semantics::new(Role::Link).label("Home").current(Current::Page));
    t.set(11, Semantics::new(Role::Link).label("Games"));
    t.set_children(1, &[2, 3, 4, 5, 6, 7, 8, 9]);
    t.set_children(3, &[10, 11]);
    t.connect();
    let u = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    for (id, _, ak) in landmarks {
        assert_eq!(node(&u, id).map(accesskit::Node::role), Some(ak));
    }
    assert_eq!(node(&u, 10).and_then(accesskit::Node::aria_current), Some(accesskit::AriaCurrent::Page));
    assert_eq!(node(&u, 11).and_then(accesskit::Node::aria_current), None);
    let reader = t.reader_view();
    let home = reader.iter().find(|n| n.id == 10).unwrap_or_else(|| unreachable!());
    assert_eq!(home.announcement(), "link, Home, current page");
    assert!(!is_control_role(Role::Navigation), "a landmark, not a control");
}

#[test]
fn deep_trees_update_in_linear_time_and_changes_reach_their_merging_ancestor() {
    // Tens of thousands of levels, a merging group near the top: a change at the bottom
    // re-sends the group (its label changed) and nothing it merged away.
    const N: u64 = 20_000;
    let started = std::time::Instant::now();
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Deep"));
    t.set(2, Semantics::new(Role::Group).merge_descendants());
    t.set_children(1, &[2]);
    let mut parent = 2;
    for id in 3..3 + N {
        t.set(id, Semantics::new(Role::Group));
        t.set_children(parent, &[id]);
        parent = id;
    }
    let leaf = 3 + N;
    t.set(leaf, Semantics::new(Role::Label).label("before"));
    t.set_children(parent, &[leaf]);
    t.connect();
    let first = t.take_update();
    assert_eq!(first.as_ref().map(|u| u.nodes.len()), Some(2), "the window and the merged group");
    t.set(leaf, Semantics::new(Role::Label).label("after"));
    let u = t.take_update();
    let ids: Vec<NodeId> =
        u.as_ref().map(|u| u.nodes.iter().map(|(id, _)| *id).collect()).unwrap_or_default();
    assert_eq!(ids, [NodeId(2)], "the group, whose label changed, and nothing merged away");
    assert_eq!(u.as_ref().and_then(|u| node(u, 2)).and_then(|n| n.label()), Some("after"));
    // Every level changed at once is one pass up, not one per level.
    for id in 3..3 + N {
        t.set(id, Semantics::new(Role::Group).label("level"));
    }
    assert_eq!(t.take_update().map(|u| u.nodes.len()), Some(1));
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(20), "linear work ({took:?})");
}

#[test]
fn a_list_box_and_its_options_reach_accesskit_with_their_selection() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Portal"));
    t.set(2, Semantics::new(Role::ListBox).label("Theme"));
    t.set(3, Semantics::new(Role::Option).label("Portal").selected(true).action(Action::Click));
    t.set(4, Semantics::new(Role::Option).label("Light").selected(false).action(Action::Click));
    t.set(5, Semantics::new(Role::ListBox).label("Fonts").multiselectable(true));
    t.set_children(1, &[2, 5]);
    t.set_children(2, &[3, 4]);
    t.connect();
    let u = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    assert_eq!(node(&u, 2).map(accesskit::Node::role), Some(accesskit::Role::ListBox));
    assert_eq!(node(&u, 3).map(accesskit::Node::role), Some(accesskit::Role::ListBoxOption));
    assert_eq!(node(&u, 3).and_then(accesskit::Node::is_selected), Some(true));
    assert_eq!(node(&u, 4).and_then(accesskit::Node::is_selected), Some(false));
    assert!(!node(&u, 2).is_some_and(accesskit::Node::is_multiselectable), "one choice at a time");
    assert!(node(&u, 5).is_some_and(accesskit::Node::is_multiselectable));
    assert_eq!(role_name(Role::ListBox), "list box");
    assert!(is_control_role(Role::Option), "an option is picked like a control");
    assert!(!is_control_role(Role::ListBox), "a container, not a control");
    let reader = t.reader_view();
    let portal = reader.iter().find(|n| n.id == 3).unwrap_or_else(|| unreachable!());
    assert_eq!(portal.announcement(), "option, Portal, selected");
}

#[test]
fn an_open_modal_makes_the_rest_of_the_window_inert() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("Portal"));
    t.set(2, Semantics::new(Role::Main).label("Page"));
    t.set(3, Semantics::new(Role::Button).label("Theme").has_popup(Popup::Dialog).action(Action::Click));
    t.set(4, Semantics::new(Role::Group));
    t.set_children(1, &[2, 4]);
    t.set_children(2, &[3]);
    t.connect();
    let u = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    assert_eq!(node(&u, 3).and_then(accesskit::Node::has_popup), Some(accesskit::HasPopup::Dialog));
    assert!(!node(&u, 2).is_some_and(accesskit::Node::is_hidden));
    // The dialog opens in the overlay layer: the page behind it goes inert.
    t.set(5, Semantics::new(Role::Dialog).label("Theme").modal(true));
    t.set(6, Semantics::new(Role::Button).label("Close").action(Action::Click));
    t.set_children(4, &[5]);
    t.set_children(5, &[6]);
    let u = t.take_update().unwrap_or_else(|| unreachable!("changed"));
    assert!(node(&u, 5).is_some_and(accesskit::Node::is_modal));
    assert!(node(&u, 2).is_some_and(accesskit::Node::is_hidden), "the page is hidden behind the modal");
    assert!(t.is_inert(2) && !t.is_inert(5) && !t.is_inert(6));
    let names: Vec<_> = t.reader_view().into_iter().filter_map(|n| n.name).collect();
    assert!(names.contains(&"Close".to_owned()) && !names.contains(&"Page".to_owned()), "{names:?}");
    // Closed: the page comes back.
    t.set(5, Semantics::new(Role::Dialog).label("Theme").modal(true).hidden(true));
    let u = t.take_update().unwrap_or_else(|| unreachable!("changed"));
    assert!(node(&u, 2).is_some_and(|n| !n.is_hidden()), "the page is back");
    assert!(!t.is_inert(2));
}

struct SilentConsumer;

impl accesskit_consumer::TreeChangeHandler for SilentConsumer {
    fn node_added(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
    fn node_updated(&mut self, _: &accesskit_consumer::NodeRef<'_>, _: &accesskit_consumer::NodeRef<'_>) {}
    fn focus_moved(
        &mut self,
        _: Option<&accesskit_consumer::NodeRef<'_>>,
        _: Option<&accesskit_consumer::NodeRef<'_>>,
    ) {
    }
    fn node_removed(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
}

fn apply(tree: &mut accesskit_consumer::Tree, update: accesskit::TreeUpdate) {
    tree.update_and_process_changes(update, &mut SilentConsumer);
}

#[test]
fn focus_falls_back_to_the_root_when_the_focused_node_is_removed() {
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("App"));
    t.set(2, Semantics::new(Role::TextInput).label("Search"));
    t.set_children(1, &[2]);
    t.set_focus(2);
    t.connect();
    let first = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    assert_eq!(first.focus, NodeId(2));
    let mut consumer = accesskit_consumer::Tree::new(first, true);
    t.remove(2);
    t.set_children(1, &[]);
    let u = t.take_update().unwrap_or_else(|| unreachable!("changed"));
    assert_eq!(u.focus, NodeId(1), "focus follows the removed node back to the window");
    assert!(node(&u, 2).is_none(), "the removed field is not sent");
    apply(&mut consumer, u);
}

#[test]
fn opening_a_palette_and_resizing_does_not_send_a_stale_focus_id() {
    // The focused field (id 3) is lower than its parent (id 10). When the command palette
    // replaces that field, an incremental update that still named 3 as focus *and* sent it
    // as a dirty node — then dropped it from the parent's children — made
    // accesskit_consumer unwrap at tree.rs:647.
    let mut t = A11yTree::new(1);
    t.set(1, Semantics::new(Role::Window).label("App"));
    t.set(10, Semantics::new(Role::Group).label("Main"));
    t.set(3, Semantics::new(Role::TextInput).label("Prompt"));
    t.set_children(1, &[10]);
    t.set_children(10, &[3]);
    t.set_focus(3);
    t.connect();
    let first = t.take_update().unwrap_or_else(|| unreachable!("connected"));
    assert_eq!(first.focus, NodeId(3));
    let mut consumer = accesskit_consumer::Tree::new(first, true);

    // Palette opens and the window resizes: the old field stays dirty and focused, but is
    // no longer a child of anyone; a new search box appears under a new parent.
    t.set(3, Semantics::new(Role::TextInput).label("Prompt"));
    t.set(20, Semantics::new(Role::Dialog).label("Command palette"));
    t.set(5, Semantics::new(Role::SearchInput).label("Search"));
    t.set_children(1, &[10, 20]);
    t.set_children(10, &[]);
    t.set_children(20, &[5]);
    let u = t.take_update().unwrap_or_else(|| unreachable!("changed"));
    let ids: Vec<NodeId> = u.nodes.iter().map(|(id, _)| *id).collect();
    assert_ne!(u.focus, NodeId(3), "unparented focus must not stay on the missing field");
    assert!(!ids.contains(&NodeId(3)), "the unparented field is not sent as an update");
    assert!(
        ids.contains(&u.focus) || u.focus == NodeId(1) || u.focus == NodeId(10),
        "focus {focus:?} must be a node the consumer already has or receives, ids={ids:?}",
        focus = u.focus
    );
    apply(&mut consumer, u);

    // Focus moves to the new search field; a resize dirties the live nodes again.
    t.set_focus(5);
    t.set(1, Semantics::new(Role::Window).label("App"));
    t.set(10, Semantics::new(Role::Group).label("Main"));
    t.set(20, Semantics::new(Role::Dialog).label("Command palette"));
    t.set(5, Semantics::new(Role::SearchInput).label("Search"));
    let u = t.take_update().unwrap_or_else(|| unreachable!("changed"));
    assert_eq!(u.focus, NodeId(5));
    assert!(node(&u, 5).is_some(), "the focused search field is in the update");
    apply(&mut consumer, u);
}
