//! Commands and their shortcuts.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

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

/// The portable name of a key in a chord's text form, or `None` for a character key.
fn key_name(key: &Key) -> Option<String> {
    Some(match key {
        Key::Character(_) => return None,
        Key::Enter => "Enter".into(),
        Key::Tab => "Tab".into(),
        Key::Escape => "Escape".into(),
        Key::Backspace => "Backspace".into(),
        Key::Delete => "Delete".into(),
        Key::Space => "Space".into(),
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::F(n) => format!("F{n}"),
        Key::Back => "Back".into(),
        Key::Named(n) => n.clone(),
    })
}

/// The key a name in a chord's text form stands for (names are matched without regard to
/// case; a single character is that character key; anything else is a named key).
fn key_from_name(name: &str) -> Key {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "enter" | "return" => return Key::Enter,
        "tab" => return Key::Tab,
        "escape" | "esc" => return Key::Escape,
        "backspace" => return Key::Backspace,
        "delete" | "del" => return Key::Delete,
        "space" => return Key::Space,
        "left" | "arrowleft" => return Key::ArrowLeft,
        "right" | "arrowright" => return Key::ArrowRight,
        "up" | "arrowup" => return Key::ArrowUp,
        "down" | "arrowdown" => return Key::ArrowDown,
        "home" => return Key::Home,
        "end" => return Key::End,
        "pageup" => return Key::PageUp,
        "pagedown" => return Key::PageDown,
        "back" => return Key::Back,
        "plus" => return Key::Character("+".into()),
        _ => {}
    }
    if let Some(n) =
        lower.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()).filter(|n| (1..=24).contains(n))
    {
        return Key::F(n);
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Key::char(c),
        _ => Key::Named(name.to_owned()),
    }
}

/// Why a chord's text did not read as a chord.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChordError {
    /// The text.
    pub text: String,
    /// What was wrong.
    pub reason: String,
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a shortcut: {}", self.text, self.reason)
    }
}

impl std::error::Error for ChordError {}

/// A chord's portable text form, the same on every platform, for files people read and edit:
/// modifiers in the order `Ctrl`, `Alt`, `Shift`, `Meta`, then the key, joined by `+`
/// (`Ctrl+Shift+Z`, `Alt+F4`, `Meta+Left`, `Ctrl+Plus`). Letters are written in capitals.
/// [`Chord::label`] is the platform's way of showing it instead.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.meta, "Meta")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match (&self.key, key_name(&self.key)) {
            (_, Some(name)) => f.write_str(&name),
            (Key::Character(c), None) if c == "+" => f.write_str("Plus"),
            (Key::Character(c), None) => f.write_str(&c.to_uppercase()),
            (_, None) => Ok(()),
        }
    }
}

/// Reads a chord's text form (see its `Display`). Modifier and key names are matched without
/// regard to case, and the usual other names are understood: `Control`, `Option`, `Cmd`,
/// `Command`, `Win` and `Super`, `Esc`, `Return`, `ArrowLeft`; `Ctrl++` is `Ctrl+Plus`.
impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Chord, ChordError> {
        let fail = |reason: &str| ChordError { text: s.to_owned(), reason: reason.to_owned() };
        let text = s.trim();
        // A `+` key written as itself: "+" or "Ctrl++".
        let (mods, key) = if text == "+" {
            ("", "+")
        } else if let Some(m) = text.strip_suffix("++") {
            (m, "+")
        } else {
            match text.rsplit_once('+') {
                Some(("", _)) => return Err(fail("a modifier is missing before the \"+\"")),
                Some((m, k)) => (m, k),
                None => ("", text),
            }
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(fail("the key is missing"));
        }
        let mut modifiers = Modifiers::NONE;
        if !mods.is_empty() {
            for part in mods.split('+') {
                let flag = match part.trim().to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => &mut modifiers.ctrl,
                    "alt" | "option" | "opt" => &mut modifiers.alt,
                    "shift" => &mut modifiers.shift,
                    "meta" | "cmd" | "command" | "win" | "super" => &mut modifiers.meta,
                    "" => return Err(fail("an empty part between two \"+\"")),
                    other => return Err(fail(&format!("{other:?} is not a modifier"))),
                };
                if *flag {
                    return Err(fail("a modifier is written twice"));
                }
                *flag = true;
            }
        }
        Ok(Chord { modifiers, key: key_from_name(key) })
    }
}

/// Where a command is registered. Inner scopes win over outer ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct Conflict {
    /// The command that already uses the chord in that scope.
    pub existing: String,
}

