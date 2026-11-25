//! Code on screen: a code view (line numbers, colors, folding, selectable) and a diff view.
//! Lines are virtual, so a file of a million lines builds only what shows.

use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::Color;
use remedy_ui_core::reactive::Signal;
use remedy_ui_text::{RichText, TextStylePatch};
use remedy_ui_engine::view::{El, Ui};
use crate::base as w;

use crate::doc_code::diff::{DiffLine, diff_lines};
use crate::doc_code::highlight::{Kind, highlight};

/// How a code view shows its code.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeOptions {
    /// The language: a name ("Rust") or a file extension ("rs").
    pub language: String,
    /// Show line numbers.
    pub line_numbers: bool,
    /// Let blocks fold (by indentation) from their first line.
    pub folding: bool,
}

impl CodeOptions {
    /// Sets `language`: the language: a name ("Rust") or a file extension ("rs").
    pub fn language(mut self, value: String) -> Self {
        self.language = value;
        self
    }

    /// Sets `line_numbers`: show line numbers.
    pub fn line_numbers(mut self, value: bool) -> Self {
        self.line_numbers = value;
        self
    }

    /// Sets `folding`: let blocks fold (by indentation) from their first line.
    pub fn folding(mut self, value: bool) -> Self {
        self.folding = value;
        self
    }
}

impl Default for CodeOptions {
    fn default() -> Self {
        CodeOptions { language: String::new(), line_numbers: true, folding: true }
    }
}

/// A diff view's row: the line's number in the old text, in the new one, and the line.
type DiffRow = (Option<usize>, Option<usize>, Line);

/// One shown line: its number, its text with colors, and whether it starts a fold.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    number: usize,
    text: RichText,
    /// The lines this one folds away (`None` when it starts no block).
    fold: Option<Range<usize>>,
    /// Folded now.
    folded: bool,
    /// A diff line's mark: `+`, `-` or nothing.
    mark: Option<char>,
}

/// The text's lines with their kinds as colors.
fn colored(text: &str, language: &str, ui: &Ui) -> Vec<RichText> {
    let theme = ui.theme();
    let lines: Vec<&str> = text.split('\n').collect();
    highlight(text, language)
        .into_iter()
        .zip(lines)
        .map(|(runs, line)| {
            let mut rt = RichText::plain(line.to_owned());
            for (r, k) in runs {
                if k != Kind::Plain {
                    rt.add_span(r, TextStylePatch::color(k.color(&theme)));
                }
            }
            rt
        })
        .collect()
}

fn indent(line: &str) -> Option<usize> {
    if line.trim().is_empty() {
        return None;
    }
    Some(line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum())
}

/// For each line, the lines it folds away: the following lines indented deeper than it
/// (blank lines inside the block included, trailing blank lines left out).
fn folds(text: &str) -> Vec<Option<Range<usize>>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let levels: Vec<Option<usize>> = lines.iter().map(|l| indent(l)).collect();
    (0..lines.len())
        .map(|i| {
            let own = levels.get(i).copied().flatten()?;
            let mut end = i;
            for (j, lvl) in levels.iter().enumerate().skip(i + 1) {
                match lvl {
                    Some(l) if *l > own => end = j,
                    Some(_) => break,
                    None => {}
                }
            }
            (end > i).then_some(i + 1..end + 1)
        })
        .collect()
}

/// The lines to show, with folded blocks left out.
fn visible(colored: &[RichText], folds: &[Option<Range<usize>>], folded: &BTreeSet<usize>) -> Vec<Line> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(text) = colored.get(i) {
        let fold = folds.get(i).cloned().flatten();
        let is_folded = folded.contains(&i) && fold.is_some();
        out.push(Line {
            number: i + 1,
            text: text.clone(),
            fold: fold.clone(),
            folded: is_folded,
            mark: None,
        });
        i = match (is_folded, fold) {
            (true, Some(r)) => r.end,
            _ => i + 1,
        };
    }
    out
}

/// The gutter: a line's number, right-aligned, wide enough for the last line's.
fn gutter(ui: &Ui, number: Option<usize>, digits: usize) -> El {
    let text = number.map_or_else(String::new, |n| format!("{n:>digits$}"));
    ui.el("code_gutter").text(RichText::plain(text)).data_text().nowrap()
}

