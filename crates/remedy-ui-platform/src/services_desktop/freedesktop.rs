//! The freedesktop side of URL schemes, launcher tasks and recent files: the app's own desktop
//! entry (`$XDG_DATA_HOME/applications/<id>.desktop`), the user's default apps
//! (`$XDG_CONFIG_HOME/mimeapps.list`) and the shared list of recent files
//! (`$XDG_DATA_HOME/recently-used.xbel`, which GTK and KDE file dialogs and docks read).
//!
//! Everything here edits files in place: an entry the app's package installed is copied to
//! the user's folder before it is changed, and other apps' lines are kept.

use std::path::{Path, PathBuf};

use crate::services_desktop::DesktopError;
use crate::services_desktop::autostart::{Entry, entry_string, exec_quote};
use crate::services_desktop::ini::{KeyFile, join, list};
use crate::services_desktop::jump::Task;

const ENTRY: &str = "Desktop Entry";
const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const ACTION: &str = "Desktop Action ";

/// Where the files are: the user's own, and the system's for entries a package installed.
#[derive(Clone, Debug)]
pub(crate) struct Xdg {
    /// `$XDG_DATA_HOME` (`~/.local/share`).
    pub(crate) data: PathBuf,
    /// `$XDG_DATA_DIRS` (`/usr/local/share`, `/usr/share`).
    pub(crate) system: Vec<PathBuf>,
    /// `$XDG_CONFIG_HOME` (`~/.config`).
    pub(crate) config: PathBuf,
}

impl Xdg {
    pub(crate) fn current() -> Result<Xdg, DesktopError> {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = var("HOME").ok_or_else(|| {
            DesktopError::unsupported("desktop entries", "the session has no home folder ($HOME)")
        })?;
        let system = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        Ok(Xdg {
            data: var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            system,
            config: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
        })
    }

    pub(crate) fn applications(&self) -> PathBuf {
        self.data.join("applications")
    }

    fn launcher(&self, app: &Entry) -> PathBuf {
        self.applications().join(desktop_id(app))
    }

    fn mimeapps(&self) -> PathBuf {
        self.config.join("mimeapps.list")
    }

    pub(crate) fn recent(&self) -> PathBuf {
        self.data.join("recently-used.xbel")
    }
}

/// The app's desktop file name (`com.ahmi.hearth.desktop`).
pub(crate) fn desktop_id(app: &Entry) -> String {
    format!("{}.desktop", app.id)
}

fn read(path: &Path) -> Result<Option<String>, DesktopError> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(DesktopError::io(path, &e)),
    }
}

fn write(path: &Path, text: &str) -> Result<(), DesktopError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| DesktopError::io(dir, &e))?;
    }
    // Written beside and renamed over, so a reader never sees half a file.
    let tmp = path.with_extension("remedy-tmp");
    std::fs::write(&tmp, text).map_err(|e| DesktopError::io(&tmp, &e))?;
    std::fs::rename(&tmp, path).map_err(|e| DesktopError::io(path, &e))
}

fn exec_line(program: &Path, args: &[String]) -> String {
    let mut exec = exec_quote(&program.to_string_lossy());
    for a in args {
        exec.push(' ');
        exec.push_str(&exec_quote(a));
    }
    exec
}

/// The app's launcher entry: the user's copy, else the one its package installed, else a new
/// one that runs the program.
fn launcher(x: &Xdg, app: &Entry) -> Result<KeyFile, DesktopError> {
    if let Some(t) = read(&x.launcher(app))? {
        return Ok(KeyFile::parse(&t));
    }
    for dir in &x.system {
        if let Some(t) = read(&dir.join("applications").join(desktop_id(app)))? {
            return Ok(KeyFile::parse(&t));
        }
    }
    let mut f = KeyFile::default();
    f.set(ENTRY, "Type", "Application");
    f.set(ENTRY, "Name", &entry_string(&app.name));
    f.set(ENTRY, "Exec", &entry_string(&exec_line(&app.program, &app.args)));
    f.set(ENTRY, "Terminal", "false");
    Ok(f)
}

/// Makes the launcher pass a URL on (`%u`) when it does not take files or URLs already.
fn takes_urls(f: &mut KeyFile) {
    let exec = f.get(ENTRY, "Exec").unwrap_or_default().to_owned();
    if !["%u", "%U", "%f", "%F"].iter().any(|c| exec.split_whitespace().any(|w| w == *c)) {
        f.set(ENTRY, "Exec", &format!("{exec} %u"));
    }
}

