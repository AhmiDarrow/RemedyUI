//! Widget kinds: what can be placed on a board. Each kind has an id, a name, typed settings
//! (the editor's property grid edits them) and a builder. The built-ins are here; apps add
//! their own (FreeSpeak's voice and chat nodes); people add scripted ones.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use remedy_ui_reactive::{Scope, Signal, untrack};
use remedy_ui_script::{Limits, Node, ScriptWidget, SettingKind, SettingSpec, TextStyle};
use remedy_ui_view::{El, Ui};
use remedy_ui_widgets_core as w;
use serde_json::{Map, Value};

/// The reactions on a widget, by its id.
pub type ReactionsFn = Rc<dyn Fn(&str) -> Signal<Vec<crate::Reaction>>>;
/// Toggles the viewer's reaction (widget id, emoji).
pub type ReactFn = Rc<dyn Fn(&str, &str)>;

/// What a board gives its widgets: data the app shares with them, and the doors out.
#[derive(Clone)]
pub struct Context {
    /// Read-only data for widgets (who is here, what is new): scripts see it as `host`.
    pub host: Signal<Value>,
    /// Opens a web link; the app asks the person first.
    pub open_link: Rc<dyn Fn(&str)>,
    /// The reactions on a widget (by its id), and toggling one of them: shared by the
    /// app with everyone viewing the board.
    pub reactions: ReactionsFn,
    /// Toggles the viewer's reaction `emoji` on widget `id`.
    pub react: ReactFn,
    /// How much a scripted widget may use.
    pub limits: Limits,
}

impl Context {
    /// A context with no data, links opened by `open_link`, and reactions kept on this device
    /// (for previews and apps that do not share them).
    pub fn local(open_link: impl Fn(&str) + 'static) -> Context {
        let store: Rc<RefCell<BTreeMap<String, Signal<Vec<crate::Reaction>>>>> = Rc::default();
        // The signals live as long as the context, not as long as whatever is being built
        // when a widget first asks for them.
        let ((), scope) = Scope::new(|| ());
        let get = move |id: &str| {
            *store.borrow_mut().entry(id.to_owned()).or_insert_with(|| {
                scope.run(|| Signal::new(Vec::new())).unwrap_or_else(|_| Signal::new(Vec::new()))
            })
        };
        let get2 = get.clone();
        Context {
            host: Signal::new(Value::Object(Map::new())),
            open_link: Rc::new(open_link),
            reactions: Rc::new(get),
            react: Rc::new(move |id, emoji| {
                let s = get2(id);
                s.update(|v| crate::reactions::toggle(v, emoji));
            }),
            limits: Limits::default(),
        }
    }
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context").field("limits", &self.limits).finish()
    }
}

/// A kind of widget.
pub trait WidgetKind {
    /// Its registry id (saved in boards).
    fn id(&self) -> &str;
    /// Its name, in the add-widget list.
    fn name(&self) -> String;
    /// Its settings.
    fn settings(&self) -> Vec<SettingSpec>;
    /// Builds a placed widget `id` with `settings` (each missing one at its default).
    fn build(&self, ui: &Ui, id: &str, settings: &Map<String, Value>, cx: &Context) -> El;
}

/// The kinds a board knows, by id.
#[derive(Default, Clone)]
pub struct Registry {
    kinds: BTreeMap<String, Rc<dyn WidgetKind>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.kinds.keys()).finish()
    }
}

impl Registry {
    /// The built-in kinds: text, links, reactions.
    pub fn with_builtins() -> Registry {
        let mut r = Registry::default();
        r.add(Rc::new(TextKind));
        r.add(Rc::new(LinksKind));
        r.add(Rc::new(ReactionsKind));
        r
    }

    /// Adds (or replaces) a kind.
    pub fn add(&mut self, kind: Rc<dyn WidgetKind>) {
        self.kinds.insert(kind.id().to_owned(), kind);
    }

    /// The kind `id`.
    pub fn get(&self, id: &str) -> Option<Rc<dyn WidgetKind>> {
        self.kinds.get(id).cloned()
    }

    /// Every kind, by name.
    pub fn all(&self) -> Vec<Rc<dyn WidgetKind>> {
        let mut v: Vec<_> = self.kinds.values().cloned().collect();
        v.sort_by_key(|k| k.name());
        v
    }

    /// The default settings of kind `id`.
    pub fn defaults(&self, id: &str) -> Map<String, Value> {
        self.get(id)
            .map(|k| k.settings().into_iter().map(|s| (s.key, s.default)).collect())
            .unwrap_or_default()
    }
}

