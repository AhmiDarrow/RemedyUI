//! The editing core: graphemes, words, undo grouping, IME, secrets.

use remedy_ui_text::*;

#[test]
fn backspace_and_delete_remove_whole_graphemes() {
    // A family emoji (four code points joined by ZWJ) and an e with a combining acute.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let text = format!("a{family}e\u{0301}b");
    let mut ed = TextEditor::new(&text);
    ed.set_selection(text.len() - 1, text.len() - 1); // before "b"
    ed.backspace(0);
    assert_eq!(ed.text(), format!("a{family}b"), "e + combining accent goes as one");
    ed.backspace(10_000);
    assert_eq!(ed.text(), "ab", "the whole family goes as one");
    ed.set_selection(0, 0);
    ed.delete_forward(20_000);
    assert_eq!(ed.text(), "b");
}

#[test]
fn arrows_step_over_graphemes_and_collapse_selections() {
    let text = "x\u{1F44D}\u{1F3FD}y"; // thumbs up + skin tone
    let mut ed = TextEditor::new(text);
    ed.set_selection(1, 1);
    ed.move_char(true, false);
    assert_eq!(ed.selection().focus, text.len() - 1, "the emoji and its modifier are one step");
    ed.select_all();
    ed.move_char(false, false);
    assert_eq!(ed.selection(), TextSelection::caret(0), "Left collapses to the start");
    ed.move_char(true, true);
    ed.move_char(true, true);
    assert_eq!(ed.selected_text(), "x\u{1F44D}\u{1F3FD}");
}

#[test]
fn word_motion_skips_spaces_and_punctuation() {
    let mut ed = TextEditor::new("Hello, brave new world");
    ed.move_to_edge(false, false);
    ed.move_word(true, false);
    assert_eq!(ed.selection().focus, 5);
    ed.move_word(true, false);
    assert_eq!(ed.selection().focus, 12, "past the comma and space to the end of brave");
    ed.move_to_edge(true, false);
    ed.move_word(false, false);
    assert_eq!(ed.selection().focus, 17, "start of world");
    assert_eq!(ed.word_at(8), 7..12);
    ed.delete_word(false, 0);
    assert_eq!(ed.text(), "Hello, brave world", "from the start of 'world', the word before is 'new '");
}

#[test]
fn ctrl_backspace_deletes_the_previous_word() {
    let mut ed = TextEditor::new("one two three");
    ed.delete_word(false, 0);
    assert_eq!(ed.text(), "one two ");
    ed.delete_word(false, 1);
    assert_eq!(ed.text(), "one ");
}

#[test]
fn typing_groups_into_word_sized_undo_steps() {
    let mut ed = TextEditor::new("");
    let mut t = 0;
    for c in "hello world".chars() {
        ed.insert(&c.to_string(), t);
        t += 100;
    }
    assert_eq!(ed.text(), "hello world");
    assert!(ed.undo());
    assert_eq!(ed.text(), "hello", "the second word (with its space) is one step");
    assert!(ed.undo());
    assert_eq!(ed.text(), "");
    assert!(!ed.undo());
    assert!(ed.redo() && ed.redo());
    assert_eq!(ed.text(), "hello world");
    assert_eq!(ed.selection(), TextSelection::caret(11));
}

#[test]
fn a_pause_starts_a_new_undo_step() {
    let mut ed = TextEditor::new("");
    ed.insert("a", 0);
    ed.insert("b", 100);
    ed.insert("c", 100 + UNDO_PAUSE_MS + 1);
    ed.undo();
    assert_eq!(ed.text(), "ab");
}

#[test]
fn backspacing_groups_and_restores_in_order() {
    let mut ed = TextEditor::new("abcdef");
    for i in 0..3 {
        ed.backspace(i * 50);
    }
    assert_eq!(ed.text(), "abc");
    ed.undo();
    assert_eq!(ed.text(), "abcdef");
    assert_eq!(ed.selection(), TextSelection::caret(6));
}

