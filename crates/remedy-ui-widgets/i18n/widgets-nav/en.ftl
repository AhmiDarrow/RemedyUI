# The navigation widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Entf, Strg…).

## Tabs, navigation items and breadcrumbs.
# A tab's close button; { $title } is the tab's title.
tab-close = Close { $title }
tab-close-keys = Delete closes the focused tab
# Read after a navigation item's name when it has a count badge ("Inbox, 3 new").
nav-badge-new = { $count ->
    [one] { $count } new
   *[other] { $count } new
}
# The "…" button in long breadcrumbs: a menu of the folded path segments.
breadcrumbs-more = More locations
# The last breadcrumb, the page being shown; { $segment } is its name.
breadcrumb-current = { $segment }, current

## Drawers, sheets and the custom title bar.
drawer-close-keys = Escape closes the panel
panel-close = Close
# The group of window controls and title at the top of a frameless window.
title-bar = Title bar
title-bar-minimize = Minimize
title-bar-maximize = Maximize
title-bar-restore = Restore
title-bar-close = Close

## The navigation stack.
stack-back = Back
# The stack goes back on Alt and the Left arrow key (in every layout direction).
stack-back-keys = Alt+Left goes back

## Toolbars and the command palette.
# A toolbar button's tooltip: { $command } is its label, { $shortcut } its keyboard shortcut.
command-tooltip = { $command } ({ $shortcut })
# Buttons in the toolbar editor that move a command; { $command } is its label.
toolbar-move-earlier = Move { $command } earlier
toolbar-move-later = Move { $command } later
palette-title = Command palette
palette-search = Search commands
palette-placeholder = Type a command…
# The list of commands in the palette.
palette-commands = Commands
palette-no-match = No matching command