/// A setting's value, or its default.
pub fn value<'a>(settings: &'a Map<String, Value>, spec: &'a SettingSpec) -> &'a Value {
    settings.get(&spec.key).unwrap_or(&spec.default)
}

fn text_setting(settings: &Map<String, Value>, key: &str) -> String {
    settings.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn spec(key: &str, label: &str, kind: SettingKind, default: Value) -> SettingSpec {
    SettingSpec { key: key.into(), label: label.into(), kind, default }
}

/// A title and some text: news, rules, a welcome.
struct TextKind;

impl WidgetKind for TextKind {
    fn id(&self) -> &str {
        "text"
    }
    fn name(&self) -> String {
        "Text".into()
    }
    fn settings(&self) -> Vec<SettingSpec> {
        vec![
            spec("title", "Title", SettingKind::Text, "Welcome".into()),
            spec("body", "Text", SettingKind::LongText, "Say hello to the house.".into()),
        ]
    }
    fn build(&self, ui: &Ui, _id: &str, s: &Map<String, Value>, _cx: &Context) -> El {
        let unit = ui.theme().spacing.unit;
        let title = s.get("title").and_then(Value::as_str).unwrap_or("Welcome").to_owned();
        let body = s.get("body").and_then(Value::as_str).unwrap_or_default().to_owned();
        let mut col = w::column(ui).gap(unit * 2.0).fill_width();
        if !title.is_empty() {
            col = col.child(w::heading(ui, 3, title));
        }
        for para in body.split("\n\n").filter(|p| !p.trim().is_empty()) {
            col = col.child(w::text(ui, para.trim().to_owned()));
        }
        col
    }
}

/// A list of links, one `Label | https://…` a line.
struct LinksKind;

/// The links in a links widget's text: `Label | url` or a bare url a line; only links the
/// system may open (web pages and mail) are kept.
pub fn parse_links(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (label, url) = match l.split_once('|') {
                Some((a, b)) => (a.trim().to_owned(), b.trim().to_owned()),
                None => (l.to_owned(), l.to_owned()),
            };
            remedy_ui_host::is_openable_url(&url)
                .then(|| (if label.is_empty() { url.clone() } else { label }, url))
        })
        .collect()
}

impl WidgetKind for LinksKind {
    fn id(&self) -> &str {
        "links"
    }
    fn name(&self) -> String {
        "Links".into()
    }
    fn settings(&self) -> Vec<SettingSpec> {
        vec![
            spec("title", "Title", SettingKind::Text, "Links".into()),
            spec(
                "links",
                "Links (Label | https://…, one a line)",
                SettingKind::LongText,
                Value::String(String::new()),
            ),
        ]
    }
    fn build(&self, ui: &Ui, _id: &str, s: &Map<String, Value>, cx: &Context) -> El {
        let unit = ui.theme().spacing.unit;
        let mut col = w::column(ui).gap(unit).fill_width();
        let title = text_setting(s, "title");
        if !title.is_empty() {
            col = col.child(w::heading(ui, 3, title));
        }
        let links = parse_links(&text_setting(s, "links"));
        if links.is_empty() {
            col = col.child(w::label(ui, "No links yet.", "muted"));
        }
        for (label, url) in links {
            let open = cx.open_link.clone();
            col = col.child(w::link(ui, label, move || open(&url)));
        }
        col
    }
}

/// Reactions on their own (a poll of feelings, a guestbook of emoji).
struct ReactionsKind;

impl WidgetKind for ReactionsKind {
    fn id(&self) -> &str {
        "reactions"
    }
    fn name(&self) -> String {
        "Reactions".into()
    }
    fn settings(&self) -> Vec<SettingSpec> {
        vec![
            spec("title", "Question", SettingKind::Text, "How is everyone?".into()),
            spec("choices", "Reactions to offer", SettingKind::Text, "👍 ❤️ 😂 🎉".into()),
        ]
    }
    fn build(&self, ui: &Ui, id: &str, s: &Map<String, Value>, cx: &Context) -> El {
        let unit = ui.theme().spacing.unit;
        let offered: Vec<String> = text_setting(s, "choices").split_whitespace().map(str::to_owned).collect();
        let shown = (cx.reactions)(id);
        let (react, wid) = (cx.react.clone(), id.to_owned());
        let mut col = w::column(ui).gap(unit * 2.0).fill_width();
        let title = text_setting(s, "title");
        if !title.is_empty() {
            col = col.child(w::text(ui, title));
        }
        col.child(crate::reactions::reactions(ui, shown, offered, move |e| react(&wid, e)))
    }
}

/// A kind written as a script. Each placed widget gets a machine of its own.
pub struct ScriptKind {
    id: String,
    source: Rc<str>,
    name: String,
    settings: Vec<SettingSpec>,
}

