//! The spell checker handle apps hold.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::SpellError;
use crate::backend::{Availability, BackendFactory, BackendReport};
use crate::engine::{self, Job};
use crate::hunspell::HunspellBackend;
use crate::languages::normalize;
use crate::personal::PersonalDictionary;

enum Thread {
    /// Not started: the thread starts with the first request.
    Idle(Vec<(String, BackendFactory)>),
    /// Requests go down the channel; the worker (a thread, or a cooperative task in a
    /// browser page) is woken for each.
    Running(Sender<Job>, remedy_ui_core::exec::Worker),
    Failed(String),
}

struct Inner {
    thread: Mutex<Thread>,
    languages: RwLock<Vec<String>>,
    personal: PersonalDictionary,
}

/// A spell checker. Cloning gives another handle to the same checker, which is safe to use
/// from any thread; all of its work runs on a thread of its own, started with the first
/// request, and ends when the last handle is dropped.
///
/// Which engine checks a language is decided at run time, per language, in this order: back
/// ends the app added, Hunspell dictionaries the app supplied, the platform's checker
/// (Windows, macOS), then Hunspell dictionaries found on the system. A language no engine can
/// check is reported by [`SpellChecker::availability`] and makes the checking calls fail with
/// [`SpellError::Unavailable`]; nothing pretends to check it.
///
/// ```
/// use remedy_ui_services_spell::SpellChecker;
/// let checker = SpellChecker::builder()
///     .platform(false)
///     .system_dictionaries(false)
///     .dictionary("en-US", "SET UTF-8\n".into(), "2\nhello\nworld\n".into())
///     .build();
/// let text = "hello wrold";
/// let marks = checker.misspellings_in(text, &["en-US"])?;
/// assert_eq!(marks.iter().filter_map(|r| text.get(r.clone())).collect::<Vec<_>>(), ["wrold"]);
/// # Ok::<(), remedy_ui_services_spell::SpellError>(())
/// ```
#[derive(Clone)]
pub struct SpellChecker {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SpellChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpellChecker({:?})", self.languages())
    }
}

/// Sets up a [`SpellChecker`].
#[derive(Default)]
pub struct SpellCheckerBuilder {
    no_platform: bool,
    no_system: bool,
    directories: Vec<PathBuf>,
    dictionaries: Vec<(String, String, String)>,
    custom: Vec<(String, BackendFactory)>,
    personal: Option<PersonalDictionary>,
    languages: Vec<String>,
}

impl SpellCheckerBuilder {
    /// Whether to use the platform's own checker where there is one (default true).
    pub fn platform(mut self, on: bool) -> Self {
        self.no_platform = !on;
        self
    }

    /// Whether to search the usual system folders for Hunspell dictionaries (default true).
    pub fn system_dictionaries(mut self, on: bool) -> Self {
        self.no_system = !on;
        self
    }

    /// Adds a folder holding Hunspell `.aff`/`.dic` pairs. Dictionaries here are preferred to
    /// the platform's checker.
    pub fn dictionary_directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directories.push(dir.into());
        self
    }

    /// Adds a Hunspell dictionary held as text (UTF-8), for `language`. Preferred to the
    /// platform's checker.
    pub fn dictionary(mut self, language: &str, aff: String, dic: String) -> Self {
        let language = normalize(language).unwrap_or_else(|_| language.to_owned());
        self.dictionaries.push((language, aff, dic));
        self
    }

    /// Adds an engine of the app's own, tried before every other. `make` runs on the checking
    /// thread; `name` is what reports call the engine if `make` fails.
    pub fn backend(mut self, name: &str, make: BackendFactory) -> Self {
        self.custom.push((name.to_owned(), make));
        self
    }

    /// Uses `personal` (shared with, say, a settings page) instead of a new empty dictionary.
    pub fn personal_dictionary(mut self, personal: PersonalDictionary) -> Self {
        self.personal = Some(personal);
        self
    }

    /// The languages to check besides the window's own. Tags that are not BCP 47 are dropped.
    pub fn languages<S: AsRef<str>>(mut self, languages: &[S]) -> Self {
        self.languages = languages.iter().filter_map(|l| normalize(l.as_ref()).ok()).collect();
        self
    }

    /// Makes the checker. Nothing runs until the first request.
    pub fn build(self) -> SpellChecker {
        let mut factories: Vec<(String, BackendFactory)> = self.custom;
        if !self.directories.is_empty() || !self.dictionaries.is_empty() {
            let (dirs, dicts) = (self.directories, self.dictionaries);
            factories.push((
                "hunspell (app dictionaries)".into(),
                Box::new(move || {
                    let mut b =
                        HunspellBackend::with_directories(dirs).labeled("hunspell (app dictionaries)");
                    for (l, aff, dic) in dicts {
                        b.add_dictionary(&l, aff, dic);
                    }
                    Ok(Box::new(b))
                }),
            ));
        }
        #[cfg(any(windows, target_os = "macos"))]
        if !self.no_platform {
            factories.push(("platform".into(), Box::new(crate::os::platform_backend)));
        }
        if !self.no_system {
            factories.push(("hunspell".into(), Box::new(|| Ok(Box::new(HunspellBackend::new())))));
        }
        SpellChecker {
            inner: Arc::new(Inner {
                thread: Mutex::new(Thread::Idle(factories)),
                languages: RwLock::new(self.languages),
                personal: self.personal.unwrap_or_default(),
            }),
        }
    }
}

