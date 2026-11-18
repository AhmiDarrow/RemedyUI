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
