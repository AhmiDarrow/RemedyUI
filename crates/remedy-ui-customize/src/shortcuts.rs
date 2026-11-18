//! The person's shortcuts: a file of command id → chords (in the portable text form,
//! `"Ctrl+Shift+Z"`), holding only the commands they rebound, applied to the app's
//! [`Commands`].

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use remedy_ui_input::{Chord, Clash};
use remedy_ui_view::Ui;
use remedy_ui_widgets_nav::Commands;
use serde_json::Value;

use crate::{Area, Customizations, CustomizeError, Notice, files};

/// The shortcuts file's name in the config folder.
pub(crate) const SHORTCUTS_FILE: &str = "shortcuts.json";

/// Reads a shortcuts file: an object of command id to a list of chords (or one chord).
fn parse(text: &str) -> Result<HashMap<String, Vec<Chord>>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("not a shortcuts file: {e}"))?;
    let Value::Object(map) = doc else {
        return Err("a shortcuts file is an object of command ids to shortcuts".into());
    };
    let mut out = HashMap::new();
    for (id, v) in map {
        let texts: Vec<&Value> = match &v {
            Value::Array(a) => a.iter().collect(),
            Value::String(_) => vec![&v],
            _ => return Err(format!("{id}: shortcuts are a list such as [\"Ctrl+S\"]")),
        };
        let mut chords = Vec::new();
        for t in texts {
            let Some(s) = t.as_str() else {
                return Err(format!("{id}: each shortcut is text such as \"Ctrl+S\""));
            };
            chords.push(s.parse::<Chord>().map_err(|e| format!("{id}: {e}"))?);
        }
        out.insert(id, chords);
    }
    Ok(out)
}

/// The file for a set of bindings: ids sorted, chords in their portable form.
fn file_text(bindings: &HashMap<String, Vec<Chord>>) -> Vec<u8> {
    let sorted: BTreeMap<&String, Vec<String>> =
        bindings.iter().map(|(id, c)| (id, c.iter().map(Chord::to_string).collect())).collect();
    files::json_text(&serde_json::to_value(sorted).unwrap_or(Value::Null))
}

impl Customizations {
    /// The shortcuts file.
    pub fn shortcuts_file(&self) -> PathBuf {
        self.0.dir.join(SHORTCUTS_FILE)
    }

    /// Turns on shortcut customization for `commands`: the person's file is read now and
    /// applied over the app's own bindings (`Commands::rebind`), which stay beneath it and are
    /// what a reset goes back to. A file that does not read is skipped with a notice; in one
    /// that gives a command a shortcut another command also answers to, only that shortcut
    /// is left out (the notice says which) and the rest applies. Bindings for commands
    /// registered later apply when they are.
    pub fn enable_shortcuts(&self, commands: &Commands) {
        *self.0.shortcuts.borrow_mut() = Some(commands.clone());
        self.reload(&[Area::Shortcuts]);
    }

    /// The commands whose shortcuts are customized.
    pub fn commands(&self) -> Option<Commands> {
        self.0.shortcuts.borrow().clone()
    }

    fn shortcut_commands(&self) -> Result<Commands, CustomizeError> {
        self.commands().ok_or(CustomizeError::NotEnabled(Area::Shortcuts))
    }

    /// Whether the person rebound `id` (tracked).
    pub fn is_rebound(&self, id: &str) -> bool {
        self.commands().is_some_and(|c| c.user_chords(id).is_some())
    }

    /// What giving `id` the shortcut `chord` would clash with (other commands in its scope
    /// that answer to it), without changing anything.
    pub fn shortcut_conflicts(&self, id: &str, chord: &Chord) -> Vec<Clash> {
        self.commands().map(|c| c.conflicts(id, std::slice::from_ref(chord))).unwrap_or_default()
    }

    /// Binds `id` to exactly `chords` (undoable). Refused, with the clashes, when another
    /// command in its scope answers to one of them.
    pub fn set_shortcuts(&self, ui: &Ui, id: &str, chords: Vec<Chord>) -> Result<(), CustomizeError> {
        let cmds = self.shortcut_commands()?;
        let label =
            cmds.info(id).map(|i| i.label).ok_or_else(|| CustomizeError::UnknownCommand(id.into()))?;
        let clashes = cmds.conflicts(id, &chords);
        if !clashes.is_empty() {
            return Err(CustomizeError::Clashes(clashes));
        }
        let mut next = cmds.bindings();
        put(&cmds, &mut next, id, chords);
        self.change(ui, format!("Change shortcuts for {label}"), None, &[Area::Shortcuts], |c| {
            c.write_shortcuts(&next)
        })
    }

    /// Adds `chord` to `id`'s shortcuts (undoable). When other commands in its scope answer
    /// to it, it is refused with the clashes, unless `take` is set: then those commands lose
    /// it, in the same change.
    pub fn add_shortcut(&self, ui: &Ui, id: &str, chord: Chord, take: bool) -> Result<(), CustomizeError> {
        let cmds = self.shortcut_commands()?;
        let info = cmds.info(id).ok_or_else(|| CustomizeError::UnknownCommand(id.into()))?;
        if info.chords.contains(&chord) {
            return Ok(());
        }
        let clashes = cmds.conflicts(id, std::slice::from_ref(&chord));
        if !clashes.is_empty() && !take {
            return Err(CustomizeError::Clashes(clashes));
        }
        let mut next = cmds.bindings();
        for other in clashes.iter().flat_map(|c| c.commands.iter()) {
            let kept: Vec<Chord> = cmds.info(other).map(|i| i.chords).unwrap_or_default();
            put(&cmds, &mut next, other, kept.into_iter().filter(|c| *c != chord).collect());
        }
        let mut mine = info.chords;
        mine.push(chord.clone());
        put(&cmds, &mut next, id, mine);
        let shown = chord.label(ui.platform());
        let label = if clashes.is_empty() {
            format!("Add {shown} to {}", info.label)
        } else {
            format!("Give {shown} to {}", info.label)
        };
        self.change(ui, label, None, &[Area::Shortcuts], |c| c.write_shortcuts(&next))
    }