/// A shortcut that more than one command in the same scope answers to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Clash {
    /// The scope the commands share.
    pub scope: Scope,
    /// The shortcut.
    pub chord: Chord,
    /// The commands using it (for [`CommandTable::conflicts`], the other commands).
    pub commands: Vec<String>,
}

/// The command table. A command's shortcuts come from three layers, each replacing the one
/// below for the commands it names: the built-in defaults ([`Command::default_chords`]),
/// the app's bindings ([`CommandTable::rebind`]: what the app restores from its own
/// settings, or a mode it switches to), and the person's own
/// ([`CommandTable::set_user_bindings`]: their customization file).
#[derive(Clone, Debug, Default)]
pub struct CommandTable {
    commands: Vec<(Scope, Command)>,
    /// The app's chords per command id, over the built-in defaults.
    app: HashMap<String, Vec<Chord>>,
    /// The person's chords per command id, over everything else.
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

    /// The chords a command answers to: the person's, else the app's, else the built-in
    /// defaults.
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        match self.user.get(id) {
            Some(c) => c.clone(),
            None => self.base_chords(id),
        }
    }

    /// The chords a command has beneath the person's own: the app's, else the built-in
    /// defaults. What resetting the person's binding goes back to.
    pub fn base_chords(&self, id: &str) -> Vec<Chord> {
        match self.app.get(id) {
            Some(c) => c.clone(),
            None => self.default_chords(id),
        }
    }

    /// The app binds `id` to `chords`, over the built-in defaults and beneath the person's
    /// own binding. A chord another command in the same scope already answers to is
    /// reported, and nothing changes.
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
        self.app.insert(id.to_string(), chords);
        Ok(())
    }

    /// Takes the app's binding of `id` away: it has its built-in defaults again (unless the
    /// person bound it).
    pub fn reset(&mut self, id: &str) {
        self.app.remove(id);
    }

    /// The shortcuts the app bound a command to, or `None` when it has its built-in ones.
    pub fn app_chords(&self, id: &str) -> Option<&[Chord]> {
        self.app.get(id).map(Vec::as_slice)
    }

    /// A command's default shortcuts, whatever the user bound it to.
    pub fn default_chords(&self, id: &str) -> Vec<Chord> {
        self.commands
            .iter()
            .find(|(_, c)| c.id == id)
            .map(|(_, c)| c.default_chords.clone())
            .unwrap_or_default()
    }

    /// The shortcuts the user bound a command to, or `None` when it has its defaults.
    pub fn user_chords(&self, id: &str) -> Option<&[Chord]> {
        self.user.get(id).map(Vec::as_slice)
    }

    /// Every user binding by command id, including ids of commands not registered (yet): a
    /// binding set before its command is registered applies once it is.
    pub fn user_bindings(&self) -> &HashMap<String, Vec<Chord>> {
        &self.user
    }

    /// Replaces every user binding at once, without checking for clashes: for restoring a
    /// saved set (check it with [`CommandTable::clashes`] after), or a set where one command
    /// takes a shortcut from another, which one-by-one rebinds would refuse halfway.
    pub fn set_user_bindings(&mut self, user: HashMap<String, Vec<Chord>>) {
        self.user = user;
    }

    /// The shortcuts in `chords` that other commands in `id`'s scopes answer to now: all of
    /// what [`CommandTable::rebind`] would refuse, without changing anything.
    pub fn conflicts(&self, id: &str, chords: &[Chord]) -> Vec<Clash> {
        let scopes: Vec<Scope> = self.commands.iter().filter(|(_, c)| c.id == id).map(|(s, _)| *s).collect();
        let mut out: Vec<Clash> = Vec::new();
        for chord in chords {
            for scope in &scopes {
                let others: Vec<String> = self
                    .commands
                    .iter()
                    .filter(|(s, c)| s == scope && c.id != id && self.chords(&c.id).contains(chord))
                    .map(|(_, c)| c.id.clone())
                    .collect();
                if !others.is_empty() && !out.iter().any(|c| c.scope == *scope && c.chord == *chord) {
                    out.push(Clash { scope: *scope, chord: chord.clone(), commands: others });
                }
            }
        }
        out
    }

    /// Every shortcut that two or more commands in one scope answer to now.
    pub fn clashes(&self) -> Vec<Clash> {
        let mut out: Vec<Clash> = Vec::new();
        for (scope, c) in &self.commands {
            for chord in self.chords(&c.id) {
                match out.iter_mut().find(|x| x.scope == *scope && x.chord == chord) {
                    Some(found) if !found.commands.contains(&c.id) => found.commands.push(c.id.clone()),
                    Some(_) => {}
                    None => out.push(Clash { scope: *scope, chord, commands: vec![c.id.clone()] }),
                }
            }
        }
        out.retain(|c| c.commands.len() > 1);
        out
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
