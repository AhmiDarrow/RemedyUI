//! Reactions: emoji with counts that people toggle, a component any widget (or app screen)
//! can use. Shown as toggle buttons that say their count and whether they are the viewer's.

use std::cell::RefCell;

use crate::base as w;
use remedy_ui_core::a11y::{Action as A11yAction, Role, Semantics};
use remedy_ui_core::reactive::{Scope, Signal, untrack};
use remedy_ui_engine::view::{El, Ui};

pub use crate::board::labels::ReactionLabels;
use crate::board::labels::fill;

/// One emoji's reactions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// The emoji.
    pub emoji: String,
    /// How many people chose it.
    pub count: u32,
    /// The viewer is one of them.
    pub mine: bool,
}

/// Toggles the viewer's `emoji` in `list`: adds it (or counts them in), or takes it back
/// (dropping an emoji nobody chose any more).
pub fn toggle(list: &mut Vec<Reaction>, emoji: &str) {
    match list.iter_mut().find(|r| r.emoji == emoji) {
        Some(r) if r.mine => {
            r.mine = false;
            r.count = r.count.saturating_sub(1);
        }
        Some(r) => {
            r.mine = true;
            r.count = r.count.saturating_add(1);
        }
        None => list.push(Reaction { emoji: emoji.to_owned(), count: 1, mine: true }),
    }
    list.retain(|r| r.count > 0);
}

/// What a reaction button says to a screen reader, in English.
pub fn spoken(r: &Reaction) -> String {
    spoken_in(r, &ReactionLabels::default())
}

/// What a reaction button says to a screen reader, in `labels`.
pub fn spoken_in(r: &Reaction, labels: &ReactionLabels) -> String {
    let n = labels.count(r.count);
    let template = if r.mine { &labels.spoken_mine } else { &labels.spoken };
    fill(template, &[("emoji", r.emoji.clone()), ("count", n)])
}

/// The reactions `shown`, plus each of `offered` not chosen yet; pressing one calls
/// `on_toggle` with its emoji. In the window's language ([`ReactionLabels::localized`] for
/// its locale, following it as it changes); [`reactions_in`] takes the words.
pub fn reactions(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
    reactions_with(ui, shown, offered, None, on_toggle)
}

/// [`reactions()`] in the words of `labels`.
pub fn reactions_in(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    labels: &ReactionLabels,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
    reactions_with(ui, shown, offered, Some(labels.clone()), on_toggle)
}

/// The reactions in `labels`, or (`None`) in the window's language.
fn reactions_with(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    labels: Option<ReactionLabels>,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
    // Read in a binding, so the window's language is followed.
    let (u1, fixed) = (ui.clone(), labels);
    let words_now = move || fixed.clone().unwrap_or_else(|| ReactionLabels::localized(&u1.locale()));
    let group_words = words_now.clone();
    let unit = ui.theme().spacing.unit;
    let row = w::wrap(ui).gap(unit);
    let on_toggle = std::rc::Rc::new(on_toggle);
    let (r2, u2) = (row.clone(), ui.clone());
    // What every button shows, by its emoji: kept as long as the row, whatever binding the
    // row was built in.
    let (all, scope) = Scope::detached(|| Signal::new(Vec::<Reaction>::new()));
    // The buttons in order, kept by emoji and updated in place: one built again on every
    // toggle would take the keyboard's focus with it.
    let buttons: RefCell<Vec<(String, El)>> = RefCell::new(Vec::new());
    row.bind(move || {
        let mut now = shown.get();
        for e in &offered {
            if !now.iter().any(|r| r.emoji == *e) {
                now.push(Reaction { emoji: e.clone(), count: 0, mine: false });
            }
        }
        untrack(|| {
            let mut kept = buttons.borrow_mut();
            kept.retain(|(emoji, b)| {
                let stays = now.iter().any(|r| r.emoji == *emoji);
                if !stays {
                    b.remove();
                }
                stays
            });
            all.set_if_changed(now.clone());
            for (at, r) in now.iter().enumerate() {
                let from = kept.iter().skip(at).position(|(emoji, _)| *emoji == r.emoji).map(|i| i + at);
                let item = match from {
                    Some(from) if from == at => continue,
                    Some(from) => kept.remove(from),
                    None => {
                        let emoji = r.emoji.clone();
                        let of = move || {
                            all.with(|v| v.iter().find(|r| r.emoji == emoji).cloned())
                                .unwrap_or_else(|| Reaction { emoji: emoji.clone(), count: 0, mine: false })
                        };
                        let (label_of, variant_of) = (of.clone(), of.clone());
                        let label = move || {
                            let r = label_of();
                            if r.count > 0 { format!("{} {}", r.emoji, r.count) } else { r.emoji }
                        };
                        let (t, emoji, words) = (on_toggle.clone(), r.emoji.clone(), words_now.clone());
                        let b = w::button(&u2, label, move || t(&emoji))
                            .variant(move || if variant_of().mine { "primary" } else { "quiet" })
                            .semantics(move || {
                                let r = of();
                                Semantics::new(Role::Button)
                                    .label(spoken_in(&r, &words()))
                                    .checked(r.mine)
                                    .action(A11yAction::Click)
                            });
                        (r.emoji.clone(), b)
                    }
                };
                // Moving a button detaches it: the keyboard's focus goes back to it.
                let focused = item.1.is_focused();
                r2.insert_child(at, &item.1);
                if focused {
                    item.1.focus();
                }
                let to = at.min(kept.len());
                kept.insert(to, item);
            }
            // The same emoji listed twice had a button each.
            let extra = now.len().min(kept.len());
            for (_, b) in kept.drain(extra..) {
                b.remove();
            }
        });
    })
    .on_removed(move || scope.dispose())
    .semantics(move || Semantics::new(Role::Group).label(group_words().group))
}

#[cfg(test)]
mod tests {
    use super::{Reaction, ReactionLabels, spoken, spoken_in, toggle};

    #[test]
    fn toggling_counts_the_viewer_in_and_out() {
        let mut v = vec![Reaction { emoji: "👍".into(), count: 2, mine: false }];
        toggle(&mut v, "👍");
        assert_eq!(v.first().map(|r| (r.count, r.mine)), Some((3, true)));
        toggle(&mut v, "🎉");
        assert_eq!(v.len(), 2);
        toggle(&mut v, "🎉");
        assert_eq!(v.len(), 1, "an emoji nobody chose any more goes");
        toggle(&mut v, "👍");
        assert_eq!(v.first().map(|r| (r.count, r.mine)), Some((2, false)));
        assert_eq!(
            spoken(&Reaction { emoji: "❤️".into(), count: 1, mine: true }),
            "❤️, 1 reaction, including yours"
        );
        let de = ReactionLabels::default().translated(|en| {
            match en {
                "{count} reactions" => "{count} Reaktionen",
                "{emoji}, {count}" => "{count} mit {emoji}",
                other => other,
            }
            .to_owned()
        });
        assert_eq!(
            spoken_in(&Reaction { emoji: "👍".into(), count: 3, mine: false }, &de),
            "3 Reaktionen mit 👍"
        );
    }
}
