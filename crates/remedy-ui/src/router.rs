//! Routes: which page the app shows, as an address (`/settings/2`), with back and forward.
//!
//! A [`Router`] keeps the history of routes, titles the window after the route, and
//! remembers each entry's scroll position. In a browser page the route is the address's
//! fragment (`#/settings/2`): the browser's back and forward buttons, its history and links
//! opened in a new tab all work, and a page opened at `#/settings/2` starts there. On the
//! desktop, links in the app's URL scheme ([`Router::follow_links`]) open their page, also
//! when the app is launched again while running.
//!
//! ```
//! use remedy_ui::router::Router;
//!
//! let router = Router::new("/home");
//! router.push("/settings/audio");
//! let route = router.route_untracked();
//! assert_eq!(route.matches("/settings/:id").and_then(|p| p.get("id").cloned()), Some("audio".into()));
//! assert!(router.back());
//! assert_eq!(router.route_untracked().path(), "/home");
//! ```

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::Vec2;
use remedy_ui_core::input::{Buttons, Key, KeyEvent, PointerPhase};
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

use crate::{Window, WindowSpec};

#[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
use remedy_ui_bridge_web::history;

/// True in a browser page, where the browser's history holds the routes.
const IN_PAGE: bool = cfg!(all(feature = "web", target_arch = "wasm32", target_os = "unknown"));

thread_local! {
    /// Each live [`Router::view`]'s reload count.
    static RELOADS: RefCell<Vec<Signal<u64>>> = const { RefCell::new(Vec::new()) };
}

/// Rebuilds every [`Router::view`] on this thread ([`App::reload`](crate::App::reload)).
#[cfg_attr(all(feature = "web", target_arch = "wasm32", target_os = "unknown"), allow(dead_code))]
pub(crate) fn reload_views() {
    // Taken out first: a rebuilt page may make views of its own.
    let views = RELOADS.with(|r| std::mem::take(&mut *r.borrow_mut()));
    let alive: Vec<Signal<u64>> = views.into_iter().filter(|s| s.try_update(|n| *n += 1).is_ok()).collect();
    RELOADS.with(|r| r.borrow_mut().extend(alive));
}

/// A page's address: its path (`/browse/Movies/Alien`) and query (`sort=year`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    path: String,
    query: String,
}

impl Route {
    /// Reads a route from a path (`/settings/2`), a fragment (`#/settings/2`), a page address
    /// (`https://host/app/#/settings/2`) or a link in the app's scheme (`portal://settings/2`).
    /// Empty segments are dropped, so `settings//2/` is `/settings/2`.
    pub fn parse(s: &str) -> Route {
        let s = s.trim();
        let s = match (s.split_once('#'), s.find("://")) {
            (Some((_, fragment)), _) => fragment,
            (None, Some(i)) => &s[i + 3..],
            (None, None) => s,
        };
        let (path, query) = s.split_once('?').unwrap_or((s, ""));
        let segments: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        Route { path: format!("/{}", segments.join("/")), query: query.to_owned() }
    }

    /// The path, as written (percent-encoded): `/browse/My%20Movies`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The path's segments, decoded: `["browse", "My Movies"]`.
    pub fn segments(&self) -> Vec<String> {
        self.path.split('/').filter(|p| !p.is_empty()).map(decode).collect()
    }

    /// A query parameter, decoded (`sort` in `/browse?sort=year`).
    pub fn query(&self, key: &str) -> Option<String> {
        self.query
            .split('&')
            .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
            .find_map(|(k, v)| (decode(k) == key).then(|| decode(v)))
    }

    /// The parameters when the route fits `pattern`: `:name` takes one segment, a final
    /// `*name` takes the rest (joined with `/`, possibly empty), anything else must be equal.
    /// `/browse/*path` fits `/browse/TV/Show` with `path` = `TV/Show`.
    pub fn matches(&self, pattern: &str) -> Option<BTreeMap<String, String>> {
        let segs = self.segments();
        let mut params = BTreeMap::new();
        let pats: Vec<&str> = pattern.split('/').filter(|p| !p.is_empty()).collect();
        for (i, p) in pats.iter().enumerate() {
            if let Some(name) = p.strip_prefix('*') {
                params.insert(name.to_owned(), segs.get(i..).unwrap_or_default().join("/"));
                return Some(params);
            }
            let seg = segs.get(i)?;
            match p.strip_prefix(':') {
                Some(name) => {
                    params.insert(name.to_owned(), seg.clone());
                }
                None if seg == p => {}
                None => return None,
            }
        }
        (segs.len() == pats.len()).then_some(params)
    }

