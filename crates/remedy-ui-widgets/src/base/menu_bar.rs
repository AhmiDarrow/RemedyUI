//! The menu bar: a row of menus along the top of a window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, PointerPhase};
use remedy_ui_core::layout::{AUTO, Align, Position};
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_core::style::States;
use remedy_ui_core::{Insets, Point, Rect};
use remedy_ui_engine::view::{El, MenuKey, Prop, RichText, Ui};

use crate::base::menus::{BarLink, Chain, MenuItem, Open, alt_shows_keys, mnemonic_text};

/// A menu bar: its menus in a row, and the state of the one open.
pub(crate) struct Bar {
    ui: Ui,
    el: El,
    menus: Vec<MenuItem>,
    /// One title per menu, then the "more" title for those that do not fit.
    titles: Vec<El>,
    /// Each title's width (the "more" title's last), measured off screen so the hidden ones
    /// are known too.
    widths: RefCell<Vec<f32>>,
    gap: f32,
    /// How many titles fit; the rest are in the "more" menu.
    shown: Signal<usize>,
    open: RefCell<Option<(usize, Rc<Chain>)>>,
    open_slot: Signal<Option<usize>>,
    /// Reached from the keyboard (F10, Alt, an access key): titles take focus, access keys
    /// show.
    active: Signal<bool>,
    restore: RefCell<Option<El>>,
    /// A menu is being opened: focus leaving the titles is not leaving the bar.
    opening: Cell<bool>,
}

impl Bar {
    fn n(&self) -> usize {
        self.menus.len()
    }

    /// The titles showing while `shown` of them fit, by slot (a menu's index, or `n` for "more").
    fn slots_of(&self, shown: usize) -> Vec<usize> {
        let mut s: Vec<usize> = (0..shown.min(self.n())).collect();
        if shown < self.n() {
            s.push(self.n());
        }
        s
    }

    fn slots(&self) -> Vec<usize> {
        self.slots_of(untrack(|| self.shown.get()))
    }

    fn slot_items(&self, slot: usize) -> Vec<MenuItem> {
        match self.menus.get(slot) {
            Some(m) => m.items(),
            None => {
                self.menus.get(untrack(|| self.shown.get())..).map(<[MenuItem]>::to_vec).unwrap_or_default()
            }
        }
    }

    fn slot_label(&self, slot: usize) -> String {
        self.menus.get(slot).map_or_else(|| crate::base::tr(&self.ui, MORE), MenuItem::label_text)
    }

    /// The app has disabled the menu of `slot` ("more" never is): it neither opens nor runs.
    fn slot_disabled(&self, slot: usize) -> bool {
        self.menus.get(slot).is_some_and(MenuItem::is_disabled)
    }

    /// The first slot that is not disabled going `delta` at a time from place `at` of
    /// `slots`, wrapping around (`at` itself last).
    fn enabled_from(&self, slots: &[usize], at: usize, delta: isize) -> Option<usize> {
        let k = slots.len() as isize;
        (1..=k)
            .filter_map(|i| usize::try_from((at as isize + delta * i).rem_euclid(k)).ok())
            .filter_map(|i| slots.get(i).copied())
            .find(|s| !self.slot_disabled(*s))
    }

    pub(crate) fn slot_at(&self, p: Point) -> Option<usize> {
        self.slots()
            .into_iter()
            .find(|s| self.titles.get(*s).and_then(El::bounds).is_some_and(|b| b.contains(p)))
    }

    fn focused_slot(&self) -> Option<usize> {
        self.slots().into_iter().find(|s| self.titles.get(*s).is_some_and(El::is_focused))
    }

    fn open_chain(&self) -> Option<Rc<Chain>> {
        self.open.borrow().as_ref().map(|(_, c)| c.clone())
    }

    /// What has focus, when that is neither the bar nor its open menu.
    fn focus_outside(&self) -> Option<El> {
        let tree = self.ui.tree();
        let f = tree.focused()?;
        let in_bar = tree.is_ancestor_or_self(self.el.id(), f);
        let in_menu = self.open_chain().is_some_and(|c| tree.is_ancestor_or_self(c.layer_id(), f));
        (!in_bar && !in_menu).then(|| self.ui.element(f))
    }

    fn remember_focus(&self) {
        if self.restore.borrow().is_none() {
            let f = self.focus_outside();
            *self.restore.borrow_mut() = f;
        }
    }

    fn set_titles_focusable(&self, on: bool) {
        for t in &self.titles {
            let _ = t.clone().focusable(on);
        }
    }

