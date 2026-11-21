//! Menus as desktops have them: items with check and radio marks, submenus, separators,
//! shortcuts, icons and access keys (mnemonics); popup and context menus; and the menu bar.
//!
//! One open menu and its submenus are a *chain* on one full-window layer: a press outside
//! every menu of the chain closes it, and each menu of the chain takes the keyboard while it
//! has focus (WAI-ARIA's menu pattern, as Windows, macOS and GNOME menus behave).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Insets, Point, Rect};
use remedy_ui_input::{Buttons, Chord, Key, KeyEvent, PointerPhase};
use remedy_ui_layout::Align;
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_style::States;
use remedy_ui_view::{El, IntoProp, Prop, RichText, TextStylePatch, Ui};

use crate::icons::{Icon, icon};
use crate::menu_bar::Bar;
use crate::overlays::{fill_window, fit_beside, fit_near_dir, place_fitted};

/// How long the pointer rests on an item before its submenu opens (or a sibling's closes).
const SUBMENU_DELAY: Duration = Duration::from_millis(300);
/// Space between a menu and what it opened from.
const GAP: f32 = 2.0;

type Items = Rc<dyn Fn() -> Vec<MenuItem>>;

#[derive(Clone)]
enum Kind {
    Action,
    Check(Prop<bool>),
    Radio(Prop<bool>),
    Submenu(Items),
    Separator,
}

#[derive(Clone)]
enum Shortcut {
    Text(Prop<String>),
    Chords(Rc<dyn Fn() -> Vec<Chord>>),
}

/// One menu entry: a command, a check or radio item, a submenu, or a separator line. Build it
/// with a constructor and refine it with the builder methods:
///
/// ```
/// use remedy_ui_reactive::Signal;
/// use remedy_ui_widgets_core::MenuItem;
///
/// let book_mode = Signal::new(false);
/// let fit = Signal::new("width");
/// let view = MenuItem::submenu(
///     "View",
///     vec![
///         MenuItem::new("Zoom In", || {}).shortcut("Ctrl++").mnemonic('i'),
///         MenuItem::separator(),
///         MenuItem::radio_signal("Fit Width", fit, "width").mnemonic('w'),
///         MenuItem::radio_signal("Fit Page", fit, "page").mnemonic('p'),
///         MenuItem::separator(),
///         MenuItem::check_signal("Book Mode", book_mode).mnemonic('b'),
///     ],
/// )
/// .mnemonic('v');
/// assert!(!view.is_separator());
/// ```
#[derive(Clone)]
pub struct MenuItem {
    kind: Kind,
    label: Prop<String>,
    danger: bool,
    action: Rc<dyn Fn()>,
    disabled: Prop<bool>,
    shortcut: Option<Shortcut>,
    icon: Option<Icon>,
    mnemonic: Option<char>,
}

impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            Kind::Separator => f.write_str("MenuItem(separator)"),
            _ => write!(f, "MenuItem({})", self.label_text()),
        }
    }
}

impl MenuItem {
    fn base(kind: Kind, label: Prop<String>, action: Rc<dyn Fn()>) -> Self {
        MenuItem {
            kind,
            label,
            danger: false,
            action,
            disabled: Prop::Value(false),
            shortcut: None,
            icon: None,
            mnemonic: None,
        }
    }

    /// An entry that runs `action`.
    pub fn new(label: impl Into<String>, action: impl Fn() + 'static) -> Self {
        Self::base(Kind::Action, Prop::Value(label.into()), Rc::new(action))
    }

    /// A line between groups of entries. Keyboard focus and type-ahead pass over it.
    pub fn separator() -> Self {
        Self::base(Kind::Separator, Prop::Value(String::new()), Rc::new(|| {}))
    }

