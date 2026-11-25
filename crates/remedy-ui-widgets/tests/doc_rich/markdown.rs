//! Markdown on a real view with the theme sheet: structure screen readers can move through,
//! links that are real links, lists, tables, code, raw HTML kept as text, and live updates.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::Role;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::El;
use remedy_ui_widgets::doc_rich::*;

mod common;
use common::*;

const DOC: &str = "# Getting started

Welcome to **Remedy**. Read the [guide](https://example.org/guide) or *ask* in ~~chat~~ the forum.

## Steps

1. Install it
2. Open it
   - with a nested point
3. Use it

- [x] done
- [ ] not yet

> Quoted words
> on two lines

```rust
fn main() { println!(\"hi\"); }
```

| Name | Size |
|------|-----:|
| a.txt | 12 |
| b.txt | 7 |

<script>alert(1)</script>

---

Last line.
";

fn shown(source: &str) -> (remedy_ui_engine::view::Ui, El, Rc<RefCell<Vec<String>>>) {
    let ui = ui();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let o = opened.clone();
    let doc = markdown(&ui, source, move |dest| o.borrow_mut().push(dest.to_owned()));
    ui.root().child(ui.el("page").column().fill_width().child(doc.clone()));
    frame(&ui);
    (ui, doc, opened)
}

#[test]
fn headings_lists_quotes_tables_and_code_have_their_structure() {
    let (_ui, doc, _) = shown(DOC);
    let headings: Vec<(Role, String)> = with_role(&doc, Role::Heading(1))
        .into_iter()
        .chain(with_role(&doc, Role::Heading(2)))
        .map(|h| (h.info().and_then(|i| i.semantics).map(|s| s.role()).unwrap_or(Role::Label), name(&h)))
        .collect();
    assert_eq!(
        headings,
        [(Role::Heading(1), "Getting started".to_owned()), (Role::Heading(2), "Steps".to_owned())]
    );
    // Three lists: the steps, the nested point, the tasks.
    assert_eq!(with_role(&doc, Role::List).len(), 3);
    let markers: Vec<String> =
        find(&doc, "md_marker").iter().filter_map(|m| m.info().and_then(|i| i.text)).collect();
    assert_eq!(markers, ["1.", "2.", "\u{2022}", "3.", "\u{2022}", "\u{2022}"]);
    let all: Vec<String> = find(&doc, "label").iter().filter_map(|l| l.info().and_then(|i| i.text)).collect();
    assert!(all.iter().any(|t| t == "\u{2611} done"), "{all:?}");
    assert!(all.iter().any(|t| t == "\u{2610} not yet"), "{all:?}");
    assert!(all.iter().any(|t| t == "Quoted words on two lines"), "{all:?}");
    // Raw HTML is text, never markup.
    assert!(all.iter().any(|t| t.contains("<script>alert(1)</script>")), "{all:?}");
    // The table: a header row and two rows of cells; the size column is right-aligned.
    let heads: Vec<String> = with_role(&doc, Role::ColumnHeader).iter().map(name).collect();
    assert_eq!(heads, ["Name", "Size"]);
    let cells: Vec<String> = with_role(&doc, Role::Cell).iter().map(name).collect();
    assert_eq!(cells, ["a.txt", "12", "b.txt", "7"]);
    let size = with_role(&doc, Role::Cell).into_iter().nth(1).unwrap_or_else(|| unreachable!("a size cell"));
    let table = first(&doc, "md_table").bounds().unwrap_or_default();
    let b = size.bounds().unwrap_or_default();
    assert!(b.max.x > table.center().x, "the number sits in the right column: {b:?} in {table:?}");
    // Code keeps its lines, in the monospace face, and says what it is.
    let code = first(&doc, "md_code");
    assert_eq!(code.info().and_then(|i| i.text).as_deref(), Some("fn main() { println!(\"hi\"); }"));
    assert!(code.info().is_some_and(|i| i.style.mono));
    let groups: Vec<String> = with_role(&doc, Role::Group).iter().map(name).collect();
    assert!(groups.contains(&"Code, rust".to_owned()), "{groups:?}");
    assert!(groups.contains(&"Quote".to_owned()), "{groups:?}");
    assert_eq!(find(&doc, "divider").len(), 1);
}

#[test]
fn quotes_nested_without_end_keep_their_words_in_a_tree_of_bounded_depth() {
    let (_ui, doc, _) = shown(&format!("{} deep words", ">".repeat(400)));
    let quotes = find(&doc, "md_quote").len();
    assert!((13..=64).contains(&quotes), "{quotes} quotes inside one another");
    assert!(texts(&doc).iter().any(|t| t == "deep words"), "{:?}", texts(&doc));
}

#[test]
fn lists_nested_without_end_keep_their_words_and_leave_no_empty_list() {
    let source: String = (0..40).map(|i| format!("{}- point {i}\n", "  ".repeat(i))).collect();
    let (_ui, doc, _) = shown(&source);
    let lists = with_role(&doc, Role::List);
    assert!((7..=32).contains(&lists.len()), "{} lists inside one another", lists.len());
    for list in &lists {
        assert!(!list.child_elements().is_empty(), "a list without items");
    }
    let said = texts(&doc);
    assert!(said.iter().any(|t| t == "point 0") && said.iter().any(|t| t == "point 39"), "{said:?}");
}

#[test]
fn styled_words_and_links_in_a_paragraph() {
    let (ui, doc, opened) = shown(DOC);
    let para = find(&doc, "label")
        .into_iter()
        .find(|l| l.info().and_then(|i| i.text).is_some_and(|t| t.starts_with("Welcome")))
        .unwrap_or_else(|| unreachable!("the paragraph"));
    assert_eq!(
        para.info().and_then(|i| i.text).as_deref(),
        Some("Welcome to Remedy. Read the guide or ask in chat the forum.")
    );
    let links = para.link_elements();
    let [guide] = links.as_slice() else { unreachable!("one link: {}", links.len()) };
    assert_eq!(name(guide), "guide");
    // Clicking it hands the destination to the app.
    click(&ui, centre(guide), Duration::from_secs(1));
    assert_eq!(*opened.borrow(), ["https://example.org/guide"]);
    // So does the keyboard.
    guide.focus();
    key(&ui, remedy_ui_core::input::Key::Enter, NONE);
    assert_eq!(opened.borrow().len(), 2);
}

#[test]
fn a_document_reads_well_and_passes_the_audit() {
    let (ui, _doc, _) = shown(DOC);
    let found = audit(&ui);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn plain_text_reads_like_the_document() {
    let t = plain_text("# Hi\n\nSome *words* and [a link](x).\n\n- one\n- two");
    assert_eq!(t, "Hi\nSome words and a link.\none\ntwo");
}

#[test]
fn a_live_document_follows_its_source() {
    let ui = ui();
    let src = Signal::new("Thinking".to_owned());
    let doc = markdown(&ui, move || src.get(), |_| {});
    ui.root().child(doc.clone());
    frame(&ui);
    assert_eq!(texts(&doc), ["Thinking"]);
    src.set("Here is **the answer**:\n\n- one".to_owned());
    frame(&ui);
    assert_eq!(texts(&doc), ["Here is the answer:", "one"]);
}

#[test]
fn headings_have_ids_and_a_markdown_element_scrolls_to_them() {
    assert_eq!(heading_id("What's new?"), "whats-new");
    assert_eq!(heading_id("  Größe & Maße 2 "), "größe--maße-2");
    let source = "# Guide\n\n## Setup\n\ntext\n\n## Setup\n\n### `cargo` use\n";
    let list = headings(source);
    let ids: Vec<&str> = list.iter().map(|(_, _, id)| id.as_str()).collect();
    assert_eq!(ids, ["guide", "setup", "setup-1", "cargo-use"]);
    assert_eq!(list.first().map(|(level, text, _)| (*level, text.as_str())), Some((1, "Guide")));
    let ui = ui();
    let doc = markdown(&ui, source, |_| {});
    let _ = ui.root().child(doc.clone());
    assert!(scroll_to_heading(&doc, "#setup-1"));
    assert!(!scroll_to_heading(&doc, "nowhere"));
}