    fn set_active(&self, on: bool) {
        if untrack(|| self.active.get()) != on {
            self.active.set(on);
        }
        self.set_titles_focusable(on);
    }

    /// Reaches the bar from the keyboard: its first title takes focus.
    fn activate(&self) {
        self.remember_focus();
        self.set_active(true);
        let slots = self.slots();
        let first = self.enabled_from(&slots, slots.len().saturating_sub(1), 1).or(slots.first().copied());
        if let Some(t) = first.and_then(|s| self.titles.get(s)) {
            t.focus();
        }
    }

    /// Leaves the bar: focus goes back where it was when `restore`.
    fn deactivate(&self, restore: bool) {
        let back = self.restore.borrow_mut().take();
        if restore
            && let Some(r) = back
            && self.ui.tree().contains(r.id())
        {
            r.focus();
        }
        self.set_active(false);
    }

    fn close_open(&self) {
        let open = self.open.borrow_mut().take();
        if let Some((_, c)) = open {
            c.dismiss();
        }
        if untrack(|| self.open_slot.get()).is_some() {
            self.open_slot.set(None);
        }
    }

    /// Opens the menu of `slot` (closing another), focusing its first entry, or its last
    /// with `last`.
    fn open_slot_menu(self: &Rc<Self>, slot: usize, keyboard: bool, last: bool) -> Option<Rc<Chain>> {
        self.remember_focus();
        self.close_open();
        let title = self.titles.get(slot)?;
        let anchor = title.bounds().unwrap_or(Rect::ZERO);
        let items = self.slot_items(slot);
        self.opening.set(true);
        let chain = Chain::open(
            &self.ui,
            items,
            Open {
                anchor,
                label: Some(self.slot_label(slot)),
                keyboard,
                last,
                restore: None,
                bar: Some(BarLink { bar: Rc::downgrade(self) }),
                rtl: self.el.is_rtl(),
                press_through: false,
            },
        );
        self.opening.set(false);
        *self.open.borrow_mut() = Some((slot, chain.clone()));
        self.open_slot.set(Some(slot));
        Some(chain)
    }

    /// Runs the plain entry `m` standing in the bar itself (no menu of its own).
    fn run_plain(&self, m: &MenuItem) {
        self.close_open();
        self.deactivate(true);
        if !m.is_disabled() {
            m.run();
        }
    }

    /// A press on a title (by pointer or assistive technology): opens its menu, or closes it
    /// when it is the one open.
    pub(crate) fn press_slot(self: &Rc<Self>, slot: usize) {
        if untrack(|| self.open_slot.get()) == Some(slot) {
            self.close_open();
            self.deactivate(true);
            return;
        }
        if self.slot_disabled(slot) {
            return;
        }
        match self.menus.get(slot) {
            Some(m) if !m.is_submenu() => self.run_plain(m),
            _ => {
                let _ = self.open_slot_menu(slot, false, false);
            }
        }
    }

    /// The pointer moved onto a title while another's menu is open: that one opens instead.
    pub(crate) fn hover_slot(self: &Rc<Self>, slot: usize) {
        let open = untrack(|| self.open_slot.get());
        if open.is_some_and(|o| o != slot)
            && !self.slot_disabled(slot)
            && self.menus.get(slot).is_none_or(MenuItem::is_submenu)
        {
            let keyboard = self.open_chain().is_some_and(|c| c.opened_by_keyboard());
            let _ = self.open_slot_menu(slot, keyboard, false);
        }
    }

    /// Left or Right at a menu's top level: the neighbouring menu opens (disabled ones are
    /// passed over).
    pub(crate) fn step(self: &Rc<Self>, delta: isize) {
        let slots = self.slots();
        let Some(open) = untrack(|| self.open_slot.get()) else { return };
        let Some(at) = slots.iter().position(|s| *s == open) else { return };
        let next = self.enabled_from(&slots, at, delta.signum()).filter(|s| *s != open);
        if let Some(s) = next {
            self.set_active(true);
            match self.menus.get(s) {
                // A plain entry in the bar has no menu to show: its title takes focus.
                Some(m) if !m.is_submenu() => {
                    self.close_open();
                    if let Some(t) = self.titles.get(s) {
                        t.focus();
                    }
                }
                _ => {
                    let _ = self.open_slot_menu(s, true, false);
                }
            }
        }
    }

    /// Escape in a menu's top level: while the bar was reached from the keyboard, the menu
    /// closes and its title keeps focus. False otherwise (the menu closes, focus goes back).
    pub(crate) fn escape_menu(&self) -> bool {
        if !untrack(|| self.active.get()) {
            return false;
        }
        let slot = untrack(|| self.open_slot.get());
        self.close_open();
        if let Some(t) = slot.and_then(|s| self.titles.get(s)) {
            t.focus();
        }
        true
    }

