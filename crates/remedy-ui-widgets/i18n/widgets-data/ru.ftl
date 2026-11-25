# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = К последним
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } новое
    [few] { $count } новых
    [many] { $count } новых
   *[other] { $count } нового
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } новое сообщение, перейти к последним
    [few] { $count } новых сообщения, перейти к последним
    [many] { $count } новых сообщений, перейти к последним
   *[other] { $count } нового сообщения, перейти к последним
}

## Icon grids.
grid-item-keys = Клавиши со стрелками — переход между элементами; Пробел и Shift — выделение
grid-keys = Shift со стрелками выделяет диапазон; Ctrl+A выделяет всё
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } элемент
    [few] { $count } элемента
    [many] { $count } элементов
   *[other] { $count } элемента
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = Фильтр: { $label }
property-none = Нет подходящих свойств

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = Изменить: { $column }
table-row-keys = Клавиши со стрелками — переход между строками; Пробел и Shift — выделение
table-resize-keys = Ctrl+Стрелка влево и Ctrl+Стрелка вправо на заголовке меняют ширину столбца
table-move-keys = Alt+Shift+Стрелка влево и Alt+Shift+Стрелка вправо перемещают столбец
# Read after a column heading's name.
table-sorted-ascending = сортировка по возрастанию
table-sorted-descending = сортировка по убыванию
table-not-sorted = без сортировки
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } строка
    [few] { $count } строки
    [many] { $count } строк
   *[other] { $count } строки
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = Нельзя переместить элемент в самого себя
tree-not-folder = Элементы можно перемещать только в папку
# Shown after an item whose children are loading.
tree-loading = Загрузка…
# The same, as a screen reader says it.
tree-loading-spoken = Загрузка
tree-keys = Клавиши со стрелками — переход и раскрытие элементов; Ctrl+X, затем Ctrl+V — перемещение
