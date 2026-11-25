-- Pulse: a theme piece with motion, timers and theme tokens. A dot's size breathes with a
-- spring, a clock ticks once a second on the frame loop, and the accent color comes from the
-- app's theme (following it when the theme changes).
local size = ui.animated(8)
local seconds = ui.signal(0)
local grow = true

ui.every(1000, function()
  seconds:update(function(s) return s + 1 end)
  size:to(grow and 16 or 8, { spring = 400, bounce = 0.3 })
  grow = not grow
end)

local accent = ui.computed(function() return ui.token("color.accent", "#888888") end)
local started = ui.signal(false)
ui.on_mount(function() started:set(true) end)

ui.mount(ui.column{ gap = 4,
  ui.element("dot", { size = size, color = accent }),
  ui.muted(ui.computed(function() return seconds:get() .. " s" end)),
})

return { name = "Pulse" }
