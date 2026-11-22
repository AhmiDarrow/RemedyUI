//! The user's languages from the POSIX locale variables. Pure, so it is tested on any machine.

/// `en_US.UTF-8` (POSIX) to `en-US` (BCP 47); `None` for the C locale or nothing.
fn tag(raw: &str) -> Option<String> {
    let base = raw.split(['.', '@']).next()?.trim();
    (!base.is_empty() && base != "C" && base != "POSIX").then(|| base.replace('_', "-"))
}

/// The locale: the first of `LC_ALL`, `LC_MESSAGES` and `LANG` that is set, as `get` reads them.
pub(crate) fn locale(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| get(k).filter(|v| !v.is_empty()))?;
    tag(&raw)
}

/// The preferred languages: `LANGUAGE`'s colon-separated list, then the locale. As in GNU
/// gettext, `LANGUAGE` counts only when the locale is not C. Empty when neither says.
pub(crate) fn locales(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let Some(locale) = locale(&get) else { return Vec::new() };
    let mut list: Vec<String> = Vec::new();
    for t in get("LANGUAGE").unwrap_or_default().split(':').filter_map(tag) {
        if !list.contains(&t) {
            list.push(t);
        }
    }
    if !list.contains(&locale) {
        list.push(locale);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::{locale, locales};

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn the_locale_comes_from_lc_all_then_lc_messages_then_lang() {
        assert_eq!(locale(env(&[("LANG", "de_DE.UTF-8")])), Some("de-DE".into()));
        assert_eq!(locale(env(&[("LANG", "de_DE.UTF-8"), ("LC_MESSAGES", "fr_FR")])), Some("fr-FR".into()));
        assert_eq!(locale(env(&[("LC_ALL", "ja_JP.UTF-8"), ("LC_MESSAGES", "fr_FR")])), Some("ja-JP".into()));
        assert_eq!(locale(env(&[("LC_ALL", ""), ("LANG", "sr_RS@latin")])), Some("sr-RS".into()));
        assert_eq!(locale(env(&[("LANG", "C.UTF-8")])), None);
        assert_eq!(locale(env(&[])), None);
    }

    #[test]
    fn language_lists_the_preferences_before_the_locale() {
        let vars = [("LANGUAGE", "pt_BR:pt:en"), ("LANG", "pt_BR.UTF-8")];
        assert_eq!(locales(env(&vars)), ["pt-BR", "pt", "en"]);
        let vars = [("LANGUAGE", "fr::de_DE.UTF-8:fr"), ("LC_MESSAGES", "en_GB.UTF-8")];
        assert_eq!(locales(env(&vars)), ["fr", "de-DE", "en-GB"], "blanks and repeats dropped");
        assert_eq!(locales(env(&[("LANG", "es_ES.UTF-8")])), ["es-ES"]);
        // GNU gettext ignores LANGUAGE in the C locale.
        assert!(locales(env(&[("LANGUAGE", "de"), ("LANG", "C")])).is_empty());
        assert!(locales(env(&[])).is_empty());
    }
}
