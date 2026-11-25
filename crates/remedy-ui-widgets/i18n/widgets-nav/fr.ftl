# The navigation widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Entf, Strg…).

## Tabs, navigation items and breadcrumbs.
# A tab's close button; { $title } is the tab's title.
tab-close = Fermer { $title }
tab-close-keys = Suppr ferme l’onglet actif
# Read after a navigation item's name when it has a count badge ("Inbox, 3 new").
nav-badge-new = { $count ->
    [one] { $count } nouveau
    [many] { $count } nouveaux
   *[other] { $count } nouveaux
}
# The "…" button in long breadcrumbs: a menu of the folded path segments.
breadcrumbs-more = Autres emplacements
# The last breadcrumb, the page being shown; { $segment } is its name.
breadcrumb-current = { $segment }, page actuelle

## Drawers, sheets and the custom title bar.
drawer-close-keys = Échap ferme le panneau
panel-close = Fermer
# The group of window controls and title at the top of a frameless window.
title-bar = Barre de titre
title-bar-minimize = Réduire
title-bar-maximize = Agrandir
title-bar-restore = Restaurer
title-bar-close = Fermer

## The navigation stack.
stack-back = Retour
# The stack goes back on Alt and the Left arrow key (in every layout direction).
stack-back-keys = Alt+Flèche gauche pour revenir en arrière

## Toolbars and the command palette.
# A toolbar button's tooltip: { $command } is its label, { $shortcut } its keyboard shortcut.
command-tooltip = { $command } ({ $shortcut })
# Buttons in the toolbar editor that move a command; { $command } is its label.
toolbar-move-earlier = Déplacer { $command } vers l’avant
toolbar-move-later = Déplacer { $command } vers l’arrière
palette-title = Palette de commandes
palette-search = Rechercher des commandes
palette-placeholder = Tapez une commande…
# The list of commands in the palette.
palette-commands = Commandes
palette-no-match = Aucune commande correspondante
