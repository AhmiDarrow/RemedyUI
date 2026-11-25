//! A rich text editor: a rich field with a formatting toolbar, a link box (Ctrl+K) and
//! suggestions when someone types "@". The toolbar shows the formatting where the caret is;
//! every button is reachable by keyboard and has its shortcut too.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Live, Role, Semantics};
use remedy_ui_core::input::Key;
use remedy_ui_core::style::States;
use remedy_ui_text::{Block, Mark, RichEditor};
use remedy_ui_view::{El, FieldKind, TextField, Ui};
use remedy_ui_widgets_core::{self as w, Icon};

/// Someone (or something) people can mention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionCandidate {
    /// The app's id for them, kept in the document.
    pub id: String,
    /// The name shown and inserted after "@".
    pub name: String,
}

/// Who matches what was typed after "@" (the app's search, best first).
pub type MentionSource = Rc<dyn Fn(&str) -> Vec<MentionCandidate>>;

/// How a rich editor behaves.
#[derive(Clone)]
#[non_exhaustive]
pub struct RichEditorOptions {
    /// The name screen readers read for it. The default, "Message", is read in the window's
    /// language.
    pub label: String,
    /// Grey text shown while it is empty.
    pub placeholder: String,
    /// Enter submits (a chat's composer) and Shift+Enter starts a new line.
    pub submit_on_enter: bool,
    /// Show the formatting toolbar.
    pub toolbar: bool,
    /// Suggest people to mention when "@" is typed.
    pub mentions: Option<MentionSource>,
}

impl RichEditorOptions {
    /// Sets `label`: the name screen readers read for it.
    pub fn label(mut self, value: String) -> Self {
        self.label = value;
        self
    }

    /// Sets `placeholder`: grey text shown while it is empty.
    pub fn placeholder(mut self, value: String) -> Self {
        self.placeholder = value;
        self
    }

    /// Sets `submit_on_enter`: enter submits (a chat's composer) and Shift+Enter starts a new line.
    pub fn submit_on_enter(mut self, value: bool) -> Self {
        self.submit_on_enter = value;
        self
    }

    /// Sets `toolbar`: show the formatting toolbar.
    pub fn toolbar(mut self, value: bool) -> Self {
        self.toolbar = value;
        self
    }

    /// Sets `mentions`: suggest people to mention when "@" is typed.
    pub fn mentions(mut self, value: MentionSource) -> Self {
        self.mentions = Some(value);
        self
    }
}

impl Default for RichEditorOptions {
    fn default() -> Self {
        RichEditorOptions {
            label: DEFAULT_LABEL.into(),
            placeholder: String::new(),
            submit_on_enter: false,
            toolbar: true,
            mentions: None,
        }
    }
}

impl std::fmt::Debug for RichEditorOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RichEditorOptions({:?})", self.label)
    }
}

/// The default label: left as it is, the editor is named in the window's language.
const DEFAULT_LABEL: &str = "Message";

/// What a toolbar button does to the editor.
type Act = Rc<dyn Fn(&mut RichEditor)>;
/// Whether a toolbar button shows as on where the caret is.
type IsOn = Box<dyn Fn(&RichEditor) -> bool>;

/// A toolbar button and what it shows as on.
struct Tool {
    button: El,
    /// Its name's message.
    label: &'static str,
    on: IsOn,
}

/// The "@" being typed before the caret: the bytes from "@" to the caret, and what follows it.
fn mention_query(ed: &RichEditor) -> Option<(Range<usize>, String)> {
    let sel = ed.selection();
    if !sel.is_caret() {
        return None;
    }
    let caret = sel.focus;
    let text = ed.text();
    let before = text.get(..caret)?;
    let at = before.rfind('@')?;
    let query = before.get(at + 1..)?;
    // "@" starts a word, and the query is one word.
    let starts_word = before.get(..at).is_none_or(|b| b.chars().next_back().is_none_or(char::is_whitespace));
    if !starts_word || query.chars().any(|c| c.is_whitespace() || c == '@') || query.len() > 64 {
        return None;
    }
    // Not inside a mention already made.
    if ed.doc().marks.iter().any(|(r, m)| matches!(m, Mark::Mention(_)) && r.start <= at && at < r.end) {
        return None;
    }
    Some((at..caret, query.to_owned()))
}

