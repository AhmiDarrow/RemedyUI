//! RemedyUI on Android: a button, a text field (with the input method's composition), copy
//! and paste, capture protection, a link, open and save dialogs, a drag out to other apps, and
//! the safe area, with every host event and the renderer written to logcat (tag `RemedyUI`)
//! so a test can follow what the bridge delivered.
//!
//! Launch options (`am start ... --es renderer cpu-first`): `cpu-first` starts the window on
//! the CPU renderer, to drive the move to the GPU with the window's next surface.
//!
//! Build, install and start it with `tools/build-apk.sh` (it is a shared library the native
//! activity loads, so it does nothing on other platforms).

#[cfg(target_os = "android")]
mod demo {
    use std::cell::RefCell;
    use std::rc::Rc;

    use remedy_ui::host::{AccessibilityEvent, FileDialog, Host, HostEvent};
    use remedy_ui::reactive::Signal;
    use remedy_ui::view::{Drag, DragData};
    use remedy_ui::widgets as w;
    use remedy_ui::{App, AppConfig, RendererKind, Runner, WindowSpec};
    use remedy_ui_bridge_android::log;

    /// What the buttons ask of the host, done with the host's own calls so the log shows
    /// the bridge's answers.
    enum Act {
        Copy(String),
        Paste,
        Protect(bool),
        OpenLink,
    }

    type Acts = Rc<RefCell<Vec<Act>>>;

    /// The app, with every event the bridge delivers logged first.
    struct Logged {
        runner: Runner,
        acts: Acts,
        /// The window, once the host announced it.
        window: Option<remedy_ui::host::WindowId>,
        /// The renderer last logged.
        renderer: Option<RendererKind>,
    }

