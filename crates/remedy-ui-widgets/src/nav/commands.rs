//! Commands: actions with a label, an icon and shortcuts, registered once and reached from
//! the keyboard, toolbars, menus and the command palette alike.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::base::{Icon, MenuItem};
use remedy_ui_core::input::{Chord, Clash, Command, CommandTable, Conflict, Scope};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};

/// One command as the widgets show it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CommandInfo {
    /// Its id.
    pub id: String,
    /// What menus, tooltips and the palette show.
    pub label: String,
    /// Its icon, for toolbars.
    pub icon: Option<Icon>,
    /// The shortcuts it answers to now (the user's or the defaults).
    pub chords: Vec<Chord>,
    /// Whether it can run now.
    pub enabled: bool,
    /// It destroys something (deletes, leaves, sends): agents must ask before running it.
    pub destructive: bool,
}

type Action = Rc<dyn Fn()>;

struct Inner {
    table: RefCell<CommandTable>,
    actions: RefCell<HashMap<String, Action>>,
    icons: RefCell<HashMap<String, Icon>>,
    destructive: RefCell<std::collections::HashSet<String>>,
    /// Commands a held shortcut runs again.
    repeats: RefCell<std::collections::HashSet<String>>,
    order: RefCell<Vec<String>>,
    /// Bumped on every change: widgets showing commands follow it.
    revision: Signal<u64>,
}

/// The app's commands. Clone it freely (a shared handle).
#[derive(Clone)]
pub struct Commands(Rc<Inner>);

impl std::fmt::Debug for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Commands({})", self.0.order.borrow().len())
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self::new()
    }
}

impl Commands {
    /// No commands.
    pub fn new() -> Commands {
        Commands(Rc::new(Inner {
            table: RefCell::new(CommandTable::new()),
            actions: RefCell::new(HashMap::new()),
            icons: RefCell::new(HashMap::new()),
            destructive: RefCell::new(std::collections::HashSet::new()),
            repeats: RefCell::new(std::collections::HashSet::new()),
            order: RefCell::new(Vec::new()),
            revision: Signal::new(0),
        }))
    }

    fn changed(&self) {
        self.0.revision.update(|r| *r += 1);
    }

    /// Registers (or replaces) a command for the whole app.
    pub fn add(
        &self,
        id: &str,
        label: impl Into<String>,
        chords: Vec<Chord>,
        icon: Option<Icon>,
        action: impl Fn() + 'static,
    ) {
        self.add_in(Scope::App, id, label, chords, icon, action);
    }

    /// Registers (or replaces) a command at a scope.
    pub fn add_in(
        &self,
        scope: Scope,
        id: &str,
        label: impl Into<String>,
        chords: Vec<Chord>,
        icon: Option<Icon>,
        action: impl Fn() + 'static,
    ) {
        let cmd = Command { id: id.to_owned(), label: label.into(), default_chords: chords, enabled: true };
        self.0.table.borrow_mut().register(scope, cmd);
        self.0.actions.borrow_mut().insert(id.to_owned(), Rc::new(action));
        match icon {
            Some(i) => self.0.icons.borrow_mut().insert(id.to_owned(), i),
            None => self.0.icons.borrow_mut().remove(id),
        };
        let mut order = self.0.order.borrow_mut();
        if !order.iter().any(|o| o == id) {
            order.push(id.to_owned());
        }
        drop(order);
        self.changed();
    }

    /// Marks a command destructive (it deletes, leaves or sends): agents must have the
    /// person confirm before running it.
    pub fn set_destructive(&self, id: &str, on: bool) {
        if on {
            self.0.destructive.borrow_mut().insert(id.to_owned());
        } else {
            self.0.destructive.borrow_mut().remove(id);
        }
        self.changed();
    }

    /// Whether holding the command's shortcut down runs it again and again (undo, redo, zoom,
    /// next and previous). Off unless set: a held shortcut runs its command once, so a key
    /// held a moment too long does not open ten windows or send a message twice.
    pub fn set_repeats(&self, id: &str, on: bool) {
        if on {
            self.0.repeats.borrow_mut().insert(id.to_owned());
        } else {
            self.0.repeats.borrow_mut().remove(id);
        }
    }

    /// Changes a command's label (after the locale changed).
    pub fn set_label(&self, id: &str, label: impl Into<String>) {
        let label = label.into();
        let found = self
            .0
            .table
            .borrow()
            .palette_all()
            .into_iter()
            .find(|(_, c)| c.id == id)
            .map(|(s, c)| (s, c.clone()));
        if let Some((scope, mut c)) = found {
            c.label = label;
            self.0.table.borrow_mut().register(scope, c);
            self.changed();
        }
    }

    /// Enables or disables a command; disabled ones show dimmed and do not run.
    pub fn set_enabled(&self, id: &str, enabled: bool) {
        self.0.table.borrow_mut().set_enabled(id, enabled);
        self.changed();
    }

    /// Runs a command if it exists and is enabled. True when it ran.
    pub fn run(&self, id: &str) -> bool {
        let enabled = self.info(id).is_some_and(|c| c.enabled);
        let action = self.0.actions.borrow().get(id).cloned();
        match action {
            Some(a) if enabled => {
                a();
                true
            }
            _ => false,
        }
    }

    /// The app binds a command to new shortcuts (restored from its own settings, or a mode),
    /// beneath the person's own bindings; a clash in the same scope is refused.
    pub fn rebind(&self, id: &str, chords: Vec<Chord>) -> Result<(), Conflict> {
        let r = self.0.table.borrow_mut().rebind(id, chords);
        self.changed();
        r
    }

    /// Takes the app's binding of a command away (its built-in shortcuts again, unless the
    /// person bound it).
    pub fn reset(&self, id: &str) {
        self.0.table.borrow_mut().reset(id);
        self.changed();
    }

