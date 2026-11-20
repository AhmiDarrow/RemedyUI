# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = Rückgängig
redo = Wiederholen
add-area = Bereich hinzufügen
area-heading = Bereich
area-title = Bereichstitel
area-title-hint = Titel (mit Eingabetaste übernehmen)
area-place = Spalte { $column }, Zeile { $row }, Breite { $columns }, Höhe { $rows }
area-keys-hint = Pfeiltasten verschieben ihn; Umschalt mit Pfeiltasten ändert seine Größe.
area-keys = Die Pfeiltasten verschieben ihn; Umschalt mit den Pfeiltasten ändert seine Größe
remove-area = Bereich entfernen
widget-heading = Widget
settings = Einstellungen
widget-keys-hint = Alt mit Auf und Ab verschiebt es; Alt mit Links und Rechts verschiebt es in den nächsten Bereich.
widget-keys = Alt mit den Pfeiltasten verschiebt es
remove-widget = Widget entfernen
nothing-selected = Wählen Sie einen Bereich oder ein Widget aus, um es zu ändern.
add-widget = Widget hinzufügen
widget-kind = Widget-Art

## The grid.
widget-name = Widget { $name }
unknown-kind = Dieses Board verwendet ein Widget, das diese App nicht hat ({ $kind }).
area-named = Bereich { $name }
area-numbered = Bereich { $number }
empty-area = Leerer Bereich: Fügen Sie ein Widget hinzu oder legen Sie eines hier ab.
empty-board = Leeres Board
empty-board-editing = Dieses Board ist leer: Fügen Sie einen Bereich hinzu.
empty-board-viewing = Hier ist noch nichts.
one-widget = Dieser Bereich enthält bereits ein Widget: Geben Sie dem anderen einen eigenen Bereich.

## Why a change was refused.
error-invalid = kein Board: { $why }
error-too-new = dieses Board wurde mit einer neueren Version erstellt (Format { $format })
error-outside = das würde über den Rand des Boards hinausgehen
error-overlap = das würde den Bereich { $area } verdecken
error-missing = { $id } ist nicht auf dem Board

## Setting editors.
time-date = { $label }: Datum
time-time = { $label }: Uhrzeit
time-not-set = Nicht festgelegt
time-clear = Löschen
choice-unavailable = { $value } (nicht verfügbar)
list-item = { $label } { $number }
list-add = Zu { $label } hinzufügen
list-remove = { $label } { $number } entfernen
picture-source = { $label }: Adresse
picture-choose = Bild auswählen

## Scripted widgets.
stopped = Dieses Widget wurde angehalten: { $why }
start-again = Neu starten

## The built-in widgets.
text-name = Text
text-title = Titel
text-title-default = Willkommen
text-body = Text
text-body-default = Sagen Sie dem Haus Hallo.
links-name = Links
links-title = Titel
links-title-default = Links
links-links = Links (Bezeichnung | https://…, einer pro Zeile)
links-none = Noch keine Links.
reactions-name = Reaktionen
reactions-question = Frage
reactions-question-default = Wie geht es allen?
reactions-offered = Angebotene Reaktionen

## The reactions component.
reaction-group = Reaktionen
reaction-one = 1 Reaktion
reaction-many = { $count } Reaktionen
reaction-count = { $count ->
    [one] { $count } Reaktion
   *[other] { $count } Reaktionen
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, einschließlich Ihrer
