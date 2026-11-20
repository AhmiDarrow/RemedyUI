# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } und { $second }
list-middle = { $first }, { $rest }
list-end = { $first } und { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Design
area-shortcuts = Tastenkürzel
area-toolbars = Symbolleisten und Menüs
area-text = Texte
area-layouts = Bildschirmlayouts

## The settings page.
customize = Anpassen
customize-status = Status der Anpassungen
nothing-to-customize = Nichts anzupassen
nothing-to-customize-hint = Diese App hat keine Anpassungen aktiviert.
reset-everything = Alles zurücksetzen
reset-everything-done = Alles ist wieder auf den Standardeinstellungen der App. „Rückgängig“ stellt es wieder her.
# Also the undo history's name for resetting everything.
reset-all-customizations = Alle Anpassungen zurücksetzen

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Zurücksetzen
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = { $name } zurücksetzen
# The undo history's name for changing one named thing ("Change Accent").
change-named = { $name } ändern
# { $error } says why.
could-not-reset = Zurücksetzen nicht möglich: { $error }
not-saved = Nicht gespeichert: { $error }
could-not-change = Änderung nicht möglich: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = { $area } konnte nicht verwendet werden ({ $file }): { $message }
notice-dismiss = Ausblenden
# The dismiss button's accessible name.
notice-dismiss-named = Hinweis zu { $file } ausblenden
notice-not-utf8 = die Datei ist kein UTF-8-Text
notice-could-not-save = konnte nicht gespeichert werden: { $error }
notice-could-not-restore = konnte nicht wiederhergestellt werden: { $detail }

## Errors.
error-not-enabled-theme = Designanpassungen sind in dieser App nicht aktiviert
error-not-enabled-shortcuts = Tastenkürzel-Anpassungen sind in dieser App nicht aktiviert
error-not-enabled-toolbars = Symbolleisten-Anpassungen sind in dieser App nicht aktiviert
error-not-enabled-text = Textanpassungen sind in dieser App nicht aktiviert
error-not-enabled-layouts = Layoutanpassungen sind in dieser App nicht aktiviert
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } wird von { $commands } verwendet
error-refused = abgelehnt: { $parts }
error-unknown-command = kein Befehl { $id }
error-unknown-slot = keine Symbolleiste { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } kann diese Art von Wert nicht aufnehmen

## Theme editor.
theme-editor = Design-Editor
theme-status = Designstatus
theme-off = Designanpassung ist deaktiviert
theme-off-hint = Diese App erlaubt keine Änderungen am Design.
section-colors = Farben
# The section of fonts and text sizes.
section-text = Schrift
section-shape = Abstände und Formen
section-motion = Animationen
reset-theme = Design zurücksetzen
# The button that opens a color's picker; its accessible name is edit-named.
edit = Bearbeiten
edit-named = { $name } bearbeiten
# Units after a number.
unit-px = px
unit-ms = ms

## Design tokens, as people call them.
token-theme-name = Name des Designs
token-color-bg = Hintergrund
token-color-panel = Bereiche
token-color-card = Karten
token-color-text = Text
token-color-muted = Gedämpfter Text
token-color-accent = Akzent
token-color-on-accent = Text auf Akzent
token-color-danger = Gefahr
token-color-warn = Warnung
token-color-ok = Erfolg
token-color-line = Linien
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Farbe „{ $name }“
token-font = Schriftart
token-font-mono = Festbreitenschrift
token-size-small = Kleine Textgröße
token-size-body = Textgröße für Fließtext
token-size-title = Titelgröße
token-spacing-unit = Abstandseinheit
token-control-height = Höhe der Bedienelemente
token-radius-small = Kleiner Eckenradius
token-radius-medium = Mittlerer Eckenradius
token-radius-large = Großer Eckenradius
token-stroke = Linienstärke
token-motion-quick = Schnelle Animation
token-motion-standard = Standardanimation
token-motion-emphasized = Betonte Animation
token-density = Dichte
token-background-image = Hintergrundbild
token-font-file = Schriftdatei „{ $name }“

