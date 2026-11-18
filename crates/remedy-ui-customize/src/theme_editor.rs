//! The theme editor: every token the person can change, edited live with RemedyUI's own
//! widgets, contrast warnings, a preview, reset per token and for the whole theme, and
//! sharing (import and export).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, Insets};
use remedy_ui_host::FileDialog;
use remedy_ui_reactive::{Signal, untrack};
use remedy_ui_services_i18n::Decimal;
use remedy_ui_style::{Density, StylePatch};
use remedy_ui_view::{Align, El, Ui};
use remedy_ui_widgets_core as w;
use remedy_ui_widgets_forms::{NumberOptions, color_picker, number_input};

use crate::theme::{COLOR_ROLES, ImportPolicy, Refusal, RefusalKind, Token, TokenValue, contrast_warnings};
use crate::{Customizations, CustomizeError};

/// A button reading "Reset" whose accessible name says what it resets; dimmed while there is
/// nothing to reset.
pub(crate) fn reset_button(
    ui: &Ui,
    what: &str,
    active: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> El {
    w::button(ui, "Reset", on_press)
        .variant("quiet")
        .semantics(Semantics::new(Role::Button).label(format!("Reset {what}")).action(Action::Click))
        .disabled_dyn(move || !active())
}

/// A token's value in the person's theme now (tracked).
fn current(c: &Customizations, token: &Token) -> Option<TokenValue> {
    c.theme().and_then(|t| token.value_in(&t))
}

fn color_of(c: &Customizations, token: &Token) -> Color {
    match current(c, token) {
        Some(TokenValue::Color(col)) => col,
        _ => Color::TRANSPARENT,
    }
}

/// Shows what went wrong with a change in the editor's status line.
fn report(status: Signal<String>, r: Result<(), CustomizeError>) {
    if let Err(e) = r {
        status.set(format!("Could not change that: {e}"));
    }
}

/// A row: the token's name, its control, and its reset.
fn row(ui: &Ui, c: &Customizations, token: &Token, control: El, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let (c1, c2, t1, t2, u2) = (c.clone(), c.clone(), token.clone(), token.clone(), ui.clone());
    w::row(ui)
        .align(Align::Center)
        .gap(unit * 2.0)
        .fill_width()
        .child(w::text(ui, token.label()).grow(1.0))
        .child(control)
        .child(reset_button(
            ui,
            &token.label(),
            move || c1.is_customized(&t1),
            move || {
                report(status, c2.reset_token(&u2, &t2));
            },
        ))
}

fn color_row(ui: &Ui, c: &Customizations, token: &Token, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let label = token.label();
    let open = Signal::new(false);
    let (c1, t1) = (c.clone(), token.clone());
    let swatch = ui.el("swatch").width(unit * 6.0).height(unit * 6.0).style_dyn(move || {
        let mut v_ = StylePatch::default();
        v_.background = Some(color_of(&c1, &t1));
        v_.radius = Some(c1.theme().map_or(4.0, |t| t.radii.small));
        v_.border = c1.theme().map(|t| t.derive().line);
        v_.border_width = c1.theme().map(|t| t.stroke.max(1.0));
        v_
    });
    let (c2, t2) = (c.clone(), token.clone());
    let hex = w::text(ui, "").variant("muted").data_text().text_dyn(move || {
        let col = color_of(&c2, &t2);
        let h = col.to_hex();
        if col.a >= 0.999 { h.chars().take(7).collect() } else { h }
    });
    let u3 = ui.clone();
    let toggle = w::button(ui, "Edit", move || {
        open.update(|o| *o = !*o);
        // Whatever was dragged in the picker was one change; the next starts another.
        u3.end_merge();
    })
    .variant("quiet");
    let t2b = toggle.clone();
    let l2 = label.clone();
    let toggle = toggle.bind(move || {
        let o = open.get();
        let _ = t2b.clone().semantics(
            Semantics::new(Role::Button).label(format!("Edit {l2}")).action(Action::Click).action(if o {
                Action::Collapse
            } else {
                Action::Expand
            }),
        );
    });
    let head = w::row(ui)
        .align(Align::Center)
        .gap(unit * 2.0)
        .fill_width()
        .child(swatch)
        .child(w::text(ui, label.clone()).grow(1.0))
        .child(hex)
        .child(toggle);
    let (c4, t4, u4) = (c.clone(), token.clone(), ui.clone());
    let head = head.child(reset_button(ui, &label, move || c4.is_customized(&t4), {
        let (c5, t5, u5) = (c.clone(), token.clone(), ui.clone());
        move || report(status, c5.reset_token(&u5, &t5))
    }));
    let body = w::column(ui).fill_width();
    let b2 = body.clone();
    let (c6, t6) = (c.clone(), token.clone());
    let body = body.bind(move || {
        let o = open.get();
        untrack(|| {
            b2.clear();
            if !o {
                return;
            }
            // Text roles are read against the background, text on the accent against it.
            let against = c6.theme().map(|t| match t6.path().get(1).map(String::as_str) {
                Some("on-accent") => t.colors.accent,
                Some("text" | "muted" | "danger" | "accent" | "line") => t.colors.bg,
                _ => t.colors.text,
            });
            let value = Signal::new(color_of(&c6, &t6));
            let picker = color_picker(&u4, t6.label(), value, against);
            let (c7, t7, c8, t8, u8) = (c6.clone(), t6.clone(), c6.clone(), t6.clone(), u4.clone());
            // Outside changes (undo, reset) show in the picker…
            let picker = picker.bind(move || {
                let now = color_of(&c7, &t7);
                if value.get_untracked().ok() != Some(now) {
                    value.set(now);
                }
            });
            // …and the picker's changes apply live.
            let picker = picker.bind(move || {
                let v = value.get();
                if untrack(|| color_of(&c8, &t8)) != v {
                    untrack(|| report(status, c8.set_token(&u8, &t8, TokenValue::Color(v))));
                }
            });
            let _ = b2.clone().child(picker);
        });
    });
    w::column(ui).gap(unit).fill_width().child(head).child(body)
}

fn number_row(
    ui: &Ui,
    c: &Customizations,
    token: &Token,
    precision: u8,
    unit_name: &str,
    status: Signal<String>,
) -> El {
    let read = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Px(v)) => f64::from(v),
            Some(TokenValue::Duration(d)) => d.as_secs_f64() * 1000.0,
            _ => 0.0,
        }
    };
    let to_dec = move |v: f64| Decimal::from_f64(v, precision).unwrap_or(Decimal::ZERO);
    let value = Signal::new(to_dec(read()));
    let options = {
        let mut v_ = NumberOptions::default();
        v_.min = Some(Decimal::ZERO);
        v_.max = None;
        v_.step = Decimal::ONE;
        v_.precision = precision;
        v_.unit = Some(unit_name.to_owned());
        v_
    };
    let field = number_input(ui, token.label(), value, options);
    let read2 = Rc::new(read);
    let r1 = read2.clone();
    let field = field.bind(move || {
        let now = to_dec(r1());
        if value.get_untracked().ok() != Some(now) {
            value.set(now);
        }
    });
    let (c2, t2, u2) = (c.clone(), token.clone(), ui.clone());
    let field = field.bind(move || {
        let v = value.get();
        if untrack(|| to_dec(read2())) == v {
            return;
        }
        let n = v.to_f64();
        let tv = match t2.kind() {
            crate::TokenKind::Duration => TokenValue::Duration(Duration::from_secs_f64(n.max(0.0) / 1000.0)),
            _ => TokenValue::Px(n as f32),
        };
        untrack(|| report(status, c2.set_token(&u2, &t2, tv)));
    });
    row(ui, c, token, field, status)
}

