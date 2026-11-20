//! Printing: Windows' own HTML printer (`mshtml.dll,PrintHTML`) shows the print dialog with
//! a preview-less page setup and prints the page as a browser would.

use std::time::{Duration, SystemTime};

use remedy_ui_host::Unsupported;

/// Writes `html` to a file of its own and opens the print dialog for it. The dialog runs in
/// its own process; the file stays until a later print removes it (pages older than a day).
pub(crate) fn print_html(title: &str, html: &str) -> Result<(), Unsupported> {
    let err = |e: std::io::Error| Unsupported::new("printing", e.to_string());
    let dir = std::env::temp_dir().join("remedy-print");
    std::fs::create_dir_all(&dir).map_err(err)?;
    tidy(&dir);
    let stem: String =
        title.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').take(60).collect();
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let stem = if stem.trim().is_empty() { "page".to_owned() } else { stem.trim().to_owned() };
    let file = dir.join(format!("{stem}-{stamp}.html"));
    std::fs::write(&file, html).map_err(err)?;
    // The system's copy, never one found on the search path.
    let system =
        std::env::var_os("SystemRoot").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from);
    std::process::Command::new(system.join("System32").join("rundll32.exe"))
        .arg("mshtml.dll,PrintHTML")
        .arg(&file)
        .spawn()
        .map(|_| ())
        .map_err(err)
}

/// Removes pages printed more than a day ago.
fn tidy(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
