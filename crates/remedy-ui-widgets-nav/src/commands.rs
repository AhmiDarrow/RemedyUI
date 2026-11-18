//! Commands: actions with a label, an icon and shortcuts, registered once and reached from
//! the keyboard, toolbars, menus and the command palette alike.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use remedy_ui_input::{Chord, Command, CommandTable, Conflict, Scope};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core::Icon;

/// One command as the widgets show it.
#[derive(Clone, Debug, PartialEq)]
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
}

type Action = Rc<dyn Fn()>;

struct Inner {
    table: RefCell<CommandTable>,
    actions: RefCell<HashMap<String, Action>>,
    icons: RefCell<HashMap<String, Icon>>,
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

    /// The user binds a command to new shortcuts; a clash in the same scope is refused.
    pub fn rebind(&self, id: &str, chords: Vec<Chord>) -> Result<(), Conflict> {
        let r = self.0.table.borrow_mut().rebind(id, chords);
        self.changed();
        r
    }

    /// Restores a command's default shortcuts.
    pub fn reset(&self, id: &str) {
        self.0.table.borrow_mut().reset(id);
        self.changed();
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
        })
    }

    /// Every command in the order they were added (tracked).
    pub fn all(&self) -> Vec<CommandInfo> {
        let ids = self.0.order.borrow().clone();
        ids.iter().filter_map(|id| self.info(id)).collect()
    }

    /// Makes the shortcuts work in a window: keys nothing inside handled run the matching
    /// command (window commands before app ones).
    pub fn attach(&self, ui: &Ui) -> El {
        let me = self.clone();
        ui.root().on_key(move |e| {
            let id = me.0.table.borrow().match_key(e, &[Scope::App, Scope::Window]).map(|c| c.id.clone());
            id.is_some_and(|id| me.run(&id))
        })
    }
}