    /// A command's default shortcuts (tracked).
    pub fn default_chords(&self, id: &str) -> Vec<Chord> {
        self.0.revision.get();
        self.0.table.borrow().default_chords(id)
    }

    /// The shortcuts beneath the person's own: the app's binding, else the built-in ones
    /// (tracked). What resetting the person's binding goes back to.
    pub fn base_chords(&self, id: &str) -> Vec<Chord> {
        self.0.revision.get();
        self.0.table.borrow().base_chords(id)
    }

    /// The shortcuts the person bound a command to, or `None` while it has the app's
    /// (tracked).
    pub fn user_chords(&self, id: &str) -> Option<Vec<Chord>> {
        self.0.revision.get();
        self.0.table.borrow().user_chords(id).map(<[Chord]>::to_vec)
    }

    /// Every binding of the person's by command id, including commands not registered yet (tracked).
    pub fn bindings(&self) -> HashMap<String, Vec<Chord>> {
        self.0.revision.get();
        self.0.table.borrow().user_bindings().clone()
    }

    /// Replaces every user binding at once (a saved set, or one where a command takes a
    /// shortcut from another), without refusing clashes: check [`Commands::clashes`] after.
    /// Bindings for commands not registered yet apply when they are.
    pub fn set_bindings(&self, bindings: HashMap<String, Vec<Chord>>) {
        self.0.table.borrow_mut().set_user_bindings(bindings);
        self.changed();
    }

    /// The person's bindings as text, to save or carry to another machine
    /// ([`crate::nav::bindings_to_json`]); [`Commands::import_bindings`] reads it back.
    pub fn export_bindings(&self) -> String {
        crate::nav::bindings_to_json(&self.bindings())
    }

    /// Replaces the person's bindings with those in `text` (written by
    /// [`Commands::export_bindings`]), as [`Commands::set_bindings`] does. Text that is not a
    /// set of bindings changes nothing and says why.
    pub fn import_bindings(&self, text: &str) -> Result<(), crate::nav::BindingsError> {
        let b = crate::nav::bindings_from_json(text)?;
        self.set_bindings(b);
        Ok(())
    }

    /// What binding `id` to `chords` would clash with, without changing anything: each
    /// shortcut other commands in its scope answer to, and those commands.
    pub fn conflicts(&self, id: &str, chords: &[Chord]) -> Vec<Clash> {
        self.0.revision.get();
        self.0.table.borrow().conflicts(id, chords)
    }

    /// Every shortcut two or more commands in one scope answer to now (tracked).
    pub fn clashes(&self) -> Vec<Clash> {
        self.0.revision.get();
        self.0.table.borrow().clashes()
    }

    /// One command (tracked).
    pub fn info(&self, id: &str) -> Option<CommandInfo> {
        self.0.revision.get();
        let t = self.0.table.borrow();
        let (_, c) = t.palette_all().into_iter().find(|(_, c)| c.id == id)?;
        Some(CommandInfo {
            id: c.id.clone(),
            label: c.label.clone(),
            icon: self.0.icons.borrow().get(id).copied(),
            chords: t.chords(id),
            enabled: c.enabled,
            destructive: self.0.destructive.borrow().contains(id),
        })
    }

    /// Every command in the order they were added (tracked).
    pub fn all(&self) -> Vec<CommandInfo> {
        let ids = self.0.order.borrow().clone();
        ids.iter().filter_map(|id| self.info(id)).collect()
    }

    /// A menu entry for command `id`, so menus, toolbars and the command palette share one
    /// source: its label, its first shortcut (written in the person's language, following
    /// rebinding), its icon, whether it can run now and whether it is destructive all come
    /// from the command, and choosing it runs the command. Add an access key with
    /// [`MenuItem::mnemonic`].
    ///
    /// ```
    /// use remedy_ui_core::input::Chord;
    /// use remedy_ui_widgets::nav::Commands;
    ///
    /// let commands = Commands::new();
    /// commands.add("file.open", "Open…", vec!["Ctrl+O".parse::<Chord>()?], None, || {});
    /// let open = commands.menu_item("file.open").mnemonic('o');
    /// assert_eq!(open.label_text(), "Open…");
    /// # Ok::<(), remedy_ui_core::input::ChordError>(())
    /// ```
    pub fn menu_item(&self, id: &str) -> MenuItem {
        let info = self.info(id);
        let (c1, c2, c3, c4) = (self.clone(), self.clone(), self.clone(), self.clone());
        let (i1, i2, i3, i4) = (id.to_owned(), id.to_owned(), id.to_owned(), id.to_owned());
        let mut item = MenuItem::new(String::new(), move || {
            let _ = c1.run(&i1);
        })
        .label(move || c2.info(&i2).map(|c| c.label).unwrap_or_default())
        .disabled(move || !c3.info(&i3).is_some_and(|c| c.enabled))
        .chords(move || c4.info(&i4).map(|c| c.chords).unwrap_or_default());
        if let Some(icon) = info.as_ref().and_then(|c| c.icon) {
            item = item.icon(icon);
        }
        if info.is_some_and(|c| c.destructive) {
            item = item.danger();
        }
        item
    }

    /// Makes the shortcuts work in a window: keys nothing inside handled run the matching
    /// command (window commands before app ones).
    pub fn attach(&self, ui: &Ui) -> El {
        let me = self.clone();
        ui.root().on_key(move |e| {
            let id = me.0.table.borrow().match_key(e, &[Scope::App, Scope::Window]).map(|c| c.id.clone());
            // A held key is still this command's: it runs again only where that was asked for.
            id.is_some_and(|id| (e.repeat && !me.0.repeats.borrow().contains(&id)) || me.run(&id))
        })
    }
}