impl std::fmt::Debug for ScriptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptKind").field("id", &self.id).finish()
    }
}

impl ScriptKind {
    /// A kind `id` from `source` (checked by loading it once with `limits`).
    pub fn new(id: &str, source: &str, limits: Limits) -> Result<ScriptKind, remedy_ui_script::ScriptError> {
        let w = ScriptWidget::load(source, limits)?;
        Ok(ScriptKind {
            id: id.to_owned(),
            source: Rc::from(source),
            name: w.name().to_owned(),
            settings: w.settings().to_vec(),
        })
    }
}

impl WidgetKind for ScriptKind {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn settings(&self) -> Vec<SettingSpec> {
        self.settings.clone()
    }
    fn build(&self, ui: &Ui, _id: &str, settings: &Map<String, Value>, cx: &Context) -> El {
        let holder = w::column(ui).fill_width();
        let widget = match ScriptWidget::load(&self.source, cx.limits) {
            Ok(w) => Rc::new(RefCell::new(w)),
            Err(e) => return holder.child(failed(ui, &e.to_string())),
        };
        // Rendered again after each button press, and when the host's data changes.
        let version = Signal::new(0u64);
        let (h2, u2, settings, host, open) =
            (holder.clone(), ui.clone(), settings.clone(), cx.host, cx.open_link.clone());
        holder.bind(move || {
            let _ = version.get();
            let data = host.get();
            untrack(|| {
                h2.clear();
                let shown = widget.borrow_mut().render(&settings, &data);
                let el = match shown {
                    Ok(node) => build_node(&u2, &node, &widget, version, &open),
                    Err(e) => failed(&u2, &e.to_string()),
                };
                let _ = h2.clone().child(el);
            });
        })
    }
}

fn failed(ui: &Ui, why: &str) -> El {
    w::label(ui, format!("This widget stopped: {why}"), "error")
}

/// Builds elements from what a script described.
pub fn build_node(
    ui: &Ui,
    node: &Node,
    widget: &Rc<RefCell<ScriptWidget>>,
    version: Signal<u64>,
    open: &Rc<dyn Fn(&str)>,
) -> El {
    let unit = ui.theme().spacing.unit;
    match node {
        Node::Column { gap, children } => {
            children.iter().fold(w::column(ui).gap(*gap).fill_width(), |c, n| {
                c.child(build_node(ui, n, widget, version, open))
            })
        }
        Node::Row { gap, children } => children
            .iter()
            .fold(w::wrap(ui).gap(*gap), |c, n| c.child(build_node(ui, n, widget, version, open))),
        Node::Text { text, style } => match style {
            TextStyle::Body => w::text(ui, text.clone()),
            TextStyle::Muted => w::label(ui, text.clone(), "muted"),
            TextStyle::Strong => w::label(ui, text.clone(), "strong"),
            TextStyle::Heading(l) => w::heading(ui, *l + 1, text.clone()),
        },
        Node::Button { label, action } => {
            let (wd, a, u2) = (widget.clone(), *action, ui.clone());
            w::button(ui, label.clone(), move || {
                let r = wd.borrow_mut().act(a);
                match r {
                    Ok(()) => version.update(|v| *v += 1),
                    Err(e) => {
                        let _ = w::toast(
                            &u2,
                            format!("This widget stopped: {e}"),
                            std::time::Duration::from_secs(4),
                        );
                    }
                }
            })
        }
        Node::Link { label, url } => {
            if remedy_ui_host::is_openable_url(url) {
                let (open, url) = (open.clone(), url.clone());
                w::link(ui, label.clone(), move || open(&url))
            } else {
                // Not a web page or mail: shown, never opened.
                w::label(ui, label.clone(), "muted")
            }
        }
        Node::Progress { label, value } => {
            w::progress(ui, label.clone(), Signal::new(Some(*value))).fill_width()
        }
        Node::Divider => w::divider(ui, false).margin_y(unit),
    }
}

trait MarginY {
    fn margin_y(self, by: f32) -> El;
}

impl MarginY for El {
    fn margin_y(self, by: f32) -> El {
        self.layout(move |l| {
            l.margin.top = by;
            l.margin.bottom = by;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::parse_links;

    #[test]
    fn links_keep_only_what_may_be_opened() {
        let v = parse_links(
            "Site | https://example.com\nmailto:a@b.c\nEvil | file:///etc/passwd\nRun | javascript:alert(1)\n\n",
        );
        assert_eq!(
            v,
            [
                ("Site".to_owned(), "https://example.com".to_owned()),
                ("mailto:a@b.c".to_owned(), "mailto:a@b.c".to_owned())
            ]
        );
    }
}
