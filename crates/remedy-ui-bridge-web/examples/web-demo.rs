//! RemedyUI in a browser: a button, a text field (input methods and soft keyboards), copy and
//! paste, a link, open and save dialogs with dropped files, the renderer, and the safe area
//! shown on screen. Installable as a PWA.
//!
//! `tools/build-web.sh` builds it into a page (with its service worker and manifest). It is a
//! WebAssembly module the page loads, so it does nothing on other platforms.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod demo {
    use remedy_ui::host::FileDialog;
    use remedy_ui::reactive::Signal;
    use remedy_ui::widgets as w;
    use remedy_ui::{App, AppConfig, WindowSpec};

    fn open_files(app: &App, status: Signal<String>) {
        let dialog = {
            let mut v_ = FileDialog::default();
            v_.title = "Open files".into();
            v_.multiple = true;
            v_
        };
        app.file_dialog(None, dialog, move |paths| {
            let sizes: Vec<String> = paths
                .iter()
                .map(|p| {
                    let n = remedy_ui::web::files::read(p).map_or(0, |b| b.len());
                    format!(
                        "{} ({n} bytes)",
                        p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
                    )
                })
                .collect();
            status.set(if sizes.is_empty() {
                "Nothing chosen".into()
            } else {
                format!("Opened {}", sizes.join(", "))
            });
        });
    }

    fn save_file(app: &App, text: String, status: Signal<String>) {
        let dialog = {
            let mut v_ = FileDialog::default();
            v_.title = "Save the message".into();
            v_.save = true;
            v_.file_name = Some("remedyui-message.txt".into());
            v_
        };
        app.file_dialog(None, dialog, move |paths| {
            if let Some(p) = paths.first() {
                let r = remedy_ui::web::files::save(p, format!("Saved by RemedyUI: {text}\n").as_bytes());
                status.set(match r {
                    Ok(()) => format!("Downloading {}", remedy_ui::web::files::download_name(p)),
                    Err(e) => e.to_string(),
                });
            }
        });
    }

    fn setup(app: &App) {
        let app2 = app.clone();
        app.open(WindowSpec::new("RemedyUI on the web"), move |ui, window| {
            let taps = Signal::new(0u32);
            let typed = Signal::new(String::new());
            let status = Signal::new(String::from("Ready"));
            let field = w::text_input(ui, "Message");
            let f2 = field.clone();
            let field = field.on_change(move || typed.set(f2.value()));
            let (copy_field, save_field) = (field.clone(), field.clone());
            let (open_app, save_app, link_app, copy_app) =
                (app2.clone(), app2.clone(), app2.clone(), app2.clone());
            let ui2 = ui.clone();
            let win = window.clone();
            let col = w::column(ui)
                .gap(12.0)
                .padding(remedy_ui::core::Insets { top: 16.0, end: 16.0, bottom: 16.0, start: 16.0 })
                .child(w::text(ui, "RemedyUI on the web").variant("title"))
                .child(ui.el("label").text_dyn(move || format!("Taps: {}", taps.get())))
                .child(w::button(ui, "Tap me", move || taps.update(|n| *n += 1)))
                .child(field.el())
                .child(ui.el("label").text_dyn(move || format!("You typed: {}", typed.get())))
                .child(
                    w::row(ui)
                        .gap(12.0)
                        .child(w::button(ui, "Copy", move || copy_app.copy(copy_field.value(), false)))
                        .child(w::button(ui, "Open link", move || {
                            link_app.open_url("https://github.com/AhmiDarrow/RemedyUI", move |r| {
                                status.set(match r {
                                    Ok(()) => "Opened the link".into(),
                                    Err(e) => e.to_string(),
                                });
                            });
                        })),
                )
                .child(
                    w::row(ui)
                        .gap(12.0)
                        .child(w::button(ui, "Open files", move || open_files(&open_app, status)))
                        .child(w::button(ui, "Save message", move || {
                            save_file(&save_app, save_field.value(), status)
                        })),
                )
                .child(ui.el("label").text_dyn(move || status.get()))
                .child(w::button(ui, "Which renderer?", move || {
                    let r = win.renderer().map_or("none yet".to_owned(), |r| format!("{r:?}"));
                    status.set(format!("Drawing on the {r} renderer"));
                }))
                .child(ui.el("label").text_dyn(move || {
                    let s = ui2.safe_area();
                    format!("Safe area: top {:.0}, bottom {:.0}", s.top, s.bottom)
                }));
            let _ = ui.root().child(col);
        });
    }

    #[wasm_bindgen::prelude::wasm_bindgen(start)]
    pub fn start() {
        if let Err(e) = remedy_ui::run(AppConfig::new("RemedyUI demo"), setup) {
            web_sys::console::error_1(&e.to_string().into());
        }
    }
}
