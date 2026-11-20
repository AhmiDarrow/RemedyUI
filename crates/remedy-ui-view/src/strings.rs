//! The engine's own words, in the window's language: the labels, menus, messages and screen
//! reader names built-in widgets show.
//!
//! Each crate keeps its words in Fluent files, one per locale (`i18n/en.ftl`, `i18n/ar.ftl`,
//! …), compiled in as a [`Strings`] catalog. A widget asks the catalog for a message in the
//! window's locale ([`Strings::get`], [`Strings::format`]); the best match among the
//! catalog's locales answers (`pt-PT` takes `pt-BR`, `zh-CN` takes `zh-Hans`), then English.
//! Plurals follow each language's CLDR rules (Arabic has six forms, Russian four), numbers
//! are written the locale's way ([`set_number_format`]), and placeholders are wrapped in
//! Unicode isolation marks so a left-to-right file name keeps its order inside an Arabic
//! sentence and the reverse.
//!
//! Apps add a language the engine does not ship, or reword a built-in message, with
//! [`add_messages`]. [`check`] is what each crate's tests run: every locale has every
//! message, uses the same placeholders, and gives every plural form its language needs.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use fluent_bundle::types::FluentNumber;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use intl_memoizer::{IntlLangMemoizer, Memoizable};
use unic_langid::LanguageIdentifier;

use crate::Ui;

type Bundle = fluent_bundle::bundle::FluentBundle<Rc<FluentResource>, IntlLangMemoizer>;
/// A catalog (by address) and a locale.
type Key = (usize, String);
/// Messages apps added: catalog name → locale → sources.
type Added = HashMap<String, BTreeMap<String, Vec<Rc<FluentResource>>>>;

