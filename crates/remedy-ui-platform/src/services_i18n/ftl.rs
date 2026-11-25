//! Editing Fluent files one message at a time, for editors that let people rewrite an app's
//! messages: messages read as text, a message checked before it is written, and a file
//! changed by replacing, adding or removing one message while everything else in it (other
//! messages, terms, comments, even parts that do not parse) stays.

use fluent_syntax::ast::{Entry, Identifier, Message, Pattern, Resource};
use fluent_syntax::parser;
use fluent_syntax::serializer::{Options, serialize_with_options};

/// One message as people edit it: its value and its attributes (`.tooltip`), each as Fluent
/// pattern text (`Hello, { $name }!`, selectors and all).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageEntry {
    /// Its id (`share-folder`).
    pub id: String,
    /// Its value, if it has one (a message may have only attributes).
    pub value: Option<String>,
    /// Its attributes, by name, in order.
    pub attributes: Vec<(String, String)>,
}

/// Why a message could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FtlError(pub String);

impl std::fmt::Display for FtlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FtlError {}

/// A pattern as the text after `id = `, with continuation lines unindented.
fn pattern_text(p: &Pattern<&str>) -> String {
    let msg = Message {
        id: Identifier { name: "x" },
        value: Some(p.clone()),
        attributes: Vec::new(),
        comment: None,
    };
    let out = serialize_with_options(&Resource { body: vec![Entry::Message(msg)] }, Options::default());
    let rest = out.strip_prefix("x =").unwrap_or(&out);
    let rest = rest.strip_suffix('\n').unwrap_or(rest);
    match rest.strip_prefix(' ') {
        // On one line.
        Some(single) if !single.contains('\n') => single.to_owned(),
        _ => rest
            .trim_start_matches('\n')
            .split('\n')
            .map(|l| l.strip_prefix("    ").unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The entry for an AST message.
fn entry_of(m: &Message<&str>) -> MessageEntry {
    MessageEntry {
        id: m.id.name.to_owned(),
        value: m.value.as_ref().map(pattern_text),
        attributes: m.attributes.iter().map(|a| (a.id.name.to_owned(), pattern_text(&a.value))).collect(),
    }
}

/// Every message in `ftl`, in file order (parts that do not parse are left out).
pub fn read_entries(ftl: &str) -> Vec<MessageEntry> {
    let res = parser::parse(ftl).unwrap_or_else(|(res, _)| res);
    res.body
        .iter()
        .filter_map(|e| match e {
            Entry::Message(m) => Some(entry_of(m)),
            _ => None,
        })
        .collect()
}

/// `text` as a pattern after `= `: continuation lines indented, a multi-line text starting
/// on its own line.
fn indented(text: &str) -> String {
    if text.contains('\n') {
        let lines: Vec<String> = text.split('\n').map(|l| format!("    {l}")).collect();
        format!("\n{}", lines.join("\n"))
    } else {
        format!(" {text}")
    }
}

/// The Fluent source for `entry`, checked: it must read back as exactly that one message.
pub fn entry_source(entry: &MessageEntry) -> Result<String, FtlError> {
    if entry.value.is_none() && entry.attributes.is_empty() {
        return Err(FtlError(format!("{} needs a value or an attribute", entry.id)));
    }
    let mut src = format!("{} =", entry.id);
    if let Some(v) = &entry.value {
        if v.trim().is_empty() {
            return Err(FtlError(format!("{}: the text is empty", entry.id)));
        }
        src.push_str(&indented(v));
    }
    for (name, v) in &entry.attributes {
        if v.trim().is_empty() {
            return Err(FtlError(format!("{}.{name}: the text is empty", entry.id)));
        }
        let body = if v.contains('\n') {
            let lines: Vec<String> = v.split('\n').map(|l| format!("        {l}")).collect();
            format!("\n{}", lines.join("\n"))
        } else {
            format!(" {v}")
        };
        src.push_str(&format!("\n    .{name} ={body}"));
    }
    src.push('\n');
    let res = parser::parse(src.as_str()).map_err(|(_, errs)| {
        FtlError(format!(
            "{}: {}",
            entry.id,
            errs.first().map(|e| format!("{:?}", e.kind)).unwrap_or_default()
        ))
    })?;
    match res.body.as_slice() {
        [Entry::Message(m)] if m.id.name == entry.id && entry_of(m) == *entry => Ok(src),
        [Entry::Message(m)] if m.id.name == entry.id => {
            Err(FtlError(format!("{}: the text does not read back as written", entry.id)))
        }
        _ => Err(FtlError(format!("{} is not one message", entry.id))),
    }
}

/// `ftl` with `entry` in it: replacing the message with its id, or added at the end.
pub fn set_entry(ftl: &str, entry: &MessageEntry) -> Result<String, FtlError> {
    let src = entry_source(entry)?;
    let new =
        parser::parse(src.as_str()).map_err(|_| FtlError(format!("{} is not one message", entry.id)))?;
    let Some(new_entry) = new.body.into_iter().next() else {
        return Err(FtlError(format!("{} is not one message", entry.id)));
    };
    let mut res = parser::parse(ftl).unwrap_or_else(|(res, _)| res);
    let at = res.body.iter().position(|e| matches!(e, Entry::Message(m) if m.id.name == entry.id));
    match at {
        Some(i) => {
            if let Some(slot) = res.body.get_mut(i) {
                *slot = new_entry;
            }
        }
        None => res.body.push(new_entry),
    }
    Ok(serialize_with_options(&res, Options { with_junk: true }))
}

/// `ftl` without the message `id` (everything else as it was).
pub fn remove_entry(ftl: &str, id: &str) -> String {
    let mut res = parser::parse(ftl).unwrap_or_else(|(res, _)| res);
    res.body.retain(|e| !matches!(e, Entry::Message(m) if m.id.name == id));
    serialize_with_options(&res, Options { with_junk: true })
}

/// The messages of `resources`, the first (highest-priority) of each id winning.
pub(crate) fn merged<'a>(
    resources: impl Iterator<Item = &'a fluent_bundle::FluentResource>,
) -> Vec<MessageEntry> {
    let mut out: Vec<MessageEntry> = Vec::new();
    for r in resources {
        for e in r.entries() {
            if let Entry::Message(m) = e
                && !out.iter().any(|x| x.id == m.id.name)
            {
                out.push(entry_of(m));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, value: Option<&str>, attrs: &[(&str, &str)]) -> MessageEntry {
        MessageEntry {
            id: id.into(),
            value: value.map(str::to_owned),
            attributes: attrs.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect(),
        }
    }

    #[test]
    fn messages_read_and_write_one_at_a_time() {
        let file = "# Shown on the toolbar.\nshare = Share a folder\n    .tooltip = Share it\n-brand = Remedy\nhello = Hello, { $name }!\n";
        let got = read_entries(file);
        assert_eq!(got.first(), Some(&e("share", Some("Share a folder"), &[("tooltip", "Share it")])));
        assert_eq!(got.get(1), Some(&e("hello", Some("Hello, { $name }!"), &[])));

        let changed =
            set_entry(file, &e("hello", Some("Hi, { $name }!"), &[])).unwrap_or_else(|x| unreachable!("{x}"));
        assert!(changed.contains("hello = Hi, { $name }!"), "{changed}");
        assert!(
            changed.contains("-brand = Remedy") && changed.contains("# Shown on the toolbar."),
            "the rest stays"
        );
        let multi = e("about", Some("Two lines\nof text"), &[("title", "About")]);
        let added = set_entry(&changed, &multi).unwrap_or_else(|x| unreachable!("{x}"));
        assert!(read_entries(&added).contains(&multi), "{added}");
        let removed = remove_entry(&added, "hello");
        assert!(!read_entries(&removed).iter().any(|m| m.id == "hello"));
        assert!(read_entries(&removed).contains(&multi));
    }

    #[test]
    fn a_message_that_does_not_read_back_is_refused() {
        assert!(entry_source(&e("x", Some("{ $unclosed"), &[])).is_err());
        assert!(entry_source(&e("x", Some(""), &[])).is_err());
        assert!(entry_source(&e("x", None, &[])).is_err());
        assert!(entry_source(&e("bad id", Some("text"), &[])).is_err());
        assert!(entry_source(&e("x", Some("ok"), &[("label", "fine")])).is_ok());
    }
}