fn handler(scheme: &str) -> String {
    format!("x-scheme-handler/{}", scheme.to_ascii_lowercase())
}

/// Registers the app as the handler of `scheme`: the launcher entry says it handles the
/// scheme, and the user's defaults choose it.
pub(crate) fn register_scheme(x: &Xdg, app: &Entry, scheme: &str) -> Result<(), DesktopError> {
    let mime = handler(scheme);
    let mut f = launcher(x, app)?;
    let mut types = list(f.get(ENTRY, "MimeType"));
    if !types.contains(&mime) {
        types.push(mime.clone());
    }
    f.set(ENTRY, "MimeType", &join(&types));
    takes_urls(&mut f);
    write(&x.launcher(app), &f.text())?;

    let id = desktop_id(app);
    let mut m = KeyFile::parse(&read(&x.mimeapps())?.unwrap_or_default());
    m.set(DEFAULTS, &mime, &format!("{id};"));
    let mut added = list(m.get(ADDED, &mime));
    added.retain(|a| *a != id);
    added.insert(0, id);
    m.set(ADDED, &mime, &join(&added));
    write(&x.mimeapps(), &m.text())
}

/// Takes the app off `scheme`: out of its launcher entry, and out of the user's defaults
/// where it is the one chosen (another app chosen since is left alone).
pub(crate) fn unregister_scheme(x: &Xdg, app: &Entry, scheme: &str) -> Result<(), DesktopError> {
    let mime = handler(scheme);
    let id = desktop_id(app);
    if let Some(t) = read(&x.launcher(app))? {
        let mut f = KeyFile::parse(&t);
        let mut types = list(f.get(ENTRY, "MimeType"));
        types.retain(|t| *t != mime);
        if types.is_empty() {
            f.remove(ENTRY, "MimeType");
        } else {
            f.set(ENTRY, "MimeType", &join(&types));
        }
        write(&x.launcher(app), &f.text())?;
    }
    if let Some(t) = read(&x.mimeapps())? {
        let mut m = KeyFile::parse(&t);
        for group in [DEFAULTS, ADDED] {
            let mut apps = list(m.get(group, &mime));
            apps.retain(|a| *a != id);
            if apps.is_empty() {
                m.remove(group, &mime);
            } else {
                m.set(group, &mime, &join(&apps));
            }
        }
        write(&x.mimeapps(), &m.text())?;
    }
    Ok(())
}

/// True when the user's defaults open `scheme` with the app.
pub(crate) fn scheme_registered(x: &Xdg, app: &Entry, scheme: &str) -> bool {
    let Ok(Some(t)) = read(&x.mimeapps()) else { return false };
    let m = KeyFile::parse(&t);
    list(m.get(DEFAULTS, &handler(scheme))).first().is_some_and(|a| *a == desktop_id(app))
}

/// Replaces the launcher's actions (what a dock's or a menu's right-click offers) with
/// `tasks`.
pub(crate) fn set_tasks(x: &Xdg, app: &Entry, tasks: &[Task]) -> Result<(), DesktopError> {
    let mut f = launcher(x, app)?;
    let old: Vec<String> = f.groups().filter(|g| g.starts_with(ACTION)).map(str::to_owned).collect();
    for g in old {
        f.remove_group(&g);
    }
    let mut ids = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let id = format!("task-{}", i + 1);
        let group = format!("{ACTION}{id}");
        f.set(&group, "Name", &entry_string(&t.label));
        f.set(&group, "Exec", &entry_string(&exec_line(&app.program, &t.args)));
        ids.push(id);
    }
    if ids.is_empty() {
        f.remove(ENTRY, "Actions");
    } else {
        f.set(ENTRY, "Actions", &join(&ids));
    }
    write(&x.launcher(app), &f.text())
}

