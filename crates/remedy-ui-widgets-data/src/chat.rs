//! Chat lists: messages of any height, any number of them, that behave like a chat. The view
//! follows new messages while it is at the bottom and stays put while someone reads higher
//! up (a "Jump to latest" button counts what arrived meanwhile); older messages loaded above
//! keep the view on what it showed; days get a header between messages and the current day
//! stays pinned at the top. Only rows entering or leaving the view are built or dropped, so
//! a row keeps its state (and any text selected in it) while it stays in view.

use std::cell::{Cell, RefCell};
use std::hash::Hash;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Insets, Size, Vec2};
use remedy_ui_input::Key;
use remedy_ui_layout::{AUTO, JumpAlign, Length, Position, VirtualList};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

/// Builds one message.
type RowFn<T> = Rc<dyn Fn(&Ui, &T) -> El>;

/// What a chat list shows the app: whether it follows the latest, and how much arrived
/// while it did not.
#[derive(Clone, Copy, Debug)]
pub struct ChatState {
    /// The view is at the bottom, following new messages.
    pub at_end: Signal<bool>,
    /// Messages that arrived while the view was higher up.
    pub unread: Signal<usize>,
    jump: Signal<u64>,
}

impl ChatState {
    /// Following the latest, nothing unread.
    pub fn new() -> ChatState {
        ChatState { at_end: Signal::new(true), unread: Signal::new(0), jump: Signal::new(0) }
    }

    /// Goes to the latest message and follows again.
    pub fn jump_to_latest(&self) {
        self.jump.update(|n| *n += 1);
    }
}

impl Default for ChatState {
    fn default() -> Self {
        ChatState::new()
    }
}

/// How the messages changed since the last time, by their keys.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// The same messages.
    Same,
    /// `n` new ones at the end.
    Appended(usize),
    /// `n` older ones at the start.
    Prepended(usize),
    /// The oldest `dropped` gone from the start (a history kept to a length), and `appended`
    /// new ones at the end.
    Trimmed {
        /// How many left the start.
        dropped: usize,
        /// How many came at the end.
        appended: usize,
    },
    /// Anything else (edits in place keep the same keys and count as `Same`).
    Replaced,
}

/// Compares the keys shown before with the keys now.
pub fn change<K: PartialEq>(before: &[K], now: &[K]) -> Change {
    if before == now {
        return Change::Same;
    }
    if now.len() > before.len() {
        let n = now.len() - before.len();
        if now.get(..before.len()) == Some(before) {
            return Change::Appended(n);
        }
        if now.get(n..) == Some(before) {
            return Change::Prepended(n);
        }
    }
    // The start dropped, maybe with new ones at the end: what stays of `before` begins `now`.
    if let Some(first) = now.first()
        && let Some(dropped) = before.iter().position(|k| k == first)
        && dropped > 0
    {
        let kept = before.len() - dropped;
        if now.len() >= kept && now.get(..kept) == before.get(dropped..) {
            return Change::Trimmed { dropped, appended: now.len() - kept };
        }
    }
    Change::Replaced
}