    /// The link to it in a page: `#/settings/2?tab=audio`.
    pub fn href(&self) -> String {
        format!("#{self}")
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.query.is_empty() {
            write!(f, "{}", self.path)
        } else {
            write!(f, "{}?{}", self.path, self.query)
        }
    }
}

/// Percent-decoding (`%20` and `+` in queries are left to the caller: `+` stays `+`).
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |at: usize| b.get(at).and_then(|c| (*c as char).to_digit(16));
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Entry {
    route: Route,
    scroll: Vec2,
    /// The app's own state for the entry ([`Router::push_with`]).
    state: String,
}

type TitleFn = Rc<dyn Fn(&Route) -> String>;
type BuildFn = Box<dyn Fn(&Ui, &Window, &Router)>;

/// How links opened elsewhere get their own window ([`Router::open_windows`]).
struct NewWindows {
    spec: WindowSpec,
    build: BuildFn,
}

struct Inner {
    entries: Vec<Entry>,
    index: usize,
    title: Option<TitleFn>,
    window: Option<Window>,
    /// The current page's scroll container ([`Router::remember_scroll`]).
    scroller: Option<El>,
    windows: Option<Rc<NewWindows>>,
    /// The path routes live under in a browser page's address, when they are not in its
    /// fragment ([`Router::for_window_at`]).
    base: Option<String>,
}

/// The app's routes: the current one (a signal pages follow), the history behind and ahead
/// of it, each route's window title and each entry's scroll position.
///
/// A `nav::NavStack` inside a routed page keeps working: its back keys
/// pop its own pages first, and reach the router once it is on its first page.
#[derive(Clone)]
pub struct Router {
    inner: Rc<RefCell<Inner>>,
    route: Signal<Route>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let i = self.inner.borrow();
        write!(f, "Router({} of {})", i.index + 1, i.entries.len())
    }
}

impl Router {
    /// A router starting at `start`, keeping its history in memory (tests, or a router that
    /// is not the window's).
    pub fn new(start: &str) -> Router {
        let route = Route::parse(start);
        Router {
            inner: Rc::new(RefCell::new(Inner {
                entries: vec![Entry { route: route.clone(), scroll: Vec2::ZERO, state: String::new() }],
                index: 0,
                title: None,
                window: None,
                scroller: None,
                windows: None,
                base: None,
            })),
            route: Signal::new(route),
        }
    }

    /// The router of `window`: titles it after the route. In a browser page it starts at the
    /// address's fragment when there is one (else at `start`, written to the address) and
    /// follows the browser's history both ways. After a reload it has its entries back, in
    /// their places and with their scroll positions: each history entry carries the list
    /// in its `history.state`, written again as the page is put away.
    pub fn for_window(window: &Window, start: &str) -> Router {
        Router::start(window, start, None)
    }

    /// The router of `window`, as [`Router::for_window`] makes it, with a browser page's
    /// routes in the address's path under `base`, not its fragment: `/app` gives
    /// `https://host/app/settings/2` (an empty base, `https://host/settings/2`). Links and
    /// the history are plain addresses, as search engines and people expect; the server must
    /// answer every path under the base with the page. Outside a browser it is
    /// [`Router::for_window`].
    pub fn for_window_at(window: &Window, start: &str, base: &str) -> Router {
        Router::start(window, start, Some(base))
    }

