## Páginas.
page-number = Página { $page }
pages-list = Páginas
page-view-value = Página { $page } de { $count }, { $zoom }%

annotation-kind = { $kind ->
    [highlight] Realce
    [underline] Sublinhado
    [strikeout] Tachado
    [squiggly] Sublinhado ondulado
    [drawing] Desenho
    [note] Nota
    [text-box] Caixa de texto
    [rectangle] Retângulo
    [ellipse] Elipse
    [line] Linha
    [arrow] Seta
   *[annotation] Anotação
}
annotation-quoting = { $what }: “{ $text }”
annotation-with-comment = { $what }, comentário: { $comment }
annotation-by = { $what }, por { $author }
annotation-selected = Selecionado: { $description }
annotation-count = { $count ->
    [one] { $count } anotação:
    [many] { $count } de anotações:
   *[other] { $count } anotações:
}

undo-add = { $kind ->
    [highlight] Adicionar realce
    [underline] Adicionar sublinhado
    [strikeout] Adicionar tachado
    [squiggly] Adicionar sublinhado ondulado
    [drawing] Adicionar desenho
    [note] Adicionar nota
    [text-box] Adicionar caixa de texto
    [rectangle] Adicionar retângulo
    [ellipse] Adicionar elipse
    [line] Adicionar linha
    [arrow] Adicionar seta
   *[annotation] Adicionar anotação
}
undo-change = { $kind ->
    [highlight] Alterar realce
    [underline] Alterar sublinhado
    [strikeout] Alterar tachado
    [squiggly] Alterar sublinhado ondulado
    [drawing] Alterar desenho
    [note] Alterar nota
    [text-box] Alterar caixa de texto
    [rectangle] Alterar retângulo
    [ellipse] Alterar elipse
    [line] Alterar linha
    [arrow] Alterar seta
   *[annotation] Alterar anotação
}
undo-delete = { $kind ->
    [highlight] Excluir realce
    [underline] Excluir sublinhado
    [strikeout] Excluir tachado
    [squiggly] Excluir sublinhado ondulado
    [drawing] Excluir desenho
    [note] Excluir nota
    [text-box] Excluir caixa de texto
    [rectangle] Excluir retângulo
    [ellipse] Excluir elipse
    [line] Excluir linha
    [arrow] Excluir seta
   *[annotation] Excluir anotação
}
undo-move = { $kind ->
    [highlight] Mover realce
    [underline] Mover sublinhado
    [strikeout] Mover tachado
    [squiggly] Mover sublinhado ondulado
    [drawing] Mover desenho
    [note] Mover nota
    [text-box] Mover caixa de texto
    [rectangle] Mover retângulo
    [ellipse] Mover elipse
    [line] Mover linha
    [arrow] Mover seta
   *[annotation] Mover anotação
}