/// The open suggestion list.
struct Suggestions {
    list: El,
    items: Vec<MentionCandidate>,
    chosen: usize,
    range: Range<usize>,
}

fn now_ms() -> u64 {
    // Undo grouping only compares times; formatting and mentions are steps of their own.
    0
}

/// A rich text editor. Returns its element and its field (for its document, its text and
/// `on_submit`).
#[track_caller]
pub fn rich_editor(ui: &Ui, options: RichEditorOptions) -> (El, TextField) {
    let unit = ui.theme().spacing.unit;
    let field = ui
        .text_field("text_input", FieldKind::Rich)
        .label(options.label.clone())
        .placeholder(options.placeholder.clone())
        .submit_on_enter(options.submit_on_enter);
    if options.label == DEFAULT_LABEL {
        // The app named it nothing: the default name follows the window's language.
        let (f, u) = (field.clone(), ui.clone());
        let _ = field.el().bind(move || {
            let _ = f.clone().label(crate::tr(&u, "editor-label"));
        });
    }
    let field_el =
        field.el().fill_width().layout(|l| l.min_height = remedy_ui_core::layout::Length::Px(unit * 10.0));
    let root = ui.el("rich_editor").column().fill_width().gap(unit);

    // ---------- the toolbar ----------
    let tools: Rc<RefCell<Vec<Tool>>> = Rc::default();
    let add = |icon: Icon,
               label: &'static str,
               act: Rc<dyn Fn(&mut RichEditor)>,
               on: Box<dyn Fn(&RichEditor) -> bool>| {
        let (f, u) = (field.clone(), ui.clone());
        let button = w::icon_button(
            ui,
            icon,
            move || crate::tr(&u, label),
            move || {
                let act = act.clone();
                let _ = f.edit_rich(move |e| act(e));
                f.el().focus();
            },
        );
        tools.borrow_mut().push(Tool { button: button.clone(), label, on });
        button
    };
    let mark = |m: Mark| -> (Act, IsOn) {
        let m2 = m.clone();
        (
            Rc::new(move |e: &mut RichEditor| e.toggle(m.clone())),
            Box::new(move |e: &RichEditor| e.active_marks().contains(&m2)),
        )
    };
    let block = |b: Block| -> (Act, IsOn) {
        (
            Rc::new(move |e: &mut RichEditor| e.set_block(b)),
            Box::new(move |e: &RichEditor| e.block().same_kind(b)),
        )
    };
    let mut buttons = Vec::new();
    for (icon, label, m) in [
        (Icon::Bold, "format-bold", Mark::Bold),
        (Icon::Italic, "format-italic", Mark::Italic),
        (Icon::Strike, "format-strikethrough", Mark::Strike),
        (Icon::Code, "format-code", Mark::Code),
    ] {
        let (act, on) = mark(m);
        buttons.push(add(icon, label, act, on));
    }
    for (icon, label, b) in [
        (Icon::Heading, "format-heading", Block::Heading(2)),
        (Icon::ListBullet, "format-bulleted-list", Block::Bullet(0)),
        (Icon::ListNumber, "format-numbered-list", Block::Numbered(0)),
        (Icon::ListTask, "format-task-list", Block::Task { done: false, level: 0 }),
        (Icon::Quote, "format-quote", Block::Quote),
        (Icon::CodeBlock, "format-code-block", Block::Code),
    ] {
        let (act, on) = block(b);
        buttons.push(add(icon, label, act, on));
    }

    // The link box: the address of the link at the caret, to change, remove or add.
    let link_open: Rc<RefCell<Option<El>>> = Rc::default();
    let open_link_box: Rc<dyn Fn(&El)> = {
        let (f, ui, link_open) = (field.clone(), ui.clone(), link_open.clone());
        Rc::new(move |anchor: &El| {
            if let Some(old) = link_open.borrow_mut().take() {
                old.remove();
            }
            let current = f.read_rich(|e| e.link_at_caret().map(|(_, u)| u)).flatten();
            let address = w::text_input(&ui, crate::tr(&ui, "link-address")).placeholder("https://");
            if let Some(u) = &current {
                address.set_value(u);
                address.select_all();
            }
            let close = {
                let (link_open, f) = (link_open.clone(), f.clone());
                Rc::new(move || {
                    if let Some(l) = link_open.borrow_mut().take() {
                        l.remove();
                    }
                    f.el().focus();
                })
            };
            let apply = {
                let (f, address, close) = (f.clone(), address.clone(), close.clone());
                Rc::new(move || {
                    let url = address.value().trim().to_owned();
                    if !url.is_empty() {
                        let _ = f.edit_rich(|e| e.set_link(Some(url), None, now_ms()));
                    }
                    close();
                })
            };
            let a2 = apply.clone();
            let address = address.on_submit(move || a2());
            let remove = {
                let (f, close) = (f.clone(), close.clone());
                move || {
                    let _ = f.edit_rich(|e| e.set_link(None, None, now_ms()));
                    close();
                }
            };
            let unit = ui.theme().spacing.unit;
            let content = w::column(&ui)
                .gap(unit * 2.0)
                .padding(remedy_ui_core::Insets::all(unit * 2.0))
                .child(address.el().width(unit * 70.0))
                .child(
                    w::row(&ui)
                        .gap(unit * 2.0)
                        .child(w::button(&ui, crate::tr(&ui, "link-apply"), move || apply()))
                        .child(w::button(&ui, crate::tr(&ui, "link-remove"), remove)),
                )
                .semantics(Semantics::new(Role::Dialog).label(crate::tr(&ui, "link-dialog")));
            let layer = w::popover(&ui, anchor, content);
            address.el().focus();
            *link_open.borrow_mut() = Some(layer);
        })
    };
    let link_button = {
        let (open, anchor) = (open_link_box.clone(), Rc::new(RefCell::new(None::<El>)));
        let (a2, u) = (anchor.clone(), ui.clone());
        let b = w::icon_button(
            ui,
            Icon::Link,
            move || crate::tr(&u, "format-link"),
            move || {
                if let Some(a) = a2.borrow().as_ref() {
                    open(a);
                }
            },
        );
        *anchor.borrow_mut() = Some(b.clone());
        tools.borrow_mut().push(Tool {
            button: b.clone(),
            label: "format-link",
            on: Box::new(|e: &RichEditor| e.link_at_caret().is_some()),
        });
        b
    };
    buttons.insert(4, link_button.clone());

    if options.toolbar {
        let u = ui.clone();
        let bar = ui
            .el("rich_toolbar")
            .row()
            .fill_width()
            .gap(unit * 0.5)
            .layout(|l| l.wrap = true)
            .children(buttons)
            .semantics(move || Semantics::new(Role::Group).label(crate::tr(&u, "formatting-toolbar")));
        let _ = root.clone().child(bar);
    }
    let _ = root.clone().child(field_el.clone());

    // Each button shows whether its formatting is on where the caret is.
    let refresh = {
        let (tools, f, u) = (tools.clone(), field.clone(), ui.clone());
        move || {
            for t in tools.borrow().iter() {
                let on = f.read_rich(|e| (t.on)(e)).unwrap_or(false);
                t.button.state(States::CHECKED, on);
                t.button.state(States::SELECTED, on);
                let (u, label) = (u.clone(), t.label);
                let _ = t.button.clone().semantics(move || {
                    Semantics::new(Role::Button).label(crate::tr(&u, label)).checked(on).action(Action::Click)
                });
            }
        }
    };
    refresh();
    field.observe_caret(refresh);

    // Ctrl+K (Command+K) opens the link box.
    {
        let (open, anchor) = (open_link_box, link_button);
        let platform_meta = ui.platform() == remedy_ui_core::input::Platform::Apple;
        let _ = field_el.on_key(move |e| {
            let cmd = if platform_meta { e.modifiers.meta } else { e.modifiers.ctrl };
            if e.pressed && cmd && matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case("k")) {
                open(&anchor);
                return true;
            }
            false
        });
    }

    // ---------- mentions ----------
    if let Some(source) = options.mentions {
        let open: Rc<RefCell<Option<Suggestions>>> = Rc::default();
        // An "@" whose list was closed with Escape stays closed until the caret leaves it.
        let dismissed: Rc<Cell<Option<usize>>> = Rc::default();
        let insert: Rc<dyn Fn(usize)> = {
            let (f, open) = (field.clone(), open.clone());
            Rc::new(move |i: usize| {
                let Some(s) = open.borrow_mut().take() else { return };
                s.list.remove();
                let Some(c) = s.items.get(i).cloned() else { return };
                let _ = f.edit_rich(|e| {
                    e.replace_with_mention(s.range.clone(), &c.id, &format!("@{}", c.name), now_ms())
                });
                f.el().focus();
            })
        };
        let show: Rc<dyn Fn()> = {
            let (f, ui, open, insert, dismissed) =
                (field.clone(), ui.clone(), open.clone(), insert.clone(), dismissed.clone());
            Rc::new(move || {
                let query = f.read_rich(mention_query).flatten();
                let close = |open: &Rc<RefCell<Option<Suggestions>>>| {
                    if let Some(s) = open.borrow_mut().take() {
                        s.list.remove();
                    }
                };
                let Some((range, q)) = query else {
                    dismissed.set(None);
                    return close(&open);
                };
                if dismissed.get() == Some(range.start) {
                    return close(&open);
                }
                let items: Vec<MentionCandidate> = source(&q).into_iter().take(8).collect();
                if items.is_empty() {
                    return close(&open);
                }
                let chosen = open.borrow().as_ref().map_or(0, |s| s.chosen.min(items.len() - 1));
                close(&open);
                let Some(caret) = f.caret_rect() else { return };
                let unit = ui.theme().spacing.unit;
                let (u, count) = (ui.clone(), items.len());
                let list = ui.el("menu").column().semantics(move || {
                    Semantics::new(Role::List).label(crate::trf(
                        &u,
                        "mention-count",
                        &[("count", count.into())],
                    ))
                });
                for (i, c) in items.iter().enumerate() {
                    let ins = insert.clone();
                    let row = ui
                        .el("menu_item")
                        .row()
                        .gap(unit * 2.0)
                        .child(w::icon(&ui, Icon::At, ui.theme().type_scale.body))
                        .child(w::text(&ui, c.name.clone()).data_text())
                        .on_pointer(move |e| {
                            if e.phase == remedy_ui_core::input::PointerPhase::Down {
                                ins(i);
                                return true;
                            }
                            false
                        })
                        .keyboard_alternative({
                            let u = ui.clone();
                            move || crate::tr(&u, "mention-keys")
                        })
                        .semantics(
                            Semantics::new(Role::ListItem)
                                .label(c.name.clone())
                                .selected(i == chosen)
                                .set_position(i + 1, items.len())
                                .live(if i == chosen { Live::Polite } else { Live::Off }),
                        );
                    row.state(States::SELECTED, i == chosen);
                    let _ = list.clone().child(row);
                }
                let list = w::place_near(&ui, list, caret, 4.0);
                let [_, overlays, _] = ui.layers();
                let _ = overlays.child(list.clone());
                *open.borrow_mut() = Some(Suggestions { list, items, chosen, range });
            })
        };
        let s2 = show.clone();
        field.observe_caret(move || s2());
        let (open2, insert2, show2, dismissed2) = (open.clone(), insert, show, dismissed);
        field.intercept_keys(move |e| {
            if !e.pressed || open2.borrow().is_none() {
                return false;
            }
            let n = open2.borrow().as_ref().map_or(0, |s| s.items.len());
            match e.key {
                Key::ArrowDown | Key::ArrowUp => {
                    if let Some(s) = open2.borrow_mut().as_mut() {
                        s.chosen =
                            if e.key == Key::ArrowDown { (s.chosen + 1) % n } else { (s.chosen + n - 1) % n };
                    }
                    show2();
                    true
                }
                Key::Enter | Key::Tab => {
                    let i = open2.borrow().as_ref().map_or(0, |s| s.chosen);
                    insert2(i);
                    true
                }
                Key::Escape => {
                    if let Some(s) = open2.borrow_mut().take() {
                        dismissed2.set(Some(s.range.start));
                        s.list.remove();
                    }
                    true
                }
                _ => false,
            }
        });
    }

    (root, field)
}
