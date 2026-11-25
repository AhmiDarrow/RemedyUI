//! A theme's files: its background image and its fonts. The theme holds references (a path
//! relative to the config folder, or the bytes inline as a `data:` URI); here they are found,
//! checked and used:
//!
//! - **Images** are only probed here (the header, for their size), then shown through the
//!   image pack's element, which decodes them off the UI thread at the size they are shown
//!   (in the sandboxed decoder when the app installed it), covering the window.
//! - **Fonts** are checked by the font engine before any window uses them, then added to
//!   each attached window; a font token naming the theme's font resolves to the family the
//!   file actually holds.
//! - **Links** (`https:` and the like) are never fetched: the engine sends nothing over the
//!   network. A theme that names one says so in a notice.
//!
//! Imported and chosen files are copied into the config folder's `theme-assets` folder, so
//! the theme keeps working when the original moves, and they are part of the theme's undo.

use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use remedy_ui_media_image::{Fit, Source};
use remedy_ui_core::style::Theme;
use remedy_ui_view::{El, Ui, WeakUi};
use serde_json::{Map, Value, json};

use crate::{Area, Customizations, CustomizeError, Notice, Words, files, locale_of, tr_now};

/// The folder in the config folder that holds a theme's files.
pub(crate) const ASSETS_DIR: &str = "theme-assets";

/// A font the theme brings, checked and ready for windows.
#[derive(Clone)]
pub(crate) struct LoadedFont {
    /// The reference it was read from.
    pub(crate) reference: String,
    pub(crate) bytes: Rc<Vec<u8>>,
}

/// A theme with its files loaded.
pub(crate) struct Loaded {
    /// The theme as windows use it (font tokens resolved to the families the files hold).
    pub(crate) theme: Theme,
    pub(crate) fonts: Vec<LoadedFont>,
    pub(crate) background: Option<(String, Source)>,
    /// What could not be used.
    pub(crate) notices: Vec<Notice>,
}

/// What a window was given of the theme's files.
pub(crate) struct WindowAssets {
    pub(crate) ui: WeakUi,
    /// The font references added to it.
    pub(crate) fonts: Vec<String>,
    /// The background reference shown and its element.
    pub(crate) background: Option<(String, El)>,
}

/// Where an asset reference points.
pub(crate) enum Located {
    /// Inline bytes (a `data:` URI).
    Bytes(Vec<u8>),
    /// A file.
    File(PathBuf),
    /// A link outside the computer, never fetched.
    Link(String),
}

fn b64_value(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    }))
}

/// Standard or URL-safe base64, padding optional, whitespace ignored.
fn base64(s: &str) -> Result<Vec<u8>, Words> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = b64_value(c)
            .ok_or_else(|| Words::msg("not-base64", vec![("char", format!("{:?}", char::from(c)).into())]))?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

/// `%xx` escapes decoded.
fn percent(s: &str) -> Result<Vec<u8>, Words> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
            let v = hex
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| Words::msg("broken-escape", Vec::new()))?;
            out.push(v);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    Ok(out)
}

/// The bytes of a `data:` URI.
pub(crate) fn data_uri(s: &str) -> Result<Vec<u8>, Words> {
    let rest = s.strip_prefix("data:").ok_or_else(|| Words::msg("not-data-uri", Vec::new()))?;
    let (meta, data) = rest.split_once(',').ok_or_else(|| Words::msg("data-uri-comma", Vec::new()))?;
    if meta.split(';').any(|p| p.eq_ignore_ascii_case("base64")) { base64(data) } else { percent(data) }
}

/// Whether `s` is a link to somewhere else (a URL that is not inline data).
pub(crate) fn is_link(s: &str) -> bool {
    match s.split_once(':') {
        Some((scheme, _)) => {
            scheme.len() > 1
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && !scheme.eq_ignore_ascii_case("data")
        }
        None => s.starts_with("//"),
    }
}

