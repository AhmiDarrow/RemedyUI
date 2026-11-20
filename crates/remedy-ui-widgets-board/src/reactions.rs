//! Reactions: emoji with counts that people toggle, a component any widget (or app screen)
//! can use. Shown as toggle buttons that say their count and whether they are the viewer's.

use remedy_ui_a11y::{Action as A11yAction, Role, Semantics};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

pub use crate::labels::ReactionLabels;
use crate::labels::fill;

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
            r.count += 1;
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
    let n = labels.count(u64::from(r.count));
    let template = if r.mine { &labels.spoken_mine } else { &labels.spoken };
    fill(template, &[("emoji", r.emoji.clone()), ("count", n)])
}

/// The reactions `shown`, plus each of `offered` not chosen yet; pressing one calls
/// `on_toggle` with its emoji. In English; [`reactions_in`] takes the words.
pub fn reactions(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
    reactions_in(ui, shown, offered, &ReactionLabels::for_locale(&ui.locale()), on_toggle)
}

/// [`reactions()`] in the words of `labels`.
pub fn reactions_in(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    labels: &ReactionLabels,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
    let words = labels.clone();
    let group = labels.group.clone();
    let unit = ui.theme().spacing.unit;
    let row = w::wrap(ui).gap(unit);
    let on_toggle = std::rc::Rc::new(on_toggle);
    let (r2, u2) = (row.clone(), ui.clone());
    row.bind(move || {
        let list = shown.get();
        untrack(|| {
            r2.clear();
            let mut all: Vec<Reaction> = list.clone();
            for e in &offered {
                if !all.iter().any(|r| r.emoji == *e) {
                    all.push(Reaction { emoji: e.clone(), count: 0, mine: false });
                }
            }
            for r in all {
                let label = if r.count > 0 { format!("{} {}", r.emoji, r.count) } else { r.emoji.clone() };
                let (t, emoji) = (on_toggle.clone(), r.emoji.clone());
                let b = w::button(&u2, label, move || t(&emoji))
                    .variant(if r.mine { "primary" } else { "quiet" })
                    .semantics(
                        Semantics::new(Role::Button)
                            .label(spoken_in(&r, &words))
                            .checked(r.mine)
                            .action(A11yAction::Click),
                    );
                let _ = r2.clone().child(b);
            }
        });
    })
    .semantics(Semantics::new(Role::Group).label(group))
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
