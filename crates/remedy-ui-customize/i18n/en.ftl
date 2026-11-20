# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } and { $second }
list-middle = { $first }, { $rest }
list-end = { $first } and { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Theme
area-shortcuts = Shortcuts
area-toolbars = Toolbars and menus
area-text = Text
area-layouts = Screen layouts

## The settings page.
customize = Customize
customize-status = Customize status
nothing-to-customize = Nothing to customize
nothing-to-customize-hint = This app has not turned customization on.
reset-everything = Reset everything
reset-everything-done = Everything is back to the app's defaults. Undo brings it back.
# Also the undo history's name for resetting everything.
reset-all-customizations = Reset all customizations

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Reset
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = Reset { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = Change { $name }
# { $error } says why.
could-not-reset = Could not reset: { $error }
not-saved = Not saved: { $error }
could-not-change = Could not change that: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = { $area } could not be used ({ $file }): { $message }
notice-dismiss = Dismiss
# The dismiss button's accessible name.
notice-dismiss-named = Dismiss the notice about { $file }
notice-not-utf8 = the file is not UTF-8 text
notice-could-not-save = could not save: { $error }
notice-could-not-restore = could not restore: { $detail }

## Errors.
error-not-enabled-theme = Theme customization is not on in this app
error-not-enabled-shortcuts = Shortcut customization is not on in this app
error-not-enabled-toolbars = Toolbar customization is not on in this app
error-not-enabled-text = Text customization is not on in this app
error-not-enabled-layouts = Layout customization is not on in this app
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } is used by { $commands }
error-refused = refused: { $parts }
error-unknown-command = no command { $id }
error-unknown-slot = no toolbar { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } does not hold that kind of value

## Theme editor.
theme-editor = Theme editor
theme-status = Theme status
theme-off = Theme customization is off
theme-off-hint = This app does not let its theme be changed.
section-colors = Colors
# The section of fonts and text sizes.
section-text = Text
section-shape = Spacing and shape
section-motion = Motion
reset-theme = Reset theme
# The button that opens a color's picker; its accessible name is edit-named.
edit = Edit
edit-named = Edit { $name }
# Units after a number.
unit-px = px
unit-ms = ms

## Design tokens, as people call them.
token-theme-name = Theme name
token-color-bg = Background
token-color-panel = Panels
token-color-card = Cards
token-color-text = Text
token-color-muted = Muted text
token-color-accent = Accent
token-color-on-accent = Text on accent
token-color-danger = Danger
token-color-warn = Warning
token-color-ok = Success
token-color-line = Lines
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Color “{ $name }”
token-font = Font
token-font-mono = Monospace font
token-size-small = Small text size
token-size-body = Body text size
token-size-title = Title size
token-spacing-unit = Spacing unit
token-control-height = Control height
token-radius-small = Small corner radius
token-radius-medium = Medium corner radius
token-radius-large = Large corner radius
token-stroke = Line width
token-motion-quick = Quick motion
token-motion-standard = Standard motion
token-motion-emphasized = Emphasized motion
token-density = Density
token-background-image = Background image
token-font-file = Font file “{ $name }”

## How tightly controls are packed.
density-compact = Compact
density-comfortable = Comfortable
density-spacious = Spacious

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Contrast
contrast-all-pass = Every color pair meets WCAG contrast.
# The contrast status read by screen readers.
contrast-status-all-pass = Every color pair meets WCAG contrast
contrast-status-failing = { $count ->
    [one] { $count } color pair is below WCAG contrast
   *[other] { $count } color pairs are below WCAG contrast
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, below the { NUMBER($needed, minimumFractionDigits: 1) }:1 it needs ({ $foreground } on { $background })
contrast-pair-text-bg = Text on the background
contrast-pair-text-panel = Text on panels
contrast-pair-text-card = Text on cards
contrast-pair-muted-bg = Muted text on the background
contrast-pair-muted-panel = Muted text on panels
contrast-pair-muted-card = Muted text on cards
contrast-pair-on-accent = Text on the accent
contrast-pair-accent-bg = The accent on the background

## The preview: sample widgets in the theme being edited.
preview = Preview
preview-group = Theme preview
preview-field = Sample field
preview-field-text = Text in a field
preview-body = Body text reads like this, at the body size.
preview-muted = Muted text, for hints and details.
preview-primary = Primary
preview-secondary = Secondary
preview-danger = Danger
preview-switch = A switch
preview-progress = Progress
preview-link = A link
preview-error = An error message.

## Sharing themes.
section-share = Share
share-hint = Themes are design-tokens files that every Remedy app can read.
theme-file = Theme file
theme-file-placeholder = A path such as themes/ocean.json
import-from-path = Import from path
export-to-path = Export to path
import-path-first = Write the theme file's path first
export-path-first = Write where to save the theme first
choose-theme = Choose a theme to import…
export-to-file = Export to a file…
# File dialog titles and the name of their file type filter.
dialog-import-theme = Import a theme
dialog-export-theme = Export the theme
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = theme
imported = Imported the theme { $name }
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Imported the theme { $name }, leaving out { $count } part
   *[other] Imported the theme { $name }, leaving out { $count } parts
}
could-not-import = Could not import that theme: { $error }
could-not-read = Could not read { $path }: { $detail }
exported = Exported the theme to { $path }
could-not-export = Could not export the theme: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = This theme carries more than colors and sizes
theme-carries-more-detail = This theme carries more than colors and sizes. These are off unless you turn them on:
allow-shaders = Allow shaders (code that runs on the graphics card)
allow-links = Allow external links (kept, never fetched)
allow-files = Use the images and fonts it brings
import = Import
import-without = Import without them
cancel = Cancel
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: a shader ({ $detail })
refusal-link = { $token }: an external link ({ $detail })
refusal-file = { $token }: a file the theme brings ({ $detail })
theme-not-object = a theme file is a JSON object
# { $detail } is the JSON reader's message.
not-design-tokens = not a design tokens file: { $detail }
history-import-theme = Import theme { $name }

## The theme's files: its background image and fonts.
section-files = Files
files-hint = Images show behind every window, sized to it; fonts are checked before use.
background-inline = Background: an image inside the theme
# { $file } is the image's path or link.
background-file = Background: { $file }
no-background = No background image
background-file-field = Background image file
background-placeholder = A path such as pictures/waves.png
use-image = Use this image
image-path-first = Write the image's path first
reset-background = Reset background image
choose-image = Choose an image…
dialog-choose-image = Choose a background image
filter-images = Images
no-fonts = The theme brings no fonts.
remove = Remove
remove-font-named = Remove font { $family }
font-file = Font file
font-file-placeholder = A path such as fonts/Ocean-Regular.ttf
font-name = Font name
font-name-placeholder = What the font is called, such as Ocean
add-font = Add font
font-first = Write the font file's path and its name first
history-change-background = Change the background image
history-remove-background = Remove the background image
history-add-font = Add the font { $family }
history-remove-font = Remove the font { $family }
font-needs-name = a font needs a name
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = the font { $family } could not be used: { $detail }
background-not-used = the background image could not be used: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = a theme's files are inside its folder
asset-needs-file = import the theme from its file, so its files can be found
asset-link = { $link } is a link, and links are not fetched
not-an-image = not an image this app can show: { $detail }
not-data-uri = not a data: URI
data-uri-comma = a data: URI has a comma before its data
broken-escape = a broken % escape
# { $char } is a character, in quotes.
not-base64 = { $char } is not base64

## Shortcut editor.
shortcut-editor = Shortcut editor
shortcut-status = Shortcut status
shortcuts-off = Shortcut customization is off
shortcuts-off-hint = This app does not let its shortcuts be changed.
search-commands = Search commands
commands = Commands
no-matching-command = No matching command
no-shortcut = No shortcut
reset-all-shortcuts = Reset all shortcuts
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Add a shortcut to { $command }
shortcut-remove-named = Remove { $chord } from { $command }
reset-shortcuts-named = Reset shortcuts for { $command }
shortcut-now-runs = { $chord } now runs { $command }
shortcut-already-runs = { $chord } already runs { $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } already runs { $commands }.
shortcut-take = Use { $chord } for { $command }
shortcut-keep = Keep it for { $commands }
history-change-shortcuts = Change shortcuts for { $command }
history-add-shortcut = Add { $chord } to { $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = Give { $chord } to { $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = left out, the rest applies: { $parts }
shortcuts-clash-two = { $commands } would both answer to { $chord }
shortcuts-clash-many = { $commands } would all answer to { $chord }

## Toolbar editor.
toolbar-editor = Toolbar editor
toolbar-status = Toolbar status
toolbars-off = Toolbar customization is off
toolbars-off-hint = This app does not let its toolbars be changed.
no-toolbars = This app has no toolbars to change.
reset-all-toolbars = Reset all toolbars
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = not a toolbars file: { $detail }
toolbars-not-object = a toolbars file is an object of slots to lists of command ids
toolbar-slot-not-list = { $slot }: a slot is a list of command ids such as ["file.save"]
toolbar-ids-not-text = { $slot }: command ids are text

## Text editor.
text-editor = Text editor
text-status = Text status
text-off = Text customization is off
text-off-hint = This app does not let its text be changed.
# { $folder } is a folder's path.
text-folder = Your message files are in { $folder }
read-files-again = Read the files again
search-text = Search text
strings = Strings
no-matching-text = No matching text
reset-all-text = Reset all text
could-not-change-text = Could not change that text: { $error }
could-not-reset-text = Could not reset that text: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = Reset “{ $text }”
change-text = Change “{ $text }”
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Messages
search-messages = Search messages
no-matching-message = No matching message
messages-hint = Fluent messages: text, placeholders such as {"{"} $name {"}"}, and choices.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = { $id } check
text-skipped = skipped: { $detail }
text-part-unread = part of the file did not read, the rest is used: { $detail }

## Layout editor.
layout-editor = Layout editor
layout-status = Layout status
layouts-off = Layout customization is off
layouts-off-hint = This app does not let its screens be rearranged.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Your layouts are in { $folder }
no-layouts = No screen that can be rearranged is open.
reset-all-layouts = Reset all layouts
layout-named = Layout { $file }
layout-check = Layout check
layout-ok = No problems: the layout can be saved.
layout-saved = Saved. The screen shows your layout.
save = Save
save-layout-named = Save layout { $file }
reset-layout-named = Reset layout { $file }
no-screen = no screen { $file } is shown
