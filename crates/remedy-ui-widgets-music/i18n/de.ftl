# Musik-Widgets: was Screenreader sagen, und Namen im Rückgängig-Verlauf.
list-separator = {", "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] Cis { $octave }
            [D] Dis { $octave }
            [E] Eis { $octave }
            [F] Fis { $octave }
            [G] Gis { $octave }
            [A] Ais { $octave }
           *[B] His { $octave }
        }
    [flat]
        { $letter ->
            [C] Ces { $octave }
            [D] Des { $octave }
            [E] Es { $octave }
            [F] Fes { $octave }
            [G] Ges { $octave }
            [A] As { $octave }
           *[B] B { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] Cisis { $octave }
            [D] Disis { $octave }
            [E] Eisis { $octave }
            [F] Fisis { $octave }
            [G] Gisis { $octave }
            [A] Aisis { $octave }
           *[B] Hisis { $octave }
        }
    [double-flat]
        { $letter ->
            [C] Ceses { $octave }
            [D] Deses { $octave }
            [E] Eses { $octave }
            [F] Feses { $octave }
            [G] Geses { $octave }
            [A] Asas { $octave }
           *[B] Heses { $octave }
        }
   *[natural]
        { $letter ->
            [C] C { $octave }
            [D] D { $octave }
            [E] E { $octave }
            [F] F { $octave }
            [G] G { $octave }
            [A] A { $octave }
           *[B] H { $octave }
        }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] punktierte ganze Note
            [half] punktierte halbe Note
            [quarter] punktierte Viertelnote
            [eighth] punktierte Achtelnote
            [sixteenth] punktierte Sechzehntelnote
           *[thirty-second] punktierte Zweiunddreißigstelnote
        }
    [double-dotted]
        { $value ->
            [whole] doppelt punktierte ganze Note
            [half] doppelt punktierte halbe Note
            [quarter] doppelt punktierte Viertelnote
            [eighth] doppelt punktierte Achtelnote
            [sixteenth] doppelt punktierte Sechzehntelnote
           *[thirty-second] doppelt punktierte Zweiunddreißigstelnote
        }
   *[none]
        { $value ->
            [whole] ganze Note
            [half] halbe Note
            [quarter] Viertelnote
            [eighth] Achtelnote
            [sixteenth] Sechzehntelnote
           *[thirty-second] Zweiunddreißigstelnote
        }
}
event-note = { $pitch }, { $length }
event-note-tied = { $pitch }, { $length }, an die nächste gebunden
event-chord = Akkord ({ $length }) aus { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] punktierte ganze Pause
            [half] punktierte halbe Pause
            [quarter] punktierte Viertelpause
            [eighth] punktierte Achtelpause
            [sixteenth] punktierte Sechzehntelpause
           *[thirty-second] punktierte Zweiunddreißigstelpause
        }
    [double-dotted]
        { $value ->
            [whole] doppelt punktierte ganze Pause
            [half] doppelt punktierte halbe Pause
            [quarter] doppelt punktierte Viertelpause
            [eighth] doppelt punktierte Achtelpause
            [sixteenth] doppelt punktierte Sechzehntelpause
           *[thirty-second] doppelt punktierte Zweiunddreißigstelpause
        }
   *[none]
        { $value ->
            [whole] ganze Pause
            [half] halbe Pause
            [quarter] Viertelpause
            [eighth] Achtelpause
            [sixteenth] Sechzehntelpause
           *[thirty-second] Zweiunddreißigstelpause
        }
}
key-no-accidentals = keine Vorzeichen
key-sharps = { $count ->
    [one] { $count } Kreuz
   *[other] { $count } Kreuze
}
key-flats = { $count ->
    [one] { $count } b
   *[other] { $count } b
}
staff-summary = { $clef ->
    [bass] Bassschlüssel
    [alto] Altschlüssel
    [tenor] Tenorschlüssel
   *[treble] Violinschlüssel
}, { $key }, { $measures ->
    [one] { $measures } Takt
   *[other] { $measures } Takte
}
staff-summary-time = { $clef ->
    [bass] Bassschlüssel
    [alto] Altschlüssel
    [tenor] Tenorschlüssel
   *[treble] Violinschlüssel
}, { $key }, { $beats }/{ $unit }-Takt, { $measures ->
    [one] { $measures } Takt
   *[other] { $measures } Takte
}
staff-editable = { $summary }. Bearbeitbar: Buchstaben setzen Noten, Nach-oben und Nach-unten verschieben sie, Ziffern setzen den Notenwert.
staff-cursor = Takt { $measure }, Schlag { $beat }: { $event }
cursor-picked = { $what }, ausgewählt
cursor-marked = { $what }, markiert
cursor-picked-marked = { $what }, ausgewählt, markiert
chord-string-muted = Saite { $string } nicht gespielt
chord-string-open = Saite { $string } leer
chord-string-fret = Saite { $string } Bund { $fret }
chord-string-fret-finger = Saite { $string } Bund { $fret } Finger { $finger }
chord-barre = Barré im Bund { $fret } über die Saiten { $from } bis { $to }
chord-barre-finger = Barré im Bund { $fret } über die Saiten { $from } bis { $to } mit Finger { $finger }
chord-starts-at = Diagramm beginnt bei Bund { $fret }
chord-named = { $label }: { $name }
position-open = Saite { $string } ({ $tuning }), leer, { $note }
position-fret = Saite { $string } ({ $tuning }), Bund { $fret }, { $note }
position-open-short = Saite { $string }, leer
position-fret-short = Saite { $string }, Bund { $fret }
fretboard-hint = Pfeiltasten bewegen über das Griffbrett, Eingabe wählt aus, Esc hebt die Auswahl auf.
fretboard-hint-picked = Pfeiltasten bewegen über das Griffbrett, Eingabe wählt aus, Esc hebt die Auswahl auf. Ausgewählt: { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } Saite
   *[other] { $strings } Saiten
}, { $frets ->
    [one] { $frets } Bund
   *[other] { $frets } Bünde
}, { $marked } markiert
technique = { $technique ->
    [hammer-on] Hammer-on
    [pull-off] Pull-off
    [slide-up] Slide aufwärts
    [slide-down] Slide abwärts
    [bend-half] Bending um einen Halbton
    [bend-whole] Bending um einen Ganzton
    [vibrato] Vibrato
    [harmonic] Flageolett
    [palm-mute] Palm Mute
    [dead] Dead Note
   *[tied] gebunden
}
technique-bend = { $steps ->
    [one] Bending um { $steps } Ganzton
   *[other] Bending um { $steps } Ganztöne
}
tab-note-fret = Saite { $string }: Bund { $fret }
tab-note-open = Saite { $string }: leer
tab-note-dead = Saite { $string }: Dead Note
tab-no-note = Saite { $string }: keine Note
tab-note-technique = { $note }, { $technique }
tab-cursor = Takt { $measure }, Schlag { $beat }, { $note }
tab-summary = Tabulatur, { $strings ->
    [one] { $strings } Saite
   *[other] { $strings } Saiten
}, { $measures ->
    [one] { $measures } Takt
   *[other] { $measures } Takte
}
tab-editable = { $summary }. Bearbeitbar: Bundnummer eingeben, Entf entfernt die Note.
edit-set-note = Note setzen
edit-add-to-chord = Note zum Akkord hinzufügen
edit-octave-up = Note eine Oktave höher
edit-octave-down = Note eine Oktave tiefer
edit-semitone-up = Note einen Halbton erhöhen
edit-semitone-down = Note einen Halbton erniedrigen
edit-step-up = Note nach oben
edit-step-down = Note nach unten
edit-delete-measure = Takt löschen
edit-delete-event = Element löschen
edit-delete-rest = Pause löschen
edit-make-rest = In Pause umwandeln
edit-insert-rest = Pause einfügen
edit-insert-measure = Takt einfügen
edit-set-length = Notenwert setzen
edit-sharpen = Kreuz setzen
edit-flatten = B setzen
edit-make-natural = Auflösungszeichen setzen
edit-cycle-dots = Punktierung wechseln
edit-shorten = Verkürzen
edit-lengthen = Verlängern
edit-tie = Binden
edit-set-fret = Bund setzen
edit-delete-note = Note löschen
edit-add-note = Note hinzufügen
edit-raise-fret = Bund erhöhen
edit-lower-fret = Bund verringern
edit-set-technique = Spieltechnik setzen