/// Where `reference` points, a path being relative to `base`.
pub(crate) fn locate(reference: &str, base: &Path) -> Result<Located, Words> {
    let r = reference.trim();
    if r.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")) {
        return data_uri(r).map(Located::Bytes);
    }
    if is_link(r) {
        return Ok(Located::Link(r.to_owned()));
    }
    let p = Path::new(r);
    Ok(Located::File(if p.is_absolute() { p.to_owned() } else { base.join(p) }))
}

/// A relative path that stays inside the folder it is relative to.
fn contained(reference: &str) -> bool {
    let p = Path::new(reference);
    !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// The bytes at a location (files read whole: a font is needed whole, and an image's decoder
/// reads it to decode it at the size shown).
fn bytes_of(at: &Located) -> Result<Vec<u8>, Words> {
    match at {
        Located::Bytes(b) => Ok(b.clone()),
        Located::File(p) => std::fs::read(p).map_err(|e| {
            let path = ("path", p.display().to_string().into());
            Words::msg("error-io", vec![path, ("detail", e.to_string().into())])
        }),
        Located::Link(l) => Err(Words::msg("asset-link", vec![("link", l.into())])),
    }
}

/// The families in a font file, checked by the font engine.
pub(crate) fn check_font(bytes: &[u8]) -> Result<Vec<String>, Words> {
    remedy_ui_text::FontCollection::new().add_owned(bytes.to_vec()).map_err(|e| Words::text(e.to_string()))
}

/// An image's size, read from its header.
pub(crate) fn check_image(bytes: &[u8]) -> Result<(u32, u32), Words> {
    remedy_ui_media_image::dimensions(bytes)
        .map_err(|e| Words::msg("not-an-image", vec![("detail", format!("{e:?}").into())]))
}

/// What kind of file an asset path in a tokens document is.
fn is_font_path(path: &[String]) -> bool {
    path.first().is_some_and(|p| p == "asset") && path.get(1).is_some_and(|p| p == "font")
}

/// Every asset reference in a tokens document: its path and its text.
pub(crate) fn references(doc: &Map<String, Value>) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    let Some(Value::Object(assets)) = doc.get("asset") else { return out };
    let text = |v: &Value| v.get("$value").unwrap_or(v).as_str().map(str::to_owned);
    if let Some(r) = assets.get("background").and_then(text) {
        out.push((vec!["asset".to_owned(), "background".to_owned()], r));
    }
    if let Some(Value::Object(fonts)) = assets.get("font") {
        for (k, v) in fonts {
            if let Some(r) = text(v) {
                out.push((vec!["asset".to_owned(), "font".to_owned(), k.clone()], r));
            }
        }
    }
    out
}

/// Sets an asset token's reference in a tokens document, keeping its `$type`.
fn set_reference(doc: &mut Map<String, Value>, path: &[String], reference: &str) {
    let kind = if is_font_path(path) { "font" } else { "image" };
    let mut cur = doc;
    let Some((last, groups)) = path.split_last() else { return };
    for g in groups {
        let slot = cur.entry(g.clone()).or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(next) = slot else { return };
        cur = next;
    }
    cur.insert(last.clone(), json!({ "$type": kind, "$value": reference }));
}

/// A free name in `dir` for a file called `name`: the name itself, or with a number added,
/// unless a file with the same bytes is already there.
fn free_name(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "asset".into());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut n = 1u32;
    let mut candidate = format!("{stem}{ext}");
    loop {
        match std::fs::read(dir.join(&candidate)) {
            Err(_) => return candidate,
            Ok(existing) if existing == bytes => return candidate,
            Ok(_) => {
                n += 1;
                candidate = format!("{stem}-{n}{ext}");
            }
        }
    }
}

impl Customizations {
    /// The folder of the theme's files.
    pub fn theme_assets_dir(&self) -> PathBuf {
        self.0.dir.join(ASSETS_DIR)
    }

