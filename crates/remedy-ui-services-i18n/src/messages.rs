//! Fluent messages: the app's strings per locale, chosen for the window's locale, with plurals,
//! selectors, locale-formatted numbers, user overrides and live reload.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use fluent_bundle::types::FluentNumber;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use intl_memoizer::{IntlLangMemoizer, Memoizable};
use remedy_ui_reactive::Signal;
use remedy_ui_view::Ui;
use unic_langid::LanguageIdentifier;

use crate::decimal::Decimal;
use crate::locale::NumberSymbols;

type Bundle = fluent_bundle::bundle::FluentBundle<Rc<FluentResource>, IntlLangMemoizer>;

/// A value passed into a message (`{ $count }`).
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// Text.
    Text(String),
    /// A number (plurals select on it; it is written the locale's way).
    Number(f64),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Text(s.to_owned())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Text(s)
    }
}

macro_rules! numeric_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(v: $t) -> Self {
                Arg::Number(v as f64)
            }
        }
    )*};
}
numeric_arg!(i32, i64, u32, u64, usize, f32, f64);

/// Something wrong with the messages, for the inspector and logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A message file did not parse; the rest of it was used.
    Syntax {
        /// The file or source.
        source: String,
        /// What the parser said.
        detail: String,
    },
    /// A message the app asked for exists in no locale.
    Missing {
        /// Its id.
        id: String,
    },
    /// A message referred to something it could not resolve (a missing variable or term).
    Format {
        /// Its id.
        id: String,
        /// What went wrong.
        detail: String,
    },
    /// A message file could not be read.
    Io {
        /// The file.
        path: String,
        /// Why.
        detail: String,
    },
}

/// Where messages come from, in priority order (later sources override earlier ones).
#[derive(Clone, Debug)]
enum Source {
    /// Compiled into the app.
    Embedded { locale: String, text: &'static str },
    /// A folder of `<locale>.ftl` files, or `<locale>/*.ftl`, read at run time.
    Folder(PathBuf),
}

/// The app's messages. Clone it freely (it is a shared handle).
#[derive(Clone)]
pub struct Messages(Rc<Inner>);

struct Inner {
    default: String,
    sources: RefCell<Vec<Source>>,
    /// Parsed resources per locale, in source order.
    resources: RefCell<HashMap<String, Vec<Rc<FluentResource>>>>,
    /// The folder each resource was read from (`None`: compiled in), parallel to `resources`.
    origins: RefCell<HashMap<String, Vec<Option<PathBuf>>>>,
    /// One bundle per requested locale tag: its fallback chain, built on first use.
    bundles: RefCell<HashMap<String, Rc<Vec<Bundle>>>>,
    problems: RefCell<Vec<Problem>>,
    /// Bumped whenever messages change: everything that formatted one formats again.
    revision: Signal<u64>,
    /// The folder watcher, if running: its stop flag and the effect that reloads.
    watcher: RefCell<Option<(Arc<AtomicBool>, remedy_ui_reactive::Effect)>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some((stop, effect)) = self.watcher.borrow_mut().take() {
            stop.store(true, Ordering::Relaxed);
            effect.dispose();
        }
    }
}

impl std::fmt::Debug for Messages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Messages(default {})", self.0.default)
    }
}

impl Messages {
    /// Messages whose last fallback is `default_locale` (the language the app is written in).
    pub fn new(default_locale: &str) -> Messages {
        Messages(Rc::new(Inner {
            default: default_locale.to_owned(),
            sources: RefCell::new(Vec::new()),
            resources: RefCell::new(HashMap::new()),
            origins: RefCell::new(HashMap::new()),
            bundles: RefCell::new(HashMap::new()),
            problems: RefCell::new(Vec::new()),
            revision: Signal::new(0),
            watcher: RefCell::new(None),
        }))
    }

    /// Adds a message file compiled into the app (`include_str!("../i18n/de.ftl")`).
    pub fn embedded(self, locale: &str, ftl: &'static str) -> Messages {
        self.0.sources.borrow_mut().push(Source::Embedded { locale: locale.to_owned(), text: ftl });
        self.reload();
        self
    }

