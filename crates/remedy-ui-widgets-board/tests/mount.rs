//! A scripted widget outside a board: `mount_script` renders it where the app puts it, with
//! pictures and the app's own elements built through its hooks, handlers that take a value,
//! and a failure that keeps the last good picture.

#![cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::Size;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::script::{
    Action, Language, Limits, Node, ScriptError, ScriptWidget, SettingSpec, describe_with, serde_json,
};
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, FailurePhase, Ui};
use remedy_ui_widgets_board::*;
use serde_json::{Map, Value, json};

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
}

fn texts(ui: &Ui) -> Vec<String> {
    let mut out = Vec::new();
    for layer in ui.layers() {
        let mut stack = vec![layer];
        while let Some(e) = stack.pop() {
            stack.extend(e.child_elements());
            if let Some(n) = e.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)) {
                out.push(n);
            }
        }
    }
    out
}

/// A widget whose description is JSON kept by the test, recording what its handlers got.
struct Fixed {
    tree: Rc<RefCell<Value>>,
    got: Rc<RefCell<Vec<(Value, Value)>>>,
    last: Option<remedy_ui_engine::script::Described>,
}

impl ScriptWidget for Fixed {
    fn name(&self) -> &str {
        "Fixed"
    }
    fn settings(&self) -> &[SettingSpec] {
        &[]
    }
    fn limits(&self) -> Limits {
        Limits::default()
    }
    fn language(&self) -> Language {
        Language::new("test")
    }
    fn render(&mut self, _: &Map<String, Value>, host: &Value) -> Result<Node, ScriptError> {
        if host.get("fail").is_some() {
            return Err(ScriptError::Runtime("asked to fail".into()));
        }
        let d = describe_with(&self.tree.borrow(), &Limits::default(), &|k| k == "dial")?;
        let node = d.node.clone();
        self.last = Some(d);
        Ok(node)
    }
    fn act(&mut self, action: Action) -> Result<(), ScriptError> {
        self.act_with(action, &Value::Null)
    }
    fn act_with(&mut self, action: Action, value: &Value) -> Result<(), ScriptError> {
        let on = self
            .last
            .as_ref()
            .ok_or_else(|| ScriptError::Shape("nothing shown".into()))?
            .handler(action)?
            .clone();
        self.got.borrow_mut().push((on, value.clone()));
        Ok(())
    }
    fn memory_used(&self) -> usize {
        0
    }
}

#[test]
fn a_mounted_script_builds_pictures_and_app_elements_and_survives_a_failure() {
    let ui = ui();
    let tree = Rc::new(RefCell::new(json!({ "kind": "column", "children": [
        { "kind": "text", "text": "Hello" },
        { "kind": "image", "src": "art/sun.png", "alt": "The sun" },
        { "kind": "dial", "label": "Volume", "on_change": "turned" },
    ] })));
    let got = Rc::new(RefCell::new(Vec::new()));
    let widget: SharedWidget =
        Rc::new(RefCell::new(Box::new(Fixed { tree: tree.clone(), got: got.clone(), last: None })));
    let stops = Rc::new(RefCell::new(Vec::<(FailurePhase, String)>::new()));
    let fire: Rc<RefCell<Option<HandlerFn>>> = Rc::default();
    let (s2, f2) = (stops.clone(), fire.clone());
    let hooks = ScriptHooks::new(|_| {}, move |phase, e| s2.borrow_mut().push((phase, e.to_string())))
        .picture(|ui, src| remedy_ui_widgets_core::text(ui, format!("picture of {src}")))
        .element(move |ui, el: AppElement| {
            *f2.borrow_mut() = el.handler("on_change");
            let label = el.props.get("label").and_then(Value::as_str).unwrap_or_default().to_owned();
            remedy_ui_widgets_core::button(ui, label, || {})
        });
    let host = Signal::new(json!({}));
    let el: El = mount_script(&ui, widget, Map::new(), host, hooks);
    let _ = ui.root().child(el);
    frame(&ui);
    let seen = texts(&ui);
    assert!(seen.iter().any(|t| t == "Hello"), "{seen:?}");
    assert!(seen.iter().any(|t| t == "The sun"), "the picture is described: {seen:?}");
    assert!(seen.iter().any(|t| t == "Volume"), "the app built its element: {seen:?}");

    // The element's handler runs the script's with the value given, then renders again.
    if let Some(t) = tree.borrow_mut().pointer_mut("/children/0/text") {
        *t = json!("Turned");
    }
    let Some(f) = fire.borrow().clone() else { unreachable!("the dial has its handler") };
    f(json!(0.5));
    frame(&ui);
    assert_eq!(got.borrow().as_slice(), &[(json!("turned"), json!(0.5))]);
    assert!(texts(&ui).iter().any(|t| t == "Turned"));

    // A failed render is reported and the last good picture stays.
    host.set(json!({ "fail": true }));
    frame(&ui);
    assert_eq!(stops.borrow().len(), 1);
    assert_eq!(stops.borrow().first().map(|s| s.0), Some(FailurePhase::Build));
    assert!(texts(&ui).iter().any(|t| t == "Turned"), "still shown");
}
