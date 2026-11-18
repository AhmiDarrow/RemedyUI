//! Chat feed: messages of any height in a chat list that follows the newest one, older
//! history loaded when the reader scrolls to the top, and a composer that sends on Enter
//! (Shift+Enter starts a new line).
//!
//! Run it with `cargo run -p remedy-ui --example chat_feed`.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::data::{ChatState, chat_list};
use remedy_ui::reactive::Signal;
use remedy_ui::view::Space;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// One message.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// Identifies the message across changes.
    pub id: u64,
    /// Who wrote it.
    pub author: String,
    /// The day it was sent: the list puts a header before each day's first message.
    pub day: String,
    /// What it says.
    pub text: String,
}

impl Message {
    fn new(id: u64, author: &str, day: &str, text: &str) -> Message {
        Message { id, author: author.to_owned(), day: day.to_owned(), text: text.to_owned() }
    }
}

/// The conversation so far: the latest messages shown, and older ones the list asks for
/// when the reader reaches the top.
pub fn conversation() -> (Vec<Message>, Vec<Message>) {
    let older = (1..=40)
        .map(|i| {
            Message::new(i, if i % 2 == 0 { "Ada" } else { "Grace" }, "Monday", &format!("Earlier note {i}"))
        })
        .collect();
    let latest = vec![
        Message::new(41, "Ada", "Tuesday", "Did the build go green?"),
        Message::new(42, "Grace", "Tuesday", "It did. The release notes are next."),
        Message::new(43, "Ada", "Tuesday", "I'll write them after lunch."),
    ];
    (latest, older)
}

/// Opens the chat window.
pub fn app(app: &App) {
    app.open(WindowSpec::new("Chat").size(420.0, 560.0), |ui, _| {
        let (latest, older) = conversation();
        ui.root().child(feed(ui, Signal::new(latest), older, "You"));
    });
}

/// The feed: the chat list over `messages`, filled from `older` as the reader scrolls up,
/// and a composer whose messages are signed by `me`.
pub fn feed(ui: &Ui, messages: Signal<Vec<Message>>, older: Vec<Message>, me: &'static str) -> El {
    let state = ChatState::new();
    // Older history waits here, newest last, until the list reaches the top.
    let archive = Rc::new(RefCell::new(older));
    let load_older = move || {
        let page: Vec<Message> = {
            let mut a = archive.borrow_mut();
            let from = a.len().saturating_sub(20);
            a.split_off(from)
        };
        if !page.is_empty() {
            messages.update(|m| {
                let mut all = page;
                all.append(m);
                *m = all;
            });
        }
    };
    let list = chat_list(
        ui,
        "Messages",
        messages,
        |m: &Message| m.id,
        |m: &Message| m.day.clone(),
        ui.theme().spacing.s(12),
        bubble,
        &state,
        load_older,
    );

    // Several lines, as people write in chats: Enter sends, Shift+Enter breaks the line.
    let composer = w::text_area(ui, "Message").placeholder("Write a message").submit_on_enter(true);
    let field = composer.clone();
    let send: Rc<dyn Fn()> = Rc::new(move || {
        let text = field.value();
        if text.trim().is_empty() {
            return;
        }
        messages.update(|m| {
            let id = m.last().map_or(1, |last| last.id + 1);
            let day = m.last().map_or_else(|| "Today".to_owned(), |last| last.day.clone());
            m.push(Message { id, author: me.to_owned(), day, text: text.trim().to_owned() });
        });
        field.set_value("");
        // Sending always shows what was sent.
        state.jump_to_latest();
    });
    let on_enter = send.clone();
    let composer = composer.on_submit(move || on_enter());
    let send_button = w::button(ui, "Send", move || send()).variant("primary");

    w::column(ui).grow(1.0).fill_width().child(list).child(
        w::row(ui)
            .gap(Space::S2)
            .padding(Space::S2)
            .fill_width()
            .child(composer.el().grow(1.0))
            .child(send_button),
    )
}

/// One message: the author over the text, which can be selected and copied.
pub fn bubble(ui: &Ui, m: &Message) -> El {
    w::column(ui)
        .gap(Space::S1)
        .padding(Space::S2)
        .fill_width()
        .child(w::text(ui, m.author.clone()).variant("strong").data_text())
        .child(w::text(ui, m.text.clone()).data_text().selectable(true))
}

fn main() {
    if let Err(e) = remedy_ui::run(AppConfig::new("Chat feed"), app) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
