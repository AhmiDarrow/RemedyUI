// Claps: a widget people write by hand. It counts claps toward a goal and greets whoever
// is looking. The same widget, compiled from Rust to WebAssembly, is
// crates/remedy-ui-script-wasm/examples/claps.

// How it looks, for its settings and the host's data. `this` is the widget's own state.
fn render(settings, host) {
    let claps = this.claps ?? 0;
    ui::column(#{ gap: 8 }, [
        ui::heading(settings.title),
        ui::text(`${claps} claps from ${host.house ?? "nobody"}`),
        ui::progress("Claps", claps.to_float() / settings.goal.to_float()),
        ui::button("Clap", "clap"),
        ui::muted(`Hello, ${host.name ?? "guest"}`),
    ])
}

// The Clap button's handler.
fn clap() {
    this.claps = (this.claps ?? 0) + 1;
}

// Its name and settings: the board's property grid edits them.
#{
    name: "Claps",
    settings: [
        #{ key: "title", label: "Title", kind: "text", value: "Claps" },
        #{ key: "goal", label: "Goal", kind: "number", min: 1, max: 100, value: 10 },
    ],
}
