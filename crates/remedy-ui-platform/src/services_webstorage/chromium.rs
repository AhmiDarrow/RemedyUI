//! Chromium's localStorage layout in its LevelDB store, and the items it decodes to.
//!
//! One store holds every origin's localStorage:
//!
//! - `VERSION` → `1`, the schema version;
//! - `META:<origin>` and `METAACCESS:<origin>` → the origin's size and last write and access
//!   times (protocol buffers; not part of the data);
//! - `_<origin>\0<key>` → `<value>`, one per item. Key and value each start with a format
//!   byte: `1` for Latin-1 (one byte per character, used when every character is below
//!   U+0100) and `0` for UTF-16LE.

use std::collections::BTreeMap;
use std::path::Path;

use crate::services_webstorage::WebStorageError;
use crate::services_webstorage::db::{self, Files};

/// One origin's localStorage: every item, keyed and valued as the page saw them.
///
/// JavaScript strings are UTF-16 and may hold a lone surrogate, which no Rust `String` can.
/// Items like that are kept as UTF-16 in [`LocalStorage::unpaired`] instead of being changed
/// or dropped, so nothing is lost; ordinary text (any language, emoji) is in
/// [`LocalStorage::items`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalStorage {
    /// The origin the items belong to, such as `http://tauri.localhost`.
    pub origin: String,
    /// The items, by key.
    pub items: BTreeMap<String, String>,
    /// Items whose key or value is not valid Unicode (a lone UTF-16 surrogate), as UTF-16.
    pub unpaired: Vec<Utf16Item>,
}

impl LocalStorage {
    /// The value of `key`, as `localStorage.getItem(key)` would return it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.items.get(key).map(String::as_str)
    }

    /// How many items there are, `unpaired` ones included (`localStorage.length`).
    pub fn len(&self) -> usize {
        self.items.len() + self.unpaired.len()
    }

    /// Whether there are no items at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Builds the storage of `origin` from UTF-16 keys and values, as a browser hands them out.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn from_utf16(origin: String, pairs: impl IntoIterator<Item = (Vec<u16>, Vec<u16>)>) -> Self {
        let mut out = Self { origin, ..Self::default() };
        for (key, value) in pairs {
            out.insert(WebString::from_utf16(key), WebString::from_utf16(value));
        }
        out
    }

    fn insert(&mut self, key: WebString, value: WebString) {
        match (key, value) {
            (WebString::Text(key), WebString::Text(value)) => {
                self.items.insert(key, value);
            }
            (key, value) => {
                self.unpaired.push(Utf16Item { key: key.into_utf16(), value: value.into_utf16() })
            }
        }
    }
}

/// An item whose key or value holds a lone UTF-16 surrogate, kept exactly as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Utf16Item {
    /// The key, as UTF-16 code units.
    pub key: Vec<u16>,
    /// The value, as UTF-16 code units.
    pub value: Vec<u16>,
}

/// A JavaScript string: Unicode text, or UTF-16 that is not valid Unicode.
enum WebString {
    Text(String),
    Utf16(Vec<u16>),
}

impl WebString {
    fn from_utf16(units: Vec<u16>) -> Self {
        match String::from_utf16(&units) {
            Ok(text) => Self::Text(text),
            Err(_) => Self::Utf16(units),
        }
    }

    fn into_utf16(self) -> Vec<u16> {
        match self {
            Self::Text(text) => text.encode_utf16().collect(),
            Self::Utf16(units) => units,
        }
    }

    /// Decodes Chromium's stored form: a format byte, then Latin-1 or UTF-16LE.
    fn decode(stored: &[u8]) -> Option<Self> {
        match stored.split_first()? {
            (1, latin1) => Some(Self::Text(latin1.iter().map(|&b| char::from(b)).collect())),
            (0, utf16) => {
                let (pairs, rest) = utf16.as_chunks::<2>();
                if !rest.is_empty() {
                    return None;
                }
                Some(Self::from_utf16(pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect()))
            }
            _ => None,
        }
    }
}

const DATA_PREFIX: &[u8] = b"_";
const META_PREFIX: &[u8] = b"META:";
const META_ACCESS_PREFIX: &[u8] = b"METAACCESS:";

/// Origins are compared without a trailing slash, as Chromium serializes them.
fn normalize(origin: &str) -> &str {
    origin.strip_suffix('/').unwrap_or(origin)
}

/// A Chromium localStorage store (WebView2, Android System WebView, Chrome), read once into
/// memory and never written.
///
/// Reading takes every live table and write-ahead log of the folder and keeps the newest
/// write of each key, so what comes back is what the app saw the last time it ran, including
/// writes it made just before it closed.
#[derive(Clone, Debug)]
pub struct ChromiumStorage {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    damaged_log_records: usize,
}

