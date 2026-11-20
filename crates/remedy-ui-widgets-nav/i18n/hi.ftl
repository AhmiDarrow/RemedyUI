# The navigation widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Entf, Strg…).

## Tabs, navigation items and breadcrumbs.
# A tab's close button; { $title } is the tab's title.
tab-close = { $title } बंद करें
tab-close-keys = Delete दबाने पर फ़ोकस वाला टैब बंद हो जाता है
# Read after a navigation item's name when it has a count badge ("Inbox, 3 new").
nav-badge-new = { $count ->
    [one] { $count } नया
   *[other] { $count } नए
}
# The "…" button in long breadcrumbs: a menu of the folded path segments.
breadcrumbs-more = और स्थान
# The last breadcrumb, the page being shown; { $segment } is its name.
breadcrumb-current = { $segment }, वर्तमान

## Drawers, sheets and the custom title bar.
drawer-close-keys = Esc दबाने पर पैनल बंद हो जाता है
panel-close = बंद करें
# The group of window controls and title at the top of a frameless window.
title-bar = शीर्षक पट्टी
title-bar-minimize = छोटा करें
title-bar-maximize = बड़ा करें
title-bar-restore = पुनर्स्थापित करें
title-bar-close = बंद करें

## The navigation stack.
stack-back = वापस
# The stack goes back on Alt and the Left arrow key (in every layout direction).
stack-back-keys = वापस जाने के लिए Alt+बायाँ तीर दबाएँ

## Toolbars and the command palette.
# A toolbar button's tooltip: { $command } is its label, { $shortcut } its keyboard shortcut.
command-tooltip = { $command } ({ $shortcut })
# Buttons in the toolbar editor that move a command; { $command } is its label.
toolbar-move-earlier = { $command } को पहले ले जाएँ
toolbar-move-later = { $command } को बाद में ले जाएँ
palette-title = कमांड पैलेट
palette-search = कमांड खोजें
palette-placeholder = कोई कमांड टाइप करें…
# The list of commands in the palette.
palette-commands = कमांड
palette-no-match = कोई मेल खाता कमांड नहीं
