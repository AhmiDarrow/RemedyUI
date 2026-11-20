//! The person's text: every string the app shows can be rewritten through message files in
//! the `text` folder of the config folder: `<locale>.json` files of source text to text for
//! a [`Catalog`], and `<locale>.ftl` Fluent files for [`Messages`]. Off until the app turns
//! it on.

use remedy_ui_view::words::fill;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use remedy_ui_services_i18n::{
    Catalog, MessageEntry, Messages, Problem, canonical_locale, read_entries, remove_entry, set_entry,
};
use remedy_ui_view::Ui;
use serde_json::Value;

use crate::{Area, Customizations, CustomizeError, Notice, files};

/// The override folder's name in the config folder.
pub(crate) const TEXT_DIR: &str = "text";

/// The text sources overridden.
#[derive(Default)]
pub(crate) struct TextArea {
    pub(crate) catalog: Option<Catalog>,
    pub(crate) messages: Option<Messages>,
}

impl Customizations {
    /// The folder of the person's message files.
    pub fn text_dir(&self) -> PathBuf {
        self.0.dir.join(TEXT_DIR)
    }

    /// Lets the person rewrite every string `catalog` gives: `<locale>.json` files in
    /// [`Customizations::text_dir`] take precedence over the app's translations. A file that
    /// does not parse is skipped with a notice.
    pub fn enable_text(&self, catalog: &Catalog) {
        catalog.set_override_folder(Some(self.text_dir()));
        self.0.text.borrow_mut().get_or_insert_with(TextArea::default).catalog = Some(catalog.clone());
        self.reload(&[Area::Text]);
    }

    /// Lets the person rewrite every Fluent message: `<locale>.ftl` files in
    /// [`Customizations::text_dir`] are read after the app's own, so a message there replaces
    /// the app's. Turn it on once per `messages`.
    pub fn enable_messages(&self, messages: &Messages) {
        let _ = messages.clone().folder(self.text_dir());
        self.0.text.borrow_mut().get_or_insert_with(TextArea::default).messages = Some(messages.clone());
        self.reload(&[Area::Text]);
    }

    /// The catalog whose strings can be rewritten.
    pub fn text_catalog(&self) -> Option<Catalog> {
        self.0.text.borrow().as_ref().and_then(|t| t.catalog.clone())
    }

    fn catalog_or_err(&self) -> Result<Catalog, CustomizeError> {
        self.text_catalog().ok_or(CustomizeError::NotEnabled(Area::Text))
    }

    /// The person's text for `source` in `locale`, if they rewrote it.
    pub fn text_override(&self, locale: &str, source: &str) -> Option<String> {
        self.0.revision.get();
        self.text_catalog().and_then(|c| c.override_in(locale, source))
    }

    /// Rewrites `source` for `locale` (undoable; typing is one entry). Text equal to the
    /// app's own, or blank, takes the override away.
    pub fn set_text(&self, ui: &Ui, locale: &str, source: &str, text: &str) -> Result<(), CustomizeError> {
        let catalog = self.catalog_or_err()?;
        let file_locale = catalog.override_locale_for(locale);
        let mut table: BTreeMap<String, String> = catalog.overrides(&file_locale).into_iter().collect();
        if text.trim().is_empty() || text == catalog.translation_in(locale, source) {
            if table.remove(source).is_none() {
                return Ok(());
            }
        } else if table.get(source).map(String::as_str) == Some(text) {
            return Ok(());
        } else {
            table.insert(source.to_owned(), text.to_owned());
        }
        let path = self.text_dir().join(format!("{file_locale}.json"));
        let key = format!("text:{file_locale}:{source}");
        self.change(ui, fill(&ui.word("Change “{text}”"), &[("text", source)]), Some(key), &[Area::Text], |_| {
            write_table(&path, &table)
        })
    }

    /// Takes the person's text for `source` in `locale` away (undoable).
    pub fn reset_text(&self, ui: &Ui, locale: &str, source: &str) -> Result<(), CustomizeError> {
        let catalog = self.catalog_or_err()?;
        let file_locale = catalog.override_locale_for(locale);
        let mut table: BTreeMap<String, String> = catalog.overrides(&file_locale).into_iter().collect();
        if table.remove(source).is_none() {
            return Ok(());
        }
        let path = self.text_dir().join(format!("{file_locale}.json"));
        self.change(ui, fill(&ui.word("Reset “{text}”"), &[("text", source)]), None, &[Area::Text], |_| write_table(&path, &table))
    }

    /// Removes every message file the person wrote (undoable).
    pub fn reset_texts(&self, ui: &Ui) -> Result<(), CustomizeError> {
        if self.0.text.borrow().is_none() {
            return Err(CustomizeError::NotEnabled(Area::Text));
        }
        let all = self.files_of(Area::Text);
        self.change(ui, ui.word("Reset all text"), None, &[Area::Text], |_| {
            for p in &all {
                files::remove(p).map_err(|e| CustomizeError::io(p, &e))?;
            }
            Ok(())
        })
    }

    /// The Fluent messages whose text can be rewritten.
    pub fn text_messages(&self) -> Option<Messages> {
        self.0.text.borrow().as_ref().and_then(|t| t.messages.clone())
    }

