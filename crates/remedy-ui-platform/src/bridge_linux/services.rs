//! Desktop services on Linux over D-Bus: the tray (StatusNotifierItem with a dbusmenu menu)
//! and file dialogs (the XDG desktop portal's FileChooser).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use remedy_ui_core::host::{FileDialog, TrayEvent, TrayIcon, TrayItem, Unsupported, Waker};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, Type, Variant, interface};

#[derive(Default)]
struct Shared {
    tray: Option<TrayIcon>,
    revision: u32,
    events: Vec<TrayEvent>,
    waker: Option<Waker>,
}

type State = Arc<Mutex<Shared>>;

fn lock(s: &State) -> std::sync::MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn push(s: &State, e: TrayEvent) {
    let waker = {
        let mut g = lock(s);
        g.events.push(e);
        g.waker.clone()
    };
    if let Some(w) = waker {
        w();
    }
}

/// StatusNotifierItem pixmaps: width, height, ARGB32 in network byte order.
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

/// The `ToolTip` property: icon name, icon pixmaps, title, description.
type ToolTip = (String, Pixmaps, String, String);

/// ARGB32 in network byte order, as StatusNotifierItem pixmaps are: every size the icon is
/// drawn at, so the host picks the one it shows.
fn pixmap(t: &TrayIcon) -> Pixmaps {
    t.images()
        .into_iter()
        .map(|img| {
            let s = i32::try_from(img.size).unwrap_or(0);
            let data = img
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, a]| {
                    let un = |c: u8| {
                        if a == 0 {
                            0
                        } else {
                            ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
                        }
                    };
                    [a, un(r), un(g), un(b)]
                })
                .collect();
            (s, s, data)
        })
        .collect()
}

struct Item(State);

#[interface(interface = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> String {
        lock(&self.0).tray.as_ref().map(|t| t.tooltip.clone()).unwrap_or_default()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.id()
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        lock(&self.0).tray.as_ref().map(pixmap).unwrap_or_default()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        let g = lock(&self.0);
        let tip = g.tray.as_ref().map(|t| t.tooltip.clone()).unwrap_or_default();
        (String::new(), Vec::new(), tip, String::new())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> zbus::OwnedObjectPath {
        zbus::OwnedObjectPath::try_from("/MenuBar").unwrap_or_else(|_| zbus::OwnedObjectPath::default())
    }
    fn activate(&self, _x: i32, _y: i32) {
        push(&self.0, TrayEvent::Activate);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        push(&self.0, TrayEvent::Activate);
    }
    fn context_menu(&self, _x: i32, _y: i32) {
        // Hosts show the dbusmenu themselves.
    }
    fn scroll(&self, _delta: i32, _orientation: &str) {}
    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// One dbusmenu node: id, properties, children (each a variant of the same shape).
#[derive(Clone, Debug, serde::Serialize, Type, Variant)]
struct Node {
    id: i32,
    props: HashMap<String, OwnedVariant>,
    children: Vec<OwnedVariant>,
}

/// A label's `&` access key (as on Windows; `&&` is a literal `&`) as dbusmenu's `_`, with
/// literal underscores doubled.
fn dbusmenu_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => match chars.next() {
                Some('&') => out.push('&'),
                Some('_') => out.push_str("__"),
                Some(k) => {
                    out.push('_');
                    out.push(k);
                }
                None => {}
            },
            '_' => out.push_str("__"),
            c => out.push(c),
        }
    }
    out
}

