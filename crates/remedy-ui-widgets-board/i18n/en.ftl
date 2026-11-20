# The board's own words: its editor and panel, its empty states, the messages of refused
# changes, the setting editors, the built-in widgets and the reactions.
# Placeholders ({ $name }) must stay; their order may change. Each message is a field of
# BoardLabels (or ReactionLabels): the id is the field's name in kebab case.
# Keyboard hints name keys as the keyboard prints them in the language.

## The editing panel.
# The undo button.
undo = Undo
# The redo button.
redo = Redo
# The button that adds an area.
add-area = Add area
# The panel's heading for an area.
area-heading = Area
# The area title field's name.
area-title = Area title
# The label over the area title field.
area-title-hint = Title (Enter to apply)
# Where an area is: its column and row (from 1), and how many columns wide and rows high
# it is. Plain numbers, so the wording should not need plural forms.
area-place = Column { $column }, row { $row }, { $columns } wide, { $rows } high
# How to move an area from the keyboard, in the panel.
area-keys-hint = Arrow keys move it; Shift with arrows resizes it.
# How to move an area from the keyboard, for assistive technology.
area-keys = The arrow keys move it; Shift with the arrow keys resizes it
# The button that removes an area.
remove-area = Remove area
# The panel's heading for a widget.
widget-heading = Widget
# The settings grid's name.
settings = Settings
# How to move a widget from the keyboard, in the panel.
widget-keys-hint = Alt with Up and Down moves it; Alt with Left and Right moves it to the next area.
# How to move a widget from the keyboard, for assistive technology.
widget-keys = Alt with the arrow keys moves it
# The button that removes a widget.
remove-widget = Remove widget
# The panel with nothing selected.
nothing-selected = Select an area or a widget to change it.
# The button that adds a widget.
add-widget = Add widget
# The list of kinds to add.
widget-kind = Widget kind

## The grid.
# A placed widget while arranging, for assistive technology: { $name } is the widget's name.
widget-name = { $name } widget
# A placed widget of a kind the app lacks: { $kind } is the kind's id.
unknown-kind = This board uses a widget this app does not have ({ $kind }).
# An area with a title, for assistive technology: { $name } is its title.
area-named = { $name } area
# An area with no title and no single widget to name it: { $number } (from 1, in reading
# order).
area-numbered = Area { $number }
# An empty area while arranging.
empty-area = Empty area: add a widget or drop one here.
# An empty board's title.
empty-board = Empty board
# An empty board while arranging.
empty-board-editing = This board is empty: add an area.
# An empty board for visitors.
empty-board-viewing = Nothing here yet.
# A second widget refused where an area holds one.
one-widget = This area already holds a widget: give the other one an area of its own.

## Why a change was refused (shown after the change, so lower case in English).
# Not a board: { $why } is a technical reason (in English).
error-invalid = not a board: { $why }
# Made by a newer version of the app: { $format } is the format's number.
error-too-new = this board was made by a newer version (format { $format })
# Past the edge of the grid.
error-outside = that would go past the edge of the board
# It would cover another area: { $area } is its title, or its id.
error-overlap = that would cover the area { $area }
# Not on the board: { $id } is an area's or a widget's id.
error-missing = { $id } is not on the board

## Setting editors.
# A time setting's date field: { $label } is the setting's name.
time-date = { $label }: date
# A time setting's time field: { $label } is the setting's name.
time-time = { $label }: time
# A time setting that is not set.
time-not-set = Not set
# Clears a time setting.
time-clear = Clear
# A stored choice that is no longer offered: { $value } is the stored value.
choice-unavailable = { $value } (not available)
# One item of a list setting: { $label } is the setting's name, { $number } the item's
# (from 1).
list-item = { $label } { $number }
# Adds an item to a list setting: { $label } is the setting's name.
list-add = Add to { $label }
# Removes an item of a list setting: { $label } is the setting's name, { $number } the
# item's.
list-remove = Remove { $label } { $number }
# A picture setting's source field: { $label } is the setting's name.
picture-source = { $label }: address
# Opens the app's picture chooser.
picture-choose = Choose picture

## Scripted widgets.
# A scripted widget that stopped: { $why } is the script's error (in English).
stopped = This widget stopped: { $why }
# Starts a stopped widget again.
start-again = Start again

## The built-in widgets.
# The built-in text widget's name.
text-name = Text
# Its title setting.
text-title = Title
# Its title until set.
text-title-default = Welcome
# Its text setting.
text-body = Text
# Its text until set.
text-body-default = Say hello to the house.
# The built-in links widget's name.
links-name = Links
# Its title setting.
links-title = Title
# Its title until set.
links-title-default = Links
# Its links setting: "Label | https://…" stays as it is typed.
links-links = Links (Label | https://…, one a line)
# It has no links.
links-none = No links yet.
# The built-in reactions widget's name.
reactions-name = Reactions
# Its question setting.
reactions-question = Question
# Its question until set.
reactions-question-default = How is everyone?
# Its setting of reactions to offer.
reactions-offered = Reactions to offer

## The reactions component.
# The group's name for screen readers.
reaction-group = Reactions
# One reaction, as counted for screen readers (for apps that set their own words; the
# board itself counts with reaction-count).
reaction-one = 1 reaction
# Several (or no) reactions: { $count } (for apps that set their own words; the board
# itself counts with reaction-count).
reaction-many = { $count } reactions
# How many reactions, as counted for screen readers, with the language's plural forms.
reaction-count = { $count ->
    [one] { $count } reaction
   *[other] { $count } reactions
}
# A reaction button for screen readers: { $emoji }, and { $count } as reaction-count says it.
reaction-spoken = { $emoji }, { $count }
# A reaction button that includes the viewer's: { $emoji }, { $count }.
reaction-spoken-mine = { $emoji }, { $count }, including yours
