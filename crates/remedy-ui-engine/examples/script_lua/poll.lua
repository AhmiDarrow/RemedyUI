-- Poll: a quick vote. Buttons pass a choice in a closure (the Lua form of a curried
-- function) and start over with another closure. `this.votes` is the widget's own state.

function vote(choice)
  if this.votes == nil then this.votes = {} end
  this.votes[choice] = (this.votes[choice] or 0) + 1
end

function render(settings)
  local votes = this.votes or {}
  local buttons = {}
  local counts = {}
  for choice in string.gmatch(settings.choices or "", "[^,]+") do
    local c = choice:match("^%s*(.-)%s*$")
    buttons[#buttons + 1] = ui.button(c, function() vote(c) end)
    counts[#counts + 1] = ui.text(c .. ": " .. tostring(votes[c] or 0))
  end
  local shown = {
    ui.heading(settings.question),
    ui.row({ gap = 4 }, table.unpack(buttons)),
  }
  for _, line in ipairs(counts) do
    shown[#shown + 1] = line
  end
  shown[#shown + 1] = ui.divider()
  shown[#shown + 1] = ui.button("Start over", function() this.votes = {} end)
  return ui.column({ gap = 8 }, table.unpack(shown))
end

return {
  name = "Poll",
  settings = {
    { key = "question", label = "Question", kind = "text", value = "Tea or coffee?" },
    { key = "choices", label = "Choices (comma between)", kind = "text", value = "Coffee, Tea" },
  },
  render = render,
}
