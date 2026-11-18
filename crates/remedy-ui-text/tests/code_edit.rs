//! Code editing: several cursors at once, indentation that follows the code, and undo that
//! puts every cursor back.

use remedy_ui_text::{CodeEditor, TextSelection};

fn carets(e: &CodeEditor) -> Vec<usize> {
    let mut v: Vec<usize> = e.cursors().iter().map(|s| s.focus).collect();
    v.sort_unstable();
    v
}

#[test]
fn typing_at_several_cursors_is_one_step() {
    let mut e = CodeEditor::new("let a = 1;\nlet b = 2;\nlet c = 3;");
    e.set_selection(4, 4);
    e.add_cursor_vertically(true);
    e.add_cursor_vertically(true);
    assert_eq!(carets(&e), [4, 15, 26]);
    e.insert("x", 0);
    e.insert("y", 50);
    assert_eq!(e.text(), "let xya = 1;\nlet xyb = 2;\nlet xyc = 3;");
    assert_eq!(carets(&e), [6, 19, 32]);
    // Undo takes back one keystroke at all three, and the cursors with it.
    assert!(e.undo());
    assert_eq!(e.text(), "let xa = 1;\nlet xb = 2;\nlet xc = 3;");
    assert_eq!(carets(&e), [5, 17, 29]);
    assert!(e.redo());
    assert_eq!(e.text(), "let xya = 1;\nlet xyb = 2;\nlet xyc = 3;");
    // Backspace at all of them; Escape leaves one cursor.
    e.backspace(10_000);
    assert_eq!(e.text(), "let xa = 1;\nlet xb = 2;\nlet xc = 3;");
    e.single_cursor();
    assert_eq!(e.cursors().len(), 1);
}

#[test]
fn ctrl_d_selects_the_word_then_its_next_appearances() {
    let mut e = CodeEditor::new("foo(bar); foo(baz); foo(qux);");
    e.set_selection(1, 1);
    e.select_next_occurrence();
    assert_eq!(e.cursors(), [TextSelection { anchor: 0, focus: 3 }]);
    e.select_next_occurrence();
    e.select_next_occurrence();
    assert_eq!(e.cursors().len(), 3);
    e.insert("go", 0);
    assert_eq!(e.text(), "go(bar); go(baz); go(qux);");
    assert_eq!(e.selected_text(), "");
}

#[test]
fn enter_keeps_and_deepens_indentation_and_tab_indents_lines() {
    let mut e = CodeEditor::new("fn main() {}");
    e.set_selection(11, 11);
    e.enter(0);
    assert_eq!(e.text(), "fn main() {\n    \n}");
    assert_eq!(carets(&e), [16]);
    e.insert("go();", 0);
    e.enter(0);
    assert_eq!(e.text(), "fn main() {\n    go();\n    \n}");
    // Backspace in leading spaces takes off one level.
    e.backspace(0);
    assert_eq!(e.text(), "fn main() {\n    go();\n\n}");
    // Tab and Shift+Tab over selected lines.
    let mut e = CodeEditor::new("a\nb\nc");
    e.set_selection(0, 5);
    e.indent(true, 0);
    assert_eq!(e.text(), "    a\n    b\n    c");
    e.indent(false, 0);
    assert_eq!(e.text(), "a\nb\nc");
    assert!(e.undo());
    assert_eq!(e.text(), "    a\n    b\n    c");
}

#[test]
fn a_column_pasted_back_gives_one_line_per_cursor() {
    let mut e = CodeEditor::new("a\nb\nc");
    e.set_selection(1, 1);
    e.add_cursor_vertically(true);
    e.add_cursor_vertically(true);
    e.paste("1\n2\n3", 0);
    assert_eq!(e.text(), "a1\nb2\nc3");
    // Home goes to the first non-blank, then to the start.
    let mut e = CodeEditor::new("    x");
    e.set_selection(5, 5);
    e.move_line_edge(false, false);
    assert_eq!(carets(&e), [4]);
    e.move_line_edge(false, false);
    assert_eq!(carets(&e), [0]);
}