    /// Adds a folder of message files read at run time: `<locale>.ftl`, or every `.ftl` in a
    /// `<locale>` subfolder. Later folders override earlier sources message by message, so a
    /// user-override folder added last replaces any string the user rewrote.
    pub fn folder(self, path: impl Into<PathBuf>) -> Messages {
        self.0.sources.borrow_mut().push(Source::Folder(path.into()));
        self.reload();
        self
    }

    /// Re-reads every source (after files changed), and re-formats every message shown.
    pub fn reload(&self) {
        let mut resources: HashMap<String, Vec<Rc<FluentResource>>> = HashMap::new();
        let mut origins: HashMap<String, Vec<Option<PathBuf>>> = HashMap::new();
        let mut problems = Vec::new();
        let mut add =
            |locale: &str, text: String, origin: &str, folder: Option<&Path>, problems: &mut Vec<Problem>| {
                let res = match FluentResource::try_new(text) {
                    Ok(r) => r,
                    Err((r, errs)) => {
                        for e in errs {
                            problems.push(Problem::Syntax {
                                source: origin.to_owned(),
                                detail: format!("{e:?}"),
                            });
                        }
                        r
                    }
                };
                resources.entry(canonical(locale)).or_default().push(Rc::new(res));
                origins.entry(canonical(locale)).or_default().push(folder.map(Path::to_path_buf));
            };
        for src in self.0.sources.borrow().iter() {
            match src {
                Source::Embedded { locale, text } => {
                    add(locale, (*text).to_owned(), &format!("embedded {locale}"), None, &mut problems)
                }
                Source::Folder(dir) => {
                    for (locale, path) in ftl_files(dir, &mut problems) {
                        match std::fs::read_to_string(&path) {
                            Ok(t) => add(&locale, t, &path.display().to_string(), Some(dir), &mut problems),
                            Err(e) => problems.push(Problem::Io {
                                path: path.display().to_string(),
                                detail: e.to_string(),
                            }),
                        }
                    }
                }
            }
        }
        *self.0.resources.borrow_mut() = resources;
        *self.0.origins.borrow_mut() = origins;
        self.0.bundles.borrow_mut().clear();
        self.0
            .problems
            .borrow_mut()
            .retain(|p| matches!(p, Problem::Missing { .. } | Problem::Format { .. }));
        self.0.problems.borrow_mut().extend(problems);
        self.0.revision.update(|r| *r += 1);
    }

    /// The locales with messages, as BCP 47 tags.
    pub fn locales(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.resources.borrow().keys().cloned().collect();
        v.sort();
        v
    }

    /// Problems found so far: files that did not parse, messages missing everywhere, and
    /// messages that could not be formatted.
    pub fn problems(&self) -> Vec<Problem> {
        self.0.problems.borrow().clone()
    }

    /// The message `id` in the window's locale (tracked: it follows locale changes and
    /// reloads).
    pub fn text(&self, ui: &Ui, id: &str) -> String {
        self.format(ui, id, &[])
    }

    /// The message `id` with arguments, in the window's locale (tracked).
    pub fn format(&self, ui: &Ui, id: &str, args: &[(&str, Arg)]) -> String {
        let locale = ui.locale();
        self.0.revision.get();
        self.format_in(&locale, id, None, args)
    }

    /// An attribute of message `id` (`.tooltip`, `.placeholder`), in the window's locale.
    pub fn attribute(&self, ui: &Ui, id: &str, attribute: &str, args: &[(&str, Arg)]) -> String {
        let locale = ui.locale();
        self.0.revision.get();
        self.format_in(&locale, id, Some(attribute), args)
    }