fn families_row(ui: &Ui, c: &Customizations, token: &Token, status: Signal<String>) -> El {
    let field = w::text_input(ui, token.label());
    let read = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Families(f)) => f.join(", "),
            _ => String::new(),
        }
    };
    let (f2, c2, t2, u2) = (field.clone(), c.clone(), token.clone(), ui.clone());
    let field = field.on_change(move || {
        let list: Vec<String> =
            f2.value().split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            report(status, c2.set_token(&u2, &t2, TokenValue::Families(list)));
        }
    });
    let (el, f3) = (field.el().width(ui.theme().spacing.unit * 60.0), field.clone());
    let el2 = el.clone();
    let el = el.bind(move || {
        let want = read();
        if !el2.is_focused() && f3.value() != want {
            untrack(|| f3.set_value(&want));
        }
    });
    row(ui, c, token, el, status)
}

fn name_row(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let token = Token::name();
    let field = w::text_input(ui, token.label());
    let (f2, c2, t2, u2) = (field.clone(), c.clone(), token.clone(), ui.clone());
    let field = field.on_change(move || {
        report(status, c2.set_token(&u2, &t2, TokenValue::Name(f2.value())));
    });
    let (el, f3, c3, t3) =
        (field.el().width(ui.theme().spacing.unit * 60.0), field.clone(), c.clone(), token.clone());
    let el2 = el.clone();
    let el = el.bind(move || {
        let want = match current(&c3, &t3) {
            Some(TokenValue::Name(n)) => n,
            _ => String::new(),
        };
        if !el2.is_focused() && f3.value() != want {
            untrack(|| f3.set_value(&want));
        }
    });
    row(ui, c, &token, el, status)
}