/// A chat list of `items`: a `Signal<Vec<T>>` replaced as a whole, a `SignalVec<T>` (pushes
/// for new messages, inserts at the front for older ones) or a [`w::ListItems`], as the
/// other lists take. `key` identifies a message across changes; `day` names the
/// section it belongs to (its date, say: a header goes before the first message of each);
/// `row` builds a message; `older` is called when the view nears the top, to load older
/// messages (it may add them later; they appear above without moving the view).
#[track_caller]
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn chat_list<T: Clone + 'static, K: Clone + Eq + Hash + 'static>(
    ui: &Ui,
    label: impl Into<String>,
    items: impl Into<w::ListItems<T>>,
    key: impl Fn(&T) -> K + 'static,
    day: impl Fn(&T) -> String + 'static,
    estimate: f32,
    row: impl Fn(&Ui, &T) -> El + 'static,
    state: &ChatState,
    older: impl Fn() + 'static,
) -> El {
    let unit = ui.theme().spacing.unit;
    let items: w::ListItems<T> = items.into();
    let label = label.into();
    let key: Rc<dyn Fn(&T) -> K> = Rc::new(key);
    let day: Rc<dyn Fn(&T) -> String> = Rc::new(day);
    let row: RowFn<T> = Rc::new(row);
    let older: Rc<dyn Fn()> = Rc::new(older);
    let state = *state;

    let vl = Rc::new(RefCell::new(VirtualList::new(0, estimate, true)));
    let keys: Rc<RefCell<Vec<K>>> = Rc::default();
    // The rows built, in order: (key, element).
    let built: Rc<RefCell<Vec<(K, El)>>> = Rc::default();
    // The first key when older messages were last asked for.
    let asked: Rc<RefCell<Option<K>>> = Rc::default();
    let first_day = Signal::new(String::new());

    let top = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let content = ui.el("virtual_rows").column().fill_width().layout(|l| l.shrink = 0.0);
    let bottom = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
    let scroller = w::scroll(ui)
        .grow(1.0)
        .fill_width()
        .layout(|l| l.min_height = Length::Px(0.0))
        .child(top.clone())
        .child(content.clone())
        .child(bottom.clone());

    // Builds and drops rows for the view, and applies the scroll the model wants.
    let refresh: Rc<dyn Fn()> = {
        let (vl, keys, built, scroller, content, ui2, key, day, row) = (
            vl.clone(),
            keys.clone(),
            built.clone(),
            scroller.clone(),
            content.clone(),
            ui.clone(),
            key.clone(),
            day.clone(),
            row.clone(),
        );
        let (asked, older) = (asked.clone(), older.clone());
        let busy = Rc::new(Cell::new(false));
        Rc::new(move || {
            // Measuring can scroll, which calls back here: one pass at a time.
            if busy.replace(true) {
                return;
            }
            let list = items.snapshot();
            let mut v = vl.borrow_mut();
            // Heights of rows on screen replace their estimates; the model keeps the anchor.
            {
                let order = keys.borrow();
                for (k, el) in built.borrow().iter() {
                    if let (Some(i), Some(b)) = (order.iter().position(|x| x == k), el.bounds())
                        && b.height() > 0.0
                    {
                        v.set_height(i, b.height());
                    }
                }
            }
            if let Some(b) = scroller.bounds() {
                v.set_viewport(b.height());
            }
            let range = v.visible_range(estimate * 4.0);
            let range = range.start.min(list.len())..range.end.min(list.len());
            let want: Vec<K> = list.get(range.clone()).unwrap_or_default().iter().map(|t| key(t)).collect();
            // Drop rows leaving the view; build rows entering it, in place.
            {
                let mut b = built.borrow_mut();
                b.retain(|(k, el)| {
                    let keep = want.contains(k);
                    if !keep {
                        el.remove();
                    }
                    keep
                });
                for (pos, i) in range.clone().enumerate() {
                    let Some(item) = list.get(i) else { continue };
                    let k = key(item);
                    if b.get(pos).is_some_and(|(bk, _)| *bk == k) {
                        continue;
                    }
                    // A new day starts with its header.
                    let today = day(item);
                    let header = i == 0 || list.get(i - 1).is_none_or(|p| day(p) != today);
                    let mut el = ui2.el("chat_row").column().fill_width();
                    if header {
                        el = el.child(
                            w::text(&ui2, today.clone())
                                .variant("muted")
                                .semantics(Semantics::new(Role::Heading(3)).label(today))
                                .layout(move |l| l.align_self = Some(remedy_ui_layout::Align::Center))
                                .padding(Insets::symmetric(unit * 2.0, 0.0)),
                        );
                    }
                    let el = el
                        .child(row(&ui2, item))
                        .semantics(Semantics::new(Role::ListItem).set_position(i + 1, list.len()));
                    content.insert_child(pos, &el);
                    b.insert(pos, (k, el));
                }
            }
            let above = v.offset_of(range.start);
            let below = (v.total_height() - v.offset_of(range.end)).max(0.0);
            let _ = top.clone().layout(move |l| l.height = Length::Px(above));
            let _ = bottom.clone().layout(move |l| l.height = Length::Px(below));
            // The day of the message at the top of the view.
            let first = v.index_at(v.scroll_offset());
            if let Some(item) = list.get(first) {
                first_day.set_if_changed(day(item));
            }
            // Where the model wants the view (anchored, or at the end).
            let want_y = v.scroll_offset();
            let now_y = ui2.scroll_offset(scroller.id()).y;
            if (want_y - now_y).abs() > 0.5 {
                scroller.scroll_to(Vec2::new(0.0, want_y));
            }
            state.at_end.set_if_changed(v.is_stuck_to_end());
            if v.is_stuck_to_end() {
                state.unread.set_if_changed(0);
            }
            // Near the top: ask for older messages, once per first message.
            let near_top = v.scroll_offset() < v.offset_of(range.start).max(1.0) + estimate * 2.0;
            let first_key = list.first().map(|t| key(t));
            let first_key_changed = *asked.borrow() != first_key;
            drop(v);
            busy.set(false);
            if near_top && first_key.is_some() && first_key_changed && !list.is_empty() {
                *asked.borrow_mut() = first_key;
                older();
            }
        })
    };

    // The person scrolled.
    let (vl2, r2) = (vl.clone(), refresh.clone());
    let scroller = scroller.on_scroll(move |o| {
        vl2.borrow_mut().scroll_to(o.y);
        r2();
    });
    // The view or rows changed size (rows measured, the window resized).
    let (r3, r4) = (refresh.clone(), refresh.clone());
    let scroller = scroller.on_resize(move |_: Size| r3());
    let content = content.on_resize(move |_: Size| r4());
    let _ = content;

    // The messages changed.
    let (vl5, keys5, built5, r5) = (vl.clone(), keys.clone(), built.clone(), refresh.clone());
    let key5 = key.clone();
    let watcher = ui.el("chat_items").hidden(true).bind(move || {
        let list = items.get();
        untrack(|| {
            let now: Vec<K> = list.iter().map(|t| key5(t)).collect();
            let before = keys5.borrow().clone();
            {
                let mut v = vl5.borrow_mut();
                match change(&before, &now) {
                    Change::Same => {}
                    Change::Appended(n) => {
                        v.push(n);
                        if !v.is_stuck_to_end() {
                            state.unread.update(|u| *u += n);
                        }
                    }
                    Change::Prepended(n) => v.insert(0, n),
                    Change::Trimmed { dropped, appended } => {
                        // The view stays on what it showed (scrolling up through history
                        // while old messages are let go does not jump to the top).
                        v.remove(0..dropped);
                        if appended > 0 {
                            v.push(appended);
                            if !v.is_stuck_to_end() {
                                state.unread.update(|u| *u += appended);
                            }
                        }
                    }
                    Change::Replaced => {
                        let stuck = v.is_stuck_to_end();
                        *v = VirtualList::new(now.len(), estimate, true);
                        if !stuck {
                            v.jump_to(0, JumpAlign::Start);
                        }
                        for (_, el) in built5.borrow_mut().drain(..) {
                            el.remove();
                        }
                    }
                }
            }
            *keys5.borrow_mut() = now;
            r5();
        });
    });

    // "Jump to latest", while reading higher up.
    let (vl6, r6) = (vl.clone(), refresh.clone());
    let jump = state.jump;
    let jumper = ui.el("chat_jump").hidden(true).bind(move || {
        if jump.get() == 0 {
            return;
        }
        untrack(|| {
            let mut v = vl6.borrow_mut();
            let n = v.len();
            if n > 0 {
                v.jump_to(n - 1, JumpAlign::End);
            }
            drop(v);
            state.unread.set(0);
            r6();
        });
    });
    let button = w::button(ui, "Jump to latest", move || state.jump_to_latest()).variant("primary");
    let (b2, at_end, unread) = (button.clone(), state.at_end, state.unread);
    let button = button.bind(move || {
        let n = unread.get();
        let text = if n > 0 { format!("{n} new") } else { "Jump to latest".to_owned() };
        if let Some(l) = b2.child_elements().into_iter().next() {
            let _ = l.text(remedy_ui_view::RichText::plain(text.clone()));
        }
        // What a screen reader says matches what the button shows.
        let spoken = if n > 0 { format!("{n} new messages, jump to latest") } else { text };
        let _ = b2
            .clone()
            .hidden(at_end.get())
            .semantics(Semantics::new(Role::Button).label(spoken).action(remedy_ui_a11y::Action::Click));
    });
    let button = button.layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: AUTO, start: AUTO, end: 16.0, bottom: 16.0 };
    });
    // The day pinned at the top.
    let pinned = w::text(ui, "").variant("muted").text(move || first_day.get()).layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets { top: 8.0, start: AUTO, end: AUTO, bottom: AUTO };
        l.align_self = Some(remedy_ui_layout::Align::Center);
    });
    let (p2, fd) = (pinned.clone(), first_day);
    let pinned = pinned.pass_through().bind(move || {
        let _ = p2.clone().hidden(fd.get().is_empty());
    });

    let (st, vl7, r7) = (state, vl.clone(), refresh.clone());
    ui.el("chat_list")
        .column()
        .grow(1.0)
        .fill_width()
        .align(remedy_ui_layout::Align::Center)
        .layout(|l| l.min_height = Length::Px(0.0))
        .child(scroller)
        .child(pinned)
        .child(button)
        .child(watcher)
        .child(jumper)
        .on_key(move |e| {
            // End goes to the latest; Home to the oldest loaded.
            if !e.pressed {
                return false;
            }
            match e.key {
                Key::End if e.modifiers.ctrl => st.jump_to_latest(),
                Key::Home if e.modifiers.ctrl => {
                    vl7.borrow_mut().jump_to(0, JumpAlign::Start);
                    r7();
                }
                _ => return false,
            }
            true
        })
        .semantics(Semantics::new(Role::List).label(label))
}

#[cfg(test)]
mod tests {
    use super::{Change, change};

    #[test]
    fn changes_are_told_apart_by_key() {
        assert_eq!(change(&[1, 2, 3], &[1, 2, 3]), Change::Same);
        assert_eq!(change(&[1, 2, 3], &[1, 2, 3, 4, 5]), Change::Appended(2));
        assert_eq!(change(&[3, 4], &[1, 2, 3, 4]), Change::Prepended(2));
        assert_eq!(change(&[1, 2], &[2, 3]), Change::Trimmed { dropped: 1, appended: 1 });
        assert_eq!(change(&[1, 2, 3, 4], &[3, 4]), Change::Trimmed { dropped: 2, appended: 0 });
        assert_eq!(change(&[1, 2, 3], &[2, 1, 3]), Change::Replaced);
        assert_eq!(change(&[1, 2], &[5, 6]), Change::Replaced);
        assert_eq!(change::<u8>(&[], &[1]), Change::Appended(1));
    }
}