/// A crate's catalog from its `i18n/<locale>.ftl` files, one per [`LOCALES`] entry:
/// `pub static STRINGS: Strings = remedy_ui_view::engine_strings!("widgets-core");`
#[macro_export]
macro_rules! engine_strings {
    ($name:literal) => {
        $crate::strings::Strings::new(
            $name,
            &[
                ("en", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/en.ftl"))),
                ("es", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/es.ftl"))),
                ("zh-Hans", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/zh-Hans.ftl"))),
                ("hi", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/hi.ftl"))),
                ("ar", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/ar.ftl"))),
                ("fr", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/fr.ftl"))),
                ("pt-BR", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/pt-BR.ftl"))),
                ("ru", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/ru.ftl"))),
                ("ja", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/ja.ftl"))),
                ("de", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/de.ftl"))),
                ("bn", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n/bn.ftl"))),
            ],
        )
    };
}

/// The view's own words (the Undo menu item's name), from `i18n/*.ftl` in this crate. Apps
/// reword them, or add a language, with [`add_messages`] under the name `view`.
pub static STRINGS: Strings = crate::engine_strings!("view");

/// The locales every engine catalog ships, English first.
pub const LOCALES: &[&str] = &["en", "es", "zh-Hans", "hi", "ar", "fr", "pt-BR", "ru", "ja", "de", "bn"];

/// One crate's words: its Fluent files, by locale, compiled in. The first locale is the
/// source every other is checked against, and the last fallback.
///
/// ```
/// use remedy_ui_view::strings::Strings;
/// static STRINGS: Strings = Strings::new(
///     "example",
///     &[
///         ("en", "files = { $count ->\n    [one] One file\n   *[other] { $count } files\n}\n"),
///         ("de", "files = { $count ->\n    [one] Eine Datei\n   *[other] { $count } Dateien\n}\n"),
///     ],
/// );
/// assert_eq!(STRINGS.format_in("de-AT", "files", &[("count", 3.into())]), "\u{2068}3\u{2069} Dateien");
/// assert_eq!(STRINGS.format_in("de", "files", &[("count", 1.into())]), "Eine Datei");
/// // Each crate's tests check its catalog; this one ships two of the eleven locales.
/// assert!(remedy_ui_view::strings::check(&STRINGS).iter().any(|p| p.contains("no ar file")));
/// ```
#[derive(Debug)]
pub struct Strings {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

/// A value for a placeholder (`{ $count }`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StrArg {
    /// Text, shown as it is (a name, a file, a label).
    Text(String),
    /// A number: plurals select on it, and it is written the locale's way.
    Number(f64),
}

impl From<&str> for StrArg {
    fn from(s: &str) -> Self {
        StrArg::Text(s.to_owned())
    }
}

impl From<String> for StrArg {
    fn from(s: String) -> Self {
        StrArg::Text(s)
    }
}

impl From<&String> for StrArg {
    fn from(s: &String) -> Self {
        StrArg::Text(s.clone())
    }
}

macro_rules! number_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for StrArg {
            fn from(v: $t) -> Self {
                StrArg::Number(v as f64)
            }
        }
    )*};
}
number_arg!(i32, i64, u8, u16, u32, u64, usize, f32, f64);

/// Writes a number for a locale (`None`: Fluent's own, ASCII digits).
pub type NumberFormat = Rc<dyn Fn(&str, f64, Option<usize>) -> Option<String>>;

thread_local! {
    /// Parsed resources: (catalog, locale) → resource, parsed once.
    static RESOURCES: RefCell<HashMap<Key, Rc<FluentResource>>> = RefCell::new(HashMap::new());
    /// Bundles by (catalog, requested locale): the fallback chain.
    static CHAINS: RefCell<HashMap<Key, Rc<Vec<Bundle>>>> = RefCell::new(HashMap::new());
    /// Messages apps added ([`add_messages`]): catalog name → locale → Fluent sources.
    static ADDED: RefCell<Added> = RefCell::new(HashMap::new());
    static NUMBERS: RefCell<Option<NumberFormat>> = const { RefCell::new(None) };
}

/// Adds or rewords a catalog's messages for `locale` (a language the engine does not ship,
/// or an app's own wording): Fluent source whose messages take precedence over the
/// catalog's for that locale. `catalog` is the crate's catalog name (`widgets-core`,
/// `widgets-forms`, … as each crate's `STRINGS` says). For this thread (the UI thread), from
/// then on. Returns what did not parse, if anything (the rest is used).
pub fn add_messages(catalog: &str, locale: &str, ftl: &str) -> Vec<String> {
    let (res, errors) = match FluentResource::try_new(ftl.to_owned()) {
        Ok(r) => (r, Vec::new()),
        Err((r, e)) => (r, e.into_iter().map(|e| format!("{e:?}")).collect()),
    };
    ADDED.with(|a| {
        a.borrow_mut().entry(catalog.to_owned()).or_default().entry(canonical(locale)).or_default().push(Rc::new(res))
    });
    CHAINS.with(|c| c.borrow_mut().clear());
    errors
}

/// Writes numbers in messages the locale's way (digits, separators): `format(locale, value,
/// fraction digits)`. The runner sets it from the localization service; without one,
/// numbers are written with ASCII digits.
pub fn set_number_format(format: Option<NumberFormat>) {
    NUMBERS.with(|n| *n.borrow_mut() = format);
    CHAINS.with(|c| c.borrow_mut().clear());
}

/// `de_DE` or `de-de` as `de-DE`.
fn canonical(tag: &str) -> String {
    tag.replace('_', "-").parse::<LanguageIdentifier>().map_or_else(|_| tag.to_owned(), |l| l.to_string())
}

/// Language identifier with likely script and region filled in (`zh-CN` → `zh-Hans-CN`).
fn maximized(tag: &str) -> Option<LanguageIdentifier> {
    let mut l: LanguageIdentifier = canonical(tag).parse().ok()?;
    l.maximize();
    Some(l)
}

/// The catalog locales that serve `requested`, best first: same language, script and region;
/// then language and script (`pt-PT` → `pt-BR`, `zh-SG` → `zh-Hans`); then the language
/// alone (`zh-TW` → `zh-Hans`, readable where no Traditional Chinese is shipped).
fn negotiate(available: &[String], requested: &str) -> Vec<String> {
    let Some(want) = maximized(requested) else { return Vec::new() };
    let mut scored: Vec<(u8, usize, &String)> = Vec::new();
    for (i, a) in available.iter().enumerate() {
        let Some(have) = maximized(a) else { continue };
        if have.language != want.language {
            continue;
        }
        let score = if have.script == want.script && have.region == want.region {
            0
        } else if have.script == want.script {
            1
        } else {
            2
        };
        scored.push((score, i, a));
    }
    scored.sort();
    scored.into_iter().map(|(_, _, a)| a.clone()).collect()
}

/// The locale's tag, for the number formatter.
struct Tag(String);

impl Memoizable for Tag {
    type Args = ();
    type Error = ();
    fn construct(lang: LanguageIdentifier, _: ()) -> Result<Self, ()> {
        Ok(Tag(lang.to_string()))
    }
}

fn format_number(v: &FluentValue<'_>, memo: &IntlLangMemoizer) -> Option<String> {
    let FluentValue::Number(FluentNumber { value, options }) = v else { return None };
    let f = NUMBERS.with(|n| n.borrow().clone())?;
    memo.with_try_get::<Tag, _, _>((), |t| f(&t.0, *value, options.maximum_fraction_digits)).ok().flatten()
}

impl Strings {
    /// A catalog named `name` (the crate's short name, which [`add_messages`] refers to) of
    /// `files`: `(locale, Fluent source)`, English first.
    pub const fn new(name: &'static str, files: &'static [(&'static str, &'static str)]) -> Strings {
        Strings { name, files }
    }

    /// The catalog's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The locales the catalog ships.
    pub fn locales(&self) -> Vec<&'static str> {
        self.files.iter().map(|(l, _)| *l).collect()
    }

    fn key(&self) -> usize {
        self as *const Strings as usize
    }

    fn resource(&self, locale: &str, text: &'static str) -> Rc<FluentResource> {
        let key = (self.key(), locale.to_owned());
        if let Some(r) = RESOURCES.with(|r| r.borrow().get(&key).cloned()) {
            return r;
        }
        // A file that does not parse fully still gives what did; `check` reports the rest.
        let res = Rc::new(FluentResource::try_new(text.to_owned()).unwrap_or_else(|(r, _)| r));
        RESOURCES.with(|r| r.borrow_mut().insert(key, res.clone()));
        res
    }

    fn chain(&self, requested: &str) -> Rc<Vec<Bundle>> {
        let key = (self.key(), requested.to_owned());
        if let Some(c) = CHAINS.with(|c| c.borrow().get(&key).cloned()) {
            return c;
        }
        let added = ADDED.with(|a| a.borrow().get(self.name).cloned()).unwrap_or_default();
        let mut available: Vec<String> = self.files.iter().map(|(l, _)| canonical(l)).collect();
        for l in added.keys() {
            if !available.contains(l) {
                available.push(l.clone());
            }
        }
        let mut order = negotiate(&available, requested);
        if let Some((source, _)) = self.files.first() {
            let source = canonical(source);
            if !order.contains(&source) {
                order.push(source);
            }
        }
        let mut bundles = Vec::new();
        for locale in order {
            let Ok(lang) = locale.parse::<LanguageIdentifier>() else { continue };
            let mut b = Bundle::new(vec![lang]);
            b.set_use_isolating(true);
            b.set_formatter(Some(format_number));
            if let Some((l, text)) = self.files.iter().find(|(l, _)| canonical(l) == locale) {
                b.add_resource_overriding(self.resource(l, text));
            }
            for r in added.get(&locale).into_iter().flatten() {
                b.add_resource_overriding(r.clone());
            }
            bundles.push(b);
        }
        let c = Rc::new(bundles);
        CHAINS.with(|ch| ch.borrow_mut().insert(key, c.clone()));
        c
    }

    /// Message `id` in the window's locale (tracked: it follows the locale).
    pub fn get(&self, ui: &Ui, id: &str) -> String {
        self.format_in(&ui.locale(), id, &[])
    }

    /// Message `id` with placeholder values, in the window's locale (tracked).
    pub fn format(&self, ui: &Ui, id: &str, args: &[(&str, StrArg)]) -> String {
        self.format_in(&ui.locale(), id, args)
    }

    /// Message `id` in `locale` (not tracked). A message no locale has comes back as its id
    /// in angle brackets, so it shows on screen; [`check`] finds them first.
    pub fn format_in(&self, locale: &str, id: &str, args: &[(&str, StrArg)]) -> String {
        let mut fargs = FluentArgs::new();
        for (k, v) in args {
            match v {
                StrArg::Text(s) => fargs.set(*k, FluentValue::from(s.clone())),
                StrArg::Number(n) => fargs.set(*k, FluentValue::from(*n)),
            }
        }
        let (id, attribute) = match id.split_once('.') {
            Some((m, a)) => (m, Some(a)),
            None => (id, None),
        };
        for bundle in self.chain(locale).iter() {
            let Some(msg) = bundle.get_message(id) else { continue };
            let pattern = match attribute {
                Some(a) => msg.get_attribute(a).map(|at| at.value()),
                None => msg.value(),
            };
            let Some(pattern) = pattern else { continue };
            let mut errors = Vec::new();
            return bundle.format_pattern(pattern, Some(&fargs), &mut errors).into_owned();
        }
        format!("\u{27E8}{id}\u{27E9}")
    }
}

/// The plural categories a language uses for whole numbers (`one`, `few`, `many`, …).
fn plural_categories(locale: &str) -> BTreeSet<&'static str> {
    use intl_pluralrules::{PluralCategory, PluralRuleType, PluralRules};
    // The rules are per language (`ru`, `pt`), whatever the region.
    let Some(lang) = maximized(locale).and_then(|l| l.language.as_str().parse::<LanguageIdentifier>().ok()) else {
        return BTreeSet::new();
    };
    let Ok(rules) = PluralRules::create(lang, PluralRuleType::CARDINAL) else { return BTreeSet::new() };
    let mut out = BTreeSet::new();
    for n in (0..=1000u32).chain([1_000_000, 2_000_000]) {
        if let Ok(c) = rules.select(n) {
            out.insert(match c {
                PluralCategory::ZERO => "zero",
                PluralCategory::ONE => "one",
                PluralCategory::TWO => "two",
                PluralCategory::FEW => "few",
                PluralCategory::MANY => "many",
                PluralCategory::OTHER => "other",
            });
        }
    }
    out
}

/// What one message uses: its placeholders, and the variant keys of each selector on a
/// variable.
#[derive(Default, Debug)]
struct Shape {
    vars: BTreeSet<String>,
    selects: Vec<(String, BTreeSet<String>)>,
}

fn walk_pattern(p: &fluent_syntax::ast::Pattern<&str>, shape: &mut Shape) {
    use fluent_syntax::ast::PatternElement;
    for el in &p.elements {
        if let PatternElement::Placeable { expression } = el {
            walk_expression(expression, shape);
        }
    }
}

fn walk_inline(e: &fluent_syntax::ast::InlineExpression<&str>, shape: &mut Shape) {
    use fluent_syntax::ast::InlineExpression;
    match e {
        InlineExpression::VariableReference { id } => {
            shape.vars.insert(id.name.to_owned());
        }
        InlineExpression::FunctionReference { arguments, .. } => {
            for a in &arguments.positional {
                walk_inline(a, shape);
            }
            for a in &arguments.named {
                walk_inline(&a.value, shape);
            }
        }
        InlineExpression::Placeable { expression } => walk_expression(expression, shape),
        _ => {}
    }
}

fn walk_expression(e: &fluent_syntax::ast::Expression<&str>, shape: &mut Shape) {
    use fluent_syntax::ast::{Expression, InlineExpression, VariantKey};
    match e {
        Expression::Inline(i) => walk_inline(i, shape),
        Expression::Select { selector, variants } => {
            walk_inline(selector, shape);
            let var = match selector {
                InlineExpression::VariableReference { id } => Some(id.name.to_owned()),
                InlineExpression::FunctionReference { arguments, .. } => {
                    arguments.positional.iter().find_map(|a| match a {
                        InlineExpression::VariableReference { id } => Some(id.name.to_owned()),
                        _ => None,
                    })
                }
                _ => None,
            };
            let keys: BTreeSet<String> = variants
                .iter()
                .map(|v| match &v.key {
                    VariantKey::Identifier { name } => (*name).to_owned(),
                    VariantKey::NumberLiteral { value } => (*value).to_owned(),
                })
                .collect();
            if let Some(var) = var {
                shape.selects.push((var, keys));
            }
            for v in variants {
                walk_pattern(&v.value, shape);
            }
        }
    }
}

/// Every message (and `message.attribute`) in a Fluent source and what it uses, or the
/// parse errors.
fn shapes(source: &str) -> Result<BTreeMap<String, Shape>, Vec<String>> {
    use fluent_syntax::ast::Entry;
    let res = match fluent_syntax::parser::parse(source) {
        Ok(r) => r,
        Err((_, errors)) => return Err(errors.into_iter().map(|e| format!("{e:?}")).collect()),
    };
    let mut out = BTreeMap::new();
    for entry in &res.body {
        if let Entry::Message(m) = entry {
            if let Some(v) = &m.value {
                let mut shape = Shape::default();
                walk_pattern(v, &mut shape);
                out.insert(m.id.name.to_owned(), shape);
            }
            for a in &m.attributes {
                let mut shape = Shape::default();
                walk_pattern(&a.value, &mut shape);
                out.insert(format!("{}.{}", m.id.name, a.id.name), shape);
            }
        }
    }
    Ok(out)
}

/// What is wrong with a catalog, for its crate's tests (empty when nothing is):
///
/// - it lacks one of [`LOCALES`], or a file does not parse;
/// - a locale lacks a message the source (English) has, or has one it does not;
/// - a message uses placeholders other than the source's (a translation must keep every
///   `{ $name }` and add none);
/// - a selector on a number does not give every plural form its language has (Arabic's
///   `zero`, `one`, `two`, `few`, `many` and `other`; Russian's `one`, `few`, `many`), and
///   each selector has a default.
pub fn check(strings: &Strings) -> Vec<String> {
    let mut problems = Vec::new();
    let Some((source_locale, source)) = strings.files.first() else {
        return vec![format!("{}: no files", strings.name)];
    };
    for l in LOCALES {
        if !strings.files.iter().any(|(f, _)| f == l) {
            problems.push(format!("{}: no {l} file", strings.name));
        }
    }
    let source = match shapes(source) {
        Ok(s) => s,
        Err(e) => return vec![format!("{} {source_locale}: does not parse: {e:?}", strings.name)],
    };
    for (locale, text) in strings.files {
        let here = match shapes(text) {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("{} {locale}: does not parse: {e:?}", strings.name));
                continue;
            }
        };
        if *locale != *source_locale && strings.files.first().is_some_and(|(_, src)| src == text) {
            problems.push(format!("{} {locale}: not translated (the same as the source)", strings.name));
        }
        if text.contains('\u{2068}') || text.contains('\u{2069}') {
            problems.push(format!("{} {locale}: isolation marks are added when formatting, not written", strings.name));
        }
        let plurals = plural_categories(locale);
        for (id, shape) in &source {
            let Some(mine) = here.get(id) else {
                problems.push(format!("{} {locale}: missing {id}", strings.name));
                continue;
            };
            if mine.vars != shape.vars {
                problems.push(format!(
                    "{} {locale}: {id} uses {:?}, the source uses {:?}",
                    strings.name, mine.vars, shape.vars
                ));
            }
            for (var, keys) in &mine.selects {
                // A selector the source uses on this variable with plural categories is a
                // plural selector: every category of this language must be there.
                let plural = shape.selects.iter().any(|(v, k)| {
                    v == var && k.iter().any(|k| ["zero", "one", "two", "few", "many", "other"].contains(&k.as_str()))
                });
                if plural {
                    let missing: Vec<&&str> = plurals.iter().filter(|c| !keys.contains(**c)).collect();
                    if !missing.is_empty() {
                        problems.push(format!(
                            "{} {locale}: {id} lacks the plural form(s) {missing:?} of ${var}",
                            strings.name
                        ));
                    }
                }
            }
        }
        for id in here.keys() {
            if !source.contains_key(id) {
                problems.push(format!("{} {locale}: {id} is not in the source", strings.name));
            }
        }
    }
    // Every message formats in every locale without errors (with sample values).
    let sample: Vec<(String, StrArg)> = source
        .values()
        .flat_map(|s| s.vars.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|v| (v, StrArg::Number(3.0)))
        .collect();
    let args: Vec<(&str, StrArg)> = sample.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    for (locale, _) in strings.files {
        for id in source.keys() {
            let out = strings.format_in(locale, id, &args);
            if out.starts_with('\u{27E8}') {
                problems.push(format!("{} {locale}: {id} does not format", strings.name));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    static T: Strings = Strings::new(
        "test",
        &[
            ("en", "hello = Hello\nitems = { $n ->\n    [one] One item\n   *[other] { $n } items\n}\nnamed = Close { $title }\n"),
            ("pt-BR", "hello = Olá\nitems = { $n ->\n    [one] Um item\n   *[other] { $n } itens\n}\nnamed = Fechar { $title }\n"),
            ("zh-Hans", "hello = 你好\nitems = { $n } 项\nnamed = 关闭 { $title }\n"),
            (
                "ar",
                "hello = مرحبا\nitems = { $n ->\n    [zero] لا عناصر\n    [one] عنصر واحد\n    [two] عنصران\n    [few] { $n } عناصر\n    [many] { $n } عنصرًا\n   *[other] { $n } عنصر\n}\nnamed = إغلاق { $title }\n",
            ),
            ("ru", "hello = Привет\nitems = { $n ->\n    [one] { $n } элемент\n   *[other] { $n } элементов\n}\nnamed = Закрыть { $title }\n"),
        ],
    );

    #[test]
    fn locales_negotiate_to_the_closest_catalog() {
        assert_eq!(T.format_in("pt-PT", "hello", &[]), "Olá", "Portuguese reads Brazilian");
        assert_eq!(T.format_in("zh-CN", "hello", &[]), "你好");
        assert_eq!(T.format_in("zh-TW", "hello", &[]), "你好", "better than English");
        assert_eq!(T.format_in("ar-EG", "hello", &[]), "مرحبا");
        assert_eq!(T.format_in("ko", "hello", &[]), "Hello", "English last");
        assert_eq!(T.format_in("de", "nope", &[]), "\u{27E8}nope\u{27E9}");
    }

    #[test]
    fn plurals_follow_each_language() {
        let n = |l: &str, n: u32| T.format_in(l, "items", &[("n", n.into())]).replace(['\u{2068}', '\u{2069}'], "");
        assert_eq!(n("en", 1), "One item");
        assert_eq!(n("en", 0), "0 items");
        assert_eq!(n("ar", 0), "لا عناصر");
        assert_eq!(n("ar", 2), "عنصران");
        assert_eq!(n("ar", 5), "5 عناصر");
        assert_eq!(n("ar", 11), "11 عنصرًا");
        assert_eq!(n("ar", 100), "100 عنصر");
        assert_eq!(n("zh-Hans", 1), "1 项");
    }

    #[test]
    fn placeholders_are_isolated_for_mixed_directions() {
        let s = T.format_in("ar", "named", &[("title", "report.pdf".into())]);
        assert_eq!(s, "إغلاق \u{2068}report.pdf\u{2069}");
    }

    #[test]
    fn numbers_take_the_locales_digits_when_a_formatter_is_set() {
        set_number_format(Some(Rc::new(|locale, v, _| {
            locale.starts_with("ar").then(|| {
                format!("{v}").chars().map(|c| c.to_digit(10).map_or(c, |d| char::from_u32(0x660 + d).unwrap_or(c))).collect()
            })
        })));
        assert_eq!(T.format_in("ar", "items", &[("n", 5.into())]), "\u{2068}٥\u{2069} عناصر");
        assert_eq!(T.format_in("en", "items", &[("n", 5.into())]), "\u{2068}5\u{2069} items");
        set_number_format(None);
    }

    #[test]
    fn apps_add_languages_and_reword_messages() {
        let errors = add_messages("test", "tr", "hello = Merhaba\n");
        assert!(errors.is_empty());
        assert_eq!(T.format_in("tr", "hello", &[]), "Merhaba");
        assert_eq!(T.format_in("tr", "named", &[("title", "x".into())]), "Close \u{2068}x\u{2069}", "the rest is English");
        add_messages("test", "en", "hello = Hi there\n");
        assert_eq!(T.format_in("en-GB", "hello", &[]), "Hi there");
    }

    #[test]
    fn the_checker_finds_what_is_wrong() {
        static BAD: Strings = Strings::new(
            "bad",
            &[
                ("en", "a = A { $x }\nb = { $n ->\n    [one] One\n   *[other] Many\n}\n"),
                ("ru", "a = А { $y }\nb = { $n ->\n    [one] Один\n   *[other] Много\n}\nc = extra\n"),
                ("ar", "a = \u{2068}A\u{2069} { $x }\n"),
            ],
        );
        let p = check(&BAD);
        let has = |s: &str| p.iter().any(|x| x.contains(s));
        assert!(has("no es file"), "{p:?}");
        assert!(has("ru: a uses"), "{p:?}");
        assert!(has("ru: b lacks the plural form(s) [\"few\", \"many\"]"), "{p:?}");
        assert!(has("ru: c is not in the source"), "{p:?}");
        assert!(has("ar: missing b"), "{p:?}");
        assert!(has("ar: isolation marks"), "{p:?}");
    }
}