fn props(item: Option<&TrayItem>) -> HashMap<String, OwnedVariant> {
    let mut m = HashMap::new();
    let mut put = |k: &str, v: Variant<'static>| {
        if let Ok(v) = OwnedVariant::try_from(v) {
            m.insert(k.to_owned(), v);
        }
    };
    match item {
        None => put("children-display", Variant::from("submenu")),
        Some(TrayItem::Separator) => put("type", Variant::from("separator")),
        Some(TrayItem::Item { label, enabled, .. }) => {
            put("label", Variant::from(dbusmenu_label(label)));
            put("enabled", Variant::from(*enabled));
            put("visible", Variant::from(true));
        }
        Some(TrayItem::Check { label, enabled, checked, .. }) => {
            put("label", Variant::from(dbusmenu_label(label)));
            put("enabled", Variant::from(*enabled));
            put("visible", Variant::from(true));
            put("toggle-type", Variant::from("checkmark"));
            put("toggle-state", Variant::from(i32::from(*checked)));
        }
        Some(TrayItem::Submenu { label, items }) => {
            put("label", Variant::from(dbusmenu_label(label)));
            put("enabled", Variant::from(!items.is_empty()));
            put("visible", Variant::from(true));
            put("children-display", Variant::from("submenu"));
        }
    }
    m
}

/// Every entry, submenus' entries right after their submenu: entry `n` (0-based) has
/// dbusmenu id `n + 1`; 0 is the root.
fn flat(items: &[TrayItem]) -> Vec<&TrayItem> {
    let mut out = Vec::new();
    for it in items {
        out.push(it);
        if let TrayItem::Submenu { items, .. } = it {
            out.extend(flat(items));
        }
    }
    out
}

/// The layout under dbusmenu id `parent` (0 for the whole menu).
fn layout(items: &[TrayItem], parent: i32) -> Node {
    fn find(items: &[TrayItem], next: &mut i32, parent: i32) -> Option<Node> {
        for it in items {
            *next += 1;
            let id = *next;
            let sub: &[TrayItem] = match it {
                TrayItem::Submenu { items, .. } => items.as_slice(),
                _ => &[],
            };
            if id == parent {
                let mut first = id;
                return Some(Node { id, props: props(Some(it)), children: nodes(sub, &mut first) });
            }
            if let Some(n) = find(sub, next, parent) {
                return Some(n);
            }
        }
        None
    }
    fn nodes(items: &[TrayItem], next: &mut i32) -> Vec<OwnedVariant> {
        items
            .iter()
            .filter_map(|it| {
                *next += 1;
                let id = *next;
                let children = match it {
                    TrayItem::Submenu { items, .. } => nodes(items, next),
                    _ => Vec::new(),
                };
                OwnedVariant::try_from(Variant::from(Node { id, props: props(Some(it)), children })).ok()
            })
            .collect()
    }
    if parent != 0
        && let Some(n) = find(items, &mut 0, parent)
    {
        return n;
    }
    Node { id: 0, props: props(None), children: nodes(items, &mut 0) }
}

struct Menu(State);