    #[cfg_attr(
        not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")),
        allow(unused_variables)
    )]
    fn start(window: &Window, start: &str, base: Option<&str>) -> Router {
        let me = Router::new(start);
        me.inner.borrow_mut().window = Some(window.clone());
        me.inner.borrow_mut().base = base.map(|b| b.trim_end_matches('/').to_owned());
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        {
            history::use_paths(base);
            let fragment = history::fragment();
            let here = (!fragment.is_empty()).then(|| Route::parse(&fragment));
            // The list this entry carried, when it is this app's and still names this page.
            let restored = history::index().zip(history::entries()).filter(|(at, list)| {
                list.get(*at as usize)
                    .is_some_and(|(r, ..)| here.as_ref().is_none_or(|h| Route::parse(r) == *h))
            });
            {
                let mut i = me.inner.borrow_mut();
                match restored {
                    Some((at, list)) => {
                        i.entries = list
                            .into_iter()
                            .map(|(r, x, y, state)| Entry {
                                route: Route::parse(&r),
                                scroll: Vec2::new(x, y),
                                state,
                            })
                            .collect();
                        i.index = at as usize;
                    }
                    None => {
                        if let (Some(h), Some(e)) = (here, i.entries.first_mut()) {
                            e.route = h;
                        }
                    }
                }
            }
            let (index, route) = {
                let i = me.inner.borrow();
                (i.index, i.entries.get(i.index).map(|e| e.route.clone()).unwrap_or_default())
            };
            me.route.set(route);
            me.write_address(index, true);
            let (m, m2) = (me.clone(), me.clone());
            history::listen(move |fragment, index| m.popped(&fragment, index), move || m2.store_current());
        }
        me
    }

    /// Ctrl (Cmd) and middle clicks on [`Router::link_to`] links open the route in a new
    /// window on the desktop: `spec` opened by [`App::open`](crate::App::open), with a
    /// router of its own starting at the route (which opens further windows the same way)
    /// handed to `build` to fill it. Without this they navigate in place; in a browser page
    /// they open a new tab whatever this says.
    pub fn open_windows(&self, spec: WindowSpec, build: impl Fn(&Ui, &Window, &Router) + 'static) {
        self.inner.borrow_mut().windows = Some(Rc::new(NewWindows { spec, build: Box::new(build) }));
    }

    /// Opens `path` in a new window ([`Router::open_windows`]), a new tab in a browser page,
    /// or in place when the app opens no windows for links.
    pub fn open_elsewhere(&self, path: &str) {
        let route = Route::parse(path);
        if IN_PAGE {
            open_in_new_tab(&route.href());
            return;
        }
        let (windows, app) = {
            let i = self.inner.borrow();
            (i.windows.clone(), i.window.as_ref().map(|w| w.app().clone()))
        };
        let (Some(windows), Some(app)) = (windows, app) else {
            self.push(path);
            return;
        };
        let (start, nw) = (route.to_string(), windows.clone());
        let _ = app.open(windows.spec.clone(), move |ui, window| {
            let router = Router::for_window(window, &start);
            router.inner.borrow_mut().windows = Some(nw.clone());
            (nw.build)(ui, window, &router);
        });
    }

    /// The current route (tracked: what reads it follows it).
    pub fn route(&self) -> Route {
        self.route.get()
    }

    /// The current route, without tracking.
    pub fn route_untracked(&self) -> Route {
        self.route.get_untracked().unwrap_or_default()
    }

    /// The route as a signal.
    pub fn signal(&self) -> Signal<Route> {
        self.route
    }

    /// Shows `path` as a new history entry (after the current one; entries ahead of it are
    /// dropped). Nothing happens when it is the current route.
    pub fn push(&self, path: &str) {
        if Route::parse(path) != self.route_untracked() {
            self.push_with(path, "");
        }
    }

    /// Shows `path` as a new history entry that carries `state`: something of the app's own
    /// that belongs to this visit and not to the address (the item a list was opened from, a
    /// form's draft, where a wizard came from). [`Router::state`] reads it while the entry
    /// is shown, also after back and forward return to it and, in a browser page, after a
    /// reload. Unlike [`Router::push`] it adds an entry even for the current route.
    pub fn push_with(&self, path: &str, state: &str) {
        let route = Route::parse(path);
        self.store_current();
        let index = {
            let mut i = self.inner.borrow_mut();
            let at = i.index + 1;
            i.entries.truncate(at);
            i.entries.push(Entry { route, scroll: Vec2::ZERO, state: state.to_owned() });
            i.index = at;
            at
        };
        self.write_address(index, false);
        self.show();
    }

    /// The state the entry shown carries ([`Router::push_with`], [`Router::set_state`]);
    /// empty when it has none.
    pub fn state(&self) -> String {
        let i = self.inner.borrow();
        i.entries.get(i.index).map(|e| e.state.clone()).unwrap_or_default()
    }

    /// Gives the entry shown another state, without moving.
    pub fn set_state(&self, state: &str) {
        let index = {
            let mut i = self.inner.borrow_mut();
            let at = i.index;
            if let Some(e) = i.entries.get_mut(at) {
                e.state = state.to_owned();
            }
            at
        };
        self.write_address(index, true);
    }

    /// The address a link to `path` has: `#/path`, or under the base of
    /// [`Router::for_window_at`] a plain path.
    pub fn href(&self, path: &str) -> String {
        let route = Route::parse(path);
        match &self.inner.borrow().base {
            Some(base) => format!("{base}{}", route.href().trim_start_matches('#')),
            None => route.href(),
        }
    }

    /// Shows `path` in place of the current entry (no new history entry).
    pub fn replace(&self, path: &str) {
        let index = {
            let mut i = self.inner.borrow_mut();
            let at = i.index;
            if let Some(e) = i.entries.get_mut(at) {
                *e = Entry { route: Route::parse(path), scroll: Vec2::ZERO, state: String::new() };
            }
            at
        };
        self.write_address(index, true);
        self.show();
    }

    /// Goes to the previous entry. False on the first one.
    pub fn back(&self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.step(-1);
        true
    }

    /// Goes to the next entry (after [`Router::back`]). False on the last one.
    pub fn forward(&self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.step(1);
        true
    }

    /// True when there is an entry behind the current one.
    pub fn can_go_back(&self) -> bool {
        self.inner.borrow().index > 0
    }

    /// True when there is an entry ahead of the current one.
    pub fn can_go_forward(&self) -> bool {
        let i = self.inner.borrow();
        i.index + 1 < i.entries.len()
    }

    /// How many entries the history holds, and which one is shown (from 0).
    pub fn position(&self) -> (usize, usize) {
        let i = self.inner.borrow();
        (i.index, i.entries.len())
    }

    /// Names each route: the window's title (the page's, in a browser) follows it.
    pub fn set_title(&self, f: impl Fn(&Route) -> String + 'static) {
        self.inner.borrow_mut().title = Some(Rc::new(f));
        self.apply_title();
    }

    /// The current route's title ([`Router::set_title`]; empty without one).
    pub fn title(&self) -> String {
        let f = self.inner.borrow().title.clone();
        f.map(|f| f(&self.route_untracked())).unwrap_or_default()
    }

    /// Makes `scroller` (a scroll container) the current page's: its position is kept with
    /// the history entry and restored when back or forward returns to it.
    pub fn remember_scroll(&self, scroller: &El) {
        let saved = {
            let mut i = self.inner.borrow_mut();
            i.scroller = Some(scroller.clone());
            i.entries.get(i.index).map_or(Vec2::ZERO, |e| e.scroll)
        };
        if saved != Vec2::ZERO {
            scroller.scroll_to(saved);
        }
    }

    /// The scroll position kept for the current entry (updated when the router leaves it).
    pub fn saved_scroll(&self) -> Vec2 {
        let i = self.inner.borrow();
        i.entries.get(i.index).map_or(Vec2::ZERO, |e| e.scroll)
    }

    /// Opens a deep link (`portal://settings/2`, `#/settings/2`) as a new entry, and brings
    /// the router's window to the front.
    pub fn open_link(&self, url: &str) {
        self.push(&Route::parse(url).to_string());
        let window = self.inner.borrow().window.clone();
        if let Some(w) = window {
            w.focus();
        }
    }

    /// Opens links in `schemes` (`portal://settings/2`) as routes: the one the app started
    /// with, and with [`AppConfig::single_instance`](crate::AppConfig::single_instance)
    /// every one opened while it runs, including from a second launch. Uses
    /// [`App::on_url`](crate::App::on_url), so register the scheme the same way.
    #[cfg(feature = "desktop")]
    pub fn follow_links(&self, app: &crate::App, schemes: &[&str]) {
        let me = self.clone();
        app.on_url(schemes, move |url| me.open_link(&url));
    }

    /// The element that shows the current route's page, built by `page` (again on every
    /// route change and on [`App::reload`](crate::App::reload)). Alt+Left and Alt+Right,
    /// the system back key (Android's back) and the mouse's back and forward buttons move
    /// through the history (in a browser page too: the bridge cancels the browser's own move
    /// on those buttons, so it happens once); a back the router cannot take is left to the
    /// window.
    pub fn view(&self, ui: &Ui, page: impl Fn(&Route) -> El + 'static) -> El {
        let outlet = w::column(ui).grow(1.0).fill_width();
        let (me, o) = (self.clone(), outlet.clone());
        let reloads = Signal::new(0u64);
        RELOADS.with(|r| r.borrow_mut().push(reloads));
        let outlet = outlet.bind(move || {
            let route = me.route.get();
            let _ = reloads.get();
            remedy_ui_core::reactive::untrack(|| {
                o.clear();
                me.inner.borrow_mut().scroller = None;
                let _ = o.clone().child(page(&route).grow(1.0));
            });
        });
        let (k, p) = (self.clone(), self.clone());
        // On the window's root: the keys arrive also when the page that had focus is gone.
        // A NavStack inside the page hears them first and pops its own pages.
        let _ = ui.root().on_key(move |e| k.key(e)).on_pointer(move |e| {
            if e.phase != PointerPhase::Down {
                return false;
            }
            if e.buttons.contains(Buttons::BACK) {
                return p.back();
            }
            e.buttons.contains(Buttons::FORWARD) && p.forward()
        });
        outlet
    }

    /// A link to `path` labelled `label`: activating it pushes the route. Screen readers
    /// hear a link; in a browser page it is an `<a href="#/path">` in the accessibility
    /// mirror. A middle or Ctrl (Cmd) click opens it elsewhere ([`Router::open_elsewhere`]):
    /// a new tab in a browser page, a new window on the desktop when the app opens them
    /// ([`Router::open_windows`]), else in place.
    #[track_caller]
    pub fn link_to(&self, ui: &Ui, path: &str, label: impl Into<String>) -> El {
        let label = label.into();
        let href = self.href(path);
        let elsewhere = Rc::new(Cell::new(false));
        let (me, el, target) = (self.clone(), elsewhere.clone(), path.to_owned());
        let (me2, target2) = (self.clone(), path.to_owned());
        w::link(ui, label.clone(), move || {
            if el.replace(false) {
                me.open_elsewhere(&target);
            } else {
                me.push(&target);
            }
        })
        .semantics(Semantics::new(Role::Link).label(label).action(Action::Click).url(href))
        .on_pointer(move |e| {
            if e.phase != PointerPhase::Down {
                return false;
            }
            if e.buttons.contains(Buttons::MIDDLE) {
                me2.open_elsewhere(&target2);
                return true;
            }
            elsewhere.set(e.modifiers.ctrl || e.modifiers.meta);
            false
        })
    }

    fn key(&self, e: &KeyEvent) -> bool {
        if !e.pressed {
            return false;
        }
        match &e.key {
            Key::Back => self.back(),
            Key::ArrowLeft if e.modifiers.alt => self.back(),
            Key::ArrowRight if e.modifiers.alt => self.forward(),
            _ => false,
        }
    }

    /// Moves `delta` entries: through the browser in a page (its popstate brings the
    /// route), directly elsewhere.
    fn step(&self, delta: i32) {
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        {
            self.store_current();
            history::go(delta);
        }
        #[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
        {
            let to = self.inner.borrow().index.saturating_add_signed(delta as isize);
            self.go_to(to);
        }
    }

    fn go_to(&self, index: usize) {
        self.save_scroll();
        {
            let mut i = self.inner.borrow_mut();
            if index >= i.entries.len() {
                return;
            }
            i.index = index;
        }
        self.show();
    }

    /// The browser moved through its history, or the fragment was edited.
    #[cfg_attr(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")), allow(dead_code))]
    fn popped(&self, fragment: &str, index: Option<u32>) {
        let route = Route::parse(fragment);
        let known = index
            .map(|i| i as usize)
            .filter(|i| self.inner.borrow().entries.get(*i).is_some_and(|e| e.route == route));
        match known {
            Some(i) => self.go_to(i),
            None => {
                // An address typed in or a link the page did not write: a new entry here.
                self.save_scroll();
                let at = {
                    let mut i = self.inner.borrow_mut();
                    let at = i.index + 1;
                    i.entries.truncate(at);
                    i.entries.push(Entry { route, scroll: Vec2::ZERO, state: String::new() });
                    i.index = at;
                    at
                };
                self.write_address(at, true);
                self.show();
            }
        }
    }

    fn save_scroll(&self) {
        let mut i = self.inner.borrow_mut();
        if let Some(s) = i.scroller.clone() {
            let at = i.index;
            if let Some(e) = i.entries.get_mut(at) {
                e.scroll = s.ui().scroll_offset(s.id());
            }
        }
    }

    fn show(&self) {
        let (route, scroll, scroller) = {
            let i = self.inner.borrow();
            let Some(e) = i.entries.get(i.index) else { return };
            (e.route.clone(), e.scroll, i.scroller.clone())
        };
        self.route.set(route);
        // A scroller that outlives the change (outside the page) goes back to the entry's
        // position; one the page rebuilt was placed by `remember_scroll`.
        let current = self.inner.borrow().scroller.clone();
        if let (Some(old), Some(now)) = (scroller, current)
            && old.id() == now.id()
        {
            now.scroll_to(scroll);
        }
        self.apply_title();
    }

    fn apply_title(&self) {
        let title = self.title();
        let window = self.inner.borrow().window.clone();
        if let (Some(w), false) = (window, title.is_empty()) {
            w.set_title(title);
        }
    }

    #[cfg_attr(
        not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")),
        allow(unused_variables)
    )]
    fn write_address(&self, index: usize, replace: bool) {
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        {
            let i = self.inner.borrow();
            let list: Vec<(String, f32, f32, String)> = i
                .entries
                .iter()
                .map(|e| (e.route.to_string(), e.scroll.x, e.scroll.y, e.state.clone()))
                .collect();
            if let Some((route, ..)) = list.get(index) {
                history::write(route, index as u32, &list, replace);
            }
        }
    }

    /// Keeps the current entry's scroll position and writes the list into its history
    /// entry, before the page leaves it (a push, a step, the page put away or reloaded).
    fn store_current(&self) {
        self.save_scroll();
        let index = self.inner.borrow().index;
        self.write_address(index, true);
    }
}