#[test]
fn paste_and_typing_replace_the_selection() {
    let mut ed = TextEditor::new("pick a word here");
    ed.set_selection(7, 11);
    ed.paste("phrase", 0);
    assert_eq!(ed.text(), "pick a phrase here");
    ed.set_selection(0, 4);
    ed.insert("P", 10_000);
    assert_eq!(ed.text(), "P a phrase here");
    ed.undo();
    assert_eq!(ed.text(), "pick a phrase here");
    ed.set_selection(0, 4);
    assert_eq!(ed.cut(20_000), "pick");
    assert_eq!(ed.text(), " a phrase here");
}

#[test]
fn ime_composition_shows_inline_and_commits_as_one_step() {
    let mut ed = TextEditor::new("I said ");
    ed.set_composition("ni", 2);
    ed.set_composition("nih", 3);
    let (shown, underline) = ed.display_text();
    assert_eq!(shown, "I said nih");
    assert_eq!(underline, Some(7..10));
    assert_eq!(ed.text(), "I said ", "not committed yet");
    ed.commit_composition("\u{4F60}\u{597D}", 0);
    assert_eq!(ed.text(), "I said \u{4F60}\u{597D}");
    assert!(ed.composition().is_none());
    ed.undo();
    assert_eq!(ed.text(), "I said ");
    ed.set_composition("x", 1);
    ed.cancel_composition();
    assert_eq!(ed.display_text(), ("I said ".to_string(), None));
}

#[test]
fn edits_in_a_large_document_stay_fast() {
    let big = "lorem ipsum dolor sit amet ".repeat(40_000); // about 1 MB
    let mut ed = TextEditor::new(&big);
    let mid = big.len() / 2;
    let start = std::time::Instant::now();
    for i in 0..1000u64 {
        ed.set_selection(mid, mid);
        ed.insert("x", i * 2000);
        ed.backspace(i * 2000 + 1);
    }
    assert_eq!(ed.len(), big.len());
    assert!(start.elapsed().as_secs_f32() < 2.0, "{:?}", start.elapsed());
}

#[test]
fn secret_editing_and_privacy() {
    let mut s = SecretEditor::new();
    for c in ["h", "u", "n", "t", "e", "r", "2"] {
        s.insert(c);
    }
    s.backspace();
    assert_eq!(s.secret().expose(), "hunter");
    assert_eq!(s.masked_len(), 6);
    s.move_char(false, false);
    s.move_char(false, true);
    s.insert("X");
    assert_eq!(s.secret().expose(), "huntXr", "Left, then Shift+Left selects the 'e'");
    assert_eq!(format!("{:?}", s.secret()), "SecretString(6 bytes)", "Debug never shows the secret");
    let taken = s.take();
    assert_eq!(taken.expose(), "huntXr");
    assert!(s.secret().is_empty());
    // Many inserts force reallocation; the text stays right.
    let mut big = SecretString::new();
    for i in 0..500 {
        big.insert(big.len(), if i % 2 == 0 { "\u{e9}" } else { "a" });
    }
    big.remove(0..2); // the first é (two bytes)
    assert_eq!(big.len(), 748);
    assert!(big.expose().starts_with("a\u{e9}a"));
    big.clear();
    assert!(big.is_empty());
}

#[test]
fn long_clusters_and_runs_of_flags_move_as_whole_characters() {
    // A letter under 200 combining marks is one character, however long.
    let stacked = format!("a{}b", "e".to_owned() + &"\u{0301}".repeat(200));
    let ed = TextEditor::new(&stacked);
    assert_eq!(ed.next_grapheme(1), stacked.len() - 1);
    assert_eq!(ed.previous_grapheme(stacked.len() - 1), 1);
    // Flags pair from the start of their run: in 40 flags, a caret after the 37th flag's
    // first letter belongs at that flag's start.
    let flags = "\u{1F1E9}\u{1F1EA}".repeat(40);
    let mut ed = TextEditor::new(&flags);
    let flag = "\u{1F1E9}\u{1F1EA}".len();
    let half = "\u{1F1E9}".len();
    ed.set_selection(36 * flag + half, 36 * flag + half);
    assert_eq!(ed.selection().focus, 36 * flag);
    assert_eq!(ed.next_grapheme(36 * flag), 37 * flag);
    assert_eq!(ed.previous_grapheme(36 * flag), 35 * flag);
}