    /// An entry that opens `items` beside itself: on Enter, Space or the arrow towards the
    /// reading direction's end (Right, or Left in a right-to-left layout), on a click, or
    /// after the pointer rests on it a moment. As a menu bar's entry, it is a top-level menu.
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::submenu_with(label, move || items.clone())
    }

    /// A submenu whose entries are built each time it opens (a list of recent files).
    pub fn submenu_with(label: impl Into<String>, items: impl Fn() -> Vec<MenuItem> + 'static) -> Self {
        Self::base(Kind::Submenu(Rc::new(items)), Prop::Value(label.into()), Rc::new(|| {}))
    }

    /// A check item showing `checked` (a value, a signal or a closure); choosing it calls
    /// `action` with the state it asks for (the opposite of the one shown).
    pub fn check(
        label: impl Into<String>,
        checked: impl IntoProp<bool>,
        action: impl Fn(bool) + 'static,
    ) -> Self {
        let checked = checked.into_prop();
        let c2 = checked.clone();
        Self::base(
            Kind::Check(checked),
            Prop::Value(label.into()),
            Rc::new(move || action(!untrack(|| c2.get()))),
        )
    }

    /// A check item that shows `on` and flips it when chosen.
    pub fn check_signal(label: impl Into<String>, on: Signal<bool>) -> Self {
        Self::check(label, on, move |v| on.set(v))
    }

    /// A radio item: one of a set of choices, checked when `checked` is; choosing it calls
    /// `action` (which picks it).
    pub fn radio(
        label: impl Into<String>,
        checked: impl IntoProp<bool>,
        action: impl Fn() + 'static,
    ) -> Self {
        Self::base(Kind::Radio(checked.into_prop()), Prop::Value(label.into()), Rc::new(action))
    }

    /// A radio item for `value` of the choice `choice`: checked while `choice` holds
    /// `value`, and setting it when chosen.
    pub fn radio_signal<T: Clone + PartialEq + 'static>(
        label: impl Into<String>,
        choice: Signal<T>,
        value: T,
    ) -> Self {
        let v2 = value.clone();
        Self::radio(label, move || choice.get() == value, move || choice.set(v2.clone()))
    }

    /// Replaces the label with `label`: a value, or a signal or closure that keeps it current
    /// (a command's label after the language changed).
    pub fn label(mut self, label: impl IntoProp<String>) -> Self {
        self.label = label.into_prop();
        self
    }

    /// Marks a destructive entry (shown in the danger color).
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// Disables the entry while `on` is true. A disabled entry stays in the menu and takes
    /// keyboard focus (so the person learns it exists, as WAI-ARIA asks), but choosing it does
    /// nothing.
    pub fn disabled(mut self, on: impl IntoProp<bool>) -> Self {
        self.disabled = on.into_prop();
        self
    }

    /// Shows `text` at the entry's end, the shortcut that does the same from anywhere
    /// ("Ctrl+O"). The menu only shows it; the shortcut itself is the app's (or its
    /// [`Commands`](https://docs.rs/remedy-ui-widgets-nav)'), so a menu made from commands
    /// shows their chords with [`MenuItem::chords`].
    pub fn shortcut(mut self, text: impl IntoProp<String>) -> Self {
        self.shortcut = Some(Shortcut::Text(text.into_prop()));
        self
    }

    /// Shows `chord` at the entry's end, written the platform's way in the person's language
    /// ("Ctrl+Shift+S", "Strg+Umschalt+S", "⇧⌘S").
    pub fn chord(self, chord: Chord) -> Self {
        self.chords(move || vec![chord.clone()])
    }

    /// Shows the first of the chords `chords` gives (read each time the menu shows them, so
    /// a rebinding shows at once), written as [`MenuItem::chord`] writes one.
    pub fn chords(mut self, chords: impl Fn() -> Vec<Chord> + 'static) -> Self {
        self.shortcut = Some(Shortcut::Chords(Rc::new(chords)));
        self
    }

    /// Shows `icon` before the label. Check and radio entries show their mark there instead.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The entry's access key: pressing `key` while its menu is open chooses it (Alt+`key` too,
    /// and for a menu bar's menu, from anywhere in the window). Its letter in the label is
    /// underlined while Alt is held and in menus opened from the keyboard; a key the label does
    /// not contain is shown after it, as "(F)".
    pub fn mnemonic(mut self, key: char) -> Self {
        self.mnemonic = key.to_lowercase().next();
        self
    }

    /// True for a separator line.
    pub fn is_separator(&self) -> bool {
        matches!(self.kind, Kind::Separator)
    }

    /// True for an entry that opens a submenu.
    pub fn is_submenu(&self) -> bool {
        matches!(self.kind, Kind::Submenu(_))
    }

    /// The label as it reads now.
    pub fn label_text(&self) -> String {
        untrack(|| self.label.get())
    }

    /// The entries of a submenu as they are now (empty for anything else).
    pub fn items(&self) -> Vec<MenuItem> {
        match &self.kind {
            Kind::Submenu(f) => f(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn is_disabled(&self) -> bool {
        untrack(|| self.disabled.get())
    }

    pub(crate) fn label_prop(&self) -> Prop<String> {
        self.label.clone()
    }

    pub(crate) fn disabled_prop(&self) -> Prop<bool> {
        self.disabled.clone()
    }

    pub(crate) fn access_key(&self) -> Option<char> {
        self.mnemonic
    }

    pub(crate) fn run(&self) {
        (self.action)();
    }

    fn role(&self) -> Role {
        match self.kind {
            Kind::Check(_) => Role::MenuItemCheckBox,
            Kind::Radio(_) => Role::MenuItemRadio,
            Kind::Separator => Role::Separator,
            Kind::Action | Kind::Submenu(_) => Role::MenuItem,
        }
    }

    fn checked(&self) -> Option<bool> {
        match &self.kind {
            Kind::Check(c) | Kind::Radio(c) => Some(c.get()),
            _ => None,
        }
    }

    fn starts_with(&self, c: char) -> bool {
        let label = self.label_text();
        label.trim_start().chars().next().is_some_and(|f| f.to_lowercase().eq(c.to_lowercase()))
    }
}

/// Alt is held where access keys exist (not on Apple platforms): they show underlined.
pub(crate) fn alt_shows_keys(ui: &Ui) -> bool {
    ui.alt_held() && ui.platform() != remedy_ui_input::Platform::Apple
}

/// The shortcut text of `s` for `ui`'s platform and language.
fn shortcut_text(ui: &Ui, s: &Shortcut) -> String {
    match s {
        Shortcut::Text(t) => t.get(),
        Shortcut::Chords(f) => {
            f().first().map(|c| c.label_in(ui.platform(), &ui.locale())).unwrap_or_default()
        }
    }
}

/// `label` with the access key `key` underlined when `show` is true: its letter where a word
/// starts with it, else its first occurrence. A key the label lacks is written after it ("ファイル (F)"), shown or not, so the width does not jump when Alt goes down.
pub(crate) fn mnemonic_text(label: &str, key: Option<char>, show: bool) -> RichText {
    let Some(key) = key else { return RichText::plain(label) };
    let mut underline = TextStylePatch::default();
    underline.underline = Some(true);
    // The key's letter at the start of a word ("Save &As"), else its first place ("E&xit").
    let same = |c: char| c.to_lowercase().eq(key.to_lowercase());
    let mut prev: Option<char> = None;
    let mut first = None;
    let mut at_word = None;
    for (i, c) in label.char_indices() {
        if same(c) {
            first = first.or(Some((i, c)));
            if prev.is_none_or(|p| !p.is_alphanumeric()) {
                at_word = Some((i, c));
                break;
            }
        }
        prev = Some(c);
    }
    let found = at_word.or(first);
    match found {
        Some((at, c)) if show => {
            let end = at + c.len_utf8();
            RichText::new()
                .text(label.get(..at).unwrap_or_default())
                .styled(label.get(at..end).unwrap_or_default(), underline)
                .text(label.get(end..).unwrap_or_default())
        }
        Some(_) => RichText::plain(label),
        None => {
            let shown = key.to_uppercase().to_string();
            let t = RichText::new().text(label).text(" (");
            let t = if show { t.styled(&shown, underline) } else { t.text(&shown) };
            t.text(")")
        }
    }
}

/// One open menu of a chain.
struct Level {
    menu: El,
    entries: Vec<Entry>,
    /// The entry of the level above that opened it.
    owner: Option<El>,
}

#[derive(Clone)]
struct Entry {
    item: MenuItem,
    el: El,
    /// For a submenu entry: its submenu is open.
    open: Signal<bool>,
}

/// The menu bar a chain belongs to.
pub(crate) struct BarLink {
    pub(crate) bar: Weak<Bar>,
}

/// One open menu with its open submenus, on one full-window layer.
pub(crate) struct Chain {
    ui: Ui,
    layer: El,
    levels: RefCell<Vec<Level>>,
    /// Where focus goes back to when the chain closes (none for a menu bar's: the bar does).
    restore: Option<El>,
    /// Opened (or since used) from the keyboard: access keys are underlined.
    keyboard: Signal<bool>,
    timer: Cell<Option<u64>>,
    bar: Option<BarLink>,
    rtl: bool,
    press_through: bool,
    closed: Cell<bool>,
    on_closed: RefCell<Vec<Box<dyn FnOnce()>>>,
}

/// How a chain is opened.
pub(crate) struct Open {
    pub(crate) anchor: Rect,
    pub(crate) label: Option<String>,
    pub(crate) keyboard: bool,
    /// Focus the last entry instead of the first (opened with Up).
    pub(crate) last: bool,
    pub(crate) restore: Option<El>,
    pub(crate) bar: Option<BarLink>,
    /// Laid out right to left (what it opened from is).
    pub(crate) rtl: bool,
    /// The press outside that closes it also reaches what is under it.
    pub(crate) press_through: bool,
}

impl Chain {
    pub(crate) fn open(ui: &Ui, items: Vec<MenuItem>, o: Open) -> Rc<Chain> {
        let layer = fill_window(ui.el("dismiss_layer")).focus_scope();
        let chain = Rc::new(Chain {
            ui: ui.clone(),
            layer: layer.clone(),
            levels: RefCell::new(Vec::new()),
            restore: o.restore,
            keyboard: Signal::new(o.keyboard),
            timer: Cell::new(None),
            bar: o.bar,
            rtl: o.rtl,
            press_through: o.press_through,
            closed: Cell::new(false),
            on_closed: RefCell::new(Vec::new()),
        });
        let (w1, w2) = (Rc::downgrade(&chain), Rc::downgrade(&chain));
        let layer = layer
            .on_pointer(move |e| w1.upgrade().is_some_and(|c| c.layer_pointer(e)))
            .on_key(move |e| {
                // Keys reach the layer itself only when focus is on none of the menus.
                if e.pressed
                    && e.key == Key::Escape
                    && let Some(c) = w2.upgrade()
                {
                    c.close(true);
                    return true;
                }
                false
            })
            .keyboard_alternative({
                let u = ui.clone();
                move || crate::tr(&u, "escape-closes")
            });
        // The chain lives as long as its layer: dropping it with the layer frees its menus.
        let keep = chain.clone();
        let layer = layer.on_removed(move || {
            keep.closed.set(true);
            if let Some(id) = keep.timer.take() {
                keep.ui.cancel_timer(id);
            }
            keep.levels.borrow_mut().clear();
        });
        ui.overlay(layer);
        chain.open_level(None, items, o.anchor, o.label, o.last);
        chain
    }

    pub(crate) fn layer_id(&self) -> remedy_ui_view::ElementId {
        self.layer.id()
    }

    pub(crate) fn opened_by_keyboard(&self) -> bool {
        untrack(|| self.keyboard.get())
    }

    /// Opens the submenu at (`depth`, `index`) from the keyboard, focus inside it.
    pub(crate) fn open_entry(self: &Rc<Self>, depth: usize, index: usize) {
        self.open_submenu(depth, index, true);
    }

    /// Chooses the entry under `p` (a press dragged from a menu bar's title and released on
    /// it). False when no entry is there.
    pub(crate) fn choose_at(self: &Rc<Self>, p: Point) -> bool {
        match self.entry_at(p) {
            Some((d, i)) => {
                self.choose(d, i, false);
                true
            }
            None => false,
        }
    }

    fn rtl(&self) -> bool {
        self.rtl
    }

    fn bar(&self) -> Option<Rc<Bar>> {
        self.bar.as_ref().and_then(|b| b.bar.upgrade())
    }

    /// Adds a menu of `items` at the end of the chain: below `anchor` for the first, beside
    /// it for submenus. Focuses its first (or last) entry.
    fn open_level(
        self: &Rc<Self>,
        owner: Option<El>,
        items: Vec<MenuItem>,
        anchor: Rect,
        label: Option<String>,
        last: bool,
    ) {
        let ui = &self.ui;
        let depth = self.levels.borrow().len();
        let rtl = self.rtl();
        let mut sem = Semantics::new(Role::Menu);
        if let Some(l) = label {
            sem = sem.label(l);
        }
        let menu = ui.el("menu").column().align(Align::Stretch).scroll().semantics(sem);
        let menu = if depth == 0 {
            let first = Point::new(anchor.min.x, anchor.max.y + GAP);
            place_fitted(ui, menu, first, f32::INFINITY, true, move |size, window| {
                fit_near_dir(anchor, size, GAP, window, rtl)
            })
        } else {
            // The first entry lines up with the entry that opened it: up by the menu's padding.
            let pad = ui.theme().spacing.unit;
            let a = Rect::new(anchor.min.x, anchor.min.y - pad, anchor.max.x, anchor.max.y);
            let first = Point::new(anchor.max.x + GAP, a.min.y);
            place_fitted(ui, menu, first, f32::INFINITY, true, move |size, window| {
                fit_beside(a, size, GAP, window, rtl)
            })
        };
        let marks =
            items.iter().any(|i| matches!(i.kind, Kind::Check(_) | Kind::Radio(_)) || i.icon.is_some());
        let n = items.iter().filter(|i| !i.is_separator()).count();
        let mut pos = 0;
        let mut entries = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            let open = Signal::new(false);
            let el = if item.is_separator() {
                separator(ui)
            } else {
                pos += 1;
                self.entry(depth, index, &item, (pos, n), marks, open)
            };
            let _ = menu.clone().child(el.clone());
            entries.push(Entry { item, el, open });
        }
        let w = Rc::downgrade(self);
        let menu = menu.on_key(move |e| w.upgrade().is_some_and(|c| c.menu_key(depth, e)));
        let _ = self.layer.clone().child(menu.clone());
        self.levels.borrow_mut().push(Level { menu, entries, owner });
        self.focus_entry(depth, if last { Pick::Last } else { Pick::First });
    }

    fn entry(
        self: &Rc<Self>,
        depth: usize,
        index: usize,
        item: &MenuItem,
        (pos, n): (usize, usize),
        marks: bool,
        open: Signal<bool>,
    ) -> El {
        let ui = &self.ui;
        let u = ui.theme().spacing.unit;
        let size = ui.theme().type_scale.body;
        // Entries read in the direction of what opened the menu (the menu itself is placed in
        // window coordinates, so it keeps the overlay layer's).
        let direction =
            if self.rtl { remedy_ui_layout::Direction::Rtl } else { remedy_ui_layout::Direction::Ltr };
        let mut row = ui
            .el("menu_item")
            .row()
            .align(Align::Center)
            .gap(u * 2.0)
            .layout(move |l| l.direction = direction);
        if item.danger {
            row = row.variant("danger");
        }
        if marks {
            let slot = ui.el("menu_mark").width(size).height(size).layout(|l| l.shrink = 0.0);
            let (s2, ui2, kind, own) = (slot.clone(), ui.clone(), item.kind.clone(), item.icon);
            let slot = slot.bind(move || {
                let glyph = match &kind {
                    Kind::Check(c) => c.get().then_some(Icon::Check),
                    Kind::Radio(c) => c.get().then_some(Icon::Dot),
                    _ => own,
                };
                untrack(|| {
                    s2.clear();
                    if let Some(g) = glyph {
                        let _ = s2.clone().child(icon(&ui2, g, size));
                    }
                });
            });
            row = row.child(slot);
        }
        let (label, key) = (item.label.clone(), item.mnemonic);
        let (ui2, kb) = (ui.clone(), self.keyboard);
        row = row.child(
            ui.el("menu_label")
                .text(move || mnemonic_text(&label.get(), key, kb.get() || alt_shows_keys(&ui2)))
                .nowrap()
                .grow(1.0),
        );
        if let Some(sc) = item.shortcut.clone() {
            let ui2 = ui.clone();
            row = row.child(
                ui.el("menu_shortcut")
                    .text(move || RichText::plain(shortcut_text(&ui2, &sc)))
                    .nowrap()
                    .layout(move |l| l.margin = Insets { top: 0.0, start: u * 4.0, end: 0.0, bottom: 0.0 }),
            );
        }
        let submenu = item.is_submenu();
        if submenu {
            // Mirrors on its own in a right-to-left menu, where submenus open to the left.
            row = row.child(icon(ui, Icon::ChevronRight, ui.theme().type_scale.small + 2.0));
        }
        let (w1, w2, w3) = (Rc::downgrade(self), Rc::downgrade(self), Rc::downgrade(self));
        let row = row
            .on_click(move || {
                if let Some(c) = w1.upgrade() {
                    c.choose(depth, index, false);
                }
            })
            .on_hover(move |inside| {
                if inside && let Some(c) = w2.upgrade() {
                    c.hovered(depth, index);
                }
            })
            .on_accessibility_action(move |a| {
                let Some(c) = w3.upgrade() else { return false };
                match a {
                    Action::Expand if submenu => {
                        c.open_submenu(depth, index, true);
                        true
                    }
                    Action::Collapse if submenu => {
                        c.close_from(depth + 1);
                        true
                    }
                    _ => false,
                }
            });
        let (r2, disabled) = (row.clone(), item.disabled.clone());
        let row = row.bind(move || r2.state(States::DISABLED, disabled.get()));
        let it = item.clone();
        let ui2 = ui.clone();
        row.semantics(move || {
            let mut s = Semantics::new(it.role()).label(it.label.get()).set_position(pos, n);
            if let Some(c) = it.checked() {
                s = s.checked(c);
            }
            if submenu {
                s = s.expanded(open.get());
            }
            if it.disabled.get() {
                s = s.disabled(true);
            } else {
                s = s.action(Action::Click);
                if submenu {
                    s = s.action(if open.get() { Action::Collapse } else { Action::Expand });
                }
            }
            if let Some(sc) = &it.shortcut {
                s = s.shortcut(shortcut_text(&ui2, sc));
            }
            if let Some(k) = it.mnemonic {
                s = s.access_key(k.to_uppercase().to_string());
            }
            s
        })
    }
}

/// Which entry to focus.
#[derive(Clone, Copy)]
enum Pick {
    First,
    Last,
    Next,
    Previous,
}

fn separator(ui: &Ui) -> El {
    let u = ui.theme().spacing.unit;
    ui.el("menu_separator")
        .height(1.0)
        .layout(move |l| {
            l.margin = Insets { top: u, start: u, end: u, bottom: u };
        })
        .semantics(Semantics::new(Role::Separator))
}

impl Chain {
    fn entries(&self, depth: usize) -> Vec<Entry> {
        self.levels.borrow().get(depth).map(|l| l.entries.clone()).unwrap_or_default()
    }

    /// Moves focus among the entries of the menu at `depth` (separators passed over;
    /// disabled entries take focus, as WAI-ARIA asks), wrapping around.
    fn focus_entry(&self, depth: usize, pick: Pick) {
        let entries = self.entries(depth);
        let usable: Vec<usize> =
            entries.iter().enumerate().filter(|(_, e)| !e.item.is_separator()).map(|(i, _)| i).collect();
        let k = usable.len();
        if k == 0 {
            return;
        }
        let cur =
            entries.iter().position(|e| e.el.is_focused()).and_then(|c| usable.iter().position(|u| *u == c));
        let next = match (pick, cur) {
            (Pick::First, _) | (Pick::Next, None) => 0,
            (Pick::Last, _) | (Pick::Previous, None) => k - 1,
            (Pick::Next, Some(a)) => (a + 1) % k,
            (Pick::Previous, Some(a)) => (a + k - 1) % k,
        };
        if let Some(e) = usable.get(next).and_then(|i| entries.get(*i)) {
            e.el.focus();
        }
    }

    /// A key on the menu at `depth` (its focused entry's, bubbled up).
    fn menu_key(self: &Rc<Self>, depth: usize, e: &KeyEvent) -> bool {
        if !e.pressed || self.closed.get() {
            return false;
        }
        let (forward, back) =
            if self.rtl() { (Key::ArrowLeft, Key::ArrowRight) } else { (Key::ArrowRight, Key::ArrowLeft) };
        let entries = self.entries(depth);
        let cur = entries.iter().position(|en| en.el.is_focused());
        let m = e.modifiers;
        if m.ctrl || m.meta {
            return false;
        }
        match &e.key {
            Key::ArrowDown => self.focus_entry(depth, Pick::Next),
            Key::ArrowUp => self.focus_entry(depth, Pick::Previous),
            Key::Home | Key::PageUp => self.focus_entry(depth, Pick::First),
            Key::End | Key::PageDown => self.focus_entry(depth, Pick::Last),
            k if *k == forward => {
                let sub = cur.and_then(|c| entries.get(c)).is_some_and(|en| en.item.is_submenu());
                match (sub, cur, self.bar()) {
                    (true, Some(c), _) => self.open_submenu(depth, c, true),
                    (false, _, Some(bar)) if depth == 0 => bar.step(1),
                    _ => {}
                }
            }
            k if *k == back => match self.bar() {
                _ if depth > 0 => self.back_to(depth),
                Some(bar) => bar.step(-1),
                None => {}
            },
            Key::Escape => {
                if depth > 0 {
                    self.back_to(depth);
                } else if !self.bar().is_some_and(|b| b.escape_menu()) {
                    self.close(true);
                }
            }
            Key::Enter | Key::Space => {
                if let Some(c) = cur {
                    self.choose(depth, c, true);
                }
            }
            Key::Tab => self.close(true),
            Key::Character(c) => {
                if let Some(c) = c.chars().next() {
                    self.type_key(depth, c);
                }
            }
            _ => return false,
        }
        if !self.closed.get() && !untrack(|| self.keyboard.get()) {
            self.keyboard.set(true);
        }
        true
    }

    /// A letter typed in the menu at `depth`: the entry with that access key is chosen (or,
    /// when several share it, the next of them focused); without one, focus moves to the next
    /// entry starting with the letter.
    fn type_key(self: &Rc<Self>, depth: usize, c: char) {
        let c = c.to_lowercase().next().unwrap_or(c);
        let entries = self.entries(depth);
        let cur = entries.iter().position(|en| en.el.is_focused());
        let keyed: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, en)| !en.item.is_separator() && en.item.mnemonic == Some(c))
            .map(|(i, _)| i)
            .collect();
        if let [only] = keyed.as_slice() {
            self.choose(depth, *only, true);
            return;
        }
        let candidates: Vec<usize> = if keyed.is_empty() {
            entries
                .iter()
                .enumerate()
                .filter(|(_, en)| !en.item.is_separator() && en.item.starts_with(c))
                .map(|(i, _)| i)
                .collect()
        } else {
            keyed
        };
        let next = candidates.iter().find(|i| cur.is_none_or(|c| **i > c)).or(candidates.first());
        if let Some(e) = next.and_then(|i| entries.get(*i)) {
            e.el.focus();
        }
    }

    /// Chooses the entry at (`depth`, `index`): a submenu opens (focusing inside it when
    /// chosen from the keyboard); anything else closes the chain, gives focus back, and runs.
    fn choose(self: &Rc<Self>, depth: usize, index: usize, keyboard: bool) {
        let Some(entry) = self.entries(depth).into_iter().nth(index) else { return };
        if entry.item.is_separator() || entry.item.is_disabled() || self.closed.get() {
            return;
        }
        if keyboard && !untrack(|| self.keyboard.get()) {
            self.keyboard.set(true);
        }
        if entry.item.is_submenu() {
            self.open_submenu(depth, index, keyboard);
            return;
        }
        let action = entry.item.action.clone();
        self.close(true);
        action();
    }

    /// Opens the submenu of the entry at (`depth`, `index`), focusing its first entry when
    /// `focus` (the keyboard), or keeping focus on the entry (the pointer resting on it).
    fn open_submenu(self: &Rc<Self>, depth: usize, index: usize, focus: bool) {
        let Some(entry) = self.entries(depth).into_iter().nth(index) else { return };
        let Kind::Submenu(items) = &entry.item.kind else { return };
        if entry.item.is_disabled() || self.closed.get() {
            return;
        }
        let already = self.levels.borrow().get(depth + 1).and_then(|l| l.owner.as_ref().map(El::id))
            == Some(entry.el.id());
        if already {
            if focus {
                self.focus_entry(depth + 1, Pick::First);
            }
            return;
        }
        self.close_from(depth + 1);
        let anchor = entry.el.bounds().unwrap_or(Rect::ZERO);
        entry.open.set(true);
        self.open_level(Some(entry.el.clone()), items(), anchor, Some(entry.item.label_text()), false);
        if !focus {
            entry.el.focus();
        }
    }

    /// Closes the menu at `depth` and focuses the entry that opened it (Left, Escape).
    fn back_to(&self, depth: usize) {
        let owner = self.levels.borrow().get(depth).and_then(|l| l.owner.clone());
        self.close_from(depth);
        if let Some(o) = owner {
            o.focus();
        }
    }

    /// Closes the menus at `depth` and deeper (never the first: that is the whole chain).
    fn close_from(&self, depth: usize) {
        let removed = {
            let mut levels = self.levels.borrow_mut();
            if depth >= levels.len() || depth == 0 {
                return;
            }
            levels.split_off(depth)
        };
        for l in &removed {
            l.menu.remove();
        }
        for e in self.entries(depth - 1) {
            if untrack(|| e.open.get()) {
                e.open.set(false);
            }
        }
    }

    /// The pointer came onto the entry at (`depth`, `index`): it takes focus (the highlight
    /// follows the pointer, and the keyboard goes on from it), and after a moment its submenu
    /// opens, or a sibling's open submenu closes.
    fn hovered(self: &Rc<Self>, depth: usize, index: usize) {
        if self.closed.get() {
            return;
        }
        let Some(entry) = self.entries(depth).into_iter().nth(index) else { return };
        if !entry.el.is_focused() {
            entry.el.focus();
        }
        if let Some(id) = self.timer.take() {
            self.ui.cancel_timer(id);
        }
        let open_here = self.levels.borrow().get(depth + 1).and_then(|l| l.owner.as_ref().map(El::id))
            == Some(entry.el.id());
        let nothing_open = self.levels.borrow().len() <= depth + 1;
        let submenu = entry.item.is_submenu() && !entry.item.is_disabled();
        if open_here || (nothing_open && !submenu) {
            return;
        }
        let w = Rc::downgrade(self);
        let id = self.ui.after(SUBMENU_DELAY, move || {
            let Some(c) = w.upgrade() else { return };
            c.timer.set(None);
            if c.closed.get() {
                return;
            }
            if submenu {
                c.open_submenu(depth, index, false);
            } else {
                c.close_from(depth + 1);
            }
        });
        self.timer.set(Some(id));
    }

    /// Pointer input that reached the layer: outside every menu of the chain.
    fn layer_pointer(self: &Rc<Self>, e: &remedy_ui_view::PointerInput) -> bool {
        let inside =
            self.levels.borrow().iter().any(|l| l.menu.bounds().is_some_and(|b| b.contains(e.position)));
        if inside || self.closed.get() {
            return false;
        }
        if let Some(bar) = self.bar()
            && let Some(slot) = bar.slot_at(e.position)
        {
            match e.phase {
                PointerPhase::Move => bar.hover_slot(slot),
                PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => bar.press_slot(slot),
                _ => {}
            }
            return e.phase == PointerPhase::Down;
        }
        if e.phase == PointerPhase::Down {
            self.close(true);
            if self.press_through {
                // The layer is gone: the same press goes on to what was under it.
                let _ = self.ui.pointer(e);
            }
            return true;
        }
        false
    }

    /// The entry under `p` in any menu of the chain, deepest first.
    fn entry_at(&self, p: Point) -> Option<(usize, usize)> {
        let levels = self.levels.borrow();
        levels.iter().enumerate().rev().find_map(|(d, l)| {
            l.entries.iter().position(|e| e.el.bounds().is_some_and(|b| b.contains(p))).map(|i| (d, i))
        })
    }

    /// Closes the whole chain: a menu bar's is told (it gives focus back), any other gives
    /// focus back itself when `restore`.
    fn close(&self, restore: bool) {
        if !self.dismiss() {
            return;
        }
        match self.bar() {
            Some(bar) => bar.menu_closed(restore),
            None => {
                if restore
                    && let Some(r) = &self.restore
                    && self.ui.tree().contains(r.id())
                {
                    r.focus();
                }
            }
        }
        for f in self.on_closed.take() {
            f();
        }
    }

    /// Takes the chain off the screen without telling anyone (a menu bar switching menus).
    /// False when it was already closed.
    pub(crate) fn dismiss(&self) -> bool {
        if self.closed.replace(true) {
            return false;
        }
        if let Some(id) = self.timer.take() {
            self.ui.cancel_timer(id);
        }
        self.layer.remove();
        true
    }
}

