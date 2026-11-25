# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = Zu den neuesten
# Shown on the button when messages arrived below.
chat-new-count = { $count } neue
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } neue Nachricht, zu den neuesten springen
   *[other] { $count } neue Nachrichten, zu den neuesten springen
}

## Icon grids.
grid-item-keys = Pfeiltasten wechseln zwischen Elementen; Leertaste und Umschalt wählen aus
grid-keys = Umschalt mit den Pfeiltasten wählt einen Bereich aus; Strg+A wählt alles aus
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } Element
   *[other] { $count } Elemente
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = { $label } filtern
property-none = Keine passenden Eigenschaften

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = { $column } bearbeiten
table-row-keys = Pfeiltasten wechseln zwischen Zeilen; Leertaste und Umschalt wählen aus
table-resize-keys = Strg+Pfeil links und Strg+Pfeil rechts auf der Überschrift ändern die Spaltenbreite
table-move-keys = Alt+Umschalt+Pfeil links und Alt+Umschalt+Pfeil rechts verschieben die Spalte
# Read after a column heading's name.
table-sorted-ascending = aufsteigend sortiert
table-sorted-descending = absteigend sortiert
table-not-sorted = nicht sortiert
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } Zeile
   *[other] { $count } Zeilen
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = Ein Element kann nicht in sich selbst verschoben werden
tree-not-folder = Elemente können nur in einen Ordner verschoben werden
# Shown after an item whose children are loading.
tree-loading = Wird geladen…
# The same, as a screen reader says it.
tree-loading-spoken = Wird geladen
tree-keys = Pfeiltasten wechseln zwischen Elementen und öffnen sie; Strg+X und dann Strg+V verschiebt sie
