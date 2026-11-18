//! Agents reach a window only after the person allows them in the trusted prompt.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui::a11y::Role;
use remedy_ui::agents::Scope;
use remedy_ui::testing::TestApp;

#[test]
fn an_agent_gets_a_token_only_when_the_person_allows_it() {
    let answers: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let a2 = answers.clone();
    let mut app = TestApp::window(move |ui, window| {
        ui.root().child(remedy_ui::widgets::button(ui, "Mute", || {}));
        let Some(agents) = window.attach_agents(None) else { unreachable!("the window is open") };
        let a3 = a2.clone();
        agents.request_access("Assistant", Scope::Read, None, move |t| a3.borrow_mut().push(t));
        let again = window.attach_agents(None).map(|a| a.granted().len());
        assert_eq!(again, Some(0), "the same access comes back, with nothing granted yet");
    });
    assert!(app.find(Role::Dialog, "Allow Assistant?").is_some(), "the prompt shows");
    // The buttons wake after a moment, so a press aimed at what was there cannot land on them.
    // That holds for assistive technology too.
    assert!(!app.press(Role::Button, "Allow"), "Allow is not pressable yet");
    assert!(answers.borrow().is_empty());
    app.host().advance(Duration::from_secs(2));
    app.frames(2);
    assert!(app.press(Role::Button, "Allow"));
    let token = answers.borrow().first().cloned().flatten();
    assert!(token.as_deref().is_some_and(|t| !t.is_empty()), "Allow hands over a token: {token:?}");
    let window = app.runner().app().windows().first().cloned();
    let granted = window.and_then(|w| w.agents()).map(|a| a.granted());
    assert_eq!(granted, Some(vec![("Assistant".to_owned(), Scope::Read)]));
    assert!(app.find(Role::Dialog, "Allow Assistant?").is_none(), "the prompt is gone");
}

#[test]
fn denying_hands_over_nothing() {
    let answers: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let a2 = answers.clone();
    let mut app = TestApp::window(move |_, window| {
        if let Some(agents) = window.attach_agents(None) {
            agents.request_access("Assistant", Scope::Act, Some(Duration::from_secs(600)), move |t| {
                a2.borrow_mut().push(t)
            });
        }
    });
    app.host().advance(Duration::from_secs(2));
    app.frames(2);
    assert!(app.press(Role::Button, "Deny"));
    assert_eq!(*answers.borrow(), vec![None]);
}