fn density_row(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let token = Token::density();
    let all = [Density::Compact, Density::Comfortable, Density::Spacious];
    let index = {
        let (c, token) = (c.clone(), token.clone());
        move || match current(&c, &token) {
            Some(TokenValue::Density(d)) => all.iter().position(|x| *x == d).unwrap_or(1),
            _ => 1,
        }
    };
    let chosen = Signal::new(index());
    let seg = w::segmented(ui, token.label(), &["Compact", "Comfortable", "Spacious"], chosen);
    let index = Rc::new(index);
    let i1 = index.clone();
    let seg = seg.bind(move || {
        let now = i1();
        if chosen.get_untracked().ok() != Some(now) {
            chosen.set(now);
        }
    });
    let (c2, t2, u2) = (c.clone(), token.clone(), ui.clone());
    let seg = seg.bind(move || {
        let i = chosen.get();
        if untrack(|| index()) != i
            && let Some(d) = all.get(i)
        {
            untrack(|| report(status, c2.set_token(&u2, &t2, TokenValue::Density(*d))));
        }
    });
    row(ui, c, &token, seg, status)
}

/// The pairs below WCAG contrast, live.
fn contrast_panel(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let list = w::column(ui).gap(unit).fill_width();
    let (l2, u, c2) = (list.clone(), ui.clone(), c.clone());
    let list = list.bind(move || {
        let warnings = c2.theme().map(|t| contrast_warnings(&t)).unwrap_or_default();
        untrack(|| {
            l2.clear();
            if warnings.is_empty() {
                let _ =
                    l2.clone().child(w::text(&u, "Every color pair meets WCAG contrast.").variant("muted"));
            }
            for wn in &warnings {
                let text = format!(
                    "{}: {:.1}:1, below the {:.1}:1 it needs ({} on {})",
                    wn.pair,
                    wn.ratio,
                    wn.needed,
                    wn.foreground.label(),
                    wn.background.label()
                );
                let _ = l2.clone().child(w::text(&u, text).variant("error"));
            }
        });
    });
    let c3 = c.clone();
    w::section(ui, "Contrast").child(list).semantics_dyn(move || {
        let n = c3.theme().map(|t| contrast_warnings(&t).len()).unwrap_or(0);
        let s = Semantics::new(Role::Status).label("Contrast");
        if n == 0 {
            s.value("Every color pair meets WCAG contrast")
        } else {
            s.value(format!("{n} color pairs are below WCAG contrast"))
        }
    })
}

/// Sample widgets in the live theme.
fn preview(ui: &Ui) -> El {
    let unit = ui.theme().spacing.unit;
    let on = Signal::new(true);
    let progress = Signal::new(Some(0.6f32));
    let field = w::text_input(ui, "Sample field");
    field.set_value("Text in a field");
    w::card(ui)
        .gap(unit * 2.0)
        .padding(Insets::all(unit * 3.0))
        .layout(|l| l.min_width = remedy_ui_layout::Length::Px(240.0))
        .semantics(Semantics::new(Role::Group).label("Theme preview"))
        .child(w::heading(ui, 2, "Preview"))
        .child(w::text(ui, "Body text reads like this, at the body size."))
        .child(w::text(ui, "Muted text, for hints and details.").variant("muted"))
        .child(
            w::wrap(ui)
                .gap(unit)
                .child(w::button(ui, "Primary", || {}).variant("primary"))
                .child(w::button(ui, "Secondary", || {}))
                .child(w::button(ui, "Danger", || {}).variant("danger")),
        )
        .child(w::switch(ui, "A switch", on))
        .child(field.el())
        .child(w::progress(ui, "Progress", progress))
        .child(w::link(ui, "A link", || {}))
        .child(w::text(ui, "An error message.").variant("error"))
}

/// A shared theme waiting for the person to decide about what it carries beyond tokens:
/// its file (when it came from one, so its files can be found), its text, and what was
/// refused.
type Pending = Option<(Option<PathBuf>, String, Vec<Refusal>)>;