#[cfg_attr(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")), allow(unused_variables))]
fn open_in_new_tab(href: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
    history::open_in_new_tab(href.trim_start_matches('#'));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_form_of_address() {
        for s in [
            "/settings/2",
            "#/settings/2",
            "settings//2/",
            "portal://settings/2",
            "https://h/app/#/settings/2",
        ] {
            assert_eq!(Route::parse(s).path(), "/settings/2", "{s}");
        }
        let r = Route::parse("#/browse/My%20Movies/Alien?sort=year&q=a%26b");
        assert_eq!(r.segments(), ["browse", "My Movies", "Alien"]);
        assert_eq!(r.query("sort").as_deref(), Some("year"));
        assert_eq!(r.query("q").as_deref(), Some("a&b"));
        assert_eq!(r.href(), "#/browse/My%20Movies/Alien?sort=year&q=a%26b");
        assert_eq!(Route::parse("").path(), "/");
    }

    #[test]
    fn matches_patterns() {
        let r = Route::parse("/browse/TV/Show");
        assert_eq!(
            r.matches("/browse/*path").and_then(|p| p.get("path").cloned()).as_deref(),
            Some("TV/Show")
        );
        assert_eq!(
            Route::parse("/browse").matches("/browse/*path").and_then(|p| p.get("path").cloned()).as_deref(),
            Some("")
        );
        assert_eq!(
            Route::parse("/settings/7")
                .matches("/settings/:id")
                .and_then(|p| p.get("id").cloned())
                .as_deref(),
            Some("7")
        );
        assert!(r.matches("/browse/:x").is_none());
        assert!(r.matches("/home").is_none());
        assert!(Route::parse("/").matches("/").is_some());
    }

    #[test]
    fn push_replace_back_forward() {
        let r = Router::new("/home");
        r.push("/browse");
        r.push("/browse");
        assert_eq!(r.position(), (1, 2), "pushing the current route adds nothing");
        r.push("/settings/1");
        assert!(r.back());
        assert_eq!(r.route_untracked().path(), "/browse");
        assert!(r.forward());
        assert_eq!(r.route_untracked().path(), "/settings/1");
        assert!(!r.forward());
        r.replace("/settings/2");
        assert_eq!(r.position(), (2, 3));
        assert!(r.back() && r.back());
        assert!(!r.back());
        assert_eq!(r.route_untracked().path(), "/home");
        r.push("/server");
        assert_eq!(r.position(), (1, 2), "a push drops what was ahead");
        assert!(!r.can_go_forward());
    }

    #[test]
    fn titles_follow_the_route() {
        let r = Router::new("/home");
        r.set_title(|route| match route.segments().first().map(String::as_str) {
            Some("settings") => "Settings".into(),
            _ => "Home".into(),
        });
        assert_eq!(r.title(), "Home");
        r.push("/settings/x");
        assert_eq!(r.title(), "Settings");
    }

    #[test]
    fn entries_carry_the_apps_state_through_back_and_forward() {
        let r = Router::new("/list");
        r.push_with("/item/7", "from=3");
        assert_eq!(r.state(), "from=3");
        r.set_state("from=3;draft=hello");
        r.push("/other");
        assert_eq!(r.state(), "");
        assert!(r.back());
        assert_eq!(
            (r.route().path().to_owned(), r.state()),
            ("/item/7".to_owned(), "from=3;draft=hello".to_owned())
        );
        // The same route again is a new entry when it carries a state.
        r.push_with("/item/7", "again");
        assert_eq!(r.position(), (2, 3));
        assert_eq!(r.href("/item/7"), "#/item/7");
    }
}