    /// Copies the files a tokens document refers to (relative to `base`) into the theme's
    /// files folder, checking each, and points the document at the copies. Inline `data:`
    /// files are checked and stay inline; links stay as they are. With `only`, just that
    /// reference. Run inside a change, so the copies are part of its undo. What is wrong is
    /// said in `locale`.
    pub(crate) fn bring_assets(
        &self,
        doc: &mut Map<String, Value>,
        base: Option<&Path>,
        only: Option<&[String]>,
        locale: &str,
    ) -> Result<(), CustomizeError> {
        let dir = self.theme_assets_dir();
        for (path, reference) in references(doc) {
            if only.is_some_and(|o| o != path.as_slice()) {
                continue;
            }
            let fail = |why: Words| {
                let at = ("path", path.join(".").into());
                let words = Words::msg("asset-problem", vec![at, ("detail", why.into())]);
                CustomizeError::Invalid(words.in_locale(locale))
            };
            let at = match base {
                Some(b) => {
                    if !is_link(&reference) && !reference.starts_with("data:") && !contained(&reference) {
                        return Err(fail(Words::msg("asset-outside", Vec::new())));
                    }
                    locate(&reference, b).map_err(fail)?
                }
                None if reference.starts_with("data:") || is_link(&reference) => {
                    locate(&reference, &dir).map_err(fail)?
                }
                None => return Err(fail(Words::msg("asset-needs-file", Vec::new()))),
            };
            let file = match &at {
                Located::Link(_) => continue,
                Located::Bytes(_) | Located::File(_) => bytes_of(&at).map_err(&fail)?,
            };
            if is_font_path(&path) {
                check_font(&file).map_err(&fail)?;
            } else {
                check_image(&file).map_err(&fail)?;
            }
            if let Located::File(p) = &at {
                let name =
                    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "asset".into());
                let name = free_name(&dir, &name, &file);
                files::write_atomic(&dir.join(&name), &file)
                    .map_err(|e| CustomizeError::io(&dir.join(&name), &e))?;
                set_reference(doc, &path, &format!("{ASSETS_DIR}/{name}"));
            }
        }
        Ok(())
    }

    /// Removes files from the theme's files folder that `doc` no longer refers to (inside a
    /// change, so undo brings them back).
    pub(crate) fn prune_assets(&self, doc: &Map<String, Value>) -> Result<(), CustomizeError> {
        let used: Vec<PathBuf> = references(doc)
            .into_iter()
            .filter_map(|(_, r)| match locate(&r, &self.0.dir) {
                Ok(Located::File(p)) => Some(p),
                _ => None,
            })
            .collect();
        for p in files::list_all(&self.theme_assets_dir()) {
            if !used.contains(&p) {
                files::remove(&p).map_err(|e| CustomizeError::io(&p, &e))?;
            }
        }
        Ok(())
    }

    /// Copies the files `doc` refers to beside an exported theme at `to`, in a
    /// `<name>-assets` folder, and points the document at them.
    pub(crate) fn send_assets(&self, doc: &mut Map<String, Value>, to: &Path) -> Result<(), CustomizeError> {
        let stem = to.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "theme".into());
        let folder = format!("{stem}-assets");
        let out_dir = to.parent().map_or_else(|| PathBuf::from(&folder), |p| p.join(&folder));
        for (path, reference) in references(doc) {
            let Ok(Located::File(p)) = locate(&reference, &self.0.dir) else { continue };
            let bytes = std::fs::read(&p).map_err(|e| CustomizeError::io(&p, &e))?;
            let name =
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "asset".into());
            let name = free_name(&out_dir, &name, &bytes);
            files::write_atomic(&out_dir.join(&name), &bytes)
                .map_err(|e| CustomizeError::io(&out_dir.join(&name), &e))?;
            set_reference(doc, &path, &format!("{folder}/{name}"));
        }
        Ok(())
    }

    /// Loads `theme`'s files: fonts checked (a font token naming a theme font resolves to the
    /// family its file holds), the background probed. Returns the theme as windows use it,
    /// the fonts, the background, and notices for what could not be used.
    pub(crate) fn load_assets(&self, theme: &Theme) -> Loaded {
        let mut out = theme.clone();
        let mut fonts = Vec::new();
        let mut notices = Vec::new();
        let path = self.theme_file();
        let mut note = |words: Words| notices.push(Notice::new(Area::Theme, path.clone(), words));
        for (declared, reference) in &theme.assets.fonts {
            let loaded = locate(reference, &self.0.dir)
                .and_then(|at| bytes_of(&at))
                .and_then(|b| check_font(&b).map(|names| (b, names)));
            match loaded {
                Ok((bytes, names)) => {
                    if !names.contains(declared) {
                        for list in [&mut out.font_families, &mut out.mono_families] {
                            if let Some(i) = list.iter().position(|f| f == declared) {
                                list.splice(i..=i, names.iter().cloned());
                            }
                        }
                    }
                    fonts.push(LoadedFont { reference: reference.clone(), bytes: Rc::new(bytes) });
                }
                Err(why) => note(Words::msg(
                    "font-not-used",
                    vec![("family", declared.into()), ("detail", why.into())],
                )),
            }
        }
        let background = theme.assets.background.as_ref().and_then(|reference| {
            let at = match locate(reference, &self.0.dir) {
                Ok(at) => at,
                Err(why) => {
                    note(Words::msg("background-not-used", vec![("detail", why.into())]));
                    return None;
                }
            };
            let checked = bytes_of(&at).and_then(|b| check_image(&b).map(|_| b));
            match (checked, at) {
                (Ok(b), Located::Bytes(_)) => Some((reference.clone(), Source::Bytes(Arc::from(b)))),
                (Ok(_), Located::File(p)) => Some((reference.clone(), Source::File(p))),
                (Ok(_), Located::Link(_)) => None,
                (Err(why), _) => {
                    note(Words::msg("background-not-used", vec![("detail", why.into())]));
                    None
                }
            }
        });
        Loaded { theme: out, fonts, background, notices }
    }

    /// Gives a window the theme's fonts it does not have yet, and its background.
    pub(crate) fn apply_assets(&self, ui: &Ui) {
        let (fonts, background) = match self.0.theme.borrow().as_ref() {
            Some(a) => (a.fonts.clone(), a.background.clone()),
            None => return,
        };
        let mut state = {
            let mut a = self.0.theme.borrow_mut();
            let Some(a) = a.as_mut() else { return };
            a.windows.retain(|w| w.ui.upgrade().is_some());
            match a.windows.iter().position(|w| w.ui.upgrade().is_some_and(|u| u.is_same(ui))) {
                Some(i) => a.windows.remove(i),
                None => WindowAssets { ui: ui.downgrade(), fonts: Vec::new(), background: None },
            }
        };
        for f in &fonts {
            if !state.fonts.contains(&f.reference) && ui.add_font((*f.bytes).clone()).is_ok() {
                state.fonts.push(f.reference.clone());
            }
        }
        let wanted = background.as_ref().map(|(r, _)| r.clone());
        if state.background.as_ref().map(|(r, _)| r.clone()) != wanted {
            if let Some((_, el)) = state.background.take() {
                el.remove();
            }
            if let Some((reference, source)) = background {
                let el = remedy_ui_widgets_core::fill_window(remedy_ui_media_image::image(
                    ui,
                    source,
                    "",
                    Fit::Cover,
                ))
                .pass_through();
                ui.root().insert_child(0, &el);
                state.background = Some((reference, el));
            }
        }
        if let Some(a) = self.0.theme.borrow_mut().as_mut() {
            a.windows.push(state);
        }
    }

    /// Shows `image` behind every window's content (a copy goes into the theme's files), or
    /// none; undoable.
    pub fn set_background(&self, ui: &Ui, image: Option<&Path>) -> Result<(), CustomizeError> {
        let (_, mut doc) = self.theme_doc()?;
        let path = vec!["asset".to_owned(), "background".to_owned()];
        let label = match image {
            Some(p) => {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
                set_reference(&mut doc, &path, &name);
                let label = tr_now(ui, "history-change-background", &[]);
                return self.write_with_assets(ui, label, doc, Some((dir, path)));
            }
            None => tr_now(ui, "history-remove-background", &[]),
        };
        crate::theme::remove_token(&mut doc, &path);
        self.write_with_assets(ui, label, doc, None)
    }

    /// Adds the font file at `file` to the theme as `family` (a copy goes into the theme's
    /// files), first in the text fonts when `use_for_text`; undoable.
    pub fn add_theme_font(
        &self,
        ui: &Ui,
        family: &str,
        file: &Path,
        use_for_text: bool,
    ) -> Result<(), CustomizeError> {
        let family = family.trim();
        if family.is_empty() {
            return Err(CustomizeError::Invalid(tr_now(ui, "font-needs-name", &[])));
        }
        let (theme, mut doc) = self.theme_doc()?;
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let at = vec!["asset".to_owned(), "font".to_owned(), family.to_owned()];
        set_reference(&mut doc, &at, &name);
        if use_for_text {
            let mut families = theme.font_families.clone();
            families.retain(|f| f != family);
            families.insert(0, family.to_owned());
            crate::theme::put_token(
                &mut doc,
                &crate::Token::font_family(),
                &crate::TokenValue::Families(families),
            );
        }
        let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let label = tr_now(ui, "history-add-font", &[("family", family.into())]);
        self.write_with_assets(ui, label, doc, Some((dir, at)))
    }

    /// Takes the theme's font `family` away (and out of the font lists); undoable.
    pub fn remove_theme_font(&self, ui: &Ui, family: &str) -> Result<(), CustomizeError> {
        let (theme, mut doc) = self.theme_doc()?;
        crate::theme::remove_token(&mut doc, &["asset".to_owned(), "font".to_owned(), family.to_owned()]);
        for (token, list) in [
            (crate::Token::font_family(), &theme.font_families),
            (crate::Token::mono_family(), &theme.mono_families),
        ] {
            if list.iter().any(|f| f == family) {
                let rest: Vec<String> = list.iter().filter(|f| *f != family).cloned().collect();
                crate::theme::put_token(&mut doc, &token, &crate::TokenValue::Families(rest));
            }
        }
        let label = tr_now(ui, "history-remove-font", &[("family", family.into())]);
        self.write_with_assets(ui, label, doc, None)
    }

    /// The theme with the person's changes as the file says it (assets as references,
    /// families as written), and the file's document.
    fn theme_doc(&self) -> Result<(Theme, Map<String, Value>), CustomizeError> {
        let (default, doc) = self.theme_parts()?;
        let text = serde_json::to_string(&Value::Object(doc.clone())).unwrap_or_else(|_| "{}".into());
        let theme = remedy_ui_themes::from_tokens(&text, &default).map_err(CustomizeError::Token)?;
        Ok((theme, doc))
    }

    /// Writes `doc` as one change, first bringing in the file one reference names (relative to
    /// the folder given with it).
    fn write_with_assets(
        &self,
        ui: &Ui,
        label: String,
        mut doc: Map<String, Value>,
        bring: Option<(PathBuf, Vec<String>)>,
    ) -> Result<(), CustomizeError> {
        let (default, _) = self.theme_parts()?;
        let locale = locale_of(ui);
        self.change(ui, label, None, &[Area::Theme], move |c| {
            if let Some((base, path)) = &bring {
                c.bring_assets(&mut doc, Some(base), Some(path), &locale)?;
            }
            let text = serde_json::to_string(&Value::Object(doc.clone())).unwrap_or_else(|_| "{}".into());
            remedy_ui_themes::from_tokens(&text, &default).map_err(CustomizeError::Token)?;
            c.write_theme(&doc)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uris_decode() {
        assert_eq!(data_uri("data:text/plain;base64,SGVsbG8="), Ok(b"Hello".to_vec()));
        assert_eq!(data_uri("data:,a%20b"), Ok(b"a b".to_vec()));
        assert!(data_uri("data:;base64,@@@").is_err());
        assert!(!is_link("data:image/png;base64,AAAA") && is_link("https://x/y.png") && !is_link("C:/x.png"));
        assert!(contained("fonts/a.ttf") && !contained("../a.ttf") && !contained("/etc/a.ttf"));
    }
}