/// Import and export.
fn sharing(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    let path = w::text_input(ui, "Theme file").placeholder("A path such as themes/ocean.json");
    let pending: Signal<Pending> = Signal::new(None);
    let import = {
        let (c, u) = (c.clone(), ui.clone());
        Rc::new(move |from: Option<PathBuf>, text: String, policy: ImportPolicy| {
            let done = match &from {
                Some(p) => c.import_theme_file(&u, p, policy),
                None => c.import_theme(&u, &text, policy),
            };
            match done {
                Ok(r) if r.dropped.is_empty() => {
                    pending.set(None);
                    status.set(format!("Imported the theme {}", r.name));
                }
                Ok(r) => {
                    pending.set(None);
                    status.set(format!(
                        "Imported the theme {}, leaving out {} part(s)",
                        r.name,
                        r.dropped.len()
                    ));
                }
                Err(CustomizeError::Refused(list)) => pending.set(Some((from, text, list))),
                Err(e) => status.set(format!("Could not import that theme: {e}")),
            }
        })
    };
    let read_and_import = {
        let f = import.clone();
        Rc::new(move |p: PathBuf| match std::fs::read_to_string(&p) {
            Ok(text) => f(Some(p), text, ImportPolicy::default()),
            Err(e) => status.set(format!("Could not read {}: {e}", p.display())),
        })
    };
    let export_to = {
        let c = c.clone();
        Rc::new(move |p: PathBuf| match c.export_theme(&p) {
            Ok(()) => status.set(format!("Exported the theme to {}", p.display())),
            Err(e) => status.set(format!("Could not export the theme: {e}")),
        })
    };
    let (p1, r1) = (path.clone(), read_and_import.clone());
    let import_btn = w::button(ui, "Import from path", move || {
        let p = p1.value();
        if p.trim().is_empty() {
            status.set("Write the theme file's path first".into());
        } else {
            r1(p.trim().into());
        }
    });
    let (p2, e2) = (path.clone(), export_to.clone());
    let export_btn = w::button(ui, "Export to path", move || {
        let p = p2.value();
        if p.trim().is_empty() {
            status.set("Write where to save the theme first".into());
        } else {
            e2(p.trim().into());
        }
    });
    let mut buttons = w::wrap(ui).gap(unit).child(import_btn).child(export_btn);
    if c.has_file_picker() {
        let (c1, r2) = (c.clone(), read_and_import.clone());
        let choose = w::button(ui, "Choose a theme to import…", move || {
            let r3 = r2.clone();
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = "Import a theme".into();
                v_.filters = vec![("Design tokens".into(), vec!["json".into()])];
                v_
            };
            c1.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    r3(p);
                }
            });
        });
        let (c2, e3) = (c.clone(), export_to.clone());
        let save = w::button(ui, "Export to a file…", move || {
            let e4 = e3.clone();
            let name = c2.theme().map_or_else(|| "theme".to_owned(), |t| t.name);
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = "Export the theme".into();
                v_.save = true;
                v_.file_name = Some(format!("{name}.json"));
                v_.filters = vec![("Design tokens".into(), vec!["json".into()])];
                v_
            };
            c2.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    e4(p);
                }
            });
        });
        buttons = buttons.child(choose).child(save);
    }
    // The decision about refused parts.
    let decide = w::column(ui).gap(unit).fill_width();
    let (d2, u) = (decide.clone(), ui.clone());
    let decide = decide.bind(move || {
        let p = pending.get();
        untrack(|| {
            d2.clear();
            let Some((from, text, list)) = p else { return };
            let has = |k: RefusalKind| list.iter().any(|r| r.kind == k);
            let allow_shaders = Signal::new(false);
            let allow_links = Signal::new(false);
            let allow_files = Signal::new(false);
            let mut box_ = w::column(&u)
                .gap(unit)
                .fill_width()
                .semantics(Semantics::new(Role::Alert).label("This theme carries more than colors and sizes"))
                .child(w::text(&u, "This theme carries more than colors and sizes. These are off unless you turn them on:").variant("strong"));
            for r in &list {
                box_ = box_.child(w::text(&u, r.to_string()).variant("muted").data_text());
            }
            if has(RefusalKind::Shader) {
                box_ = box_.child(w::switch(
                    &u,
                    "Allow shaders (code that runs on the graphics card)",
                    allow_shaders,
                ));
            }
            if has(RefusalKind::ExternalUrl) {
                box_ = box_.child(w::switch(&u, "Allow external links (kept, never fetched)", allow_links));
            }
            if has(RefusalKind::Asset) {
                box_ = box_.child(w::switch(&u, "Use the images and fonts it brings", allow_files));
            }
            let (f1, f2, t1, t2, from1, from2) =
                (import.clone(), import.clone(), text.clone(), text, from.clone(), from);
            let row = w::wrap(&u)
                .gap(unit)
                .child(
                    w::button(&u, "Import", move || {
                        f1(
                            from1.clone(),
                            t1.clone(),
                            ImportPolicy {
                                allow_shaders: allow_shaders.get_untracked().unwrap_or(false),
                                allow_external_urls: allow_links.get_untracked().unwrap_or(false),
                                allow_assets: allow_files.get_untracked().unwrap_or(false),
                                drop_refused: true,
                            },
                        )
                    })
                    .variant("primary"),
                )
                .child(w::button(&u, "Import without them", move || {
                    f2(
                        from2.clone(),
                        t2.clone(),
                        ImportPolicy { drop_refused: true, ..ImportPolicy::default() },
                    )
                }))
                .child(w::button(&u, "Cancel", move || pending.set(None)));
            let _ = d2.clone().child(box_.child(row));
        });
    });
    w::section(ui, "Share")
        .child(w::text(ui, "Themes are design-tokens files that every Remedy app can read."))
        .child(path.el().fill_width())
        .child(buttons)
        .child(decide)
}

