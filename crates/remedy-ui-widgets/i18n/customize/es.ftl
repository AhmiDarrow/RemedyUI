# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } y { $second }
list-middle = { $first }, { $rest }
list-end = { $first } y { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Tema
area-shortcuts = Atajos de teclado
area-toolbars = Barras de herramientas y menús
area-text = Textos
area-layouts = Diseños de pantalla

## The settings page.
customize = Personalizar
customize-status = Estado de la personalización
nothing-to-customize = Nada que personalizar
nothing-to-customize-hint = Esta aplicación no ha activado la personalización.
reset-everything = Restablecer todo
reset-everything-done = Todo ha vuelto a los valores predeterminados de la aplicación. Puede recuperarlo con Deshacer.
# Also the undo history's name for resetting everything.
reset-all-customizations = Restablecer todas las personalizaciones

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Restablecer
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = Restablecer { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = Cambiar { $name }
# { $error } says why.
could-not-reset = No se pudo restablecer: { $error }
not-saved = No se guardó: { $error }
could-not-change = No se pudo hacer ese cambio: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = No se pudo usar { $area } ({ $file }): { $message }
notice-dismiss = Descartar
# The dismiss button's accessible name.
notice-dismiss-named = Descartar el aviso sobre { $file }
notice-not-utf8 = el archivo no es texto UTF-8
notice-could-not-save = no se pudo guardar: { $error }
notice-could-not-restore = no se pudo restaurar: { $detail }

## Errors.
error-not-enabled-theme = La personalización del tema no está activada en esta aplicación
error-not-enabled-shortcuts = La personalización de los atajos no está activada en esta aplicación
error-not-enabled-toolbars = La personalización de las barras de herramientas no está activada en esta aplicación
error-not-enabled-text = La personalización de los textos no está activada en esta aplicación
error-not-enabled-layouts = La personalización de los diseños no está activada en esta aplicación
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $commands } ya usa { $chord }
error-refused = rechazado: { $parts }
error-unknown-command = no existe el comando { $id }
error-unknown-slot = no existe la barra de herramientas { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } no admite ese tipo de valor

## Theme editor.
theme-editor = Editor de temas
theme-status = Estado del tema
theme-off = La personalización del tema está desactivada
theme-off-hint = Esta aplicación no permite cambiar su tema.
section-colors = Colores
# The section of fonts and text sizes.
section-text = Tipografía
section-shape = Espaciado y forma
section-motion = Movimiento
reset-theme = Restablecer tema
# The button that opens a color's picker; its accessible name is edit-named.
edit = Editar
edit-named = Editar { $name }
# Units after a number.
unit-px = px
unit-ms = ms

## Design tokens, as people call them.
token-theme-name = Nombre del tema
token-color-bg = Fondo
token-color-panel = Paneles
token-color-card = Tarjetas
token-color-text = Texto
token-color-muted = Texto atenuado
token-color-accent = Énfasis
token-color-on-accent = Texto sobre énfasis
token-color-danger = Peligro
token-color-warn = Advertencia
token-color-ok = Éxito
token-color-line = Líneas
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Color «{ $name }»
token-font = Fuente
token-font-mono = Fuente monoespaciada
token-size-small = Tamaño de texto pequeño
token-size-body = Tamaño del texto principal
token-size-title = Tamaño de título
token-spacing-unit = Unidad de espaciado
token-control-height = Altura de los controles
token-radius-small = Radio de esquina pequeño
token-radius-medium = Radio de esquina mediano
token-radius-large = Radio de esquina grande
token-stroke = Grosor de línea
token-motion-quick = Movimiento rápido
token-motion-standard = Movimiento estándar
token-motion-emphasized = Movimiento enfatizado
token-density = Densidad
token-background-image = Imagen de fondo
token-font-file = Archivo de fuente «{ $name }»

## How tightly controls are packed.
density-compact = Compacta
density-comfortable = Cómoda
density-spacious = Amplia

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Contraste
contrast-all-pass = Todos los pares de colores cumplen el contraste WCAG.
# The contrast status read by screen readers.
contrast-status-all-pass = Todos los pares de colores cumplen el contraste WCAG
contrast-status-failing = { $count ->
    [one] { $count } par de colores no alcanza el contraste WCAG
    [many] { $count } de pares de colores no alcanzan el contraste WCAG
   *[other] { $count } pares de colores no alcanzan el contraste WCAG
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, por debajo del { NUMBER($needed, minimumFractionDigits: 1) }:1 necesario ({ $foreground } sobre { $background })
contrast-pair-text-bg = Texto sobre el fondo
contrast-pair-text-panel = Texto sobre paneles
contrast-pair-text-card = Texto sobre tarjetas
contrast-pair-muted-bg = Texto atenuado sobre el fondo
contrast-pair-muted-panel = Texto atenuado sobre paneles
contrast-pair-muted-card = Texto atenuado sobre tarjetas
contrast-pair-on-accent = Texto sobre el color de énfasis
contrast-pair-accent-bg = El color de énfasis sobre el fondo

## The preview: sample widgets in the theme being edited.
preview = Vista previa
preview-group = Vista previa del tema
preview-field = Campo de ejemplo
preview-field-text = Texto en un campo
preview-body = El texto principal se ve así, en su tamaño normal.
preview-muted = Texto atenuado, para sugerencias y detalles.
preview-primary = Principal
preview-secondary = Secundario
preview-danger = Peligro
preview-switch = Un interruptor
preview-progress = Progreso
preview-link = Un enlace
preview-error = Un mensaje de error.

## Sharing themes.
section-share = Compartir
share-hint = Los temas son archivos de design tokens que cualquier aplicación Remedy puede leer.
theme-file = Archivo de tema
theme-file-placeholder = Una ruta como themes/ocean.json
import-from-path = Importar desde la ruta
export-to-path = Exportar a la ruta
import-path-first = Escriba primero la ruta del archivo de tema
export-path-first = Escriba primero dónde guardar el tema
choose-theme = Elegir un tema para importar…
export-to-file = Exportar a un archivo…
# File dialog titles and the name of their file type filter.
dialog-import-theme = Importar un tema
dialog-export-theme = Exportar el tema
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = tema
imported = Se importó el tema { $name }
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Se importó el tema { $name } sin { $count } elemento
    [many] Se importó el tema { $name } sin { $count } de elementos
   *[other] Se importó el tema { $name } sin { $count } elementos
}
could-not-import = No se pudo importar ese tema: { $error }
could-not-read = No se pudo leer { $path }: { $detail }
exported = Se exportó el tema a { $path }
could-not-export = No se pudo exportar el tema: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = Este tema incluye más que colores y tamaños
theme-carries-more-detail = Este tema incluye más que colores y tamaños. Lo siguiente queda desactivado a menos que lo active:
allow-shaders = Permitir sombreadores (código que se ejecuta en la tarjeta gráfica)
allow-links = Permitir enlaces externos (se conservan, nunca se descargan)
allow-files = Usar las imágenes y fuentes que incluye
import = Importar
import-without = Importar sin ellos
cancel = Cancelar
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: un sombreador ({ $detail })
refusal-link = { $token }: un enlace externo ({ $detail })
refusal-file = { $token }: un archivo que incluye el tema ({ $detail })
theme-not-object = un archivo de tema es un objeto JSON
# { $detail } is the JSON reader's message.
not-design-tokens = no es un archivo de design tokens: { $detail }
history-import-theme = Importar el tema { $name }

## The theme's files: its background image and fonts.
section-files = Archivos
files-hint = Las imágenes se muestran detrás de cada ventana, a su tamaño; las fuentes se comprueban antes de usarlas.
background-inline = Fondo: una imagen incluida en el tema
# { $file } is the image's path or link.
background-file = Fondo: { $file }
no-background = Sin imagen de fondo
background-file-field = Archivo de imagen de fondo
background-placeholder = Una ruta como pictures/waves.png
use-image = Usar esta imagen
image-path-first = Escriba primero la ruta de la imagen
reset-background = Restablecer imagen de fondo
choose-image = Elegir una imagen…
dialog-choose-image = Elegir una imagen de fondo
filter-images = Imágenes
no-fonts = El tema no incluye fuentes.
remove = Quitar
remove-font-named = Quitar la fuente { $family }
font-file = Archivo de fuente
font-file-placeholder = Una ruta como fonts/Ocean-Regular.ttf
font-name = Nombre de la fuente
font-name-placeholder = Cómo se llama la fuente, por ejemplo Ocean
add-font = Añadir fuente
font-first = Escriba primero la ruta del archivo de fuente y su nombre
history-change-background = Cambiar la imagen de fondo
history-remove-background = Quitar la imagen de fondo
history-add-font = Añadir la fuente { $family }
history-remove-font = Quitar la fuente { $family }
font-needs-name = una fuente necesita un nombre
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = no se pudo usar la fuente { $family }: { $detail }
background-not-used = no se pudo usar la imagen de fondo: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = los archivos de un tema están dentro de su carpeta
asset-needs-file = importe el tema desde su archivo para que se encuentren sus archivos
asset-link = { $link } es un enlace, y los enlaces no se descargan
not-an-image = no es una imagen que esta aplicación pueda mostrar: { $detail }
not-data-uri = no es un URI data:
data-uri-comma = un URI data: lleva una coma antes de sus datos
broken-escape = una secuencia de escape % incorrecta
# { $char } is a character, in quotes.
not-base64 = { $char } no es base64

## Shortcut editor.
shortcut-editor = Editor de atajos
shortcut-status = Estado de los atajos
shortcuts-off = La personalización de los atajos está desactivada
shortcuts-off-hint = Esta aplicación no permite cambiar sus atajos.
search-commands = Buscar comandos
commands = Comandos
no-matching-command = Ningún comando coincide
no-shortcut = Sin atajo
reset-all-shortcuts = Restablecer todos los atajos
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Añadir un atajo a { $command }
shortcut-remove-named = Quitar { $chord } de { $command }
reset-shortcuts-named = Restablecer los atajos de { $command }
shortcut-now-runs = { $chord } ahora ejecuta { $command }
shortcut-already-runs = { $chord } ya ejecuta { $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } ya ejecuta { $commands }.
shortcut-take = Usar { $chord } para { $command }
shortcut-keep = Mantenerlo para { $commands }
history-change-shortcuts = Cambiar los atajos de { $command }
history-add-shortcut = Añadir { $chord } a { $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = Asignar { $chord } a { $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = se omitió, el resto se aplica: { $parts }
shortcuts-clash-two = { $commands } responderían ambos a { $chord }
shortcuts-clash-many = { $commands } responderían todos a { $chord }

## Toolbar editor.
toolbar-editor = Editor de barras de herramientas
toolbar-status = Estado de las barras de herramientas
toolbars-off = La personalización de las barras de herramientas está desactivada
toolbars-off-hint = Esta aplicación no permite cambiar sus barras de herramientas.
no-toolbars = Esta aplicación no tiene barras de herramientas que se puedan cambiar.
reset-all-toolbars = Restablecer todas las barras de herramientas
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = no es un archivo de barras de herramientas: { $detail }
toolbars-not-object = un archivo de barras de herramientas es un objeto que asigna a cada posición una lista de identificadores de comandos
toolbar-slot-not-list = { $slot }: una posición es una lista de identificadores de comandos como ["file.save"]
toolbar-ids-not-text = { $slot }: los identificadores de comandos son texto

## Text editor.
text-editor = Editor de textos
text-status = Estado de los textos
text-off = La personalización de los textos está desactivada
text-off-hint = Esta aplicación no permite cambiar sus textos.
# { $folder } is a folder's path.
text-folder = Sus archivos de mensajes están en { $folder }
read-files-again = Volver a leer los archivos
search-text = Buscar textos
strings = Cadenas
no-matching-text = Ningún texto coincide
reset-all-text = Restablecer todos los textos
could-not-change-text = No se pudo cambiar ese texto: { $error }
could-not-reset-text = No se pudo restablecer ese texto: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = Restablecer «{ $text }»
change-text = Cambiar «{ $text }»
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Mensajes
search-messages = Buscar mensajes
no-matching-message = Ningún mensaje coincide
messages-hint = Mensajes de Fluent: texto, marcadores de posición como {"{"} $name {"}"} y opciones.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = Comprobación de { $id }
text-skipped = omitido: { $detail }
text-part-unread = no se pudo leer parte del archivo; se usa el resto: { $detail }

## Layout editor.
layout-editor = Editor de diseños
layout-status = Estado de los diseños
layouts-off = La personalización de los diseños está desactivada
layouts-off-hint = Esta aplicación no permite reorganizar sus pantallas.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Sus diseños están en { $folder }
no-layouts = No hay abierta ninguna pantalla que se pueda reorganizar.
reset-all-layouts = Restablecer todos los diseños
layout-named = Diseño { $file }
layout-check = Comprobación del diseño
layout-ok = Sin problemas: el diseño se puede guardar.
layout-saved = Guardado. La pantalla muestra su diseño.
save = Guardar
save-layout-named = Guardar el diseño { $file }
reset-layout-named = Restablecer el diseño { $file }
no-screen = no se muestra ninguna pantalla { $file }
