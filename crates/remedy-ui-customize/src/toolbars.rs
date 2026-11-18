//! The person's toolbars and menus: per slot the app declares, the list of command ids it
//! shows, in order. The file holds only the slots that differ from the app's defaults.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use remedy_ui_reactive::{Effect, Signal, untrack};
use remedy_ui_view::Ui;
use remedy_ui_widgets_nav::Commands;
use serde_json::Value;

use crate::{Area, Customizations, CustomizeError, Notice, files};

/// The toolbars file's name in the config folder.
pub(crate) const TOOLBARS_FILE: &str = "toolbars.json";

/// A slot the app declared.
#[derive(Clone)]
pub(crate) struct Slot {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) defaults: Vec<String>,
    pub(crate) value: Signal<Vec<String>>,
    /// What the slot held after the last load or save: a different value is the person's
    /// edit (through [`remedy_ui_widgets_nav::toolbar_editor`] or the app).
    pub(crate) known: Rc<RefCell<Vec<String>>>,
    pub(crate) watch: Effect,
}

/// The toolbars as customized.
pub(crate) struct ToolbarArea {
    pub(crate) commands: Commands,
    pub(crate) slots: Vec<Slot>,
    /// The file's slots as last read (empty when there is none or it failed).
    pub(crate) file: BTreeMap<String, Vec<String>>,
}

fn parse(text: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("not a toolbars file: {e}"))?;
    let Value::Object(map) = doc else {
        return Err("a toolbars file is an object of slots to lists of command ids".into());
    };
    let mut out = BTreeMap::new();
    for (slot, v) in map {
        let Some(items) = v.as_array() else {
            return Err(format!("{slot}: a slot is a list of command ids such as [\"file.save\"]"));
        };
        let ids = items
            .iter()
            .map(|i| i.as_str().map(str::to_owned).ok_or_else(|| format!("{slot}: command ids are text")))
            .collect::<Result<Vec<String>, String>>()?;
        out.insert(slot, ids);
    }
    Ok(out)
}

impl Customizations {
    /// The toolbars file.
    pub fn toolbars_file(&self) -> PathBuf {
        self.0.dir.join(TOOLBARS_FILE)
    }

    /// Turns on toolbar and menu customization over `commands`: the person's file is read now
    /// (a bad one is skipped with a notice). Declare each slot with
    /// [`Customizations::toolbar_slot`].
    pub fn enable_toolbars(&self, commands: &Commands) {
        // Slots declared before keep their lists and watchers.
        let slots =
            self.0.toolbars.borrow_mut().as_mut().map(|a| std::mem::take(&mut a.slots)).unwrap_or_default();
        *self.0.toolbars.borrow_mut() =
            Some(ToolbarArea { commands: commands.clone(), slots, file: BTreeMap::new() });
        self.reload(&[Area::Toolbars]);
    }

    /// Declares a toolbar or menu slot with the commands the app shows by default, and gives
    /// its list (the person's, when they changed it). Hand it to
    /// [`remedy_ui_widgets_nav::toolbar`] and [`remedy_ui_widgets_nav::toolbar_editor`]: each
    /// edit is saved and goes into `ui`'s undo history. Declaring a slot again gives the same
    /// list. `None` while toolbars are not on.
    pub fn toolbar_slot(
        &self,
        ui: &Ui,
        id: &str,
        label: &str,
        defaults: &[&str],
    ) -> Option<Signal<Vec<String>>> {
        let mut area = self.0.toolbars.borrow_mut();
        let area = area.as_mut()?;
        if let Some(s) = area.slots.iter().find(|s| s.id == id) {
            return Some(s.value);
        }
        let defaults: Vec<String> = defaults.iter().map(|d| (*d).to_owned()).collect();
        let start = area.file.get(id).cloned().unwrap_or_else(|| defaults.clone());
        let known = Rc::new(RefCell::new(start.clone()));
        let me = Rc::downgrade(&self.0);
        let (wui, k2, id2) = (ui.downgrade(), known.clone(), id.to_owned());
        let (value, watch) = self.in_owner(|| {
            let value = Signal::new(start);
            let watch = Effect::new(move || {
                let Ok(now) = value.try_get() else { return };
                if now == *k2.borrow() {
                    return;
                }
                let (Some(inner), Some(ui)) = (me.upgrade(), wui.upgrade()) else { return };
                untrack(|| Customizations(inner).slot_edited(&ui, &id2, now));
            });
            (value, watch)
        });
        area.slots.push(Slot { id: id.to_owned(), label: label.to_owned(), defaults, value, known, watch });
        Some(value)
    }

    /// The declared slots: id and label, in the order declared.
    pub fn toolbar_slots(&self) -> Vec<(String, String)> {
        self.0
            .toolbars
            .borrow()
            .as_ref()
            .map(|a| a.slots.iter().map(|s| (s.id.clone(), s.label.clone())).collect())
            .unwrap_or_default()
    }

