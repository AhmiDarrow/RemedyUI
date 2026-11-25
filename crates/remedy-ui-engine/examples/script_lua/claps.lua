-- Claps: a widget people write by hand. It counts claps toward a goal and greets whoever
-- is looking. `this` is the widget's own state. The same widget, compiled from Rust to
-- WebAssembly, is crates/remedy-ui-engine/script-wasm/examples/claps. A widget that keeps its
-- elements and follows signals is examples/kept.lua.

function clap()
  this.claps = (this.claps or 0) + 1
end

function render(settings, host)
  local claps = this.claps or 0
  local house = (host and host.house) or "nobody"
  local name = (host and host.name) or "guest"
  local goal = (settings and settings.goal) or 10
  return ui.column{ gap = 8,
    ui.heading(settings.title),
    ui.text(claps .. " claps from " .. house),
    ui.progress("Claps", claps / goal),
    ui.button("Clap", "clap"),
    ui.muted("Hello, " .. name),
  }
end

return {
  name = "Claps",
  settings = {
    { key = "title", label = "Title", kind = "text", value = "Claps" },
    { key = "goal", label = "Goal", kind = "number", min = 1, max = 100, value = 10 },
  },
  render = render,
}
