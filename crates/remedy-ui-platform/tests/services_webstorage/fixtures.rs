//! Stores Chrome wrote: headless Chrome 157 with a fresh profile, on a page served from
//! `http://127.0.0.1:<port>`, closed cleanly, and its `Default/Local Storage/leveldb` copied
//! here byte for byte (LevelDB's text `LOG` files, which name the profile's path, left out).
//!
//! - `basic`: one session. Items in ASCII, German (Latin-1), emoji and Arabic (UTF-16), keys in
//!   both formats, an empty value, JSON, a 50,000-character value (its log record spans two
//!   32 KiB blocks), then, in a later write batch, an overwrite and a `removeItem`. Everything
//!   is in the write-ahead log `000003.log`.
//! - `compacted`: three sessions. The first writes 400 items of 2–6 KB, which Chrome moved into
//!   the Snappy-compressed table `000003.ldb` when it opened the store again; the second and
//!   third overwrite and remove some of them and add one, in the reused log `000004.log`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use remedy_ui_platform::services_webstorage::{ChromiumStorage, WebStorageError};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/services_webstorage").join("fixtures").join(name)
}

/// The 50,000-character value the page wrote:
/// `Array.from({length: 50000}, (_, i) => String.fromCharCode(97 + (i * 7) % 26)).join('')`.
fn big_value() -> String {
    (0..50_000u32).filter_map(|i| char::from_u32(97 + (i * 7) % 26)).collect()
}

#[test]
fn basic_store_reads_exactly() -> Result {
    let dir = fixture("basic");
    assert!(std::fs::metadata(dir.join("000003.log"))?.len() > 32 * 1024, "the log spans blocks");
    let store = ChromiumStorage::read_dir(&dir)?;
    assert_eq!(store.origins(), ["http://127.0.0.1:47123"]);
    assert_eq!(store.damaged_log_records(), 0);

    let saved = store.local_storage("http://127.0.0.1:47123")?;
    let want: BTreeMap<String, String> = [
        ("ascii", "hello world".to_owned()),
        ("german", "Grüße aus Köln".to_owned()),
        ("emoji", "🎉 party ✓".to_owned()),
        ("arabic", "مرحبا بالعالم".to_owned()),
        ("schlüssel", "latin-1 key".to_owned()),
        ("ключ", "utf-16 key".to_owned()),
        ("empty", String::new()),
        ("craftcalc.settings", r#"{"units":"g","lang":"de","lye":"NaOH"}"#.to_owned()),
        ("big", big_value()),
        ("counter", "2".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(saved.items, want);
    assert!(saved.unpaired.is_empty());
    assert_eq!(saved.origin, "http://127.0.0.1:47123");

    // The same origin written with a trailing slash, and origins that are not there.
    assert_eq!(store.local_storage("http://127.0.0.1:47123/")?.items, want);
    assert!(store.local_storage("http://127.0.0.1:47124")?.is_empty());
    assert_eq!(store.first_of(&["http://tauri.localhost", "https://tauri.localhost"])?, None);
    assert_eq!(store.first_of(&["http://localhost", "http://127.0.0.1:47123"])?.map(|s| s.items), Some(want));
    Ok(())
}

#[test]
fn compacted_store_merges_table_and_log() -> Result {
    let dir = fixture("compacted");
    assert!(dir.join("000003.ldb").is_file(), "the fixture has a table");
    let store = ChromiumStorage::read_dir(&dir)?;
    assert_eq!(store.origins(), ["http://127.0.0.1:47124"]);
    assert_eq!(store.damaged_log_records(), 0);

    let saved = store.local_storage("http://127.0.0.1:47124")?;
    let mut want = BTreeMap::new();
    for i in 0..400 {
        let key = format!("item-{i:03}");
        let value = format!("{key};").repeat(250 + i);
        want.insert(key, value);
    }
    // Session 2: overwrite (log over table), removeItem (log deletion over table value), new.
    want.insert("item-005".into(), "overwritten in session 2".into());
    want.remove("item-007");
    want.insert("after".into(), "Grüße 🎉".into());
    // Session 3, in the same reused log.
    want.remove("item-009");
    want.insert("item-010".into(), "overwritten in session 3".into());
    assert_eq!(saved.len(), 399);
    assert_eq!(saved.items, want);
    Ok(())
}

#[test]
fn from_files_matches_read_dir() -> Result {
    for name in ["basic", "compacted"] {
        let dir = fixture(name);
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            files.push((entry.file_name().to_string_lossy().into_owned(), std::fs::read(entry.path())?));
        }
        let origin = ChromiumStorage::read_dir(&dir)?.origins();
        let from_disk =
            ChromiumStorage::read_dir(&dir)?.first_of(&[origin.first().map_or("", String::as_str)])?;
        let in_memory = ChromiumStorage::from_files(files)?;
        assert_eq!(in_memory.origins(), origin);
        assert_eq!(in_memory.first_of(&[origin.first().map_or("", String::as_str)])?, from_disk, "{name}");
    }
    Ok(())
}

/// Every file's name, contents and modification time.
fn state(dir: &Path) -> Result<BTreeMap<String, (Vec<u8>, SystemTime)>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        out.insert(
            entry.file_name().to_string_lossy().into_owned(),
            (std::fs::read(entry.path())?, meta.modified()?),
        );
    }
    Ok(out)
}

#[test]
fn reading_leaves_the_source_byte_identical() -> Result {
    for name in ["basic", "compacted"] {
        let dir = fixture(name);
        let before = state(&dir)?;
        let store = ChromiumStorage::read_dir(&dir)?;
        let _ = store.local_storage("http://127.0.0.1:47123")?;
        let _ = store.local_storage("http://127.0.0.1:47124")?;
        assert_eq!(state(&dir)?, before, "{name}: no file added, removed, changed or touched");
    }
    Ok(())
}

#[test]
fn a_missing_folder_is_not_found() {
    let result = ChromiumStorage::read_dir(fixture("no-such-store"));
    assert!(matches!(result, Err(WebStorageError::NotFound { .. })));
}

/// A store in use: the log held open for writing and the lock file held exclusively, with the
/// sharing Chromium's LevelDB uses on Windows.
#[cfg(windows)]
#[test]
fn a_store_in_use_reads_on_windows() -> Result {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;

    let dir = std::env::temp_dir().join(format!("remedy-webstorage-in-use-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    for entry in std::fs::read_dir(fixture("compacted"))? {
        let entry = entry?;
        std::fs::copy(entry.path(), dir.join(entry.file_name()))?;
    }
    let result = (|| -> Result {
        let _log = std::fs::OpenOptions::new()
            .append(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(dir.join("000004.log"))?;
        let _lock =
            std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(dir.join("LOCK"))?;
        let store = ChromiumStorage::read_dir(&dir)?;
        assert_eq!(store.local_storage("http://127.0.0.1:47124")?.len(), 399);
        Ok(())
    })();
    std::fs::remove_dir_all(&dir)?;
    result
}
