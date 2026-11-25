# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = Ir a lo más reciente
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } nuevo
    [many] { $count } nuevos
   *[other] { $count } nuevos
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } mensaje nuevo, ir a lo más reciente
    [many] { $count } de mensajes nuevos, ir a lo más reciente
   *[other] { $count } mensajes nuevos, ir a lo más reciente
}

## Icon grids.
grid-item-keys = Las teclas de flecha recorren los elementos; Espacio y Mayús seleccionan
grid-keys = Mayús con las teclas de flecha selecciona un rango; Ctrl+A selecciona todo
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } elemento
    [many] { $count } de elementos
   *[other] { $count } elementos
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = Filtrar { $label }
property-none = Ninguna propiedad coincide

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = Editar { $column }
table-row-keys = Las teclas de flecha recorren las filas; Espacio y Mayús seleccionan
table-resize-keys = Ctrl+Flecha izquierda y Ctrl+Flecha derecha en el encabezado cambian el ancho de la columna
table-move-keys = Alt+Mayús+Flecha izquierda y Alt+Mayús+Flecha derecha mueven la columna
# Read after a column heading's name.
table-sorted-ascending = orden ascendente
table-sorted-descending = orden descendente
table-not-sorted = sin ordenar
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } fila
    [many] { $count } de filas
   *[other] { $count } filas
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = Un elemento no se puede mover dentro de sí mismo
tree-not-folder = Los elementos solo se pueden mover a una carpeta
# Shown after an item whose children are loading.
tree-loading = Cargando…
# The same, as a screen reader says it.
tree-loading-spoken = Cargando
tree-keys = Las teclas de flecha recorren y abren los elementos; Ctrl+X y luego Ctrl+V los mueven