    /// The message in a given locale (not tracked): for logs, files and other windows.
    /// A message missing in every locale comes back as its id in angle brackets, so it is
    /// visible on screen and listed in [`Messages::problems`].
    pub fn format_in(&self, locale: &str, id: &str, attribute: Option<&str>, args: &[(&str, Arg)]) -> String {
        let chain = self.chain(locale);
        let mut fargs = FluentArgs::new();
        for (k, v) in args {
            match v {
                Arg::Text(s) => fargs.set(*k, FluentValue::from(s.clone())),
                Arg::Number(n) => fargs.set(*k, FluentValue::from(*n)),
            }
        }
        for bundle in chain.iter() {
            let Some(msg) = bundle.get_message(id) else { continue };
            let pattern = match attribute {
                Some(a) => msg.get_attribute(a).map(|at| at.value()),
                None => msg.value(),
            };
            let Some(pattern) = pattern else { continue };
            let mut errors = Vec::new();
            let out = bundle.format_pattern(pattern, Some(&fargs), &mut errors).to_string();
            if !errors.is_empty() {
                self.note(Problem::Format { id: id.to_owned(), detail: format!("{errors:?}") });
            }
            return out;
        }
        self.note(Problem::Missing { id: attribute.map_or_else(|| id.to_owned(), |a| format!("{id}.{a}")) });
        format!("\u{27E8}{id}\u{27E9}")
    }

    fn note(&self, p: Problem) {
        let mut ps = self.0.problems.borrow_mut();
        if !ps.contains(&p) {
            ps.push(p);
        }
    }

    /// The bundles to try for a requested locale: the best matches among the available ones,
    /// then the default.
    /// Every message as `locale` shows it, with its value and attributes as Fluent text: the
    /// best matching locales first, then the default, and within a locale later sources over
    /// earlier ones. Sources read from the folder `without` are left out (what the app's own
    /// messages say, beneath a person's overrides). Tracked.
    pub fn entries(&self, locale: &str, without: Option<&Path>) -> Vec<crate::ftl::MessageEntry> {
        self.0.revision.get();
        let resources = self.0.resources.borrow();
        let origins = self.0.origins.borrow();
        let mut ordered: Vec<Rc<FluentResource>> = Vec::new();
        for lang in self.negotiated(locale) {
            let (Some(list), Some(from)) = (resources.get(&lang), origins.get(&lang)) else { continue };
            for (r, o) in list.iter().zip(from).rev() {
                if without.is_none() || o.as_deref() != without {
                    ordered.push(r.clone());
                }
            }
        }
        crate::ftl::merged(ordered.iter().map(|r| &**r))
    }

    /// The available locales that serve `requested`, best first, then the default.
    fn negotiated(&self, requested: &str) -> Vec<String> {
        let resources = self.0.resources.borrow();
        let available: Vec<LanguageIdentifier> = resources.keys().filter_map(|k| k.parse().ok()).collect();
        let default: Option<LanguageIdentifier> = self.0.default.parse().ok();
        let want: Vec<LanguageIdentifier> = requested.parse().ok().into_iter().collect();
        fluent_langneg::negotiate_languages(
            &want,
            &available,
            default.as_ref(),
            fluent_langneg::NegotiationStrategy::Filtering,
        )
        .into_iter()
        .map(|l| l.to_string())
        .collect()
    }

    fn chain(&self, requested: &str) -> Rc<Vec<Bundle>> {
        if let Some(c) = self.0.bundles.borrow().get(requested) {
            return c.clone();
        }
        let resources = self.0.resources.borrow();
        let available: Vec<LanguageIdentifier> = resources.keys().filter_map(|k| k.parse().ok()).collect();
        let default: Option<LanguageIdentifier> = self.0.default.parse().ok();
        let want: Vec<LanguageIdentifier> = requested.parse().ok().into_iter().collect();
        let chosen = fluent_langneg::negotiate_languages(
            &want,
            &available,
            default.as_ref(),
            fluent_langneg::NegotiationStrategy::Filtering,
        );
        let mut bundles = Vec::new();
        for lang in chosen {
            let key = lang.to_string();
            let Some(list) = resources.get(&key) else { continue };
            let mut b = Bundle::new(vec![lang.clone()]);
            // Isolation marks keep a right-to-left name inside a left-to-right sentence (and
            // the reverse) from scrambling it.
            b.set_use_isolating(true);
            b.set_formatter(Some(format_number));
            for r in list {
                b.add_resource_overriding(r.clone());
            }
            bundles.push(b);
        }
        let c = Rc::new(bundles);
        drop(resources);
        self.0.bundles.borrow_mut().insert(requested.to_owned(), c.clone());
        c
    }

