//! The words a board says: every fixed string of its editor, its empty states, its built-in
//! widgets and its reactions. `Default` is English; `localized` takes them from the board's
//! Fluent catalog (`i18n/*.ftl`, the eleven languages the engine ships), which is what a
//! board left with the defaults speaks, in the window's locale; apps may give their own.
//!
//! Words with `{name}` placeholders are templates: the board fills them after translation,
//! so a translation keeps the placeholders and may move them. Counts that need plural
//! forms (reactions) take the catalog's plural message while the words are the catalog's.

use remedy_ui_engine::view::Ui;
use remedy_ui_engine::view::strings::StrArg;

use crate::model::BoardError;

/// Defines a struct of words: each field a `String` with its English default, plus nested
/// word structs, `translated` that passes every word through a translator, and `localized`
/// that takes every word from the catalog ([`crate::STRINGS`]): message `$prefix` and the
/// field's name in kebab case.
macro_rules! words {
    (
        $(#[$m:meta])*
        $name:ident $prefix:literal {
            $( $(#[$fm:meta])* $f:ident: $en:literal, )*
        }
        $( nested { $( $(#[$nm:meta])* $nf:ident: $nt:ty, )* } )?
    ) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub struct $name {
            $( $(#[$fm])* pub $f: String, )*
            $( $( $(#[$nm])* pub $nf: $nt, )* )?
            /// The catalog locale these words came from ([`Self::localized`]), while they
            /// are its words; counts then take the language's plural forms.
            catalog_locale: Option<String>,
        }

        impl Default for $name {
            /// The English words.
            fn default() -> Self {
                $name {
                    $( $f: $en.to_owned(), )*
                    $( $( $nf: <$nt>::default(), )* )?
                    catalog_locale: None,
                }
            }
        }

        impl $name {
            /// Every word passed through `translate`, which is given a word as it is now (the
            /// English one, by default) and returns it in the person's language, keeping any
            /// `{name}` placeholders: a catalog keyed by source text (such as
            /// `remedy_ui::i18n::Catalog`) or the app's own `t()`.
            pub fn translated(&self, translate: impl Fn(&str) -> String) -> Self {
                $name {
                    $( $f: translate(&self.$f), )*
                    $( $( $nf: self.$nf.translated(&translate), )* )?
                    catalog_locale: None,
                }
            }

            /// Every word in `locale` (a BCP 47 tag such as `de-DE`; the closest language
            /// the engine ships answers, else English) from the board's catalog
            /// ([`crate::STRINGS`], with any rewording an app added under `widgets-board`).
            /// Templates keep their `{name}` placeholders for the board to fill, and while
            /// the words are left as they come, counts take the language's plural forms.
            /// A board whose [`crate::Context::labels`] are left at their default uses
            /// these, for the window's locale.
            pub fn localized(locale: &str) -> Self {
                $name {
                    $( $f: message(locale, $prefix, stringify!($f), $en), )*
                    $( $( $nf: <$nt>::localized(locale), )* )?
                    catalog_locale: Some(locale.to_owned()),
                }
            }

            /// Every word with the id of its catalog message (the one [`Self::localized`]
            /// reads, and [`remedy_ui_engine::view::strings::add_messages`] rewords), nested words
            /// included, in field order.
            pub fn words(&self) -> Vec<(String, &str)> {
                #[allow(unused_mut)]
                let mut out = vec![$( (message_id($prefix, stringify!($f)), self.$f.as_str()), )*];
                $( $( out.extend(self.$nf.words()); )* )?
                out
            }
        }
    };
}

words! {
    /// The words of the reactions component ([`crate::reactions()`]).
    ReactionLabels "reaction-" {
        /// The group's name for screen readers.
        group: "Reactions",
        /// One reaction, as counted for screen readers.
        one: "1 reaction",
        /// Several (or no) reactions: `{count}`.
        many: "{count} reactions",
        /// A reaction button for screen readers: `{emoji}` and `{count}` (a count from `one`
        /// or `many`).
        spoken: "{emoji}, {count}",
        /// A reaction button that includes the viewer's: `{emoji}`, `{count}`.
        spoken_mine: "{emoji}, {count}, including yours",
    }
}

words! {
    /// Every word a board says: its editor and panel, its empty states, the messages of
    /// refused changes, the setting editors, the built-in widgets and the reactions. Set on
    /// [`crate::Context::labels`]; `Default` is English, and a board left with it speaks
    /// the window's language ([`BoardLabels::localized`]).
    ///
    /// ```
    /// # use remedy_ui_widgets_board::BoardLabels;
    /// let de = BoardLabels::localized("de-DE");
    /// assert_eq!(de.undo, "Rückgängig");
    /// assert_eq!(de.list_remove, "{label} {number} entfernen");
    /// ```
    ///
    /// ```
    /// # use remedy_ui_widgets_board::BoardLabels;
    /// let fr = BoardLabels::default().translated(|en| match en {
    ///     "Undo" => "Annuler".to_owned(),
    ///     other => other.to_owned(),
    /// });
    /// assert_eq!(fr.undo, "Annuler");
    /// ```
    BoardLabels "" {
        /// The undo button.
        undo: "Undo",
        /// The redo button.
        redo: "Redo",
        /// The button that adds an area.
        add_area: "Add area",
        /// The panel's heading for an area.
        area_heading: "Area",
        /// The area title field's name.
        area_title: "Area title",
        /// The label over the area title field.
        area_title_hint: "Title (Enter to apply)",
        /// Where an area is: `{column}`, `{row}` (from 1), `{columns}`, `{rows}`.
        area_place: "Column {column}, row {row}, {columns} wide, {rows} high",
        /// How to move an area from the keyboard, in the panel.
        area_keys_hint: "Arrow keys move it; Shift with arrows resizes it.",
        /// How to move an area from the keyboard, for assistive technology.
        area_keys: "The arrow keys move it; Shift with the arrow keys resizes it",
        /// The button that removes an area.
        remove_area: "Remove area",
        /// The panel's heading for a widget.
        widget_heading: "Widget",
        /// The settings grid's name.
        settings: "Settings",
        /// How to move a widget from the keyboard, in the panel.
        widget_keys_hint: "Alt with Up and Down moves it; Alt with Left and Right moves it to the next area.",
        /// How to move a widget from the keyboard, for assistive technology.
        widget_keys: "Alt with the arrow keys moves it",
        /// The button that removes a widget.
        remove_widget: "Remove widget",
        /// The panel with nothing selected.
        nothing_selected: "Select an area or a widget to change it.",
        /// The button that adds a widget.
        add_widget: "Add widget",
        /// The list of kinds to add.
        widget_kind: "Widget kind",
        /// A placed widget while arranging, for assistive technology: `{name}`.
        widget_name: "{name} widget",
        /// A placed widget of a kind the app lacks: `{kind}`.
        unknown_kind: "This board uses a widget this app does not have ({kind}).",
        /// An area with a title, for assistive technology: `{name}`.
        area_named: "{name} area",
        /// An area with no title and no single widget to name it: `{number}` (from 1, in
        /// reading order).
        area_numbered: "Area {number}",
        /// An empty area while arranging.
        empty_area: "Empty area: add a widget or drop one here.",
        /// An empty board's title.
        empty_board: "Empty board",
        /// An empty board while arranging.
        empty_board_editing: "This board is empty: add an area.",
        /// An empty board for visitors.
        empty_board_viewing: "Nothing here yet.",
        /// A second widget refused where an area holds one.
        one_widget: "This area already holds a widget: give the other one an area of its own.",
        /// Not a board: `{why}`.
        error_invalid: "not a board: {why}",
        /// Made by a newer version: `{format}`.
        error_too_new: "this board was made by a newer version (format {format})",
        /// Past the edge of the grid.
        error_outside: "that would go past the edge of the board",
        /// It would cover another area: `{area}` (its title, or its id).
        error_overlap: "that would cover the area {area}",
        /// Not on the board: `{id}`.
        error_missing: "{id} is not on the board",
        /// A time setting's date field: `{label}`.
        time_date: "{label}: date",
        /// A time setting's time field: `{label}`.
        time_time: "{label}: time",
        /// A time setting that is not set.
        time_not_set: "Not set",
        /// Clears a time setting.
        time_clear: "Clear",
        /// A stored choice that is no longer offered: `{value}`.
        choice_unavailable: "{value} (not available)",
        /// One item of a list setting: `{label}`, `{number}` (from 1).
        list_item: "{label} {number}",
        /// Adds an item to a list setting: `{label}`.
        list_add: "Add to {label}",
        /// Removes an item of a list setting: `{label}`, `{number}`.
        list_remove: "Remove {label} {number}",
        /// A picture setting's source field: `{label}`.
        picture_source: "{label}: address",
        /// Opens the app's picture chooser.
        picture_choose: "Choose picture",
        /// A scripted widget that stopped: `{why}`.
        stopped: "This widget stopped: {why}",
        /// Starts a stopped widget again.
        start_again: "Start again",
        /// The built-in text widget's name.
        text_name: "Text",
        /// Its title setting.
        text_title: "Title",
        /// Its title until set.
        text_title_default: "Welcome",
        /// Its text setting.
        text_body: "Text",
        /// Its text until set.
        text_body_default: "Say hello to the house.",
        /// The built-in links widget's name.
        links_name: "Links",
        /// Its title setting.
        links_title: "Title",
        /// Its title until set.
        links_title_default: "Links",
        /// Its links setting.
        links_links: "Links (Label | https://…, one a line)",
        /// It has no links.
        links_none: "No links yet.",
        /// The built-in reactions widget's name.
        reactions_name: "Reactions",
        /// Its question setting.
        reactions_question: "Question",
        /// Its question until set.
        reactions_question_default: "How is everyone?",
        /// Its setting of reactions to offer.
        reactions_offered: "Reactions to offer",
    }
    nested {
        /// The reactions component's words.
        reactions: ReactionLabels,
    }
}

/// `template` with each `{name}` filled from `args`.
pub(crate) fn fill(template: &str, args: &[(&str, String)]) -> String {
    remedy_ui_platform::services_i18n::fill(template, args)
}

/// `text` without the Unicode isolation marks Fluent wraps placeholders in: the board fills
/// its own placeholders, as it does in words apps give it.
fn unisolated(text: String) -> String {
    if text.contains(['\u{2068}', '\u{2069}']) { text.replace(['\u{2068}', '\u{2069}'], "") } else { text }
}

/// The `{name}` placeholders of an English word.
fn placeholders(en: &str) -> Vec<&str> {
    en.split('{').skip(1).filter_map(|part| part.split_once('}').map(|(name, _)| name)).collect()
}

/// The catalog message of field `field` of a word struct: `prefix` and the field's name in
/// kebab case.
fn message_id(prefix: &str, field: &str) -> String {
    format!("{prefix}{}", field.replace('_', "-"))
}

/// Field `field` of a word struct in `locale`: catalog message `prefix` + the field's name in
/// kebab case, each placeholder of the English word `en` given as its own `{name}`, so the
/// word comes back as a template with the board's placeholders where the language puts
/// them.
fn message(locale: &str, prefix: &str, field: &str, en: &str) -> String {
    let id = message_id(prefix, field);
    let names = placeholders(en);
    let values: Vec<String> = names.iter().map(|n| format!("{{{n}}}")).collect();
    let args: Vec<(&str, StrArg)> =
        names.iter().zip(&values).map(|(n, v)| (*n, StrArg::Text(v.clone()))).collect();
    unisolated(crate::STRINGS.format_in(locale, &id, &args))
}

impl ReactionLabels {
    /// How many reactions, as `one` and `many` say, or in the plural forms of the catalog's
    /// language while these are its words as [`ReactionLabels::localized`] gave them.
    pub(crate) fn count(&self, n: u32) -> String {
        if let Some(locale) = self.catalog_locale.as_deref() {
            let fresh = ReactionLabels::localized(locale);
            if fresh.one == self.one && fresh.many == self.many {
                return unisolated(crate::STRINGS.format_in(
                    locale,
                    "reaction-count",
                    &[("count", n.into())],
                ));
            }
        }
        match n {
            1 => self.one.clone(),
            n => fill(&self.many, &[("count", n.to_string())]),
        }
    }
}

impl BoardLabels {
    /// The words a board built in `ui` says: these, unless they are the English defaults
    /// ([`BoardLabels::default`], what [`crate::Context::local`] starts with), which give
    /// way to [`BoardLabels::localized`] for the window's locale (read tracked, so a board
    /// built again on a locale change speaks the new language).
    pub(crate) fn for_window(&self, ui: &Ui) -> BoardLabels {
        if *self == BoardLabels::default() { BoardLabels::localized(&ui.locale()) } else { self.clone() }
    }

    /// Why a change was refused, in these words. `area` names an area by its id (its title,
    /// where it has one).
    pub fn error(&self, e: &BoardError, area: impl Fn(&str) -> String) -> String {
        match e {
            BoardError::Invalid(why) => fill(&self.error_invalid, &[("why", why.clone())]),
            BoardError::TooNew(v) => fill(&self.error_too_new, &[("format", v.to_string())]),
            BoardError::OutsideGrid => self.error_outside.clone(),
            BoardError::Overlap(id) => fill(&self.error_overlap, &[("area", area(id))]),
            BoardError::Missing(id) => fill(&self.error_missing, &[("id", id.clone())]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_is_translated_and_placeholders_are_filled_after() {
        let upper = BoardLabels::default().translated(|s| s.replace(" area", " AREA"));
        assert_eq!(upper.add_area, "Add AREA");
        assert_eq!(upper.area_named, "{name} AREA");
        assert_eq!(upper.reactions.group, "Reactions", "nested words go through too");
        let said = upper.error(&BoardError::Overlap("area1".into()), |_| "Welcome".into());
        assert_eq!(said, "that would cover the AREA Welcome");
        let de = BoardLabels::default()
            .translated(|s| if s == "{count} reactions" { "{count} Reaktionen".into() } else { s.into() });
        assert_eq!(de.reactions.many, "{count} Reaktionen");
    }

    #[test]
    fn catalog_words_count_with_the_languages_plurals() {
        let ru = ReactionLabels::localized("ru");
        assert_eq!(ru.count(1), "1 реакция");
        assert_eq!(ru.count(3), "3 реакции");
        assert_eq!(ru.count(5), "5 реакций");
        assert_eq!(ru.count(21), "21 реакция");
        let ar = ReactionLabels::localized("ar-EG");
        assert_eq!(ar.count(0), "لا تفاعلات");
        assert_eq!(ar.count(2), "تفاعلان");
        assert_eq!(ar.count(11), "11 تفاعلًا");
        // Reworded words are the app's: their own one and many count.
        let mut mine = ReactionLabels::localized("ru");
        mine.many = "реакций: {count}".into();
        assert_eq!(mine.count(3), "реакций: 3");
        let translated = ReactionLabels::localized("ru").translated(str::to_owned);
        assert_eq!(translated.count(3), "Реакций: 3", "translated words are the app's too");
        assert_eq!(ReactionLabels::default().count(1), "1 reaction");
        assert_eq!(ReactionLabels::localized("en").count(0), "0 reactions");
        assert_eq!(
            BoardLabels::localized("ar").area_place,
            "العمود {column}، الصف {row}، العرض {columns}، الارتفاع {rows}"
        );
    }
}
