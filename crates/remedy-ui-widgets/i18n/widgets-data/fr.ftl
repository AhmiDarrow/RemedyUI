# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = Aller aux plus récents
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } nouveau
    [many] { $count } nouveaux
   *[other] { $count } nouveaux
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } nouveau message, aller aux plus récents
    [many] { $count } de nouveaux messages, aller aux plus récents
   *[other] { $count } nouveaux messages, aller aux plus récents
}

## Icon grids.
grid-item-keys = Les touches fléchées passent d’un élément à l’autre ; Espace et Maj sélectionnent
grid-keys = Maj avec les touches fléchées sélectionne une plage ; Ctrl+A sélectionne tout
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } élément
    [many] { $count } d’éléments
   *[other] { $count } éléments
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = Filtrer { $label }
property-none = Aucune propriété ne correspond

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = Modifier { $column }
table-row-keys = Les touches fléchées passent d’une ligne à l’autre ; Espace et Maj sélectionnent
table-resize-keys = Ctrl+Flèche gauche et Ctrl+Flèche droite sur l’en-tête redimensionnent la colonne
table-move-keys = Alt+Maj+Flèche gauche et Alt+Maj+Flèche droite déplacent la colonne
# Read after a column heading's name.
table-sorted-ascending = tri croissant
table-sorted-descending = tri décroissant
table-not-sorted = non trié
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } ligne
    [many] { $count } de lignes
   *[other] { $count } lignes
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = Un élément ne peut pas être déplacé dans lui-même
tree-not-folder = Les éléments ne peuvent être déplacés que dans un dossier
# Shown after an item whose children are loading.
tree-loading = Chargement…
# The same, as a screen reader says it.
tree-loading-spoken = Chargement
tree-keys = Les touches fléchées parcourent et ouvrent les éléments ; Ctrl+X puis Ctrl+V les déplacent