/// A check running on the checking thread. Drop it to stop caring about the answer; the
/// thread then skips it if it has not started.
pub struct PendingCheck {
    rx: Receiver<Result<Vec<Range<usize>>, SpellError>>,
    cancel: Arc<AtomicBool>,
}

impl PendingCheck {
    /// The answer if it has arrived, without waiting.
    pub fn poll(&mut self) -> Option<Result<Vec<Range<usize>>, SpellError>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(SpellError::Stopped("the check was dropped before it finished".into())))
            }
        }
    }

    /// Waits for the answer.
    pub fn wait(self) -> Result<Vec<Range<usize>>, SpellError> {
        remedy_ui_core::exec::recv(&self.rx)
            .unwrap_or_else(|_| Err(SpellError::Stopped("the check was dropped before it finished".into())))
    }
}

impl Drop for PendingCheck {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn languages_of<S: AsRef<str>>(languages: &[S]) -> Result<Vec<String>, SpellError> {
    let mut out: Vec<String> = Vec::new();
    for l in languages {
        let n = normalize(l.as_ref())?;
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Ok(out)
}

impl Default for SpellChecker {
    fn default() -> Self {
        SpellChecker::new()
    }
}

impl SpellChecker {
    /// A checker with the platform's engine and the system's Hunspell dictionaries.
    pub fn new() -> Self {
        SpellCheckerBuilder::default().build()
    }

    /// Starts setting up a checker.
    pub fn builder() -> SpellCheckerBuilder {
        SpellCheckerBuilder::default()
    }

    /// The user's personal dictionary, shared by every field using this checker.
    pub fn personal(&self) -> PersonalDictionary {
        self.inner.personal.clone()
    }