impl ChromiumStorage {
    /// Reads the store in `dir`, the `Local Storage/leveldb` folder of a web view's profile
    /// ([`crate::services_webstorage::tauri_windows_dir`] and [`crate::services_webstorage::android_webview_dir`] say where those are).
    ///
    /// The folder is only read: each file is opened read-only, nothing is created, locked or
    /// changed in it, and it may be in use by a running app while it is read. If that app
    /// changes the store meanwhile, the read starts over until it sees one consistent state.
    ///
    /// # Errors
    ///
    /// [`WebStorageError::NotFound`] when the folder does not exist, [`WebStorageError::Io`]
    /// when a file cannot be read, and [`WebStorageError::Corrupt`] or
    /// [`WebStorageError::UnsupportedCompression`] when a table cannot be decoded. Damage in
    /// a write-ahead log is skipped as Chromium itself skips it when it opens the store, and
    /// counted in [`ChromiumStorage::damaged_log_records`].
    pub fn read_dir(dir: impl AsRef<Path>) -> Result<Self, WebStorageError> {
        let snapshot = db::read_folder(dir.as_ref())?;
        Ok(Self { entries: snapshot.entries, damaged_log_records: snapshot.damaged_log_records })
    }

    /// Reads a store from its files' names and contents, for a store the app already holds in
    /// memory (files a person dropped on a page, an archive). The names are the files' names
    /// within the folder (`CURRENT`, `MANIFEST-000001`, `000003.log`, `000005.ldb`); other
    /// files are ignored.
    ///
    /// # Errors
    ///
    /// As [`ChromiumStorage::read_dir`], without the folder's errors.
    pub fn from_files<N, B>(files: impl IntoIterator<Item = (N, B)>) -> Result<Self, WebStorageError>
    where
        N: Into<String>,
        B: Into<Vec<u8>>,
    {
        let contents: BTreeMap<String, Vec<u8>> =
            files.into_iter().map(|(n, b)| (n.into(), b.into())).collect();
        let names = contents.keys().cloned().collect();
        let snapshot = db::read_files(Files { names, contents })?;
        Ok(Self { entries: snapshot.entries, damaged_log_records: snapshot.damaged_log_records })
    }