## How tightly controls are packed.
density-compact = Kompakt
density-comfortable = Komfortabel
density-spacious = Großzügig

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Kontrast
contrast-all-pass = Alle Farbpaare erfüllen den WCAG-Kontrast.
# The contrast status read by screen readers.
contrast-status-all-pass = Alle Farbpaare erfüllen den WCAG-Kontrast
contrast-status-failing = { $count ->
    [one] { $count } Farbpaar liegt unter dem WCAG-Kontrast
   *[other] { $count } Farbpaare liegen unter dem WCAG-Kontrast
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, unter den benötigten { NUMBER($needed, minimumFractionDigits: 1) }:1 ({ $foreground } auf { $background })
contrast-pair-text-bg = Text auf dem Hintergrund
contrast-pair-text-panel = Text auf Bereichen
contrast-pair-text-card = Text auf Karten
contrast-pair-muted-bg = Gedämpfter Text auf dem Hintergrund
contrast-pair-muted-panel = Gedämpfter Text auf Bereichen
contrast-pair-muted-card = Gedämpfter Text auf Karten
contrast-pair-on-accent = Text auf dem Akzent
contrast-pair-accent-bg = Akzent auf dem Hintergrund

## The preview: sample widgets in the theme being edited.
preview = Vorschau
preview-group = Designvorschau
preview-field = Beispielfeld
preview-field-text = Text in einem Feld
preview-body = So sieht Fließtext in der Standardgröße aus.
preview-muted = Gedämpfter Text für Hinweise und Details.
preview-primary = Primär
preview-secondary = Sekundär
preview-danger = Gefahr
preview-switch = Ein Schalter
preview-progress = Fortschritt
preview-link = Ein Link
preview-error = Eine Fehlermeldung.

## Sharing themes.
section-share = Teilen
share-hint = Designs sind Design-Token-Dateien, die jede Remedy-App lesen kann.
theme-file = Designdatei
theme-file-placeholder = Ein Pfad wie themes/ocean.json
import-from-path = Aus Pfad importieren
export-to-path = In Pfad exportieren
import-path-first = Geben Sie zuerst den Pfad der Designdatei ein
export-path-first = Geben Sie zuerst an, wo das Design gespeichert werden soll
choose-theme = Design zum Importieren auswählen …
export-to-file = In Datei exportieren …
# File dialog titles and the name of their file type filter.
dialog-import-theme = Design importieren
dialog-export-theme = Design exportieren
filter-design-tokens = Design-Tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = design
imported = Design { $name } importiert
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Design { $name } importiert, { $count } Bestandteil wurde ausgelassen
   *[other] Design { $name } importiert, { $count } Bestandteile wurden ausgelassen
}
could-not-import = Dieses Design konnte nicht importiert werden: { $error }
could-not-read = { $path } konnte nicht gelesen werden: { $detail }
exported = Design nach { $path } exportiert
could-not-export = Das Design konnte nicht exportiert werden: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = Dieses Design enthält mehr als Farben und Größen
theme-carries-more-detail = Dieses Design enthält mehr als Farben und Größen. Folgendes bleibt deaktiviert, sofern Sie es nicht einschalten:
allow-shaders = Shader zulassen (Code, der auf der Grafikkarte ausgeführt wird)
allow-links = Externe Links zulassen (werden beibehalten, aber nie abgerufen)
allow-files = Mitgelieferte Bilder und Schriftarten verwenden
import = Importieren
import-without = Ohne diese importieren
cancel = Abbrechen
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: ein Shader ({ $detail })
refusal-link = { $token }: ein externer Link ({ $detail })
refusal-file = { $token }: eine mitgelieferte Datei ({ $detail })
theme-not-object = eine Designdatei ist ein JSON-Objekt
# { $detail } is the JSON reader's message.
not-design-tokens = keine Design-Token-Datei: { $detail }
history-import-theme = Design { $name } importieren

## The theme's files: its background image and fonts.
section-files = Dateien
files-hint = Bilder erscheinen hinter jedem Fenster in dessen Größe; Schriftarten werden vor der Verwendung geprüft.
background-inline = Hintergrund: ein im Design enthaltenes Bild
# { $file } is the image's path or link.
background-file = Hintergrund: { $file }
no-background = Kein Hintergrundbild
background-file-field = Hintergrundbilddatei
background-placeholder = Ein Pfad wie pictures/waves.png
use-image = Dieses Bild verwenden
image-path-first = Geben Sie zuerst den Pfad des Bildes ein
reset-background = Hintergrundbild zurücksetzen
choose-image = Bild auswählen …
dialog-choose-image = Hintergrundbild auswählen
filter-images = Bilder
no-fonts = Das Design bringt keine Schriftarten mit.
remove = Entfernen
remove-font-named = Schriftart { $family } entfernen
font-file = Schriftdatei
font-file-placeholder = Ein Pfad wie fonts/Ocean-Regular.ttf
font-name = Name der Schriftart
font-name-placeholder = Wie die Schriftart heißt, z. B. Ocean
add-font = Schriftart hinzufügen
font-first = Geben Sie zuerst den Pfad der Schriftdatei und ihren Namen ein
history-change-background = Hintergrundbild ändern
history-remove-background = Hintergrundbild entfernen
history-add-font = Schriftart { $family } hinzufügen
history-remove-font = Schriftart { $family } entfernen
font-needs-name = eine Schriftart braucht einen Namen
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = die Schriftart { $family } konnte nicht verwendet werden: { $detail }
background-not-used = das Hintergrundbild konnte nicht verwendet werden: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = die Dateien eines Designs liegen in seinem Ordner
asset-needs-file = importieren Sie das Design aus seiner Datei, damit seine Dateien gefunden werden
asset-link = { $link } ist ein Link, und Links werden nicht abgerufen
not-an-image = kein Bild, das diese App anzeigen kann: { $detail }
not-data-uri = kein data:-URI
data-uri-comma = ein data:-URI hat vor seinen Daten ein Komma
broken-escape = eine fehlerhafte %-Escape-Sequenz
# { $char } is a character, in quotes.
not-base64 = { $char } ist kein Base64-Zeichen