    impl remedy_ui::host::App for Logged {
        fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
            match &event {
                // Frames, timers and wakes come many times a second.
                HostEvent::Frame { .. } | HostEvent::Timer(_) | HostEvent::Wake => {}
                HostEvent::Accessibility(_, AccessibilityEvent::Action(r)) => {
                    log(&format!("event Accessibility action {:?} on node {:?}", r.action, r.target_node));
                }
                HostEvent::Resumed => {
                    log(&format!("event Resumed (device memory {:?} bytes)", host.total_memory()))
                }
                HostEvent::Resized { window, .. } => {
                    self.window = Some(*window);
                    log(&format!("event {event:?} (safe area {:?})", host.safe_area(*window)));
                }
                e => log(&format!("event {e:?}")),
            }
            self.runner.event(host, event);
            let now = self.runner.app().windows().first().and_then(|w| w.renderer());
            if now.is_some() && now != self.renderer {
                self.renderer = now;
                log(&format!("renderer now {now:?}"));
            }
            let acts = std::mem::take(&mut *self.acts.borrow_mut());
            for a in acts {
                match a {
                    Act::Copy(text) => {
                        let r = host.set_clipboard_text(&text, true);
                        log(&format!("clipboard set {text:?} (sensitive): {r:?}"));
                    }
                    Act::Paste => log(&format!("clipboard read: {:?}", host.clipboard_text())),
                    Act::Protect(on) => {
                        if let Some(id) = self.window {
                            log(&format!(
                                "capture protection {on}: {:?}",
                                host.set_capture_protected(id, on)
                            ));
                        }
                    }
                    Act::OpenLink => {
                        log(&format!("open_url: {:?}", host.open_url("https://example.com/")));
                    }
                }
            }
        }
    }

    fn open_file(app: &App) {
        let dialog = {
            let mut v_ = FileDialog::default();
            v_.title = "Open a text file".into();
            v_.filters = vec![("Text".into(), vec!["txt".into()])];
            v_
        };
        app.file_dialog(None, dialog, |paths| {
            log(&format!("open dialog chose {paths:?}"));
            for p in paths {
                match std::fs::read_to_string(&p) {
                    Ok(t) => log(&format!("opened {} holds {:?}", p.display(), t.trim_end())),
                    Err(e) => log(&format!("opened {} could not be read: {e}", p.display())),
                }
            }
        });
    }

    fn save_file(app: &App, text: String) {
        let dialog = {
            let mut v_ = FileDialog::default();
            v_.title = "Save the message".into();
            v_.save = true;
            v_.file_name = Some("remedyui-saved.txt".into());
            v_.filters = vec![("Text".into(), vec!["txt".into()])];
            v_
        };
        app.file_dialog(None, dialog, move |paths| {
            log(&format!("save dialog chose {paths:?}"));
            if let Some(p) = paths.first() {
                let r = std::fs::write(p, format!("Saved by RemedyUI: {text}\n"));
                log(&format!("wrote {}: {r:?}", p.display()));
            }
        });
    }

    fn setup(app: &App, acts: Acts) {
        let app2 = app.clone();
        app.open(WindowSpec::new("RemedyUI on Android"), move |ui, _window| {
            let taps = Signal::new(0u32);
            let typed = Signal::new(String::new());
            let protected = Signal::new(false);
            let field = w::text_input(ui, "Message");
            let f2 = field.clone();
            let field = field.on_change(move || {
                let v = f2.value();
                log(&format!("text field now {v:?}"));
                typed.set(v);
            });
            let a = acts.clone();
            let copy_field = field.clone();
            let save_field = field.clone();
            let (open_app, save_app) = (app2.clone(), app2.clone());
            let ui2 = ui.clone();
            let col = w::column(ui)
                .gap(12.0)
                .padding(remedy_ui::core::Insets { top: 16.0, end: 16.0, bottom: 16.0, start: 16.0 })
                .child(w::text(ui, "RemedyUI on Android").variant("title"))
                // First, so it shows in a small (freeform or split-screen) window too.
                .child(w::text(ui, "Drag me out").draggable(
                    || Some(Drag::new(DragData::text("Dragged out of RemedyUI"))),
                    |effect| log(&format!("drag out ended: {effect:?}")),
                ))
                .child(ui.el("label").text(move || format!("Taps: {}", taps.get())))
                .child(w::button(ui, "Tap me", move || {
                    taps.update(|n| *n += 1);
                    log(&format!("button tapped, taps = {}", taps.get_untracked().unwrap_or(0)));
                }))
                .child(field.el())
                .child(ui.el("label").text(move || format!("You typed: {}", typed.get())))
                .child(
                    w::row(ui)
                        .gap(12.0)
                        .child(w::button(ui, "Copy", {
                            let a = a.clone();
                            move || a.borrow_mut().push(Act::Copy(copy_field.value()))
                        }))
                        .child(w::button(ui, "Read clipboard", {
                            let a = a.clone();
                            move || a.borrow_mut().push(Act::Paste)
                        })),
                )
                .child(
                    w::row(ui)
                        .gap(12.0)
                        .child(w::button(ui, "Protect", {
                            let a = a.clone();
                            move || {
                                let on = !protected.get_untracked().unwrap_or(false);
                                protected.set(on);
                                a.borrow_mut().push(Act::Protect(on));
                            }
                        }))
                        .child(w::button(ui, "Open link", {
                            let a = a.clone();
                            move || a.borrow_mut().push(Act::OpenLink)
                        })),
                )
                .child(ui.el("label").text(move || {
                    format!("Capture protection: {}", if protected.get() { "on" } else { "off" })
                }))
                .child(
                    w::row(ui)
                        .gap(12.0)
                        .child(w::button(ui, "Open file", move || open_file(&open_app)))
                        .child(w::button(ui, "Save file", move || save_file(&save_app, save_field.value()))),
                )
                .child(ui.el("label").text(move || {
                    let s = ui2.safe_area();
                    format!("Safe area: top {:.0}, bottom {:.0}", s.top, s.bottom)
                }));
            let _ = ui.root().child(col);
        });
    }

    fn app_main() {
        let acts: Acts = Rc::default();
        let a = acts.clone();
        let mut config = AppConfig::new("RemedyUI demo");
        let choice = remedy_ui_bridge_android::launch_extra("renderer");
        if choice.as_deref() == Some("cpu-first") {
            config = config.cpu_first();
        }
        log(&format!("demo starting (renderer option {choice:?})"));
        let runner = Runner::new(config, move |app: &App| setup(app, a));
        match remedy_ui_bridge_android::run(Logged { runner, acts, window: None, renderer: None }) {
            Ok(()) => log("demo finished"),
            Err(e) => log(&format!("demo failed: {e}")),
        }
    }

    remedy_ui_bridge_android::main!(app_main);
}