/// The files the theme brings: a background image and fonts.
fn files_section(ui: &Ui, c: &Customizations, status: Signal<String>) -> El {
    let unit = ui.theme().spacing.unit;
    // The background.
    let c0 = c.clone();
    let shown = w::text(ui, "").variant("muted").data_text().text_dyn(move || {
        match c0.theme().and_then(|t| t.assets.background) {
            Some(r) if r.starts_with("data:") => "Background: an image inside the theme".to_owned(),
            Some(r) => format!("Background: {r}"),
            None => "No background image".to_owned(),
        }
    });
    let image_path =
        w::text_input(ui, "Background image file").placeholder("A path such as pictures/waves.png");
    let (c1, u1, p1) = (c.clone(), ui.clone(), image_path.clone());
    let use_image = w::button(ui, "Use this image", move || {
        let p = p1.value();
        if p.trim().is_empty() {
            status.set("Write the image's path first".into());
        } else {
            report(status, c1.set_background(&u1, Some(Path::new(p.trim()))));
        }
    });
    let (c2, u2, c3) = (c.clone(), ui.clone(), c.clone());
    let remove = reset_button(
        ui,
        "background image",
        move || c3.theme().is_some_and(|t| t.assets.background.is_some()),
        move || report(status, c2.set_background(&u2, None)),
    );
    let mut image_row =
        w::wrap(ui).gap(unit).align(Align::Center).child(image_path.el().grow(1.0)).child(use_image);
    if c.has_file_picker() {
        let (c4, u4) = (c.clone(), ui.clone());
        image_row = image_row.child(w::button(ui, "Choose an image…", move || {
            let (c5, u5) = (c4.clone(), u4.clone());
            let dialog = {
                let mut v_ = FileDialog::default();
                v_.title = "Choose a background image".into();
                v_.filters = vec![(
                    "Images".into(),
                    ["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"].map(String::from).to_vec(),
                )];
                v_
            };
            c4.pick(dialog, move |paths| {
                if let Some(p) = paths.into_iter().next() {
                    report(status, c5.set_background(&u5, Some(&p)));
                }
            });
        }));
    }
    // The fonts.
    let list = w::column(ui).gap(unit).fill_width();
    let (l2, u, c6) = (list.clone(), ui.clone(), c.clone());
    let list = list.bind(move || {
        let fonts: Vec<String> =
            c6.theme().map(|t| t.assets.fonts.keys().cloned().collect()).unwrap_or_default();
        untrack(|| {
            l2.clear();
            if fonts.is_empty() {
                let _ = l2.clone().child(w::text(&u, "The theme brings no fonts.").variant("muted"));
            }
            for family in fonts {
                let (c7, u7, f7) = (c6.clone(), u.clone(), family.clone());
                let remove = w::button(&u, "Remove", move || report(status, c7.remove_theme_font(&u7, &f7)))
                    .variant("quiet")
                    .semantics(
                        Semantics::new(Role::Button)
                            .label(format!("Remove font {family}"))
                            .action(Action::Click),
                    );
                let row = w::row(&u)
                    .gap(unit * 2.0)
                    .align(Align::Center)
                    .child(w::text(&u, family).grow(1.0))
                    .child(remove);
                let _ = l2.clone().child(row);
            }
        });
    });
    let font_path = w::text_input(ui, "Font file").placeholder("A path such as fonts/Ocean-Regular.ttf");
    let font_name = w::text_input(ui, "Font name").placeholder("What the font is called, such as Ocean");
    let (c8, u8, fp, fname) = (c.clone(), ui.clone(), font_path.clone(), font_name.clone());
    let add = w::button(ui, "Add font", move || {
        let (p, n) = (fp.value(), fname.value());
        if p.trim().is_empty() || n.trim().is_empty() {
            status.set("Write the font file's path and its name first".into());
        } else {
            report(status, c8.add_theme_font(&u8, n.trim(), Path::new(p.trim()), true));
        }
    });
    w::section(ui, "Files")
        .child(w::text(ui, "Images show behind every window, sized to it; fonts are checked before use."))
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(shown.grow(1.0)).child(remove))
        .child(image_row)
        .child(list)
        .child(
            w::wrap(ui)
                .gap(unit)
                .align(Align::Center)
                .child(font_path.el().grow(1.0))
                .child(font_name.el().grow(1.0))
                .child(add),
        )
}

