//! Commands and their shortcuts.

use std::collections::HashMap;

use crate::events::{Key, KeyEvent, Modifiers, Platform};

/// A key and modifiers that trigger a command.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Modifiers that must be held (exactly).
    pub modifiers: Modifiers,
    /// The key.
    pub key: Key,
}

impl Chord {
    /// A chord.
    pub fn new(modifiers: Modifiers, key: Key) -> Self {
        Chord { modifiers, key }
    }

    /// The primary modifier (Ctrl, or Command on Apple platforms) plus `key`.
    pub fn primary(platform: Platform, key: Key) -> Self {
        Chord { modifiers: Modifiers::primary(platform), key }
    }

    /// How to show it in a menu or tooltip on `platform`, such as "Ctrl+Shift+Z" or "⇧⌘Z".
    pub fn label(&self, platform: Platform) -> String {
        let key = match &self.key {
            Key::Character(s) => s.to_uppercase(),
            Key::F(n) => format!("F{n}"),
            Key::Named(n) => n.clone(),
            other => format!("{other:?}"),
        };
        let m = self.modifiers;
        match platform {
            Platform::Apple => {
                let mut s = String::new();
                if m.ctrl {
                    s.push('⌃');
                }
                if m.alt {
                    s.push('⌥');
                }
                if m.shift {
                    s.push('⇧');
                }
                if m.meta {
                    s.push('⌘');
                }
                s + &key
            }
            Platform::Standard => {
                let mut parts = Vec::new();
                if m.ctrl {
                    parts.push("Ctrl");
                }
                if m.alt {
                    parts.push("Alt");
                }
                if m.shift {
                    parts.push("Shift");
                }
                if m.meta {
                    parts.push("Win");
                }
                parts.push(&key);
                parts.join("+")
            }
        }
    }
}

/// Where a command is registered. Inner scopes win over outer ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// The whole app.
    App,
    /// One window.
    Window,
    /// One screen (a page in a navigation stack).
    Screen,
    /// One element (a text field, a list) while it has focus.
    Element,
}

/// A command: an action with a label, default shortcuts and an enabled state. Menus,
/// toolbars, tooltips, the command palette, accessibility and agents all read the same table.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// A stable id, such as `"message.reply"`.
    pub id: String,
    /// What menus and the palette show.
    pub label: String,
    /// Default shortcuts.
    pub default_chords: Vec<Chord>,
    /// Whether it can run now.
    pub enabled: bool,
}

/// A conflict found when the user rebinds a shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The command that already uses the chord in that scope.
    pub existing: String,
}

/// The command table with the user's rebindings.
#[derive(Clone, Debug, Default)]
pub struct CommandTable {
    commands: Vec<(Scope, Command)>,
    /// The user's chords per command id, replacing the defaults when present.
    user: HashMap<String, Vec<Chord>>,
}

impl CommandTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a command at a scope.
    pub fn register(&mut self, scope: Scope, command: Command) {
        self.commands.retain(|(s, c)| !(*s == scope && c.id == command.id));
        self.commands.push((scope, command));
    }

    /// Enables or disables a command.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        for (_, c) in self.commands.iter_mut().filter(|(_, c)| c.id == id) {
            c.enabled = enabled;
        }
    }

    /// The chords a command answers to: the user's, or its defaults.
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        if let Some(c) = self.user.get(id) {
            return c.clone();
        }
        self.commands
            .iter()
            .find(|(_, c)| c.id == id)
            .map(|(_, c)| c.default_chords.clone())
            .unwrap_or_default()
    }

    /// The user rebinds `id` to `chords`. A chord another command in the same scope already
    /// uses is reported, and nothing changes.
    pub fn rebind(&mut self, id: &str, chords: Vec<Chord>) -> Result<(), Conflict> {
        let scope = self.commands.iter().find(|(_, c)| c.id == id).map(|(s, _)| *s);
        for (s, c) in &self.commands {
            if c.id == id || Some(*s) != scope {
                continue;
            }
            let theirs = self.chords(&c.id);
            if chords.iter().any(|ch| theirs.contains(ch)) {
                return Err(Conflict { existing: c.id.clone() });
            }
        }
        self.user.insert(id.to_string(), chords);
        Ok(())
    }

    /// Restores a command's default shortcuts.
    pub fn reset(&mut self, id: &str) {
        self.user.remove(id);
    }

    /// The command a key press runs, among the scopes active now (innermost first). Disabled
    /// commands are skipped so an outer one can answer; key releases, auto-repeats of
    /// commands, and keys an input method is composing never match.
    pub fn match_key(&self, e: &KeyEvent, active: &[Scope]) -> Option<&Command> {
        if !e.pressed || e.composing {
            return None;
        }
        let chord = Chord { modifiers: e.modifiers, key: e.key.clone() };
        let mut scopes = active.to_vec();
        scopes.sort_by(|a, b| b.cmp(a));
        for scope in scopes {
            let found = self
                .commands
                .iter()
                .rev()
                .filter(|(s, c)| *s == scope && c.enabled)
                .find(|(_, c)| self.chords(&c.id).contains(&chord));
            if let Some((_, c)) = found {
                return Some(c);
            }
        }
        None
    }

    /// Every command with its scope, enabled or not (for editors and toolbars that show
    /// disabled commands dimmed).
    pub fn palette_all(&self) -> Vec<(Scope, &Command)> {
        self.commands.iter().map(|(s, c)| (*s, c)).collect()
    }

    /// Every enabled command, for a command palette.
    pub fn palette(&self) -> Vec<&Command> {
        self.commands.iter().map(|(_, c)| c).filter(|c| c.enabled).collect()
    }
}
