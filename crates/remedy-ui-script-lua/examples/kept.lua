-- Claps, the other way: the widget keeps its elements and changes them from Lua. A signal
-- holds the count; the text and the bar follow it, and the button changes it. The
-- render-each-time twin is examples/claps.lua.
local claps = ui.signal(0)
local goal = ui.computed(function() return (ui.settings:get() or {}).goal or 10 end)
local line = ui.computed(function()
  return claps:get() .. " claps from " .. ((ui.host:get() or {}).house or "nobody")
end)

local bar = ui.progress("Claps", 0)
ui.effect(function() bar:set("value", claps:get() / goal:get()) end)

ui.mount(ui.column{ gap = 8,
  ui.heading(ui.computed(function() return (ui.settings:get() or {}).title or "Claps" end)),
  ui.text(line),
  bar,
  ui.button("Clap", function() claps:update(function(n) return n + 1 end) end),
})

return {
  name = "Claps",
  settings = {
    { key = "title", label = "Title", kind = "text", value = "Claps" },
    { key = "goal", label = "Goal", kind = "number", min = 1, max = 100, value = 10 },
  },
}
