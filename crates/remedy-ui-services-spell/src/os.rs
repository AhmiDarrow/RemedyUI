//! The platform's own spell checker: the only unsafe code in this crate.
//!
//! - Windows 8 and later: `ISpellChecker`, through COM.
//! - macOS: `NSSpellChecker`.
//!
//! Neither is used for the user's own words; those live in the crate's personal dictionary so
//! they behave the same on every back end.

#[cfg(any(windows, target_os = "macos"))]
use crate::backend::SpellBackend;

/// The platform's spell checker, made on the thread that will use it.
#[cfg(windows)]
pub(crate) fn platform_backend() -> Result<Box<dyn SpellBackend>, String> {
    win::WindowsBackend::new().map(|b| Box::new(b) as Box<dyn SpellBackend>)
}

/// The platform's spell checker, made on the thread that will use it.
#[cfg(target_os = "macos")]
pub(crate) fn platform_backend() -> Result<Box<dyn SpellBackend>, String> {
    Ok(Box::new(mac::MacBackend::new()))
}

/// Picks the best of the tags a platform lists for the one asked for: the same tag, then the
/// bare language, then another region of the language. Separators and case do not matter.
#[cfg(any(windows, target_os = "macos"))]
fn closest<'a>(available: &'a [String], language: &str) -> Option<&'a String> {
    let flat = |s: &str| s.to_lowercase().replace('-', "_");
    let want = flat(language);
    let primary = want.split('_').next().unwrap_or(&want).to_owned();
    available
        .iter()
        .find(|a| flat(a) == want)
        .or_else(|| available.iter().find(|a| flat(a) == primary))
        .or_else(|| available.iter().find(|a| flat(a).starts_with(&format!("{primary}_"))))
}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;

    use windows::Win32::Globalization::{
        ISpellChecker, ISpellCheckerFactory, ISpellingError, SpellCheckerFactory,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize, IEnumString,
    };
    use windows::core::{HSTRING, PWSTR};

    use crate::backend::{Prepared, SpellBackend};

    /// Windows' `ISpellChecker`, owned by the checking thread (COM objects stay on the thread
    /// that made them).
    pub(crate) struct WindowsBackend {
        factory: Option<ISpellCheckerFactory>,
        checkers: HashMap<String, ISpellChecker>,
        supported: Option<Vec<String>>,
        com_started: bool,
    }

    /// Reads every string out of a COM string enumerator, freeing each as COM requires.
    fn strings(en: &IEnumString) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let mut slot = [PWSTR::null()];
            let mut fetched = 0u32;
            // SAFETY: `slot` has room for the one string asked for, and `fetched` is a valid
            // place for the count; both outlive the call.
            let hr = unsafe { en.Next(&mut slot, Some(&mut fetched)) };
            let [text] = slot;
            // S_OK (0) means one string was fetched; S_FALSE (1) means the list ended.
            if hr.0 != 0 || fetched != 1 || text.is_null() {
                break;
            }
            // SAFETY: COM returned a NUL-terminated string that we own; it is read once and
            // then freed with CoTaskMemFree, as the interface requires.
            unsafe {
                if let Ok(s) = text.to_string() {
                    out.push(s);
                }
                CoTaskMemFree(Some(text.0.cast_const().cast()));
            }
        }
        out
    }

    impl WindowsBackend {
        pub(crate) fn new() -> Result<Self, String> {
            // SAFETY: starts COM for this thread. A second start on the thread only adds a
            // reference, and a thread already in another COM mode reports it, which is fine:
            // COM is usable either way. Every start that succeeded is ended in `drop`.
            let started = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let mut this = WindowsBackend {
                factory: None,
                checkers: HashMap::new(),
                supported: None,
                com_started: started,
            };
            // SAFETY: the class id and flags are the documented ones for the spell checker
            // factory; the result is a COM interface that owns its reference.
            let factory: ISpellCheckerFactory =
                unsafe { CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) }.map_err(
                    |e| format!("Windows has no spell checker here (it needs Windows 8 or later): {e}"),
                )?;
            this.factory = Some(factory);
            Ok(this)
        }

        fn supported(&mut self) -> Vec<String> {
            if self.supported.is_none() {
                let list = self
                    .factory
                    .as_ref()
                    // SAFETY: a plain call on a live interface; the returned enumerator is
                    // read to the end by `strings`.
                    .and_then(|f| unsafe { f.SupportedLanguages() }.ok())
                    .map(|e| strings(&e))
                    .unwrap_or_default();
                self.supported = Some(list);
            }
            self.supported.clone().unwrap_or_default()
        }
    }

    impl SpellBackend for WindowsBackend {
        fn name(&self) -> &str {
            "windows"
        }

        fn prepare(&mut self, language: &str) -> Prepared {
            let list = self.supported();
            let Some(tag) = super::closest(&list, language).cloned() else { return Prepared::Missing };
            let Some(factory) = self.factory.as_ref() else { return Prepared::Missing };
            // SAFETY: the tag is an HSTRING that outlives the call.
            match unsafe { factory.CreateSpellChecker(&HSTRING::from(tag.as_str())) } {
                Ok(c) => {
                    self.checkers.insert(language.to_owned(), c);
                    Prepared::Ready { dictionary: tag }
                }
                Err(e) => Prepared::Broken(format!("Windows could not open the {tag} spell checker: {e}")),
            }
        }

        fn languages(&mut self) -> Vec<String> {
            self.supported().iter().filter_map(|t| crate::languages::normalize(t).ok()).collect()
        }

        fn check_word(&mut self, language: &str, word: &str) -> Result<bool, String> {
            let c = self.checkers.get(language).ok_or_else(|| format!("{language} was not prepared"))?;
            let text = HSTRING::from(word);
            // SAFETY: `text` outlives the call; the enumerator and error are owned interfaces.
            unsafe {
                let errors = c.Check(&text).map_err(|e| e.to_string())?;
                let mut first: Option<ISpellingError> = None;
                let hr = errors.Next(&mut first);
                // S_OK with an error means the word is misspelled; S_FALSE means none.
                Ok(!(hr.0 == 0 && first.is_some()))
            }
        }

        fn suggest(&mut self, language: &str, word: &str) -> Result<Vec<String>, String> {
            let c = self.checkers.get(language).ok_or_else(|| format!("{language} was not prepared"))?;
            let text = HSTRING::from(word);
            // SAFETY: `text` outlives the call; the enumerator is read to its end.
            let list = unsafe { c.Suggest(&text) }.map_err(|e| e.to_string())?;
            Ok(strings(&list))
        }
    }

    impl Drop for WindowsBackend {
        fn drop(&mut self) {
            // The interfaces go before COM does.
            self.checkers.clear();
            self.factory = None;
            if self.com_started {
                // SAFETY: balances the successful CoInitializeEx on this thread in `new`.
                unsafe { CoUninitialize() };
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::HashMap;

    use objc2::rc::Retained;
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSInteger, NSRange, NSString};

    use crate::backend::{Prepared, SpellBackend};

    /// `NSNotFound`: what a range's location is when nothing was found.
    const NOT_FOUND: usize = isize::MAX as usize;

    /// AppKit's `NSSpellChecker`, driven from the checking thread. AppKit's headers mark the
    /// classes that must stay on the main thread (`NS_SWIFT_UI_ACTOR`), and objc2 turns that
    /// into a `MainThreadMarker` argument; `NSSpellChecker` carries no such mark, so its shared
    /// checker may be used here, off the UI thread, without stalling it.
    pub(crate) struct MacBackend {
        checker: Retained<NSSpellChecker>,
        tag: NSInteger,
        available: Vec<String>,
        chosen: HashMap<String, Retained<NSString>>,
    }

    impl MacBackend {
        pub(crate) fn new() -> Self {
            let checker = NSSpellChecker::sharedSpellChecker();
            let tag = NSSpellChecker::uniqueSpellDocumentTag();
            let available = checker.availableLanguages().iter().map(|l| l.to_string()).collect();
            MacBackend { checker, tag, available, chosen: HashMap::new() }
        }
    }

    impl SpellBackend for MacBackend {
        fn name(&self) -> &str {
            "macos"
        }

        fn prepare(&mut self, language: &str) -> Prepared {
            match super::closest(&self.available, language) {
                Some(name) => {
                    self.chosen.insert(language.to_owned(), NSString::from_str(name));
                    Prepared::Ready { dictionary: name.clone() }
                }
                None => Prepared::Missing,
            }
        }

        fn languages(&mut self) -> Vec<String> {
            self.available.iter().filter_map(|t| crate::languages::normalize(t).ok()).collect()
        }

        fn check_word(&mut self, language: &str, word: &str) -> Result<bool, String> {
            let lang = self.chosen.get(language).ok_or_else(|| format!("{language} was not prepared"))?;
            let text = NSString::from_str(word);
            // SAFETY: the word count out-parameter is null, which the method documents as
            // allowed; the strings outlive the call.
            let range = unsafe {
                self.checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                    &text,
                    0,
                    Some(lang),
                    false,
                    self.tag,
                    std::ptr::null_mut(),
                )
            };
            Ok(range.location == NOT_FOUND)
        }

        fn suggest(&mut self, language: &str, word: &str) -> Result<Vec<String>, String> {
            let lang = self.chosen.get(language).ok_or_else(|| format!("{language} was not prepared"))?;
            let text = NSString::from_str(word);
            let range = NSRange::new(0, text.length());
            let guesses = self.checker.guessesForWordRange_inString_language_inSpellDocumentWithTag(
                range,
                &text,
                Some(lang),
                self.tag,
            );
            Ok(guesses.map(|a| a.iter().map(|s| s.to_string()).collect()).unwrap_or_default())
        }
    }

    impl Drop for MacBackend {
        fn drop(&mut self) {
            self.checker.closeSpellDocumentWithTag(self.tag);
        }
    }
}