/// The theme editor: colors (with a picker and contrast readout each, and warnings for pairs
/// below WCAG contrast), fonts and text sizes, spacing, radii, line width, motion and
/// density, each with its reset, applied live to every attached window, beside a preview;
/// import and export; and a reset for the whole theme. Every change is undoable.
#[track_caller]
pub fn theme_editor(ui: &Ui, c: &Customizations) -> El {
    let unit = ui.theme().spacing.unit;
    let Some(theme) = c.theme() else {
        return w::empty_state(
            ui,
            "Theme customization is off",
            "This app does not let its theme be changed.",
        );
    };
    let status = Signal::new(String::new());
    let mut colors = w::section(ui, "Colors");
    for (role, _) in COLOR_ROLES {
        colors = colors.child(color_row(ui, c, &Token::color(role), status));
    }
    for name in theme.colors.custom.keys() {
        colors = colors.child(color_row(ui, c, &Token::custom_color(name), status));
    }
    let px = |t: Token| number_row(ui, c, &t, 1, "px", status);
    let ms = |t: Token| number_row(ui, c, &t, 0, "ms", status);
    let text = w::section(ui, "Text")
        .child(name_row(ui, c, status))
        .child(families_row(ui, c, &Token::font_family(), status))
        .child(families_row(ui, c, &Token::mono_family(), status))
        .child(px(Token::size("small")))
        .child(px(Token::size("body")))
        .child(px(Token::size("title")));
    let shape = w::section(ui, "Spacing and shape")
        .child(px(Token::spacing_unit()))
        .child(px(Token::control_height()))
        .child(px(Token::radius("small")))
        .child(px(Token::radius("medium")))
        .child(px(Token::radius("large")))
        .child(px(Token::stroke()))
        .child(density_row(ui, c, status));
    let motion = w::section(ui, "Motion")
        .child(ms(Token::motion("quick")))
        .child(ms(Token::motion("standard")))
        .child(ms(Token::motion("emphasized")));
    let (c2, u2, c3) = (c.clone(), ui.clone(), c.clone());
    let reset_all = reset_button(
        ui,
        "theme",
        move || !c3.customized_tokens().is_empty(),
        move || {
            report(status, c2.reset_theme(&u2));
        },
    );
    let status_line = w::text(ui, "")
        .variant("muted")
        .text_dyn(move || status.get())
        .semantics_dyn(move || Semantics::new(Role::Status).label("Theme status").value(status.get()));
    let controls = w::column(ui)
        .gap(unit * 4.0)
        .grow(1.0)
        .layout(|l| l.min_width = remedy_ui_layout::Length::Px(280.0))
        .child(colors)
        .child(contrast_panel(ui, c))
        .child(text)
        .child(shape)
        .child(motion)
        .child(files_section(ui, c, status))
        .child(sharing(ui, c, status))
        .child(w::row(ui).gap(unit * 2.0).align(Align::Center).child(reset_all).child(status_line));
    w::column(ui)
        .gap(unit * 3.0)
        .fill_width()
        .semantics(Semantics::new(Role::Group).label("Theme editor"))
        .child(w::heading(ui, 2, "Theme"))
        .child(w::wrap(ui).gap(unit * 4.0).fill_width().child(controls).child(preview(ui)))
}
