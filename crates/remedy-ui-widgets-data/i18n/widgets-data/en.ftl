# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = Jump to latest
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } new
   *[other] { $count } new
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } new message, jump to latest
   *[other] { $count } new messages, jump to latest
}

## Icon grids.
grid-item-keys = Arrow keys move between items; Space and Shift select
grid-keys = Shift with the arrow keys selects a range; Ctrl+A selects all
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = Filter { $label }
property-none = No properties match

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = Edit { $column }
table-row-keys = Arrow keys move between rows; Space and Shift select
table-resize-keys = Ctrl+Left and Ctrl+Right on the heading resize the column
table-move-keys = Alt+Shift+Left and Alt+Shift+Right move the column
# Read after a column heading's name.
table-sorted-ascending = sorted ascending
table-sorted-descending = sorted descending
table-not-sorted = not sorted
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } row
   *[other] { $count } rows
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = An item cannot move into itself
tree-not-folder = Items can only move into a folder
# Shown after an item whose children are loading.
tree-loading = Loading…
# The same, as a screen reader says it.
tree-loading-spoken = Loading
tree-keys = Arrow keys move and open items; Ctrl+X then Ctrl+V moves them
