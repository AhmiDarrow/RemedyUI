//! Freedesktop key files (desktop entries, `mimeapps.list`): groups of `Key=Value` lines,
//! edited in place so whatever else a file holds (translations, other apps' defaults,
//! comments) stays as it was.

/// A key file: the lines before the first group, then each `[Group]` and its lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyFile {
    preamble: Vec<String>,
    groups: Vec<(String, Vec<String>)>,
}

impl KeyFile {
    pub(crate) fn parse(text: &str) -> KeyFile {
        let mut f = KeyFile::default();
        for line in text.lines() {
            let t = line.trim();
            if let Some(name) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                f.groups.push((name.to_owned(), Vec::new()));
            } else if let Some((_, lines)) = f.groups.last_mut() {
                lines.push(line.to_owned());
            } else {
                f.preamble.push(line.to_owned());
            }
        }
        // Blank lines between groups are written back by `text`.
        for (_, lines) in &mut f.groups {
            while lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
        }
        f
    }

    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        for l in &self.preamble {
            out.push_str(l);
            out.push('\n');
        }
        for (i, (name, lines)) in self.groups.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push('[');
            out.push_str(name);
            out.push_str("]\n");
            for l in lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        out
    }

    pub(crate) fn groups(&self) -> impl Iterator<Item = &str> {
        self.groups.iter().map(|(n, _)| n.as_str())
    }

    pub(crate) fn get(&self, group: &str, key: &str) -> Option<&str> {
        let (_, lines) = self.groups.iter().find(|(n, _)| n == group)?;
        lines.iter().find_map(|l| value_of(l, key))
    }

    /// Sets `key` (replacing it where it is, else appending), creating the group if needed.
    pub(crate) fn set(&mut self, group: &str, key: &str, value: &str) {
        let line = format!("{key}={value}");
        if !self.groups.iter().any(|(n, _)| n == group) {
            self.groups.push((group.to_owned(), Vec::new()));
        }
        let Some((_, lines)) = self.groups.iter_mut().find(|(n, _)| n == group) else { return };
        match lines.iter_mut().find(|l| value_of(l, key).is_some()) {
            Some(l) => *l = line,
            None => lines.push(line),
        }
    }

    pub(crate) fn remove(&mut self, group: &str, key: &str) {
        if let Some((_, lines)) = self.groups.iter_mut().find(|(n, _)| n == group) {
            lines.retain(|l| value_of(l, key).is_none());
        }
    }

    pub(crate) fn remove_group(&mut self, group: &str) {
        self.groups.retain(|(n, _)| n != group);
    }
}

/// The value of `line` when it sets exactly `key` (not `key[de]`, not `keyboard`).
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(key)?;
    let rest = rest.trim_start();
    rest.strip_prefix('=').map(str::trim)
}

/// A `;`-separated list value's entries.
pub(crate) fn list(value: Option<&str>) -> Vec<String> {
    value.unwrap_or_default().split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// A list value from entries (with the trailing `;` the specification asks for).
pub(crate) fn join(items: &[String]) -> String {
    items.iter().map(|i| format!("{i};")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_keep_what_they_do_not_touch() {
        let src = "# made by hand\n[Desktop Entry]\nName=Hearth\nName[de]=Herd\nMimeType=text/plain;\n\n[Desktop Action new]\nName=New\nExec=hearth --new\n";
        let mut f = KeyFile::parse(src);
        assert_eq!(f.get("Desktop Entry", "Name"), Some("Hearth"), "Name[de] is not Name");
        assert_eq!(list(f.get("Desktop Entry", "MimeType")), vec!["text/plain"]);
        f.set("Desktop Entry", "MimeType", "text/plain;x-scheme-handler/hearth;");
        f.set("Default Applications", "x-scheme-handler/hearth", "hearth.desktop");
        let out = f.text();
        assert!(out.starts_with("# made by hand\n[Desktop Entry]\nName=Hearth\nName[de]=Herd\n"), "{out}");
        assert!(out.contains("\n\n[Desktop Action new]\nName=New\n"), "{out}");
        assert!(out.ends_with("[Default Applications]\nx-scheme-handler/hearth=hearth.desktop\n"), "{out}");
        assert_eq!(KeyFile::parse(&out), f, "it reads back the same");
        f.remove_group("Desktop Action new");
        f.remove("Desktop Entry", "Name");
        assert!(!f.groups().any(|g| g == "Desktop Action new"));
        assert_eq!(f.get("Desktop Entry", "Name[de]"), Some("Herd"));
        assert_eq!(f.get("Desktop Entry", "Name"), None);
    }
}
