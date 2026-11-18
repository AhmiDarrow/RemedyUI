//! A person's shortcut bindings as text, to save, share or carry to another machine: a JSON
//! object of command id to a list of chords in their portable form (`"Ctrl+Shift+Z"`). The
//! customization layer keeps its shortcuts file in this form too.

use std::collections::{BTreeMap, HashMap};

use remedy_ui_input::Chord;
use serde_json::Value;

/// Why text is not a set of bindings: what was wrong, naming the command where there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingsError {
    reason: String,
}

impl BindingsError {
    fn new(reason: impl Into<String>) -> Self {
        BindingsError { reason: reason.into() }
    }

    /// What was wrong.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for BindingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for BindingsError {}

/// Bindings as text: command ids in order, chords in their portable form.
///
/// ```
/// use std::collections::HashMap;
///
/// use remedy_ui_input::Chord;
/// use remedy_ui_widgets_nav::{bindings_from_json, bindings_to_json};
///
/// let mut b = HashMap::new();
/// b.insert("file.save".to_owned(), vec!["Ctrl+S".parse::<Chord>()?]);
/// let text = bindings_to_json(&b);
/// assert_eq!(bindings_from_json(&text).ok(), Some(b));
/// # Ok::<(), remedy_ui_input::ChordError>(())
/// ```
pub fn bindings_to_json(bindings: &HashMap<String, Vec<Chord>>) -> String {
    let sorted: BTreeMap<&String, Vec<String>> =
        bindings.iter().map(|(id, c)| (id, c.iter().map(Chord::to_string).collect())).collect();
    let map: serde_json::Map<String, Value> = sorted
        .into_iter()
        .map(|(id, chords)| (id.clone(), Value::Array(chords.into_iter().map(Value::String).collect())))
        .collect();
    let mut s = serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

/// Reads bindings written by [`bindings_to_json`] (or by hand): an object of command id to a
/// list of chords, or to one chord.
pub fn bindings_from_json(text: &str) -> Result<HashMap<String, Vec<Chord>>, BindingsError> {
    let doc: Value =
        serde_json::from_str(text).map_err(|e| BindingsError::new(format!("not a shortcuts file: {e}")))?;
    let Value::Object(map) = doc else {
        return Err(BindingsError::new("a shortcuts file is an object of command ids to shortcuts"));
    };
    let mut out = HashMap::new();
    for (id, v) in map {
        let texts: Vec<&Value> = match &v {
            Value::Array(a) => a.iter().collect(),
            Value::String(_) => vec![&v],
            _ => return Err(BindingsError::new(format!("{id}: shortcuts are a list such as [\"Ctrl+S\"]"))),
        };
        let mut chords = Vec::new();
        for t in texts {
            let Some(s) = t.as_str() else {
                return Err(BindingsError::new(format!("{id}: each shortcut is text such as \"Ctrl+S\"")));
            };
            chords.push(s.parse::<Chord>().map_err(|e| BindingsError::new(format!("{id}: {e}")))?);
        }
        out.insert(id, chords);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_read_back_and_bad_files_say_why() {
        let Ok(b) =
            bindings_from_json(r#"{ "file.save": ["Ctrl+S", "F2"], "edit.find": "Ctrl+F", "app.quit": [] }"#)
        else {
            unreachable!("a good file reads")
        };
        assert_eq!(b.get("file.save").map(Vec::len), Some(2));
        assert_eq!(b.get("app.quit").map(Vec::len), Some(0));
        assert_eq!(bindings_from_json(&bindings_to_json(&b)).ok(), Some(b));
        let Err(e) = bindings_from_json(r#"{ "file.save": ["Hyper+S"] }"#) else {
            unreachable!("Hyper is no key")
        };
        assert!(e.reason().contains("file.save"), "{e}");
        assert!(bindings_from_json(r#"{ "file.save": 5 }"#).is_err());
        assert!(bindings_from_json("[]").is_err());
    }

    #[test]
    fn rebindings_export_and_import_between_apps() {
        use remedy_ui_input::{Key, Modifiers};
        let ctrl = |c: char| Chord::new(Modifiers::CTRL, Key::char(c));
        let a = crate::Commands::new();
        a.add("file.save", "Save", vec![ctrl('s')], None, || {});
        // The person moved Save to Ctrl+W.
        a.set_bindings(HashMap::from([("file.save".to_owned(), vec![ctrl('w')])]));
        let text = a.export_bindings();
        assert!(text.contains("file.save"), "{text}");
        let b = crate::Commands::new();
        b.add("file.save", "Save", vec![ctrl('s')], None, || {});
        let Ok(()) = b.import_bindings(&text) else { unreachable!("the export reads back") };
        assert_eq!(b.user_chords("file.save"), Some(vec![ctrl('w')]));
        assert!(b.import_bindings("not json").is_err());
        assert_eq!(b.user_chords("file.save"), Some(vec![ctrl('w')]), "a bad file changes nothing");
    }
}
