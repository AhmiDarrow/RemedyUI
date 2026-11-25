//! Field-level tracking through `#[derive(Store)]`.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui_core::reactive::{Effect, Segment, Store, StoreError, batch};
use remedy_ui_macros::Store;

#[derive(Store, Clone, PartialEq, Debug)]
struct Message {
    body: String,
    edited: bool,
}

#[derive(Store, Clone, PartialEq, Debug)]
struct Room {
    name: String,
    messages: Vec<Message>,
}

fn counting(f: impl Fn() + 'static) -> Rc<Cell<u32>> {
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    Effect::new(move || {
        f();
        r.set(r.get() + 1);
    });
    runs
}

fn room() -> Store<Room> {
    Store::new(Room {
        name: "general".into(),
        messages: vec![
            Message { body: "hello".into(), edited: false },
            Message { body: "world".into(), edited: false },
        ],
    })
}

#[test]
fn a_field_write_wakes_only_its_readers_and_their_ancestors() -> Result<(), StoreError> {
    let room = room();
    let r = room;
    let name_reader = counting(move || {
        let _ = r.name().get();
    });
    let r = room;
    let edited0 = counting(move || {
        let _ = r.messages().at(0).edited().get();
    });
    let r = room;
    let body0 = counting(move || {
        let _ = r.messages().at(0).body().get();
    });
    let r = room;
    let edited1 = counting(move || {
        let _ = r.messages().at(1).edited().get();
    });
    let r = room;
    let whole_list = counting(move || {
        let _ = r.messages().len();
    });

    room.messages().at(0).edited().set(true)?;

    assert_eq!(edited0.get(), 2, "the field itself");
    assert_eq!(whole_list.get(), 2, "an ancestor of the field");
    assert_eq!(body0.get(), 1, "a sibling field");
    assert_eq!(edited1.get(), 1, "the same field of another message");
    assert_eq!(name_reader.get(), 1, "an unrelated field");
    assert!(room.messages().at(0).edited().get()?);
    Ok(())
}

#[test]
fn replacing_a_parent_wakes_its_children() -> Result<(), StoreError> {
    let room = room();
    let r = room;
    let body1 = counting(move || {
        let _ = r.messages().at(1).body().get();
    });
    room.messages()
        .set(vec![Message { body: "a".into(), edited: false }, Message { body: "b".into(), edited: true }])?;
    assert_eq!(body1.get(), 2);
    assert_eq!(room.messages().at(1).body().get()?, "b");
    Ok(())
}

#[test]
fn set_if_changed_and_batching() -> Result<(), StoreError> {
    let room = room();
    let r = room;
    let name_reader = counting(move || {
        let _ = r.name().get();
    });
    room.name().set_if_changed("general".into())?;
    assert_eq!(name_reader.get(), 1);
    batch(|| {
        let _ = room.name().set("a".into());
        let _ = room.name().set("b".into());
    });
    assert_eq!(name_reader.get(), 2, "one run for the batch");
    Ok(())
}

#[test]
fn indexes_past_the_end_are_missing_not_panics() {
    let room = room();
    assert_eq!(room.messages().at(9).body().get(), Err(StoreError::Missing));
    assert_eq!(room.messages().at(9).edited().set(true), Err(StoreError::Missing));
    assert_eq!(
        room.messages().at(1).edited().path(),
        [Segment::Field(1), Segment::Index(1), Segment::Field(1)]
    );
}