    /// Takes `chord` off `id` (undoable).
    pub fn remove_shortcut(&self, ui: &Ui, id: &str, chord: &Chord) -> Result<(), CustomizeError> {
        let cmds = self.shortcut_commands()?;
        let info = cmds.info(id).ok_or_else(|| CustomizeError::UnknownCommand(id.into()))?;
        if !info.chords.contains(chord) {
            return Ok(());
        }
        let mut next = cmds.bindings();
        put(&cmds, &mut next, id, info.chords.into_iter().filter(|c| c != chord).collect());
        let label = format!("Remove {} from {}", chord.label(ui.platform()), info.label);
        self.change(ui, label, None, &[Area::Shortcuts], |c| c.write_shortcuts(&next))
    }

    /// Gives `id` its default shortcuts back (undoable). Refused, with the clashes, when the
    /// person has since given one of them to another command.
    pub fn reset_shortcut(&self, ui: &Ui, id: &str) -> Result<(), CustomizeError> {
        let cmds = self.shortcut_commands()?;
        let mut next = cmds.bindings();
        if next.remove(id).is_none() {
            return Ok(());
        }
        let clashes = cmds.conflicts(id, &cmds.base_chords(id));
        if !clashes.is_empty() {
            return Err(CustomizeError::Clashes(clashes));
        }
        let label = cmds.info(id).map_or_else(|| id.to_owned(), |i| i.label);
        self.change(ui, format!("Reset shortcuts for {label}"), None, &[Area::Shortcuts], |c| {
            c.write_shortcuts(&next)
        })
    }

    /// Gives every command its default shortcuts back (undoable).
    pub fn reset_shortcuts(&self, ui: &Ui) -> Result<(), CustomizeError> {
        self.shortcut_commands()?;
        let path = self.shortcuts_file();
        self.change(ui, "Reset all shortcuts", None, &[Area::Shortcuts], |_| {
            files::remove(&path).map_err(|e| CustomizeError::io(&path, &e))
        })
    }

    fn write_shortcuts(&self, bindings: &HashMap<String, Vec<Chord>>) -> Result<(), CustomizeError> {
        let path = self.shortcuts_file();
        let bytes = (!bindings.is_empty()).then(|| file_text(bindings));
        files::put(&path, bytes.as_deref()).map_err(|e| CustomizeError::io(&path, &e))
    }

    /// Reads the shortcuts file and applies it.
    pub(crate) fn reload_shortcuts(&self) {
        let Some(cmds) = self.commands() else { return };
        let path = self.shortcuts_file();
        let read = match files::read(&path) {
            Ok(None) => Ok(HashMap::new()),
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .map_err(|_| "the file is not UTF-8 text".to_owned())
                .and_then(|t| parse(&t)),
            Err(e) => Err(e.to_string()),
        };
        let mut notices = Vec::new();
        match read {
            Ok(mut bindings) => {
                cmds.set_bindings(bindings.clone());
                // A shortcut the file gives a command that another in its scope also answers
                // to is left out (of every command the file gives it); the rest of the file
                // applies.
                let clashes: Vec<Clash> = cmds
                    .clashes()
                    .into_iter()
                    .filter(|c| c.commands.iter().any(|id| bindings.contains_key(id)))
                    .collect();
                if !clashes.is_empty() {
                    let mut parts = Vec::new();
                    for c in &clashes {
                        for id in &c.commands {
                            if let Some(list) = bindings.get_mut(id) {
                                list.retain(|ch| *ch != c.chord);
                            }
                        }
                        let all = if c.commands.len() == 2 { "both" } else { "all" };
                        parts.push(format!("{} would {all} answer to {}", list(&c.commands), c.chord));
                    }
                    cmds.set_bindings(bindings);
                    notices.push(Notice {
                        area: Area::Shortcuts,
                        path,
                        message: format!("left out, the rest applies: {}", parts.join("; ")),
                    });
                }
            }
            Err(message) => {
                cmds.set_bindings(HashMap::new());
                notices.push(Notice { area: Area::Shortcuts, path, message });
            }
        }
        self.set_notices(Area::Shortcuts, None, notices);
    }
}

/// `a`, `a and b`, `a, b and c`.
fn list(ids: &[String]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Records `chords` as `id`'s binding, or drops the binding when they are its defaults.
fn put(cmds: &Commands, bindings: &mut HashMap<String, Vec<Chord>>, id: &str, chords: Vec<Chord>) {
    if chords == cmds.base_chords(id) {
        bindings.remove(id);
    } else {
        bindings.insert(id.to_owned(), chords);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_read_lists_single_chords_and_report_mistakes() {
        let Ok(b) = parse(r#"{ "file.save": ["Ctrl+S", "F2"], "edit.find": "Ctrl+F", "app.quit": [] }"#)
        else {
            unreachable!("a good file reads")
        };
        assert_eq!(b.get("file.save").map(Vec::len), Some(2));
        assert_eq!(b.get("edit.find").map(Vec::len), Some(1));
        assert_eq!(b.get("app.quit").map(Vec::len), Some(0), "no shortcut at all is a choice");
        assert!(parse(r#"{ "file.save": ["Hyper+S"] }"#).is_err_and(|e| e.contains("file.save")));
        assert!(parse(r#"{ "file.save": 5 }"#).is_err());
        assert!(parse("[]").is_err());
        assert_eq!(list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
