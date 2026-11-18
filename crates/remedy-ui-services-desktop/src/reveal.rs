//! Showing a file in the system's file manager, selected: Explorer on Windows, the Finder on
//! macOS, and on Linux the file manager that implements `org.freedesktop.FileManager1` (with
//! the folder opened by `xdg-open` when none does).

use std::path::Path;

/// Opens the file manager at `path`'s folder with `path` selected (a folder opens itself
/// selected in its parent).
pub fn reveal(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err("That file is no longer available.".into());
    }
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    platform(&strip_verbatim(&path))
}

/// Windows' canonical paths start with `\\?\`, which the shell does not parse.
fn strip_verbatim(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}").into(),
        None => s.strip_prefix(r"\\?\").map_or_else(|| p.to_path_buf(), Into::into),
    }
}

#[cfg(windows)]
fn platform(path: &Path) -> Result<(), String> {
    crate::os::reveal(path)
}

#[cfg(target_os = "macos")]
fn platform(path: &Path) -> Result<(), String> {
    run(std::process::Command::new("/usr/bin/open").arg("-R").arg(path))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform(path: &Path) -> Result<(), String> {
    let shown = run(std::process::Command::new("dbus-send").args([
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.FileManager1",
        "--type=method_call",
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1.ShowItems",
        &format!("array:string:{}", file_uri(path)),
        "string:",
    ]));
    if shown.is_ok() {
        return shown;
    }
    let folder = path.parent().unwrap_or(path);
    run(std::process::Command::new("xdg-open").arg(folder))
}

#[cfg(unix)]
fn run(c: &mut std::process::Command) -> Result<(), String> {
    let out = c.stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// A `file://` URI for an absolute path, with everything outside the unreserved set and `/`
/// percent-encoded byte by byte.
pub fn file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    // A drive letter's colon ("C:") stays as it is.
    let drive =
        s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && s.as_bytes().get(1) == Some(&b':');
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) || (b == b':' && i == 1 && drive) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(super::file_uri(Path::new("/home/a b/Ré.txt")), "file:///home/a%20b/R%C3%A9.txt");
        assert_eq!(super::file_uri(Path::new("C:/Users/x#1")), "file:///C:/Users/x%231");
        assert_eq!(super::file_uri(Path::new("/a:b")), "file:///a%3Ab");
    }

    #[test]
    fn verbatim_prefixes_are_dropped() {
        assert_eq!(super::strip_verbatim(Path::new(r"\\?\C:\a")), Path::new(r"C:\a"));
        assert_eq!(super::strip_verbatim(Path::new(r"\\?\UNC\srv\share\f")), Path::new(r"\\srv\share\f"));
        assert_eq!(super::strip_verbatim(Path::new("/tmp/x")), Path::new("/tmp/x"));
    }

    #[test]
    fn a_missing_file_is_said_plainly() {
        let r = super::reveal(Path::new("/definitely/not/here/at-all.txt"));
        assert_eq!(r, Err("That file is no longer available.".into()));
    }
}
