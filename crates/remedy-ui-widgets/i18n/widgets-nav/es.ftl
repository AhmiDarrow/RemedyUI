# The navigation widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Entf, Strg…).

## Tabs, navigation items and breadcrumbs.
# A tab's close button; { $title } is the tab's title.
tab-close = Cerrar { $title }
tab-close-keys = Supr cierra la pestaña activa
# Read after a navigation item's name when it has a count badge ("Inbox, 3 new").
nav-badge-new = { $count ->
    [one] { $count } nuevo
    [many] { $count } nuevos
   *[other] { $count } nuevos
}
# The "…" button in long breadcrumbs: a menu of the folded path segments.
breadcrumbs-more = Más ubicaciones
# The last breadcrumb, the page being shown; { $segment } is its name.
breadcrumb-current = { $segment }, actual

## Drawers, sheets and the custom title bar.
drawer-close-keys = Esc cierra el panel
panel-close = Cerrar
# The group of window controls and title at the top of a frameless window.
title-bar = Barra de título
title-bar-minimize = Minimizar
title-bar-maximize = Maximizar
title-bar-restore = Restaurar
title-bar-close = Cerrar

## The navigation stack.
stack-back = Atrás
# The stack goes back on Alt and the Left arrow key (in every layout direction).
stack-back-keys = Alt+Flecha izquierda vuelve atrás

## Toolbars and the command palette.
# A toolbar button's tooltip: { $command } is its label, { $shortcut } its keyboard shortcut.
command-tooltip = { $command } ({ $shortcut })
# Buttons in the toolbar editor that move a command; { $command } is its label.
toolbar-move-earlier = Mover { $command } antes
toolbar-move-later = Mover { $command } después
palette-title = Paleta de comandos
palette-search = Buscar comandos
palette-placeholder = Escriba un comando…
# The list of commands in the palette.
palette-commands = Comandos
palette-no-match = Ningún comando coincide
