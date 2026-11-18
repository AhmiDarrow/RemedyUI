//! The user's own words: ones they added, and ones they chose to ignore for now.

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Default)]
struct State {
    words: BTreeSet<String>,
    ignored: HashSet<String>,
    version: u64,
}

/// A personal dictionary. Words added to it are accepted in every language and kept until the
/// user removes them; the app saves them with [`PersonalDictionary::to_text`] and loads them
/// back with [`PersonalDictionary::load_text`]. Words ignored are accepted until the app
/// ends and are never part of the saved list.
///
/// A word added in lowercase is accepted in any capitalization (`ahmi` accepts `Ahmi`); a word
/// added with capitals is accepted only as written.
///
/// Clones share one dictionary, so a field's checker and a settings page see the same words.
///
/// ```
/// use remedy_ui_services_spell::PersonalDictionary;
/// let d = PersonalDictionary::default();
/// assert!(d.add("Remedy"));
/// assert!(d.contains("Remedy") && !d.contains("remedy"));
/// d.ignore("teh");
/// assert!(d.contains("teh"));
/// assert_eq!(d.to_text(), "Remedy\n");
/// ```
#[derive(Clone, Default)]
pub struct PersonalDictionary {
    state: Arc<RwLock<State>>,
}

impl std::fmt::Debug for PersonalDictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.read();
        write!(f, "PersonalDictionary({} words, {} ignored)", s.words.len(), s.ignored.len())
    }
}

impl PersonalDictionary {
    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// True when both are the same dictionary (clones of one).
    pub fn same(&self, other: &PersonalDictionary) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    /// Words are kept in one Unicode form (NFC), so "é" typed as one character or as "e"
    /// and an accent is the same word.
    fn form(word: &str) -> String {
        icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(word.trim()).into_owned()
    }

    /// The words it accepts changed: fields checked with it check again.
    fn changed(&self) {
        crate::field::dictionary_changed(self);
    }

    /// Adds `word` to the saved words. Returns false for text that is not one word (empty, or
    /// with spaces in it) or that was there already.
    pub fn add(&self, word: &str) -> bool {
        let word = Self::form(word);
        if word.is_empty() || word.chars().any(char::is_whitespace) {
            return false;
        }
        let added = {
            let mut s = self.write();
            let was_ignored = s.ignored.remove(&word);
            let added = s.words.insert(word);
            if added {
                s.version += 1;
            }
            added || was_ignored
        };
        if added {
            self.changed();
        }
        added
    }

    /// Removes `word` from the saved words and from the ignored ones. Returns true if it was
    /// in either.
    pub fn remove(&self, word: &str) -> bool {
        let word = Self::form(word);
        let gone = {
            let mut s = self.write();
            let saved = s.words.remove(&word);
            if saved {
                s.version += 1;
            }
            saved | s.ignored.remove(&word)
        };
        if gone {
            self.changed();
        }
        gone
    }

    /// Accepts `word` until the app ends, without saving it.
    pub fn ignore(&self, word: &str) {
        let word = Self::form(word);
        if !word.is_empty() && self.write().ignored.insert(word) {
            self.changed();
        }
    }

    /// Stops ignoring every word that was ignored for the session.
    pub fn clear_ignored(&self) {
        let had = {
            let mut s = self.write();
            let had = !s.ignored.is_empty();
            s.ignored.clear();
            had
        };
        if had {
            self.changed();
        }
    }

    /// True when `word` is saved or ignored (see the type's docs for capitals).
    pub fn contains(&self, word: &str) -> bool {
        let word = Self::form(word);
        let word = word.as_str();
        let s = self.read();
        let accepted = |w: &str| s.words.contains(w) || s.ignored.contains(w);
        if accepted(word) {
            return true;
        }
        let lower = word.to_lowercase();
        lower != word && accepted(&lower)
    }

    /// The saved words in alphabetical order.
    pub fn words(&self) -> Vec<String> {
        self.read().words.iter().cloned().collect()
    }

    /// A number that changes whenever the saved words change, for an app deciding when to
    /// save. Ignoring a word does not change it.
    pub fn version(&self) -> u64 {
        self.read().version
    }

    /// The saved words, one per line, to write to the app's settings.
    pub fn to_text(&self) -> String {
        let s = self.read();
        let mut out = String::new();
        for w in &s.words {
            out.push_str(w);
            out.push('\n');
        }
        out
    }

    /// Replaces the saved words with the lines of `text` (blank lines and lines starting with
    /// `#` are skipped). Returns how many words were loaded.
    pub fn load_text(&self, text: &str) -> usize {
        let words: BTreeSet<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.chars().any(char::is_whitespace))
            .map(Self::form)
            .collect();
        let n = words.len();
        let changed = {
            let mut s = self.write();
            let changed = s.words != words;
            if changed {
                s.words = words;
                s.version += 1;
            }
            changed
        };
        if changed {
            self.changed();
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_words_are_accepted_and_listed_in_order() {
        let d = PersonalDictionary::default();
        assert!(d.add("zebra") && d.add("Ahmi") && d.add("  apple "));
        assert!(!d.add("zebra"), "already there");
        assert!(!d.add(""));
        assert!(!d.add("two words"));
        assert_eq!(d.words(), ["Ahmi", "apple", "zebra"]);
        assert!(d.contains("apple") && d.contains("Ahmi"));
        assert!(!d.contains("ahmi"), "a capitalized word is accepted only as written");
        assert!(d.contains("Zebra") && d.contains("ZEBRA"), "a lowercase word is accepted in any case");
    }

    #[test]
    fn ignored_words_last_for_the_session_and_are_not_saved() {
        let d = PersonalDictionary::default();
        let v = d.version();
        d.ignore("teh");
        assert!(d.contains("teh"));
        assert_eq!(d.version(), v, "ignoring changes nothing to save");
        assert_eq!(d.to_text(), "");
        d.clear_ignored();
        assert!(!d.contains("teh"));
    }

    #[test]
    fn adding_replaces_an_ignore_and_removing_clears_both() {
        let d = PersonalDictionary::default();
        d.ignore("foo");
        assert!(d.add("foo"));
        d.clear_ignored();
        assert!(d.contains("foo"), "added words survive clearing the ignored ones");
        assert!(d.remove("foo"));
        assert!(!d.contains("foo"));
        d.ignore("bar");
        assert!(d.remove("bar"), "an ignored word can be removed too");
        assert!(!d.remove("bar"));
    }

    #[test]
    fn the_word_list_round_trips_through_text() {
        let d = PersonalDictionary::default();
        d.add("Remedy");
        d.add("hunspell");
        let saved = d.to_text();
        assert_eq!(saved, "Remedy\nhunspell\n");
        let e = PersonalDictionary::default();
        assert_eq!(e.load_text(&format!("# my words\n\n{saved}two words\n")), 2);
        assert_eq!(e.words(), d.words());
        let v = e.version();
        e.load_text(&saved);
        assert_eq!(e.version(), v, "loading the same list is no change");
        e.load_text("other\n");
        assert_eq!(e.words(), ["other"], "loading replaces");
        assert_ne!(e.version(), v);
    }

    #[test]
    fn versions_change_with_the_saved_words() {
        let d = PersonalDictionary::default();
        let v0 = d.version();
        d.add("x1");
        let v1 = d.version();
        assert!(v1 > v0);
        d.add("x1");
        assert_eq!(d.version(), v1);
        d.remove("x1");
        assert!(d.version() > v1);
    }

    #[test]
    fn clones_share_the_words() {
        let a = PersonalDictionary::default();
        let b = a.clone();
        a.add("shared");
        assert!(b.contains("shared"));
    }
}