/// The launcher's actions as (name, `Exec` line), in order.
#[cfg(test)]
pub(crate) fn tasks(x: &Xdg, app: &Entry) -> Vec<(String, String)> {
    let Ok(Some(t)) = read(&x.launcher(app)) else { return Vec::new() };
    let f = KeyFile::parse(&t);
    list(f.get(ENTRY, "Actions"))
        .iter()
        .filter_map(|id| {
            let g = format!("{ACTION}{id}");
            Some((f.get(&g, "Name")?.to_owned(), f.get(&g, "Exec")?.to_owned()))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Recent files: the XBEL bookmark file GLib's GBookmarkFile and KDE's KRecentDocument share.

const XBEL_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n";
const XBEL_TAIL: &str = "</xbel>\n";

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The bookmarks in an XBEL file, each the whole `<bookmark …>…</bookmark>` element, with what
/// comes before the first and after the last.
fn bookmarks(text: &str) -> (String, Vec<String>, String) {
    let mut out = Vec::new();
    let mut rest = text;
    let mut head = None;
    while let Some(start) = rest.find("<bookmark ") {
        if head.is_none() {
            head = Some(text[..text.len() - rest.len() + start].to_owned());
        }
        let Some(len) = rest[start..].find("</bookmark>") else { break };
        let end = start + len + "</bookmark>".len();
        out.push(rest[start..end].to_owned());
        rest = &rest[end..];
    }
    match head {
        Some(h) => (h, out, rest.trim_start_matches([' ', '\n']).to_owned()),
        None => match text.find("</xbel>") {
            Some(i) => (text[..i].to_owned(), out, text[i..].to_owned()),
            None => (XBEL_HEAD.to_owned(), out, XBEL_TAIL.to_owned()),
        },
    }
}

fn href(bookmark: &str) -> Option<&str> {
    let i = bookmark.find("href=\"")? + 6;
    let len = bookmark[i..].find('"')?;
    Some(&bookmark[i..i + len])
}

/// The `<bookmark:application …/>` elements of a bookmark.
fn applications(bookmark: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = bookmark;
    while let Some(i) = rest.find("<bookmark:application ") {
        let Some(len) = rest[i..].find("/>") else { break };
        out.push(rest[i..i + len + 2].to_owned());
        rest = &rest[i + len + 2..];
    }
    out
}

fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let i = element.find(&key)? + key.len();
    let len = element[i..].find('"')?;
    Some(&element[i..i + len])
}

/// A broad media type from the file's extension, which is what GTK's list shows icons by.
fn mime_type(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/x-wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "zip" => "application/zip",
        _ if path.is_dir() => "inode/directory",
        _ => "application/octet-stream",
    }
}

/// An ISO 8601 UTC time with microseconds, as GLib writes them.
pub(crate) fn iso8601(t: std::time::SystemTime) -> String {
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_micros()
    )
}

fn application(app: &Entry, now: &str, count: u32) -> String {
    let exec = format!("'{} %u'", exec_line(&app.program, &app.args));
    format!(
        "<bookmark:application name=\"{}\" exec=\"{}\" modified=\"{now}\" count=\"{count}\"/>",
        xml(&app.name),
        xml(&exec)
    )
}

fn bookmark(uri: &str, mime: &str, added: &str, now: &str, apps: &[String]) -> String {
    let mut s = format!(
        "  <bookmark href=\"{}\" added=\"{added}\" modified=\"{now}\" visited=\"{now}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{mime}\"/>\n        <bookmark:applications>\n",
        xml(uri)
    );
    for a in apps {
        s.push_str("          ");
        s.push_str(a);
        s.push('\n');
    }
    s.push_str("        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>");
    s
}

fn write_xbel(path: &Path, head: &str, marks: &[String], tail: &str) -> Result<(), DesktopError> {
    // The head ends with the first bookmark's indentation; every bookmark is indented alike.
    let mut text = head.trim_end_matches(' ').to_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    for m in marks {
        if !m.starts_with(' ') {
            text.push_str("  ");
        }
        text.push_str(m);
        text.push('\n');
    }
    text.push_str(tail);
    write(path, &text)
}

/// Adds `file` to the recent files as opened by the app now (moving it to the front of lists
/// sorted by time, and counting the use).
pub(crate) fn add_recent(
    x: &Xdg,
    app: &Entry,
    file: &Path,
    now: std::time::SystemTime,
) -> Result<(), DesktopError> {
    let path = x.recent();
    let text = read(&path)?.unwrap_or_default();
    let (head, mut marks, tail) = bookmarks(&text);
    let uri = crate::services_desktop::file_uri(file);
    let now = iso8601(now);
    let name = format!("name=\"{}\"", xml(&app.name));
    let (added, mut apps, count) = match marks.iter().position(|m| href(m) == Some(xml(&uri).as_str())) {
        Some(i) => {
            let old = marks.remove(i);
            let apps = applications(&old);
            let count = apps
                .iter()
                .find(|a| a.contains(&name))
                .and_then(|a| attr(a, "count")?.parse::<u32>().ok())
                .unwrap_or(0);
            (attr(&old, "added").unwrap_or(&now).to_owned(), apps, count)
        }
        None => (now.clone(), Vec::new(), 0),
    };
    apps.retain(|a| !a.contains(&name));
    apps.push(application(app, &now, count.saturating_add(1)));
    marks.push(bookmark(&uri, mime_type(file), &added, &now, &apps));
    write_xbel(&path, &head, &marks, &tail)
}

