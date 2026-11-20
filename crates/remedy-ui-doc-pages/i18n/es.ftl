## Páginas.
page-number = Página { $page }
pages-list = Páginas
page-view-value = Página { $page } de { $count }, { $zoom } %

annotation-kind = { $kind ->
    [highlight] Resaltado
    [underline] Subrayado
    [strikeout] Tachado
    [squiggly] Subrayado ondulado
    [drawing] Dibujo
    [note] Nota
    [text-box] Cuadro de texto
    [rectangle] Rectángulo
    [ellipse] Elipse
    [line] Línea
    [arrow] Flecha
   *[annotation] Anotación
}
annotation-quoting = { $what }: «{ $text }»
annotation-with-comment = { $what }, comentario: { $comment }
annotation-by = { $what }, de { $author }
annotation-selected = Seleccionado: { $description }
annotation-count = { $count ->
    [one] { $count } anotación:
    [many] { $count } de anotaciones:
   *[other] { $count } anotaciones:
}

undo-add = { $kind ->
    [highlight] Añadir resaltado
    [underline] Añadir subrayado
    [strikeout] Añadir tachado
    [squiggly] Añadir subrayado ondulado
    [drawing] Añadir dibujo
    [note] Añadir nota
    [text-box] Añadir cuadro de texto
    [rectangle] Añadir rectángulo
    [ellipse] Añadir elipse
    [line] Añadir línea
    [arrow] Añadir flecha
   *[annotation] Añadir anotación
}
undo-change = { $kind ->
    [highlight] Cambiar resaltado
    [underline] Cambiar subrayado
    [strikeout] Cambiar tachado
    [squiggly] Cambiar subrayado ondulado
    [drawing] Cambiar dibujo
    [note] Cambiar nota
    [text-box] Cambiar cuadro de texto
    [rectangle] Cambiar rectángulo
    [ellipse] Cambiar elipse
    [line] Cambiar línea
    [arrow] Cambiar flecha
   *[annotation] Cambiar anotación
}
undo-delete = { $kind ->
    [highlight] Eliminar resaltado
    [underline] Eliminar subrayado
    [strikeout] Eliminar tachado
    [squiggly] Eliminar subrayado ondulado
    [drawing] Eliminar dibujo
    [note] Eliminar nota
    [text-box] Eliminar cuadro de texto
    [rectangle] Eliminar rectángulo
    [ellipse] Eliminar elipse
    [line] Eliminar línea
    [arrow] Eliminar flecha
   *[annotation] Eliminar anotación
}
undo-move = { $kind ->
    [highlight] Mover resaltado
    [underline] Mover subrayado
    [strikeout] Mover tachado
    [squiggly] Mover subrayado ondulado
    [drawing] Mover dibujo
    [note] Mover nota
    [text-box] Mover cuadro de texto
    [rectangle] Mover rectángulo
    [ellipse] Mover elipse
    [line] Mover línea
    [arrow] Mover flecha
   *[annotation] Mover anotación
}
