//! Reactions: emoji with counts that people toggle, a component any widget (or app screen)
//! can use. Shown as toggle buttons that say their count and whether they are the viewer's.

use remedy_ui_a11y::{Action as A11yAction, Role, Semantics};
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;

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

/// What a reaction button says to a screen reader.
pub fn spoken(r: &Reaction) -> String {
    let n = match r.count {
        1 => "1 reaction".to_owned(),
        n => format!("{n} reactions"),
    };
    if r.mine { format!("{}, {n}, including yours", r.emoji) } else { format!("{}, {n}", r.emoji) }
}

/// The reactions `shown`, plus each of `offered` not chosen yet; pressing one calls
/// `on_toggle` with its emoji.
pub fn reactions(
    ui: &Ui,
    shown: Signal<Vec<Reaction>>,
    offered: Vec<String>,
    on_toggle: impl Fn(&str) + 'static,
) -> El {
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
                            .label(spoken(&r))
                            .checked(r.mine)
                            .action(A11yAction::Click),
                    );
                let _ = r2.clone().child(b);
            }
        });
    })
    .semantics(Semantics::new(Role::Group).label("Reactions"))
}

#[cfg(test)]
mod tests {
    use super::{Reaction, spoken, toggle};

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
    }
}