/// A code view of `source` (re-colored when it changes).
#[track_caller]
pub fn code_view(ui: &Ui, label: impl Into<String>, source: Signal<String>, options: CodeOptions) -> El {
    let folded: Signal<BTreeSet<usize>> = Signal::new(BTreeSet::new());
    let lines: Signal<Vec<Line>> = Signal::new(Vec::new());
    let digits = Signal::new(1usize);
    let ui2 = ui.clone();
    let opts = Rc::new(options);
    let o2 = opts.clone();
    let list = w::virtual_list(ui, label, lines, ui.theme().type_scale.body * 1.45, {
        let ui = ui.clone();
        move |_, line: &Line| {
            let unit = ui.theme().spacing.unit;
            let row = ui.el("code_line").row().fill_width().gap(unit * 2.0);
            let mut row = row;
            if o2.line_numbers {
                row = row.child(gutter(&ui, Some(line.number), digits.get_untracked().unwrap_or(1)));
            }
            if o2.folding {
                let toggle = match &line.fold {
                    Some(_) => {
                        let n = line.number - 1;
                        let id = if line.folded { "code-unfold-line" } else { "code-fold-line" };
                        let icon = if line.folded { w::Icon::ChevronRight } else { w::Icon::ChevronDown };
                        let (u, number) = (ui.clone(), line.number);
                        let label = move || crate::doc_code::STRINGS.format(&u, id, &[("line", number.into())]);
                        w::icon_button(&ui, icon, label, move || {
                            folded.update(|f| {
                                if !f.remove(&n) {
                                    f.insert(n);
                                }
                            });
                        })
                        .variant("code_fold")
                    }
                    None => ui.el("code_fold_space"),
                };
                row = row.child(toggle);
            }
            let text = ui.el("code_text").text(line.text.clone()).selectable(true).data_text().nowrap();
            row.child(text)
        }
    });
    let el = ui.el("code_view").column().fill_width().grow(1.0).child(list);
    el.bind(move || {
        let src = source.get();
        let f = folded.get();
        let c = colored(&src, &opts.language, &ui2);
        digits.set(c.len().to_string().len());
        let fl = if opts.folding { folds(&src) } else { vec![None; c.len()] };
        lines.set(visible(&c, &fl, &f));
    })
}

/// A diff view: `old` turned into `new`, line by line, with removed and added lines marked
/// (and named for screen readers), colored as `language`.
#[track_caller]
pub fn diff_view(ui: &Ui, label: impl Into<String>, old: &str, new: &str, language: &str) -> El {
    let old_c = colored(old, language, ui);
    let new_c = colored(new, language, ui);
    let pick = |v: &[RichText], n: usize| v.get(n - 1).cloned().unwrap_or_default();
    let lines: Vec<DiffRow> = diff_lines(old, new)
        .into_iter()
        .map(|d| match d {
            DiffLine::Same(_, a, b) => (
                Some(a),
                Some(b),
                Line { number: b, text: pick(&new_c, b), fold: None, folded: false, mark: None },
            ),
            DiffLine::Removed(_, a) => (
                Some(a),
                None,
                Line { number: a, text: pick(&old_c, a), fold: None, folded: false, mark: Some('-') },
            ),
            DiffLine::Added(_, b) => (
                None,
                Some(b),
                Line { number: b, text: pick(&new_c, b), fold: None, folded: false, mark: Some('+') },
            ),
        })
        .collect();
    let digits = old.split('\n').count().max(new.split('\n').count()).to_string().len();
    let items: Signal<Vec<DiffRow>> = Signal::new(lines);
    let list = w::virtual_list(ui, label, items, ui.theme().type_scale.body * 1.45, {
        let ui = ui.clone();
        move |_, (a, b, line): &DiffRow| {
            let unit = ui.theme().spacing.unit;
            let (variant, said) = match line.mark {
                Some('+') => ("added", "code-diff-added"),
                Some('-') => ("removed", "code-diff-removed"),
                _ => ("", "code-diff-unchanged"),
            };
            let mark = line.mark.map_or_else(|| " ".to_owned(), |c| c.to_string());
            let text = ui.el("code_text").text(line.text.clone()).selectable(true).data_text().nowrap();
            ui.el("code_line")
                .variant(variant)
                .row()
                .fill_width()
                .gap(unit * 2.0)
                .child(gutter(&ui, *a, digits))
                .child(gutter(&ui, *b, digits))
                .child(ui.el("code_mark").text(RichText::plain(mark)).nowrap())
                .child(text)
                .semantics({
                    let (u, code) = (ui.clone(), line.text.as_str().to_owned());
                    move || {
                        Semantics::new(Role::ListItem)
                            .label(crate::doc_code::STRINGS.format(&u, said, &[("text", code.as_str().into())]))
                            .action(Action::Focus)
                    }
                })
        }
    });
    ui.el("code_view").column().fill_width().grow(1.0).child(list)
}

/// The color code in `kind` gets under the view's theme, for apps drawing code themselves.
pub fn kind_color(ui: &Ui, kind: Kind) -> Color {
    kind.color(&ui.theme())
}
