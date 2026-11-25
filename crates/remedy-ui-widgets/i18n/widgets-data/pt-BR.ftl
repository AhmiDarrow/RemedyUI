# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = Ir para as mais recentes
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } nova
    [many] { $count } novas
   *[other] { $count } novas
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } nova mensagem, ir para as mais recentes
    [many] { $count } de novas mensagens, ir para as mais recentes
   *[other] { $count } novas mensagens, ir para as mais recentes
}

## Icon grids.
grid-item-keys = As teclas de seta navegam entre os itens; Espaço e Shift selecionam
grid-keys = Shift com as teclas de seta seleciona um intervalo; Ctrl+A seleciona tudo
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [one] { $count } item
    [many] { $count } de itens
   *[other] { $count } itens
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = Filtrar { $label }
property-none = Nenhuma propriedade corresponde

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = Editar { $column }
table-row-keys = As teclas de seta navegam entre as linhas; Espaço e Shift selecionam
table-resize-keys = Ctrl+Seta para a esquerda e Ctrl+Seta para a direita no cabeçalho redimensionam a coluna
table-move-keys = Alt+Shift+Seta para a esquerda e Alt+Shift+Seta para a direita movem a coluna
# Read after a column heading's name.
table-sorted-ascending = ordem crescente
table-sorted-descending = ordem decrescente
table-not-sorted = sem ordenação
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } linha
    [many] { $count } de linhas
   *[other] { $count } linhas
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = Um item não pode ser movido para dentro de si mesmo
tree-not-folder = Os itens só podem ser movidos para uma pasta
# Shown after an item whose children are loading.
tree-loading = Carregando…
# The same, as a screen reader says it.
tree-loading-spoken = Carregando
tree-keys = As teclas de seta navegam e abrem os itens; Ctrl+X e depois Ctrl+V os movem
