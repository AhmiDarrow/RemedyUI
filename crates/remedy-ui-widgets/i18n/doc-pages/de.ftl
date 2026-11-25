## Seiten.
page-number = Seite { $page }
pages-list = Seiten
page-view-value = Seite { $page } von { $count }, { $zoom } %

annotation-kind = { $kind ->
    [highlight] Hervorhebung
    [underline] Unterstreichung
    [strikeout] Durchstreichung
    [squiggly] Wellenlinie
    [drawing] Zeichnung
    [note] Notiz
    [text-box] Textfeld
    [rectangle] Rechteck
    [ellipse] Ellipse
    [line] Linie
    [arrow] Pfeil
    [image] Bild
   *[annotation] Anmerkung
}
annotation-quoting = { $what }: „{ $text }“
annotation-with-comment = { $what }, Kommentar: { $comment }
annotation-by = { $what }, von { $author }
annotation-selected = Ausgewählt: { $description }
annotation-count = { $count ->
    [one] { $count } Anmerkung:
   *[other] { $count } Anmerkungen:
}

undo-add = { $kind ->
    [highlight] Hervorhebung hinzufügen
    [underline] Unterstreichung hinzufügen
    [strikeout] Durchstreichung hinzufügen
    [squiggly] Wellenlinie hinzufügen
    [drawing] Zeichnung hinzufügen
    [note] Notiz hinzufügen
    [text-box] Textfeld hinzufügen
    [rectangle] Rechteck hinzufügen
    [ellipse] Ellipse hinzufügen
    [line] Linie hinzufügen
    [arrow] Pfeil hinzufügen
    [image] Bild hinzufügen
   *[annotation] Anmerkung hinzufügen
}
undo-change = { $kind ->
    [highlight] Hervorhebung ändern
    [underline] Unterstreichung ändern
    [strikeout] Durchstreichung ändern
    [squiggly] Wellenlinie ändern
    [drawing] Zeichnung ändern
    [note] Notiz ändern
    [text-box] Textfeld ändern
    [rectangle] Rechteck ändern
    [ellipse] Ellipse ändern
    [line] Linie ändern
    [arrow] Pfeil ändern
    [image] Bild ändern
   *[annotation] Anmerkung ändern
}
undo-delete = { $kind ->
    [highlight] Hervorhebung löschen
    [underline] Unterstreichung löschen
    [strikeout] Durchstreichung löschen
    [squiggly] Wellenlinie löschen
    [drawing] Zeichnung löschen
    [note] Notiz löschen
    [text-box] Textfeld löschen
    [rectangle] Rechteck löschen
    [ellipse] Ellipse löschen
    [line] Linie löschen
    [arrow] Pfeil löschen
    [image] Bild löschen
   *[annotation] Anmerkung löschen
}
undo-move = { $kind ->
    [highlight] Hervorhebung verschieben
    [underline] Unterstreichung verschieben
    [strikeout] Durchstreichung verschieben
    [squiggly] Wellenlinie verschieben
    [drawing] Zeichnung verschieben
    [note] Notiz verschieben
    [text-box] Textfeld verschieben
    [rectangle] Rechteck verschieben
    [ellipse] Ellipse verschieben
    [line] Linie verschieben
    [arrow] Pfeil verschieben
    [image] Bild verschieben
   *[annotation] Anmerkung verschieben
}