    /// The open menu closed (an entry chosen, a press outside, Tab).
    pub(crate) fn menu_closed(&self, restore: bool) {
        *self.open.borrow_mut() = None;
        if untrack(|| self.open_slot.get()).is_some() {
            self.open_slot.set(None);
        }
        self.deactivate(restore);
    }

    /// A title lost focus: unless it went to another title or into a menu being opened, the
    /// bar is left (a click elsewhere).
    fn focus_left(&self) {
        if self.opening.get() || self.open.borrow().is_some() || !untrack(|| self.active.get()) {
            return;
        }
        let tree = self.ui.tree();
        let in_bar = tree.focused().is_some_and(|f| tree.is_ancestor_or_self(self.el.id(), f));
        if !in_bar {
            self.restore.borrow_mut().take();
            self.deactivate(false);
        }
    }

    /// The bar takes a window menu key now: it is on screen and nothing modal (a dialog,
    /// another menu) holds focus.
    fn usable(&self) -> bool {
        let tree = self.ui.tree();
        if !tree.contains(self.el.id()) || !self.el.bounds().is_some_and(|b| b.width() > 0.0) {
            return false;
        }
        match tree.focused() {
            Some(f) => {
                let scope = tree.focus_scope(f);
                scope == tree.root()
                    || tree.is_ancestor_or_self(self.el.id(), f)
                    || self.open_chain().is_some_and(|c| c.layer_id() == scope)
            }
            None => true,
        }
    }

