//! Code views on a real view with the theme sheet: colors by kind, folding, diffs, grammars
//! added at run time, and what screen readers get.

use remedy_ui_core::Size;
use remedy_ui_doc_code::*;
use remedy_ui_reactive::Signal;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    for _ in 0..4 {
        let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
        for r in ui.take_requests() {
            if let Request::ScrollTo { scroller, offset } = r {
                ui.set_scroll_offset(scroller, offset);
            }
        }
    }
}

fn find(el: &El, widget: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

fn texts(el: &El) -> Vec<String> {
    find(el, "code_text").iter().filter_map(|t| t.info().and_then(|i| i.text)).collect()
}

const RUST: &str = "// says hi\nfn main() {\n    let n = 42;\n    println!(\"hi {n}\");\n}\n";

#[test]
fn rust_code_gets_its_kinds() {
    assert!(knows("rs") && knows("Rust"));
    let lines = highlight(RUST, "rs");
    let kind_at = |line: usize, word: &str| {
        let text = RUST.split('\n').nth(line).unwrap_or_default();
        let at = text.find(word).unwrap_or_default();
        lines.get(line).and_then(|runs| runs.iter().find(|(r, _)| r.contains(&at)).map(|(_, k)| *k))
    };
    assert_eq!(kind_at(0, "says"), Some(Kind::Comment));
    assert_eq!(kind_at(1, "fn"), Some(Kind::Keyword));
    assert_eq!(kind_at(1, "main"), Some(Kind::Function));
    assert_eq!(kind_at(2, "42"), Some(Kind::Constant));
    assert_eq!(kind_at(3, "hi"), Some(Kind::String));
    // An unknown language is plain, line for line.
    let plain = highlight("a\nb", "no-such-language");
    assert_eq!(plain.len(), 2);
    assert!(plain.iter().flatten().all(|(_, k)| *k == Kind::Plain));
}

#[test]
fn the_theme_colors_code_and_a_theme_token_overrides_it() {
    let mut theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let keyword = Kind::Keyword.color(&theme);
    assert_ne!(keyword, Kind::Comment.color(&theme));
    let pink = remedy_ui_core::Color::rgb8(255, 0, 170);
    theme.colors.custom.insert("code-keyword".into(), pink);
    assert_eq!(Kind::Keyword.color(&theme), pink);
}

#[test]
fn a_code_view_numbers_colors_and_folds() {
    let ui = ui();
    let src = Signal::new(RUST.to_owned());
    let view =
        code_view(&ui, "main.rs", src, CodeOptions { language: "rs".into(), ..CodeOptions::default() });
    ui.root().child(ui.el("col").column().height(600.0).fill_width().child(view.clone()));
    frame(&ui);
    assert_eq!(texts(&view).first().map(String::as_str), Some("// says hi"));
    let gutters: Vec<String> = find(&view, "code_gutter")
        .iter()
        .filter_map(|g| g.info().and_then(|i| i.text))
        .map(|t| t.trim().to_owned())
        .collect();
    assert_eq!(gutters.first().map(String::as_str), Some("1"));
    // Fold `fn main() {`: its body goes, the closing brace stays.
    let fold = find(&view, "icon_button").into_iter().next().unwrap_or_else(|| unreachable!("a fold button"));
    let name = fold.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned));
    assert_eq!(name.as_deref(), Some("Fold line 2"));
    let _ = ui.tree().dispatch(fold.id(), &remedy_ui_view::Click { position: None });
    frame(&ui);
    let shown = texts(&view);
    assert!(shown.contains(&"fn main() {".to_owned()), "{shown:?}");
    assert!(!shown.iter().any(|t| t.contains("let n")), "the body is folded away: {shown:?}");
    assert!(shown.contains(&"}".to_owned()));
    let audit = remedy_ui_devtools::audit(&ui);
    assert!(audit.is_empty(), "{:#?}", audit.iter().map(|p| p.message()).collect::<Vec<_>>());
}