#[interface(interface = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
    fn get_layout(&self, parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Node) {
        let g = lock(&self.0);
        let items = g.tray.as_ref().map(|t| t.menu.clone()).unwrap_or_default();
        (g.revision, layout(&items, parent))
    }
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let g = lock(&self.0);
        let items = g.tray.as_ref().map(|t| t.menu.clone()).unwrap_or_default();
        ids.into_iter()
            .map(|id| {
                let all = flat(&items);
                let item =
                    usize::try_from(id).ok().and_then(|i| i.checked_sub(1)).and_then(|i| all.get(i).copied());
                (id, props(item))
            })
            .collect()
    }
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedVariant> {
        let mut all = self.get_group_properties(vec![id], Vec::new());
        all.pop()
            .and_then(|(_, mut p)| p.remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }
    fn event(&self, id: i32, event_id: &str, _data: OwnedVariant, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let entry = {
            let g = lock(&self.0);
            let items = g.tray.as_ref().map(|t| t.menu.clone()).unwrap_or_default();
            usize::try_from(id)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| flat(&items).get(i)?.chosen_id())
        };
        if let Some(id) = entry {
            push(&self.0, TrayEvent::Menu(id));
        }
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> Vec<i32> {
        for (id, ev, data, ts) in events {
            self.event(id, &ev, data, ts);
        }
        Vec::new()
    }
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

/// The tray: a StatusNotifierItem on its D-Bus connection, or, on X11 desktops without a
/// StatusNotifier host, an icon docked in the XEmbed system tray.
#[derive(Default)]
pub(crate) struct Tray {
    state: State,
    conn: Option<zbus::Connection>,
    xembed: Option<crate::bridge_linux::xtray::XTray>,
}

impl Tray {
    pub(crate) fn set_waker(&mut self, waker: Waker) {
        lock(&self.state).waker = Some(waker);
    }

    pub(crate) fn take_events(&mut self) -> Vec<TrayEvent> {
        std::mem::take(&mut lock(&self.state).events)
    }

    pub(crate) fn set(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
        let err = |e: zbus::Error| Unsupported::new("the tray", e.to_string());
        let Some(t) = tray else {
            lock(&self.state).tray = None;
            // Dropping the connection releases our bus name; hosts remove the item.
            self.conn = None;
            // Dropping the XEmbed icon destroys its window; the tray undocks it.
            self.xembed = None;
            return Ok(());
        };
        if let Some(x) = &self.xembed {
            x.set(t);
            return Ok(());
        }
        {
            let mut g = lock(&self.state);
            g.tray = Some(t.clone());
            g.revision = g.revision.wrapping_add(1);
        }
        if let Some(conn) = &self.conn {
            let revision = lock(&self.state).revision;
            return zbus::block_on(async {
                let item = conn.object_server().interface::<_, Item>("/StatusNotifierItem").await?;
                Item::new_icon(item.signal_emitter()).await?;
                Item::new_tool_tip(item.signal_emitter()).await?;
                let menu = conn.object_server().interface::<_, Menu>("/MenuBar").await?;
                Menu::layout_updated(menu.signal_emitter(), revision, 0).await
            })
            .map_err(err);
        }
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let state = self.state.clone();
        let conn = zbus::block_on(async {
            let conn = zbus::conn::Builder::session()
                .name(name.as_str())
                .serve_at("/StatusNotifierItem", Item(state.clone()))
                .serve_at("/MenuBar", Menu(state))
                .build()
                .await?;
            conn.call_method(
                Some("org.kde.StatusNotifierWatcher"),
                "/StatusNotifierWatcher",
                Some("org.kde.StatusNotifierWatcher"),
                "RegisterStatusNotifierItem",
                &(name.as_str(),),
            )
            .await?;
            Ok::<_, zbus::Error>(conn)
        });
        match conn {
            Ok(conn) => self.conn = Some(conn),
            Err(sni) => {
                let state = self.state.clone();
                let x = crate::bridge_linux::xtray::XTray::start(t, move |e| push(&state, e)).map_err(|x| {
                    Unsupported::new(
                        "the tray",
                        format!("the desktop has no StatusNotifier host ({sni}), and {}", x.reason),
                    )
                })?;
                self.xembed = Some(x);
            }
        }
        Ok(())
    }
}

/// `file://` URIs from the portal to paths (percent-decoding, local files only).
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let Some(h) = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Runs the portal's open or save dialog and waits for the answer.
pub(crate) fn file_dialog(parent: &str, d: &FileDialog) -> Result<Vec<PathBuf>, Unsupported> {
    use std::pin::Pin;
    let err =
        |e: zbus::Error| Unsupported::new("file dialogs", format!("the desktop portal did not answer: {e}"));
    zbus::block_on(async {
        let conn = zbus::Connection::session().await?;
        let token = format!("remedyui{}", std::process::id());
        let sender =
            conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
        let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        // Subscribe before asking, so the answer cannot be missed.
        let request = zbus::Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            handle.as_str(),
            "org.freedesktop.portal.Request",
        )
        .await?;
        let mut responses = request.receive_signal("Response").await?;
        let mut opts: HashMap<&str, Variant<'_>> = HashMap::new();
        opts.insert("handle_token", Variant::from(token.as_str()));
        opts.insert("modal", Variant::from(true));
        if d.multiple {
            opts.insert("multiple", Variant::from(true));
        }
        if d.folders {
            opts.insert("directory", Variant::from(true));
        }
        let filters: Vec<(String, Vec<(u32, String)>)> = d
            .filters
            .iter()
            .map(|(name, exts)| {
                // An extension of `*` is every file, with or without an extension.
                let globs =
                    exts.iter().map(|e| (0u32, if e == "*" { "*".to_owned() } else { format!("*.{e}") }));
                (name.clone(), globs.collect())
            })
            .collect();
        if !filters.is_empty() {
            opts.insert("filters", Variant::from(filters));
        }
        let folder = d.directory.as_ref().map(|p| {
            use std::os::unix::ffi::OsStrExt;
            let mut b = p.as_os_str().as_bytes().to_vec();
            b.push(0);
            b
        });
        if let Some(f) = &folder {
            opts.insert("current_folder", Variant::from(f.as_slice()));
        }
        if let (true, Some(name)) = (d.save, &d.file_name) {
            opts.insert("current_name", Variant::from(name.as_str()));
        }
        let chooser = zbus::Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
        )
        .await?;
        let method = if d.save { "SaveFile" } else { "OpenFile" };
        let _: zbus::OwnedObjectPath = chooser.call(method, &(parent, d.title.as_str(), opts)).await?;
        let msg =
            std::future::poll_fn(|cx| futures_core::Stream::poll_next(Pin::new(&mut responses), cx)).await;
        let Some(msg) = msg else { return Ok(Vec::new()) };
        let (code, results): (u32, HashMap<String, OwnedVariant>) = msg.body().deserialize()?;
        if code != 0 {
            return Ok(Vec::new()); // cancelled
        }
        let uris: Vec<String> =
            results.get("uris").and_then(|v| Vec::<String>::try_from(v.clone()).ok()).unwrap_or_default();
        Ok(uris.iter().filter_map(|u| uri_to_path(u)).collect())
    })
    .map_err(err)
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn access_keys_become_dbusmenu_underscores() {
        assert_eq!(dbusmenu_label("&Open"), "_Open");
        assert_eq!(dbusmenu_label("Salt && &Pepper"), "Salt & _Pepper");
        assert_eq!(dbusmenu_label("snake_case"), "snake__case");
        assert_eq!(dbusmenu_label("Plain"), "Plain");
    }

    #[test]
    fn submenus_and_checks_have_stable_ids() {
        let items = vec![
            TrayItem::Item { id: 9, label: "Open".into(), enabled: true, default: true },
            TrayItem::submenu("Server", vec![TrayItem::item(4, "Restart"), TrayItem::check(5, "Logs", true)]),
            TrayItem::Separator,
        ];
        let all = flat(&items);
        assert_eq!(
            all.iter().map(|i| i.label()).collect::<Vec<_>>(),
            ["Open", "Server", "Restart", "Logs", ""]
        );
        let [_, server_item, _, logs, _] = all.as_slice() else { unreachable!("five entries") };
        assert_eq!(logs.chosen_id(), Some(5), "dbusmenu id 4 chooses the app's 5");
        let p = props(Some(logs));
        assert_eq!(p.get("toggle-type").and_then(|v| <&str>::try_from(v).ok()), Some("checkmark"));
        assert_eq!(p.get("toggle-state").and_then(|v| i32::try_from(v).ok()), Some(1));
        let root = layout(&items, 0);
        assert_eq!(root.children.len(), 3);
        let server = layout(&items, 2);
        assert_eq!(server.id, 2);
        assert_eq!(server.children.len(), 2, "the submenu's own layout");
        assert_eq!(
            props(Some(server_item)).get("children-display").and_then(|v| <&str>::try_from(v).ok()),
            Some("submenu")
        );
    }
}