    fn window_key(self: &Rc<Self>, k: &MenuKey) -> bool {
        if !self.usable() {
            return false;
        }
        match k {
            MenuKey::AltTap | MenuKey::F10 => {
                if self.open.borrow().is_some() || untrack(|| self.active.get()) {
                    self.close_open();
                    self.deactivate(true);
                } else {
                    self.activate();
                }
                true
            }
            MenuKey::Mnemonic(c) => match self.menu_with_key(c.chars().next()) {
                Some(i) => {
                    self.open_mnemonic(i);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    fn menu_with_key(&self, c: Option<char>) -> Option<usize> {
        let c = c.and_then(|c| c.to_lowercase().next())?;
        self.menus.iter().position(|m| m.access_key() == Some(c) && !m.is_disabled())
    }

    /// Opens menu `i` from the keyboard: directly, or inside "more" when it does not fit.
    fn open_mnemonic(self: &Rc<Self>, i: usize) {
        let Some(m) = self.menus.get(i) else { return };
        self.remember_focus();
        self.set_active(true);
        if !m.is_submenu() {
            self.run_plain(m);
            return;
        }
        let shown = untrack(|| self.shown.get());
        if i < shown {
            let _ = self.open_slot_menu(i, true, false);
        } else if let Some(chain) = self.open_slot_menu(self.n(), true, false) {
            chain.open_entry(0, i - shown);
        }
    }

    /// A key while a title has focus.
    fn title_key(self: &Rc<Self>, e: &KeyEvent) -> bool {
        if !e.pressed || e.modifiers.ctrl || e.modifiers.meta {
            return false;
        }
        let slots = self.slots();
        let Some(cur) = self.focused_slot() else { return false };
        let at = slots.iter().position(|s| *s == cur).unwrap_or(0);
        let (forward, back) = if self.el.is_rtl() {
            (Key::ArrowLeft, Key::ArrowRight)
        } else {
            (Key::ArrowRight, Key::ArrowLeft)
        };
        // Disabled menus are passed over: `from` is the place before the first one tried.
        let focus = |from: usize, delta: isize| {
            if let Some(t) = self.enabled_from(&slots, from, delta).and_then(|s| self.titles.get(s)) {
                t.focus();
            }
        };
        match &e.key {
            key if *key == forward => focus(at, 1),
            key if *key == back => focus(at, -1),
            Key::Home => focus(slots.len().saturating_sub(1), 1),
            Key::End => focus(0, -1),
            Key::ArrowDown | Key::Enter | Key::Space => self.key_open(cur, false),
            Key::ArrowUp => self.key_open(cur, true),
            Key::Escape | Key::Tab => {
                self.close_open();
                self.deactivate(true);
            }
            Key::Character(c) => {
                if let Some(i) = self.menu_with_key(c.chars().next()) {
                    self.open_mnemonic(i);
                }
            }
            _ => return false,
        }
        true
    }

    fn key_open(self: &Rc<Self>, slot: usize, last: bool) {
        if self.slot_disabled(slot) {
            return;
        }
        match self.menus.get(slot) {
            Some(m) if !m.is_submenu() => self.run_plain(m),
            _ => {
                let _ = self.open_slot_menu(slot, true, last);
            }
        }
    }

    /// Fits the titles into the bar's width: those past it go into the "more" menu.
    fn fit(&self) {
        let Some(b) = self.el.bounds() else { return };
        let pad = self.el.info().map(|i| i.style.padding).unwrap_or(Insets::ZERO);
        let avail = b.width() - pad.start - pad.end;
        let widths = self.widths.borrow().clone();
        let n = self.n();
        if widths.len() != n + 1 || widths.iter().any(|w| *w <= 0.0) || avail <= 0.0 {
            return;
        }
        let gap = self.gap;
        let all: f32 = widths.iter().take(n).sum::<f32>() + gap * n.saturating_sub(1) as f32;
        let shown = if all <= avail + 0.5 {
            n
        } else {
            let mut used = widths.get(n).copied().unwrap_or_default();
            let mut k = 0;
            while k < n
                && let Some(w) = widths.get(k)
                && used + gap + w <= avail + 0.5
            {
                used += gap + w;
                k += 1;
            }
            k
        };
        if shown == untrack(|| self.shown.get()) {
            return;
        }
        if self.open.borrow().is_some() {
            self.close_open();
            self.deactivate(false);
        }
        self.shown.set(shown);
        for (i, t) in self.titles.iter().enumerate() {
            let hide = if i < n { i >= shown } else { shown == n };
            let _ = t.clone().hidden(hide);
        }
    }
}

/// The "more" title's accessible name (its message); it shows "…".
const MORE: &str = "menu-more";

/// A menu bar: a row of menus along the top of a window, each a [`MenuItem::submenu`] (with
/// a [`MenuItem::mnemonic`] for its access key), as desktop apps have.
///
/// - A click on a title opens its menu (a press, a drag down and a release on an entry
///   chooses it in one go); while one is open, the pointer moving onto another title opens
///   that one instead.
/// - F10, or Alt pressed and released on its own, reaches the bar from anywhere in the window
///   (and again leaves it); Alt with a menu's letter opens that menu from anywhere, a text
///   field included. Left and Right move between menus, open or not (mirrored in a
///   right-to-left layout); Down, Enter or Space opens one, Up opens it at its last entry;
///   Escape closes it, and Escape again (or Tab) gives focus back where it was.
/// - Access keys are underlined while Alt is held and once the bar is reached from the
///   keyboard.
/// - In a window too narrow for every title, those that do not fit move into a "…" menu at
///   the end instead of being cut off.
///
/// ```
/// use remedy_ui_widgets::base::{MenuItem, menu_bar};
/// # use remedy_ui_core::style::{StyleSheet, Theme};
/// # let ui = remedy_ui_engine::view::Ui::new(Theme::dark_forest(), StyleSheet::new(), remedy_ui_text::FontCollection::new());
///
/// let bar = menu_bar(
///     &ui,
///     vec![
///         MenuItem::submenu("File", vec![MenuItem::new("Open…", || {}).shortcut("Ctrl+O").mnemonic('o')])
///             .mnemonic('f'),
///         MenuItem::submenu("Help", vec![MenuItem::new("About", || {}).mnemonic('a')]).mnemonic('h'),
///     ],
/// );
/// let _ = ui.root().child(bar);
/// ```
#[track_caller]
pub fn menu_bar(ui: &Ui, menus: Vec<MenuItem>) -> El {
    let gap = ui.theme().spacing.unit * 0.5;
    let n = menus.len();
    let shown = Signal::new(n);
    let active = Signal::new(false);
    let open_slot: Signal<Option<usize>> = Signal::new(None);
    let labels: Vec<(Prop<String>, Option<char>)> = menus
        .iter()
        .map(|m| (m.label_prop(), m.access_key()))
        .chain(std::iter::once((Prop::Value("…".to_owned()), None)))
        .collect();
    let titles: Vec<El> = labels
        .iter()
        .map(|(label, key)| {
            let (l2, ui2, key) = (label.clone(), ui.clone(), *key);
            ui.el("menu_bar_item")
                .text(move || mnemonic_text(&l2.get(), key, active.get() || alt_shows_keys(&ui2)))
                .nowrap()
                .layout(|l| l.shrink = 0.0)
                .keeps_focus_on_press(true)
        })
        .collect();
    let bar_el = ui
        .el("menu_bar")
        .row()
        .align(Align::Center)
        .gap(gap)
        .fill_width()
        .clip()
        .semantics(Semantics::new(Role::MenuBar))
        .keyboard_alternative({
            let u = ui.clone();
            move || crate::base::tr(&u, "menu-bar-keys")
        });
    let bar = Rc::new(Bar {
        ui: ui.clone(),
        el: bar_el.clone(),
        menus,
        titles: titles.clone(),
        widths: RefCell::new(vec![0.0; n + 1]),
        gap,
        shown,
        open: RefCell::new(None),
        open_slot,
        active,
        restore: RefCell::new(None),
        opening: Cell::new(false),
    });
    let mut bar_el = bar_el;
    for (slot, title) in titles.into_iter().enumerate() {
        bar_el = bar_el.child(bar_title(&bar, slot, title, &labels));
    }
    // Off-screen copies of the titles measure them, shown or not.
    let mut measure = ui
        .el("menu_bar_measure")
        .row()
        .gap(gap)
        .pass_through()
        .style(remedy_ui_core::style::StylePatch::default().opacity(0.0))
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = Insets { top: 0.0, start: 0.0, end: AUTO, bottom: AUTO };
        })
        .semantics(Semantics::new(Role::Group).hidden(true));
    for (slot, (label, _)) in labels.iter().enumerate() {
        let (l3, b) = (label.clone(), bar.clone());
        measure = measure.child(
            ui.el("menu_bar_item")
                .text(move || RichText::plain(l3.get()))
                .nowrap()
                .layout(|l| l.shrink = 0.0)
                .pass_through()
                .semantics(Semantics::new(Role::Label).hidden(true))
                .on_resize(move |size| {
                    if let Some(w) = b.widths.borrow_mut().get_mut(slot) {
                        *w = size.width;
                    }
                    b.fit();
                }),
        );
    }
    let b1 = bar.clone();
    let bar_el = bar_el.child(measure).on_resize(move |_| b1.fit());
    let weak = Rc::downgrade(&bar);
    let keys = ui.add_menu_keys(move |k| weak.upgrade().is_some_and(|b| b.window_key(k)));
    let ui2 = ui.clone();
    bar_el.on_removed(move || {
        ui2.remove_menu_keys(keys);
        bar.close_open();
    })
}

/// One title of the bar: menu `slot`'s, or "more" for `slot == n`.
fn bar_title(bar: &Rc<Bar>, slot: usize, title: El, labels: &[(Prop<String>, Option<char>)]) -> El {
    let n = bar.n();
    let is_more = slot == n;
    let submenu = bar.menus.get(slot).is_none_or(MenuItem::is_submenu);
    let (label, key) = labels.get(slot).cloned().unwrap_or((Prop::Value(String::new()), None));
    let disabled = bar.menus.get(slot).map_or(Prop::Value(false), MenuItem::disabled_prop);
    let (b1, b2, b3, b4, b5) = (bar.clone(), bar.clone(), bar.clone(), bar.clone(), bar.clone());
    let open_slot = bar.open_slot;
    let shown = bar.shown;
    let (t2, t3, d2) = (title.clone(), title.clone(), disabled.clone());
    let title = title
        .on_click(move || b1.press_slot(slot))
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                b2.press_slot(slot);
                true
            }
            // Pressed on the title, dragged into its menu, released on an entry: chosen.
            PointerPhase::Up => b2.open_chain().is_some_and(|chain| chain.choose_at(e.position)),
            _ => false,
        })
        .on_key(move |e| b3.title_key(e))
        .on_focus_change(move |on| {
            if !on {
                b4.focus_left();
            }
        })
        .focusable(false)
        .keyboard_alternative({
            let u = bar.ui.clone();
            move || crate::base::tr(&u, "menu-title-keys")
        })
        .bind(move || t2.state(States::SELECTED, open_slot.get() == Some(slot)))
        .bind(move || t3.state(States::DISABLED, d2.get()))
        .semantics(move || {
            let slots = b5.slots_of(shown.get());
            let pos = slots.iter().position(|s| *s == slot).map_or(1, |p| p + 1);
            let name = if is_more { crate::base::tr(&b5.ui, MORE) } else { label.get() };
            let mut s = Semantics::new(Role::MenuItem).label(name).set_position(pos, slots.len());
            if submenu {
                s = s.expanded(open_slot.get() == Some(slot));
            }
            if disabled.get() {
                s = s.disabled(true);
            } else {
                s = s.action(Action::Click);
            }
            if let Some(k) = key {
                s = s.access_key(k.to_uppercase().to_string());
            }
            s
        });
    if is_more { title.hidden(true) } else { title }
}