    /// Re-reads the message folders whenever a file in them changes (checked twice a second
    /// off the UI thread). For development, and for user overrides edited while the app runs.
    pub fn watch(&self) {
        let folders: Vec<PathBuf> = self
            .0
            .sources
            .borrow()
            .iter()
            .filter_map(|s| match s {
                Source::Folder(p) => Some(p.clone()),
                Source::Embedded { .. } => None,
            })
            .collect();
        if folders.is_empty() || self.0.watcher.borrow().is_some() {
            return;
        }
        let Ok(waker) = remedy_ui_reactive::waker() else { return };
        let changed = Signal::new(0u64);
        let me = Rc::downgrade(&self.0);
        let first = std::cell::Cell::new(true);
        // The effect reloads on the UI thread when the watcher reports a change.
        let effect = remedy_ui_reactive::Effect::new(move || {
            changed.get();
            if !first.replace(false)
                && let Some(inner) = me.upgrade()
            {
                remedy_ui_reactive::untrack(|| Messages(inner).reload());
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        *self.0.watcher.borrow_mut() = Some((stop.clone(), effect));
        std::thread::Builder::new()
            .name("remedy-i18n-watch".into())
            .spawn(move || {
                let stamp = |folders: &[PathBuf]| {
                    let mut v: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
                    for f in folders {
                        for (_, p) in ftl_files(f, &mut Vec::new()) {
                            if let Ok(m) = std::fs::metadata(&p) {
                                v.push((p, m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
                            }
                        }
                    }
                    v.sort();
                    v
                };
                let mut last = stamp(&folders);
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                    let now = stamp(&folders);
                    if now != last {
                        last = now;
                        if waker.post(move || changed.update(|c| *c += 1)).is_err() {
                            // The UI thread is gone.
                            return;
                        }
                    }
                }
            })
            .ok();
    }
}

/// `de_DE` or `de-de` as `de-DE`.
fn canonical(tag: &str) -> String {
    tag.replace('_', "-").parse::<LanguageIdentifier>().map_or_else(|_| tag.to_owned(), |l| l.to_string())
}

/// `<locale>.ftl` files in `dir`, and `.ftl` files in `<locale>` subfolders.
fn ftl_files(dir: &Path, problems: &mut Vec<Problem>) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                problems.push(Problem::Io { path: dir.display().to_string(), detail: e.to_string() });
            }
            return out;
        }
    };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            if stem.parse::<LanguageIdentifier>().is_err() {
                continue;
            }
            let mut inner: Vec<PathBuf> =
                std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).collect();
            inner.sort();
            out.extend(
                inner
                    .into_iter()
                    .filter(|f| f.extension().is_some_and(|x| x == "ftl"))
                    .map(|f| (stem.clone(), f)),
            );
        } else if p.extension().is_some_and(|x| x == "ftl") && stem.parse::<LanguageIdentifier>().is_ok() {
            out.push((stem, p));
        }
    }
    out
}

/// Numbers in messages, written the locale's way (digits, separators, grouping).
struct LocaleNumbers(NumberSymbols);

impl Memoizable for LocaleNumbers {
    type Args = ();
    type Error = ();
    fn construct(lang: LanguageIdentifier, _: ()) -> Result<Self, ()> {
        Ok(LocaleNumbers(NumberSymbols::for_locale(&lang.to_string())))
    }
}

fn format_number(v: &FluentValue<'_>, memo: &IntlLangMemoizer) -> Option<String> {
    let FluentValue::Number(FluentNumber { value, options }) = v else { return None };
    let max = options.maximum_fraction_digits.map_or(crate::decimal::MAX_SCALE, |m| m.min(28) as u8);
    let d = Decimal::from_f64(*value, max)?;
    let min = options.minimum_fraction_digits.unwrap_or(0).min(28) as u8;
    let places = d.scale().clamp(min, max.max(min));
    memo.with_try_get::<LocaleNumbers, _, _>((), |n| {
        let mut sym = n.0.clone();
        if !options.use_grouping {
            sym.group = None;
        }
        sym.format(d, places)
    })
    .ok()
}