#[test]
fn a_diff_view_marks_and_names_changes() {
    let ui = ui();
    let view = diff_view(&ui, "Changes", "a\nb\nc", "a\nB\nc\nd", "txt");
    ui.root().child(ui.el("col").column().height(600.0).fill_width().child(view.clone()));
    frame(&ui);
    let rows = find(&view, "code_line");
    let said: Vec<String> = rows
        .iter()
        .filter_map(|r| r.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)))
        .collect();
    assert_eq!(said, ["Unchanged: a", "Removed: b", "Added: B", "Unchanged: c", "Added: d"]);
    let variants: Vec<&str> = rows.iter().filter_map(|r| r.info().map(|i| i.variant)).collect();
    assert_eq!(variants, ["", "removed", "added", "", "added"]);
}

#[test]
fn an_apps_own_grammar_colors_its_language() {
    let grammar = r#"%YAML 1.2
---
name: Tiny
file_extensions: [tiny]
scope: source.tiny
contexts:
  main:
    - match: '\b(say)\b'
      scope: keyword.control.tiny
    - match: '"[^"]*"'
      scope: string.quoted.tiny
"#;
    assert_eq!(add_grammar(grammar).as_deref(), Ok("Tiny"));
    assert!(knows("tiny") && languages().contains(&"Tiny".to_owned()));
    let lines = highlight("say \"hello\"", "tiny");
    let kinds: Vec<Kind> = lines.iter().flatten().map(|(_, k)| *k).collect();
    assert!(kinds.contains(&Kind::Keyword) && kinds.contains(&Kind::String), "{lines:?}");
    assert!(add_grammar("not: [a grammar").is_err());
}

#[test]
fn highlighting_after_an_edit_colors_only_what_changed() {
    let mut src: String = (0..2000).map(|i| format!("fn f{i}() {{ let x = {i}; }}\n")).collect();
    let mut h = Highlighter::new("rs");
    h.update(&src);
    assert_eq!(h.last_work, 2001);
    // One line changed in the middle: a handful of lines colored, the same result as from scratch.
    src = src.replacen("let x = 1000;", "let y = 1000;", 1);
    let incremental: Vec<LineRuns> = h.update(&src).to_vec();
    assert!(h.last_work < 5, "{} lines colored again", h.last_work);
    assert_eq!(incremental, highlight(&src, "rs"));
    // Opening a block comment changes every line after it.
    src = src.replacen("fn f10()", "/* fn f10()", 1);
    let after: Vec<LineRuns> = h.update(&src).to_vec();
    assert!(h.last_work > 1900);
    assert_eq!(after, highlight(&src, "rs"));
}

#[test]
fn the_code_editor_types_at_several_cursors_and_numbers_its_lines() {
    use remedy_ui_input::{Key, KeyEvent, Modifiers};
    let ui = ui();
    let (el, field) = code_editor(
        &ui,
        "main.rs",
        "let a = 1;\nlet b = 2;",
        CodeOptions { language: "rs".into(), ..CodeOptions::default() },
    );
    ui.root().child(ui.el("col").column().height(400.0).fill_width().child(el.clone()));
    frame(&ui);
    field.el().focus();
    let key = |k: Key, text: Option<&str>, m: Modifiers| {
        ui.key(&KeyEvent {
            key: k,
            physical: String::new(),
            modifiers: m,
            pressed: true,
            repeat: false,
            composing: false,
            text: text.map(str::to_owned),
        });
    };
    let _ = field.edit_code(|e| e.set_selection(4, 4));
    key(Key::ArrowDown, None, Modifiers { ctrl: true, alt: true, ..Modifiers::NONE });
    key(Key::Character("z".into()), Some("z"), Modifiers::NONE);
    assert_eq!(field.value(), "let za = 1;\nlet zb = 2;");
    key(Key::Escape, None, Modifiers::NONE);
    assert_eq!(field.read_code(|e| e.cursors().len()), Some(1));
    // Enter keeps the line's indentation, and the gutter numbers follow.
    key(Key::End, None, Modifiers::NONE);
    key(Key::Enter, None, Modifiers::NONE);
    frame(&ui);
    let numbers: Vec<String> = find(&el, "code_gutter")
        .iter()
        .filter_map(|g| g.info().and_then(|i| i.text))
        .map(|t| t.trim().to_owned())
        .collect();
    assert_eq!(numbers, ["1", "2", "3"]);
    let audit = remedy_ui_devtools::audit(&ui);
    assert!(audit.is_empty(), "{:#?}", audit.iter().map(|p| p.message()).collect::<Vec<_>>());
}