    /// The origins that have localStorage in this store, sorted.
    pub fn origins(&self) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        for key in self.entries.keys() {
            let origin = if let Some(rest) = key.strip_prefix(DATA_PREFIX) {
                rest.split(|&b| b == 0).next().filter(|_| rest.contains(&0))
            } else if let Some(rest) = key.strip_prefix(META_ACCESS_PREFIX) {
                Some(rest)
            } else {
                key.strip_prefix(META_PREFIX)
            };
            if let Some(origin) = origin.and_then(|o| std::str::from_utf8(o).ok()) {
                out.insert(origin.to_owned());
            }
        }
        out.into_iter().collect()
    }

    /// The localStorage of `origin` (`http://tauri.localhost`, `https://localhost`): empty
    /// when the store has none for it.
    ///
    /// # Errors
    ///
    /// [`WebStorageError::Corrupt`] when an item is not in the form Chromium writes (a format
    /// byte other than Latin-1 or UTF-16, or UTF-16 with an odd number of bytes). No item is
    /// skipped quietly.
    pub fn local_storage(&self, origin: &str) -> Result<LocalStorage, WebStorageError> {
        let origin = normalize(origin);
        let mut prefix = DATA_PREFIX.to_vec();
        prefix.extend_from_slice(origin.as_bytes());
        prefix.push(0);
        let mut out = LocalStorage { origin: origin.to_owned(), ..LocalStorage::default() };
        for (key, value) in self.entries.range(prefix.clone()..) {
            let Some(stored_key) = key.strip_prefix(prefix.as_slice()) else { break };
            let bad = |what: &str| {
                WebStorageError::corrupt(
                    "localStorage",
                    format!("an item of {origin} has a malformed {what}"),
                )
            };
            let key = WebString::decode(stored_key).ok_or_else(|| bad("key"))?;
            let value = WebString::decode(value).ok_or_else(|| bad("value"))?;
            out.insert(key, value);
        }
        Ok(out)
    }

    /// The localStorage of the first of `origins` that this store has, such as
    /// [`crate::services_webstorage::TAURI_WINDOWS_ORIGINS`]; `None` when it has none of them.
    ///
    /// # Errors
    ///
    /// As [`ChromiumStorage::local_storage`].
    pub fn first_of(&self, origins: &[&str]) -> Result<Option<LocalStorage>, WebStorageError> {
        let present = self.origins();
        match origins.iter().find(|o| present.iter().any(|p| p == normalize(o))) {
            Some(origin) => self.local_storage(origin).map(Some),
            None => Ok(None),
        }
    }

    /// Damaged stretches of the write-ahead logs that were skipped, as Chromium skips them
    /// when it opens the store; 0 for a healthy store. What the app last saw of those writes
    /// is gone from the store itself, not only from this read.
    pub fn damaged_log_records(&self) -> usize {
        self.damaged_log_records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_latin1(s: &str) -> Vec<u8> {
        let mut out = vec![1];
        out.extend(s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')));
        out
    }

    fn stored_utf16(units: &[u16]) -> Vec<u8> {
        let mut out = vec![0];
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    fn data_key(origin: &str, key: &[u8]) -> Vec<u8> {
        let mut out = b"_".to_vec();
        out.extend_from_slice(origin.as_bytes());
        out.push(0);
        out.extend_from_slice(key);
        out
    }

    fn store(entries: Vec<(Vec<u8>, Vec<u8>)>) -> ChromiumStorage {
        ChromiumStorage { entries: entries.into_iter().collect(), damaged_log_records: 0 }
    }

    #[test]
    fn items_decode_from_both_formats() -> Result<(), WebStorageError> {
        let o = "https://localhost";
        let emoji: Vec<u16> = "🎉 ok".encode_utf16().collect();
        let lone = [0xD83C, u16::from(b'x')];
        let s = store(vec![
            (b"VERSION".to_vec(), b"1".to_vec()),
            (b"META:https://localhost".to_vec(), vec![8, 1]),
            (data_key(o, &stored_latin1("grüße")), stored_latin1("Köln ÿ")),
            (data_key(o, &stored_utf16(&"ключ".encode_utf16().collect::<Vec<_>>())), stored_utf16(&emoji)),
            (data_key(o, &stored_latin1("lone")), stored_utf16(&lone)),
            (data_key(o, &stored_latin1("empty")), vec![1]),
            (data_key("https://other.example", &stored_latin1("x")), stored_latin1("y")),
        ]);
        assert_eq!(s.origins(), ["https://localhost", "https://other.example"]);
        let ls = s.local_storage("https://localhost/")?;
        assert_eq!(ls.origin, "https://localhost");
        assert_eq!(ls.get("grüße"), Some("Köln ÿ"));
        assert_eq!(ls.get("ключ"), Some("🎉 ok"));
        assert_eq!(ls.get("empty"), Some(""));
        assert_eq!(ls.get("lone"), None);
        assert_eq!(ls.unpaired, [Utf16Item { key: "lone".encode_utf16().collect(), value: lone.to_vec() }]);
        assert_eq!(ls.len(), 4);
        assert!(s.local_storage("http://localhost")?.is_empty());
        assert_eq!(s.first_of(&["http://localhost", "https://localhost"])?.map(|l| l.len()), Some(4));
        assert_eq!(s.first_of(&["http://tauri.localhost"])?, None);
        Ok(())
    }

    #[test]
    fn malformed_items_are_errors() {
        let o = "http://a";
        for (key, value) in [
            (stored_latin1("k"), vec![0, 1, 2, 3]), // UTF-16 with an odd byte count
            (stored_latin1("k"), vec![7, 1]),       // unknown format
            (vec![], stored_latin1("v")),           // key without a format byte
            (stored_latin1("k"), vec![]),           // value without a format byte
        ] {
            let s = store(vec![(data_key(o, &key), value)]);
            assert!(matches!(s.local_storage(o), Err(WebStorageError::Corrupt { .. })));
        }
    }

    #[test]
    fn utf16_pairs_from_a_browser() {
        let text = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let ls = LocalStorage::from_utf16(
            "https://localhost".into(),
            [
                (text("a"), text("مرحبا 🎉")),
                (vec![0xDC00], text("lone low surrogate key")),
                (text("b"), vec![0xD800, 0xD800]),
            ],
        );
        assert_eq!(ls.get("a"), Some("مرحبا 🎉"));
        assert_eq!(ls.len(), 3);
        assert_eq!(ls.unpaired.len(), 2);
        assert_eq!(ls.unpaired.first().map(|i| i.key.clone()), Some(vec![0xDC00]));
        assert_eq!(ls.unpaired.get(1).map(|i| i.value.clone()), Some(vec![0xD800, 0xD800]));
    }

    #[test]
    fn an_origin_that_is_a_prefix_of_another_stays_separate() -> Result<(), WebStorageError> {
        let s = store(vec![
            (data_key("http://localhost", &stored_latin1("a")), stored_latin1("1")),
            (data_key("http://localhost:8080", &stored_latin1("b")), stored_latin1("2")),
        ]);
        let ls = s.local_storage("http://localhost")?;
        assert_eq!(ls.items.len(), 1);
        assert_eq!(ls.get("a"), Some("1"));
        Ok(())
    }
}