/// Takes the app's uses out of the recent files; files no other app opened leave the list.
pub(crate) fn clear_recent(x: &Xdg, app: &Entry) -> Result<(), DesktopError> {
    let path = x.recent();
    let Some(text) = read(&path)? else { return Ok(()) };
    let (head, marks, tail) = bookmarks(&text);
    let name = format!("name=\"{}\"", xml(&app.name));
    let kept: Vec<String> = marks
        .into_iter()
        .filter_map(|m| {
            let apps = applications(&m);
            if !apps.iter().any(|a| a.contains(&name)) {
                return Some(m);
            }
            let others: Vec<&String> = apps.iter().filter(|a| !a.contains(&name)).collect();
            if others.is_empty() {
                return None;
            }
            let mut out = m.clone();
            for a in apps.iter().filter(|a| a.contains(&name)) {
                out = out.replacen(a.as_str(), "", 1);
            }
            // The application's own line is left empty: drop it.
            Some(out.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n"))
        })
        .collect();
    write_xbel(&path, &head, &kept, &tail)
}

/// The recent files the app opened, as URIs, oldest first.
#[cfg(test)]
pub(crate) fn recent(x: &Xdg, app: &Entry) -> Vec<String> {
    let text = read(&x.recent()).ok().flatten().unwrap_or_default();
    let name = format!("name=\"{}\"", xml(&app.name));
    bookmarks(&text)
        .1
        .iter()
        .filter(|m| applications(m).iter().any(|a| a.contains(&name)))
        .filter_map(|m| href(m).map(|h| h.replace("&amp;", "&")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("remedy-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn places(root: &Path) -> Xdg {
        Xdg { data: root.join("data"), system: vec![root.join("system")], config: root.join("config") }
    }

    fn app() -> Entry {
        Entry {
            name: "Hearth".into(),
            id: "com.example.hearth".into(),
            program: "/opt/Hearth/hearth".into(),
            args: vec!["--quiet".into()],
            run_key: false,
        }
    }

    #[test]
    fn a_scheme_names_the_app_in_its_entry_and_the_defaults() {
        let root = temp("scheme");
        let x = places(&root);
        // Another app's choices are already there.
        write(&x.mimeapps(), "[Default Applications]\ntext/html=firefox.desktop;\n").ok();
        register_scheme(&x, &app(), "Hearth").ok();
        let entry = std::fs::read_to_string(x.launcher(&app())).unwrap_or_default();
        assert!(entry.contains("MimeType=x-scheme-handler/hearth;\n"), "{entry}");
        assert!(entry.contains("Exec=/opt/Hearth/hearth --quiet %u\n"), "the URL is passed on: {entry}");
        let m = std::fs::read_to_string(x.mimeapps()).unwrap_or_default();
        assert!(m.contains("text/html=firefox.desktop;"), "{m}");
        assert!(m.contains("x-scheme-handler/hearth=com.example.hearth.desktop;"), "{m}");
        assert!(scheme_registered(&x, &app(), "hearth"));
        // Registering twice changes nothing.
        register_scheme(&x, &app(), "hearth").ok();
        assert_eq!(std::fs::read_to_string(x.launcher(&app())).unwrap_or_default(), entry);

        unregister_scheme(&x, &app(), "hearth").ok();
        assert!(!scheme_registered(&x, &app(), "hearth"));
        let m = std::fs::read_to_string(x.mimeapps()).unwrap_or_default();
        assert!(!m.contains("hearth"), "{m}");
        assert!(m.contains("text/html=firefox.desktop;"));
        let entry = std::fs::read_to_string(x.launcher(&app())).unwrap_or_default();
        assert!(!entry.contains("MimeType"), "{entry}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_packaged_entry_is_copied_before_it_changes() {
        let root = temp("packaged");
        let x = places(&root);
        let packaged = "[Desktop Entry]\nType=Application\nName=Hearth\nName[fr]=Foyer\nIcon=hearth\nExec=hearth %F\nMimeType=text/plain;\n";
        write(&root.join("system/applications/com.example.hearth.desktop"), packaged).ok();
        register_scheme(&x, &app(), "hearth").ok();
        let entry = std::fs::read_to_string(x.launcher(&app())).unwrap_or_default();
        assert!(
            entry.contains("Name[fr]=Foyer\nIcon=hearth\nExec=hearth %F\n"),
            "kept as installed: {entry}"
        );
        assert!(entry.contains("MimeType=text/plain;x-scheme-handler/hearth;"), "{entry}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tasks_become_desktop_actions() {
        let root = temp("tasks");
        let x = places(&root);
        register_scheme(&x, &app(), "hearth").ok();
        // A label or an argument with a line break or a backslash stays one key.
        let odd = [Task::new("Two\nlines", &["a\\b \"c\""])];
        set_tasks(&x, &app(), &odd).ok();
        assert_eq!(
            super::tasks(&x, &app()),
            vec![("Two\\nlines".to_owned(), "/opt/Hearth/hearth \"a\\\\\\\\b \\\\\"c\\\\\"\"".to_owned())]
        );
        let tasks = [Task::new("New note", &["--new"]), Task::new("Open \"Inbox\"", &["--open", "my inbox"])];
        set_tasks(&x, &app(), &tasks).ok();
        assert_eq!(
            super::tasks(&x, &app()),
            vec![
                ("New note".to_owned(), "/opt/Hearth/hearth --new".to_owned()),
                ("Open \"Inbox\"".to_owned(), "/opt/Hearth/hearth --open \"my inbox\"".to_owned()),
            ]
        );
        let entry = std::fs::read_to_string(x.launcher(&app())).unwrap_or_default();
        assert!(entry.contains("Actions=task-1;task-2;"), "{entry}");
        assert!(entry.contains("x-scheme-handler/hearth"), "the scheme stays: {entry}");
        set_tasks(&x, &app(), &tasks[..1]).ok();
        assert_eq!(super::tasks(&x, &app()).len(), 1, "a shorter list replaces the longer one");
        set_tasks(&x, &app(), &[]).ok();
        let entry = std::fs::read_to_string(x.launcher(&app())).unwrap_or_default();
        assert!(!entry.contains("Actions") && !entry.contains("Desktop Action"), "{entry}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recent_files_join_the_shared_list_and_leave_others_alone() {
        let root = temp("recent");
        let x = places(&root);
        let other = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n  <bookmark href=\"file:///home/sam/shared.txt\" added=\"2026-01-01T00:00:00.000000Z\" modified=\"2026-01-01T00:00:00.000000Z\" visited=\"2026-01-01T00:00:00.000000Z\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"text/plain\"/>\n        <bookmark:applications>\n          <bookmark:application name=\"gedit\" exec=\"&apos;gedit %u&apos;\" modified=\"2026-01-01T00:00:00.000000Z\" count=\"3\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n</xbel>\n";
        write(&x.recent(), other).ok();
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
        add_recent(&x, &app(), Path::new("/home/sam/notes & plans.md"), t).ok();
        add_recent(&x, &app(), Path::new("/home/sam/shared.txt"), t).ok();
        add_recent(&x, &app(), Path::new("/home/sam/shared.txt"), t).ok();
        let text = std::fs::read_to_string(x.recent()).unwrap_or_default();
        assert_eq!(
            recent(&x, &app()),
            vec![
                "file:///home/sam/notes%20%26%20plans.md".to_owned(),
                "file:///home/sam/shared.txt".to_owned()
            ],
            "{text}"
        );
        assert!(text.contains("name=\"gedit\""), "gedit's use is kept: {text}");
        assert!(text.contains("count=\"2\""), "the second use is counted: {text}");
        assert!(text.contains("added=\"2026-01-01T00:00:00.000000Z\""), "first added stays: {text}");
        assert!(text.contains("<mime:mime-type type=\"text/markdown\"/>"), "{text}");
        assert!(text.ends_with("</xbel>\n"), "{text}");
        assert!(text.lines().all(|l| l.is_empty() || !l.trim().is_empty()), "no blank-looking lines: {text}");
        assert_eq!(text.matches("\n  <bookmark ").count(), 2, "{text}");

        clear_recent(&x, &app()).ok();
        let text = std::fs::read_to_string(x.recent()).unwrap_or_default();
        assert!(recent(&x, &app()).is_empty(), "{text}");
        assert!(
            text.contains("file:///home/sam/shared.txt") && text.contains("gedit"),
            "gedit's file stays: {text}"
        );
        assert!(!text.contains("notes"), "only this app's file leaves: {text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn times_are_written_as_glib_does() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_micros(1_000_000_000_123_456);
        assert_eq!(iso8601(t), "2001-09-09T01:46:40.123456Z");
        assert_eq!(iso8601(std::time::UNIX_EPOCH), "1970-01-01T00:00:00.000000Z");
    }
}
