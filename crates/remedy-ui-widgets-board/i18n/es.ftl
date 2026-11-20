# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = Deshacer
redo = Rehacer
add-area = Añadir área
area-heading = Área
area-title = Título del área
area-title-hint = Título (Intro para aplicar)
area-place = Columna { $column }, fila { $row }, ancho { $columns }, alto { $rows }
area-keys-hint = Las flechas la mueven; Mayús con las flechas cambia su tamaño.
area-keys = Las teclas de flecha la mueven; Mayús con las teclas de flecha cambia su tamaño
remove-area = Quitar área
widget-heading = Widget
settings = Configuración
widget-keys-hint = Alt con Arriba y Abajo lo mueve; Alt con Izquierda y Derecha lo mueve al área siguiente.
widget-keys = Alt con las teclas de flecha lo mueve
remove-widget = Quitar widget
nothing-selected = Seleccione un área o un widget para cambiarlo.
add-widget = Añadir widget
widget-kind = Tipo de widget

## The grid.
widget-name = Widget { $name }
unknown-kind = Este tablero usa un widget que esta aplicación no tiene ({ $kind }).
area-named = Área { $name }
area-numbered = Área { $number }
empty-area = Área vacía: añada un widget o suelte uno aquí.
empty-board = Tablero vacío
empty-board-editing = Este tablero está vacío: añada un área.
empty-board-viewing = Aún no hay nada aquí.
one-widget = Esta área ya contiene un widget: dele al otro un área propia.

## Why a change was refused.
error-invalid = no es un tablero: { $why }
error-too-new = este tablero se creó con una versión más reciente (formato { $format })
error-outside = eso se saldría del borde del tablero
error-overlap = eso cubriría el área { $area }
error-missing = { $id } no está en el tablero

## Setting editors.
time-date = { $label }: fecha
time-time = { $label }: hora
time-not-set = Sin definir
time-clear = Borrar
choice-unavailable = { $value } (no disponible)
list-item = { $label } { $number }
list-add = Añadir a { $label }
list-remove = Quitar { $label } { $number }
picture-source = { $label }: dirección
picture-choose = Elegir imagen

## Scripted widgets.
stopped = Este widget se detuvo: { $why }
start-again = Volver a iniciar

## The built-in widgets.
text-name = Texto
text-title = Título
text-title-default = Bienvenida
text-body = Texto
text-body-default = Salude a la casa.
links-name = Enlaces
links-title = Título
links-title-default = Enlaces
links-links = Enlaces (Etiqueta | https://…, uno por línea)
links-none = Aún no hay enlaces.
reactions-name = Reacciones
reactions-question = Pregunta
reactions-question-default = ¿Cómo están todos?
reactions-offered = Reacciones disponibles

## The reactions component.
reaction-group = Reacciones
reaction-one = 1 reacción
reaction-many = { $count } reacciones
reaction-count = { $count ->
    [one] { $count } reacción
    [many] { $count } de reacciones
   *[other] { $count } reacciones
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, incluida la suya
