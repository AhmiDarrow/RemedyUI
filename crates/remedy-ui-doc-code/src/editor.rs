//! The code editor: a code field (several cursors, indentation that follows the code, lines
//! never wrapped) with line numbers, colored as it is typed.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_text::RichText;
use remedy_ui_view::{CodeColors, El, FieldKind, TextField, Ui};
use remedy_ui_widgets_core as w;

use crate::highlight::{Highlighter, Kind};
use crate::view::CodeOptions;

/// A code editor holding `text`. Returns its element and its field (for its text, cursors
/// and changes).
#[track_caller]
pub fn code_editor(ui: &Ui, label: impl Into<String>, text: &str, options: CodeOptions) -> (El, TextField) {
    let label = label.into();
    let field = ui.text_field("code_input", FieldKind::Code).label(label.clone());
    field.set_value(text);
    let _ = field.edit_code(|e| e.set_selection(0, 0));
    // Colors: only the lines an edit touched are colored again.
    let highlighter = Rc::new(RefCell::new(Highlighter::new(&options.language)));
    let colors: CodeColors = Rc::new(move |text: &str, theme: &remedy_ui_style::Theme| {
        let mut h = highlighter.borrow_mut();
        let mut out = Vec::new();
        let mut start = 0;
        for (line, runs) in text.split('\n').zip(h.update(text)) {
            for (r, k) in runs {
                if *k != Kind::Plain {
                    out.push((start + r.start..start + r.end, k.color(theme)));
                }
            }
            start += line.len() + 1;
        }
        out
    });
    field.set_code_colors(colors);
    let unit = ui.theme().spacing.unit;
    let row = ui.el("code_editor_row").row().fill_width();
    if options.line_numbers {
        // The gutter's numbers follow the line count.
        let gutter = ui.el("code_editor_gutter").column().semantics(Semantics::new(Role::Group).hidden(true));
        let shown = Rc::new(std::cell::Cell::new(0usize));
        let (g2, f2, ui2) = (gutter.clone(), field.clone(), ui.clone());
        let renumber = move || {
            let lines = f2.value().split('\n').count();
            if shown.replace(lines) == lines {
                return;
            }
            g2.clear();
            let digits = lines.to_string().len();
            for n in 1..=lines {
                let _ = g2.clone().child(
                    ui2.el("code_gutter").text(RichText::plain(format!("{n:>digits$}"))).data_text().nowrap(),
                );
            }
        };
        renumber();
        field.observe_text(renumber);
        let _ = row.clone().child(gutter.padding(remedy_ui_core::Insets {
            top: 0.0,
            end: unit * 2.0,
            bottom: 0.0,
            start: unit,
        }));
    }
    let row = row.child(field.el().grow(1.0));
    let scroller = w::scroll(ui).scroll_x().fill_width().grow(1.0).child(row);
    let el = ui.el("code_view").column().fill_width().grow(1.0).child(scroller);
    (el, field)
}