    /// The person's Fluent file for `locale`.
    pub fn messages_file(&self, locale: &str) -> PathBuf {
        self.text_dir().join(format!("{}.ftl", canonical_locale(locale)))
    }

    /// Every Fluent message in `locale` (tracked): as shown now (with the person's override),
    /// the app's own (beneath it), and whether the person rewrote it.
    pub fn message_entries(&self, locale: &str) -> Vec<(MessageEntry, MessageEntry, bool)> {
        self.0.revision.get();
        let Some(m) = self.text_messages() else { return Vec::new() };
        let dir = self.text_dir();
        let own = read_entries(&std::fs::read_to_string(self.messages_file(locale)).unwrap_or_default());
        let apps = m.entries(locale, Some(&dir));
        let shown = m.entries(locale, None);
        shown
            .into_iter()
            .map(|e| {
                let app = apps.iter().find(|a| a.id == e.id).cloned().unwrap_or_else(|| e.clone());
                let mine = own.iter().any(|o| o.id == e.id);
                (e, app, mine)
            })
            .collect()
    }

    /// Rewrites one Fluent message for `locale` (undoable; typing is one entry). The message
    /// is checked first: Fluent text that does not read back as written is refused with the
    /// reason. A message equal to the app's own takes the override away.
    pub fn set_message(&self, ui: &Ui, locale: &str, entry: &MessageEntry) -> Result<(), CustomizeError> {
        let messages = self.text_messages().ok_or(CustomizeError::NotEnabled(Area::Text))?;
        let path = self.messages_file(locale);
        let file = std::fs::read_to_string(&path).unwrap_or_default();
        let app = messages.entries(locale, Some(&self.text_dir())).into_iter().find(|a| a.id == entry.id);
        let next = if app.as_ref() == Some(entry) {
            if !read_entries(&file).iter().any(|e| e.id == entry.id) {
                return Ok(());
            }
            remove_entry(&file, &entry.id)
        } else {
            set_entry(&file, entry).map_err(|e| CustomizeError::Invalid(e.to_string()))?
        };
        let key = format!("message:{}:{}", canonical_locale(locale), entry.id);
        self.change(ui, fill(&ui.word("Change {name}"), &[("name", &entry.id)]), Some(key), &[Area::Text], |_| {
            write_text(&path, &next)
        })
    }

    /// Takes the person's version of message `id` in `locale` away (undoable).
    pub fn reset_message(&self, ui: &Ui, locale: &str, id: &str) -> Result<(), CustomizeError> {
        if self.text_messages().is_none() {
            return Err(CustomizeError::NotEnabled(Area::Text));
        }
        let path = self.messages_file(locale);
        let file = std::fs::read_to_string(&path).unwrap_or_default();
        if !read_entries(&file).iter().any(|e| e.id == id) {
            return Ok(());
        }
        let next = remove_entry(&file, id);
        self.change(ui, fill(&ui.word("Reset {name}"), &[("name", id)]), None, &[Area::Text], |_| write_text(&path, &next))
    }

    /// Reads the message files again (after the person edited them by hand).
    pub fn reload_text(&self) {
        self.reload(&[Area::Text]);
    }

    pub(crate) fn reload_text_area(&self) {
        let (catalog, messages) = match self.0.text.borrow().as_ref() {
            Some(t) => (t.catalog.clone(), t.messages.clone()),
            None => return,
        };
        let dir = self.text_dir();
        let mut notices = Vec::new();
        if let Some(c) = catalog {
            c.reload_overrides();
            notices.extend(c.override_problems().into_iter().map(|p| Notice {
                area: Area::Text,
                path: p.path,
                message: format!("skipped: {}", p.detail),
            }));
        }
        if let Some(m) = messages {
            m.reload();
            for p in m.problems() {
                let (path, message) = match p {
                    Problem::Syntax { source, detail } => (
                        PathBuf::from(source),
                        format!("part of the file did not read, the rest is used: {detail}"),
                    ),
                    Problem::Io { path, detail } => (PathBuf::from(path), detail),
                    // Missing messages and formatting problems are the app's, not the file's.
                    _ => continue,
                };
                if path.starts_with(&dir) && !notices.iter().any(|n: &Notice| n.path == path) {
                    notices.push(Notice { area: Area::Text, path, message });
                }
            }
        }
        self.set_notices(Area::Text, None, notices);
    }
}

fn write_table(path: &Path, table: &BTreeMap<String, String>) -> Result<(), CustomizeError> {
    let bytes =
        (!table.is_empty()).then(|| files::json_text(&serde_json::to_value(table).unwrap_or(Value::Null)));
    files::put(path, bytes.as_deref()).map_err(|e| CustomizeError::io(path, &e))
}

/// Writes a Fluent file, or removes it when nothing is left in it.
fn write_text(path: &Path, text: &str) -> Result<(), CustomizeError> {
    let bytes = (!text.trim().is_empty()).then_some(text.as_bytes());
    files::put(path, bytes).map_err(|e| CustomizeError::io(path, &e))
}