    /// The list of a declared slot.
    pub fn toolbar(&self, id: &str) -> Option<Signal<Vec<String>>> {
        self.0.toolbars.borrow().as_ref()?.slots.iter().find(|s| s.id == id).map(|s| s.value)
    }

    /// The commands toolbars show.
    pub fn toolbar_commands(&self) -> Option<Commands> {
        self.0.toolbars.borrow().as_ref().map(|a| a.commands.clone())
    }

    /// Whether the person changed slot `id` (tracked).
    pub fn is_toolbar_customized(&self, id: &str) -> bool {
        self.0.revision.get();
        self.0.toolbars.borrow().as_ref().is_some_and(|a| a.file.contains_key(id))
    }

    /// Sets a slot's commands (undoable), as the toolbar editor does.
    pub fn set_toolbar(&self, ui: &Ui, id: &str, ids: Vec<String>) -> Result<(), CustomizeError> {
        let slot = self.slot(id)?;
        if slot.value.get_untracked().ok().as_ref() == Some(&ids) {
            return Ok(());
        }
        self.save_slot(ui, &slot, ids)
    }

    /// Gives slot `id` the app's commands back (undoable).
    pub fn reset_toolbar(&self, ui: &Ui, id: &str) -> Result<(), CustomizeError> {
        let slot = self.slot(id)?;
        let mut file = self.toolbar_file_map()?;
        if file.remove(id).is_none() {
            return Ok(());
        }
        self.change(ui, format!("Reset {}", slot.label), None, &[Area::Toolbars], |c| c.write_toolbars(&file))
    }

    /// Gives every slot the app's commands back (undoable).
    pub fn reset_toolbars(&self, ui: &Ui) -> Result<(), CustomizeError> {
        self.toolbar_file_map()?;
        let path = self.toolbars_file();
        self.change(ui, "Reset all toolbars", None, &[Area::Toolbars], |_| {
            files::remove(&path).map_err(|e| CustomizeError::io(&path, &e))
        })
    }

    fn slot(&self, id: &str) -> Result<Slot, CustomizeError> {
        let area = self.0.toolbars.borrow();
        let area = area.as_ref().ok_or(CustomizeError::NotEnabled(Area::Toolbars))?;
        area.slots.iter().find(|s| s.id == id).cloned().ok_or_else(|| CustomizeError::UnknownSlot(id.into()))
    }

    fn toolbar_file_map(&self) -> Result<BTreeMap<String, Vec<String>>, CustomizeError> {
        self.0
            .toolbars
            .borrow()
            .as_ref()
            .map(|a| a.file.clone())
            .ok_or(CustomizeError::NotEnabled(Area::Toolbars))
    }

    /// A slot's list changed outside a load: the person edited it.
    fn slot_edited(&self, ui: &Ui, id: &str, now: Vec<String>) {
        let Ok(slot) = self.slot(id) else { return };
        // A save that fails puts the slot back as the file has it and shows a notice.
        let _ = self.save_slot(ui, &slot, now);
    }

    fn save_slot(&self, ui: &Ui, slot: &Slot, ids: Vec<String>) -> Result<(), CustomizeError> {
        let mut file = self.toolbar_file_map()?;
        if ids == slot.defaults {
            file.remove(&slot.id);
        } else {
            file.insert(slot.id.clone(), ids);
        }
        self.change(ui, format!("Change {}", slot.label), None, &[Area::Toolbars], |c| {
            c.write_toolbars(&file)
        })
    }

    fn write_toolbars(&self, file: &BTreeMap<String, Vec<String>>) -> Result<(), CustomizeError> {
        let path = self.toolbars_file();
        let bytes =
            (!file.is_empty()).then(|| files::json_text(&serde_json::to_value(file).unwrap_or(Value::Null)));
        files::put(&path, bytes.as_deref()).map_err(|e| CustomizeError::io(&path, &e))
    }

    /// Reads the toolbars file and gives every slot its list.
    pub(crate) fn reload_toolbars(&self) {
        if self.0.toolbars.borrow().is_none() {
            return;
        }
        let path = self.toolbars_file();
        let read = match files::read(&path) {
            Ok(None) => Ok(BTreeMap::new()),
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .map_err(|_| "the file is not UTF-8 text".to_owned())
                .and_then(|t| parse(&t)),
            Err(e) => Err(e.to_string()),
        };
        let (file, notices) = match read {
            Ok(f) => (f, Vec::new()),
            Err(message) => (BTreeMap::new(), vec![Notice { area: Area::Toolbars, path, message }]),
        };
        let slots = {
            let mut area = self.0.toolbars.borrow_mut();
            let Some(area) = area.as_mut() else { return };
            area.file = file.clone();
            area.slots.clone()
        };
        for s in slots {
            let v = file.get(&s.id).cloned().unwrap_or_else(|| s.defaults.clone());
            *s.known.borrow_mut() = v.clone();
            s.value.set_if_changed(v);
        }
        self.set_notices(Area::Toolbars, None, notices);
    }
}

impl Drop for ToolbarArea {
    fn drop(&mut self) {
        for s in &self.slots {
            s.watch.dispose();
        }
    }
}