/// An open menu: its layer, and a way to hear when it closes.
#[derive(Clone)]
pub(crate) struct OpenMenu(Rc<Chain>);

impl OpenMenu {
    pub(crate) fn layer(&self) -> El {
        self.0.layer.clone()
    }

    pub(crate) fn is_open(&self) -> bool {
        !self.0.closed.get()
    }

    pub(crate) fn close(&self) {
        self.0.close(true);
    }

    pub(crate) fn on_closed(&self, f: impl FnOnce() + 'static) {
        self.0.on_closed.borrow_mut().push(Box::new(f));
    }
}

/// Opens a menu of `items` below `anchor`; `keyboard` when the keyboard opened it (its access
/// keys show underlined), `last` to focus its last entry (opened with Up). Focus goes back
/// where it was when it closes.
pub(crate) fn open_menu(
    from: &El,
    anchor: Rect,
    items: Vec<MenuItem>,
    keyboard: bool,
    last: bool,
) -> OpenMenu {
    let ui = from.ui();
    let restore = ui.tree().focused().map(|f| ui.element(f));
    let rtl = from.is_rtl();
    let o = Open { anchor, label: None, keyboard, last, restore, bar: None, rtl, press_through: false };
    OpenMenu(Chain::open(ui, items, o))
}

/// Opens a menu below `anchor`. Returns the menu's layer; it closes itself when an entry is
/// chosen, on a press outside, on Escape (in a submenu, Escape closes only the submenu) and on
/// Tab, and gives focus back to what had it.
#[track_caller]
pub fn menu(ui: &Ui, anchor: &El, items: Vec<MenuItem>) -> El {
    let b = anchor.bounds().unwrap_or(Rect::ZERO);
    let _ = ui;
    open_menu(anchor, b, items, false, false).layer()
}