## Shortcut editor.
shortcut-editor = Tastenkürzel-Editor
shortcut-status = Status der Tastenkürzel
shortcuts-off = Tastenkürzel-Anpassung ist deaktiviert
shortcuts-off-hint = Diese App erlaubt keine Änderungen an ihren Tastenkürzeln.
search-commands = Befehle durchsuchen
commands = Befehle
no-matching-command = Kein passender Befehl
no-shortcut = Kein Tastenkürzel
reset-all-shortcuts = Alle Tastenkürzel zurücksetzen
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Tastenkürzel für { $command } hinzufügen
shortcut-remove-named = { $chord } von { $command } entfernen
reset-shortcuts-named = Tastenkürzel für { $command } zurücksetzen
shortcut-now-runs = { $chord } führt jetzt { $command } aus
shortcut-already-runs = { $chord } führt bereits { $commands } aus
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } führt bereits { $commands } aus.
shortcut-take = { $chord } für { $command } verwenden
shortcut-keep = Für { $commands } beibehalten
history-change-shortcuts = Tastenkürzel für { $command } ändern
history-add-shortcut = { $chord } zu { $command } hinzufügen
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = { $chord } an { $command } vergeben
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = ausgelassen, der Rest gilt: { $parts }
shortcuts-clash-two = { $commands } würden beide auf { $chord } reagieren
shortcuts-clash-many = { $commands } würden alle auf { $chord } reagieren

## Toolbar editor.
toolbar-editor = Symbolleisten-Editor
toolbar-status = Status der Symbolleisten
toolbars-off = Symbolleisten-Anpassung ist deaktiviert
toolbars-off-hint = Diese App erlaubt keine Änderungen an ihren Symbolleisten.
no-toolbars = Diese App hat keine Symbolleisten, die sich ändern lassen.
reset-all-toolbars = Alle Symbolleisten zurücksetzen
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = keine Symbolleistendatei: { $detail }
toolbars-not-object = eine Symbolleistendatei ist ein Objekt, das jedem Platz eine Liste von Befehls-IDs zuordnet
toolbar-slot-not-list = { $slot }: ein Platz ist eine Liste von Befehls-IDs wie ["file.save"]
toolbar-ids-not-text = { $slot }: Befehls-IDs sind Text

## Text editor.
text-editor = Texteditor
text-status = Status der Texte
text-off = Textanpassung ist deaktiviert
text-off-hint = Diese App erlaubt keine Änderungen an ihren Texten.
# { $folder } is a folder's path.
text-folder = Ihre Meldungsdateien befinden sich in { $folder }
read-files-again = Dateien erneut einlesen
search-text = Texte durchsuchen
strings = Zeichenfolgen
no-matching-text = Kein passender Text
reset-all-text = Alle Texte zurücksetzen
could-not-change-text = Dieser Text konnte nicht geändert werden: { $error }
could-not-reset-text = Dieser Text konnte nicht zurückgesetzt werden: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = „{ $text }“ zurücksetzen
change-text = „{ $text }“ ändern
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Meldungen
search-messages = Meldungen durchsuchen
no-matching-message = Keine passende Meldung
messages-hint = Fluent-Meldungen: Text, Platzhalter wie {"{"} $name {"}"} und Auswahlen.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = Prüfung von { $id }
text-skipped = übersprungen: { $detail }
text-part-unread = ein Teil der Datei konnte nicht gelesen werden, der Rest wird verwendet: { $detail }

## Layout editor.
layout-editor = Layout-Editor
layout-status = Status der Layouts
layouts-off = Layoutanpassung ist deaktiviert
layouts-off-hint = Diese App erlaubt nicht, ihre Bildschirme neu anzuordnen.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Ihre Layouts befinden sich in { $folder }
no-layouts = Es ist kein Bildschirm geöffnet, der sich neu anordnen lässt.
reset-all-layouts = Alle Layouts zurücksetzen
layout-named = Layout { $file }
layout-check = Layoutprüfung
layout-ok = Keine Probleme: Das Layout kann gespeichert werden.
layout-saved = Gespeichert. Der Bildschirm zeigt jetzt Ihr Layout.
save = Speichern
save-layout-named = Layout { $file } speichern
reset-layout-named = Layout { $file } zurücksetzen
no-screen = kein Bildschirm { $file } wird angezeigt