    /// The languages checked besides each window's own (a multilingual user's other
    /// languages), as BCP 47 tags.
    pub fn languages(&self) -> Vec<String> {
        self.inner.languages.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Sets the languages checked besides each window's own. A word is right if any of them
    /// accepts it. Fails, changing nothing, if a tag is not BCP 47.
    pub fn set_languages<S: AsRef<str>>(&self, languages: &[S]) -> Result<(), SpellError> {
        let l = languages_of(languages)?;
        *self.inner.languages.write().unwrap_or_else(PoisonError::into_inner) = l;
        Ok(())
    }

    /// The languages to check for a window in `locale`: the locale first, then the checker's
    /// other languages, without repeats.
    pub fn languages_for(&self, locale: &str) -> Vec<String> {
        let mut out: Vec<String> = normalize(locale).into_iter().collect();
        for l in self.languages() {
            if !out.contains(&l) {
                out.push(l);
            }
        }
        out
    }

    fn send(&self, job: Job) -> Result<(), SpellError> {
        let mut t = self.inner.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if let Thread::Idle(factories) = &mut *t {
            let factories = std::mem::take(factories);
            let (tx, rx) = mpsc::channel();
            let personal = self.inner.personal.clone();
            // The back ends are made on the worker and stay there (platform checkers are
            // bound to the thread that made them).
            *t = match remedy_ui_core::exec::Worker::spawn(
                "remedy-spell",
                move || engine::Worker::new(factories, personal, rx),
                engine::Worker::turn,
            ) {
                Ok(worker) => Thread::Running(tx, worker),
                Err(e) => Thread::Failed(e.to_string()),
            };
        }
        match &*t {
            Thread::Running(tx, worker) => {
                tx.send(job).map_err(|_| SpellError::Stopped("it has exited".into()))?;
                worker.notify();
                Ok(())
            }
            Thread::Failed(why) => Err(SpellError::Stopped(why.clone())),
            Thread::Idle(_) => Err(SpellError::Stopped("it did not start".into())),
        }
    }

    fn ask<T>(&self, make: impl FnOnce(Sender<T>) -> Job) -> Result<T, SpellError> {
        let (tx, rx) = mpsc::channel();
        self.send(make(tx))?;
        remedy_ui_core::exec::recv(&rx).map_err(|_| SpellError::Stopped("it ended before answering".into()))
    }

    /// Starts checking `text` in `languages` on the checking thread and returns at once.
    /// Poll the [`PendingCheck`] for the byte ranges of the misspelled words.
    pub fn check_async<S: AsRef<str>>(
        &self,
        text: &str,
        languages: &[S],
    ) -> Result<PendingCheck, SpellError> {
        self.check_async_notify(text, languages, None)
    }

    /// Like [`SpellChecker::check_async`], and calls `notify` on the checking thread once the
    /// answer is waiting, so a caller can wake its own thread instead of polling. `notify` is
    /// not called for a check that was dropped.
    pub fn check_async_notify<S: AsRef<str>>(
        &self,
        text: &str,
        languages: &[S],
        notify: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<PendingCheck, SpellError> {
        let languages = languages_of(languages)?;
        let (reply, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.send(Job::Check { text: text.to_owned(), languages, cancel: cancel.clone(), reply, notify })?;
        Ok(PendingCheck { rx, cancel })
    }

    /// The byte ranges of the misspelled words in `text`, checked in `languages`. Waits for
    /// the answer; use [`SpellChecker::check_async`] on a thread that must not wait.
    pub fn misspellings_in<S: AsRef<str>>(
        &self,
        text: &str,
        languages: &[S],
    ) -> Result<Vec<Range<usize>>, SpellError> {
        self.check_async(text, languages)?.wait()
    }

    /// Like [`SpellChecker::misspellings_in`], in the checker's own languages.
    pub fn misspellings(&self, text: &str) -> Result<Vec<Range<usize>>, SpellError> {
        self.misspellings_in(text, &self.languages())
    }

    /// Whether the word is spelled right in any of `languages`. A word in a script none of
    /// them is written in is not judged and counts as right.
    pub fn check_word_in<S: AsRef<str>>(&self, word: &str, languages: &[S]) -> Result<bool, SpellError> {
        let languages = languages_of(languages)?;
        self.ask(|reply| Job::Word { word: word.to_owned(), languages, reply })?
    }

    /// Like [`SpellChecker::check_word_in`], in the checker's own languages.
    pub fn check_word(&self, word: &str) -> Result<bool, SpellError> {
        self.check_word_in(word, &self.languages())
    }

    /// Corrections for `word`, best first, from each of `languages` in turn. Waits, but not
    /// behind a long check: requests like this go first.
    pub fn suggest_in<S: AsRef<str>>(&self, word: &str, languages: &[S]) -> Result<Vec<String>, SpellError> {
        let languages = languages_of(languages)?;
        self.ask(|reply| Job::Suggest { word: word.to_owned(), languages, reply })?
    }

    /// Like [`SpellChecker::suggest_in`], in the checker's own languages.
    pub fn suggest(&self, word: &str) -> Result<Vec<String>, SpellError> {
        self.suggest_in(word, &self.languages())
    }

    /// Whether `language` can be checked, and by which engine.
    pub fn availability(&self, language: &str) -> Result<Availability, SpellError> {
        let language = normalize(language)?;
        self.ask(|reply| Job::Availability { language, reply })
    }

    /// The engines this checker tried to start, and whether each is running.
    pub fn backends(&self) -> Result<Vec<BackendReport>, SpellError> {
        self.ask(|reply| Job::Report { reply })
    }

    /// The languages the engines say they can check, for a settings page.
    pub fn available_languages(&self) -> Result<Vec<String>, SpellError> {
        self.ask(|reply| Job::Languages { reply })
    }

    /// Adds `word` to the personal dictionary. Same as `personal().add(word)`.
    pub fn add_word(&self, word: &str) -> bool {
        self.inner.personal.add(word)
    }

    /// Accepts `word` until the app ends. Same as `personal().ignore(word)`.
    pub fn ignore_word(&self, word: &str) {
        self.inner.personal.ignore(word);
    }
}