/// Opens a menu below `anchor` as [`menu`] does, except that the press outside that closes it
/// also reaches what is under it: a click on a video player's picture both closes the menu
/// and plays or pauses, as in a web page's player.
#[track_caller]
pub fn menu_press_through(ui: &Ui, anchor: &El, items: Vec<MenuItem>) -> El {
    let b = anchor.bounds().unwrap_or(Rect::ZERO);
    let from = anchor.ui();
    let restore = from.tree().focused().map(|f| from.element(f));
    let o = Open {
        anchor: b,
        label: None,
        keyboard: false,
        last: false,
        restore,
        bar: None,
        rtl: anchor.is_rtl(),
        press_through: true,
    };
    let _ = ui;
    Chain::open(&from, items, o).layer.clone()
}

/// Opens a menu at the pointer on a secondary press (right click) on `target`, or with the
/// Menu key or Shift+F10 while it has focus.
#[track_caller]
pub fn context_menu(target: &El, items: impl Fn() -> Vec<MenuItem> + 'static) -> El {
    context_menu_at(target, move |_| items())
}

/// Like [`context_menu`], for menus that depend on what is under the pointer: `items` gets the
/// window point of the press, or `None` when the menu was opened from the keyboard.
#[track_caller]
pub fn context_menu_at(target: &El, items: impl Fn(Option<Point>) -> Vec<MenuItem> + 'static) -> El {
    let items = Rc::new(items);
    let (t1, items2, t2) = (target.clone(), items.clone(), target.clone());
    target
        .clone()
        .on_pointer(move |e| {
            if e.phase == PointerPhase::Down && e.buttons.contains(Buttons::SECONDARY) {
                let at = Rect::from_points(e.position, e.position);
                let _ = open_menu(&t1, at, items(Some(e.position)), false, false);
                return true;
            }
            false
        })
        .on_key(move |e| {
            let menu_key = matches!(&e.key, Key::Named(n) if n == "ContextMenu")
                || (e.key == Key::F(10) && e.modifiers.shift);
            if e.pressed && menu_key {
                let b = t2.bounds().unwrap_or(Rect::ZERO);
                let at = Rect::new(b.min.x, b.max.y, b.min.x, b.max.y);
                let _ = open_menu(&t2, at, items2(None), true, false);
                return true;
            }
            false
        })
        // The same menu opens from the keyboard wherever focus is inside the target.
        .keyboard_alternative({
            let u = target.ui().clone();
            move || crate::tr(&u, "context-menu-keys")
        })
}

#[cfg(test)]
mod tests {
    use super::mnemonic_text;

    #[test]
    fn access_keys_are_underlined_only_when_shown() {
        let t = mnemonic_text("Save As…", Some('a'), true);
        assert_eq!(t.as_str(), "Save As…");
        let underlined: Vec<&str> = t
            .spans()
            .iter()
            .filter(|s| s.patch.underline == Some(true))
            .filter_map(|s| t.as_str().get(s.range.clone()))
            .collect();
        assert_eq!(underlined, ["A"], "the A that starts a word, whatever its case");
        let exit = mnemonic_text("Exit", Some('x'), true);
        assert_eq!(exit.spans().first().map(|s| s.range.clone()), Some(1..2), "else its first place");
        assert!(mnemonic_text("Save As…", Some('a'), false).spans().is_empty());
        // A key the label lacks is written after it, shown or not, so nothing moves.
        let jp = mnemonic_text("ファイル", Some('f'), false);
        assert_eq!(jp.as_str(), "ファイル (F)");
        assert_eq!(mnemonic_text("ファイル", Some('f'), true).as_str(), jp.as_str());
        assert_eq!(mnemonic_text("Über", Some('ü'), true).spans().len(), 1);
    }
}
