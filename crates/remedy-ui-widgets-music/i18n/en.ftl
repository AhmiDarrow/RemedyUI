# The music widgets' own words: almost all of it is what screen readers say about notes,
# chords, frets and strings, and the names edits have in the undo history. Placeholders
# ({ $name }) must stay; their order may change. Use the words musicians use in the
# language (the names of note values, clefs and accidentals, guitar techniques).
#
# Selectors that are not numbers:
#   $letter: a note letter, C D E F G A or B (languages that name notes do re mi may select
#     on it); $accidental: natural, sharp, flat, double-sharp, double-flat;
#   $value: whole, half, quarter, eighth, sixteenth, thirty-second;
#   $dots: none, dotted, double-dotted;
#   $clef: treble, bass, alto, tenor.

# Between the items of a list said aloud ("C 4, E 4, G 4").
list-separator = {", "}

## Pitches, as said: { $octave } is the octave number (octave 4 holds middle C).
pitch = { $accidental ->
    [sharp] { $letter } sharp { $octave }
    [flat] { $letter } flat { $octave }
    [double-sharp] { $letter } double sharp { $octave }
    [double-flat] { $letter } double flat { $octave }
   *[natural] { $letter } { $octave }
}

## Notes and rests on a staff.
# A note's length, as said before "note": "dotted quarter" in "C 4 dotted quarter note".
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] dotted whole
            [half] dotted half
            [quarter] dotted quarter
            [eighth] dotted eighth
            [sixteenth] dotted sixteenth
           *[thirty-second] dotted thirty-second
        }
    [double-dotted]
        { $value ->
            [whole] double dotted whole
            [half] double dotted half
            [quarter] double dotted quarter
            [eighth] double dotted eighth
            [sixteenth] double dotted sixteenth
           *[thirty-second] double dotted thirty-second
        }
   *[none]
        { $value ->
            [whole] whole
            [half] half
            [quarter] quarter
            [eighth] eighth
            [sixteenth] sixteenth
           *[thirty-second] thirty-second
        }
}
# { $pitch } is a pitch ("C sharp 4"); { $length } is note-length.
event-note = { $pitch } { $length } note
event-note-tied = { $pitch } { $length } note, tied to the next
# { $pitches } lists the chord's pitches.
event-chord = { $length } chord of { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] dotted whole rest
            [half] dotted half rest
            [quarter] dotted quarter rest
            [eighth] dotted eighth rest
            [sixteenth] dotted sixteenth rest
           *[thirty-second] dotted thirty-second rest
        }
    [double-dotted]
        { $value ->
            [whole] double dotted whole rest
            [half] double dotted half rest
            [quarter] double dotted quarter rest
            [eighth] double dotted eighth rest
            [sixteenth] double dotted sixteenth rest
           *[thirty-second] double dotted thirty-second rest
        }
   *[none]
        { $value ->
            [whole] whole rest
            [half] half rest
            [quarter] quarter rest
            [eighth] eighth rest
            [sixteenth] sixteenth rest
           *[thirty-second] thirty-second rest
        }
}

## The note staff, for screen readers.
key-no-accidentals = no sharps or flats
key-sharps = { $count ->
    [one] { $count } sharp
   *[other] { $count } sharps
}
key-flats = { $count ->
    [one] { $count } flat
   *[other] { $count } flats
}
# { $key } is one of the key- messages; { $beats }/{ $unit } the time signature ("3/4").
staff-summary = { $clef ->
    [bass] bass clef
    [alto] alto clef
    [tenor] tenor clef
   *[treble] treble clef
}, { $key }, { $measures ->
    [one] { $measures } measure
   *[other] { $measures } measures
}
staff-summary-time = { $clef ->
    [bass] bass clef
    [alto] alto clef
    [tenor] tenor clef
   *[treble] treble clef
}, { $key }, { $beats }/{ $unit } time, { $measures ->
    [one] { $measures } measure
   *[other] { $measures } measures
}
# { $summary } is staff-summary.
staff-editable = { $summary }. Editable: letters set notes, Up and Down move them, digits set lengths.
# Where the cursor is: { $beat } may be a fraction ("2.5"); { $event } is an event- message.
staff-cursor = measure { $measure }, beat { $beat }: { $event }
# The place under the cursor is the one picked; { $what } is what is said about it.
cursor-picked = { $what }, picked
cursor-marked = { $what }, marked
cursor-picked-marked = { $what }, picked, marked

## Chord diagrams: strings are numbered from the highest (1).
chord-string-muted = string { $string } muted
chord-string-open = string { $string } open
chord-string-fret = string { $string } fret { $fret }
chord-string-fret-finger = string { $string } fret { $fret } finger { $finger }
chord-barre = barre at fret { $fret } across strings { $from } to { $to }
chord-barre-finger = barre at fret { $fret } across strings { $from } to { $to } with finger { $finger }
chord-starts-at = diagram starts at fret { $fret }
# The diagram's name: { $label } the app's, { $name } the chord's ("Am7").
chord-named = { $label }: { $name }

## Fretboards: { $tuning } is the open string's pitch, { $note } the note at the place.
position-open = string { $string } ({ $tuning }), open, { $note }
position-fret = string { $string } ({ $tuning }), fret { $fret }, { $note }
position-open-short = string { $string }, open
position-fret-short = string { $string }, fret { $fret }
fretboard-hint = Arrow keys move over the neck, Enter picks, Escape clears.
fretboard-hint-picked = Arrow keys move over the neck, Enter picks, Escape clears. Picked: { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } string
   *[other] { $strings } strings
}, { $frets ->
    [one] { $frets } fret
   *[other] { $frets } frets
}, { $marked } marked

## Tablature.
technique = { $technique ->
    [hammer-on] hammer-on
    [pull-off] pull-off
    [slide-up] slide up
    [slide-down] slide down
    [bend-half] bend a half step
    [bend-whole] bend a whole step
    [vibrato] vibrato
    [harmonic] harmonic
    [palm-mute] palm mute
    [dead] dead note
   *[tied] tied
}
# A bend of more than a whole step; { $steps } is in whole steps and may be a fraction (1.5).
technique-bend = { $steps ->
    [one] bend { $steps } step
   *[other] bend { $steps } steps
}
tab-note-fret = string { $string }: fret { $fret }
tab-note-open = string { $string }: open
tab-note-dead = string { $string }: dead note
tab-no-note = string { $string }: no note
# { $note } is a tab-note- message, { $technique } a technique.
tab-note-technique = { $note }, { $technique }
tab-cursor = measure { $measure }, beat { $beat }, { $note }
tab-summary = Tablature, { $strings ->
    [one] { $strings } string
   *[other] { $strings } strings
}, { $measures ->
    [one] { $measures } measure
   *[other] { $measures } measures
}
tab-editable = { $summary }. Editable: type a fret number, Delete removes the note.

## Edits, as the undo history names them ("Undo Set note").
edit-set-note = Set note
edit-add-to-chord = Add note to chord
edit-octave-up = Move note up an octave
edit-octave-down = Move note down an octave
edit-semitone-up = Raise note a semitone
edit-semitone-down = Lower note a semitone
edit-step-up = Move note up
edit-step-down = Move note down
edit-delete-measure = Delete measure
# An event is a note, a chord or a rest.
edit-delete-event = Delete event
edit-delete-rest = Delete rest
edit-make-rest = Make rest
edit-insert-rest = Insert rest
edit-insert-measure = Insert measure
edit-set-length = Set length
edit-sharpen = Sharpen
edit-flatten = Flatten
edit-make-natural = Make natural
edit-cycle-dots = Cycle dots
edit-shorten = Shorten
edit-lengthen = Lengthen
edit-tie = Tie
edit-set-fret = Set fret
edit-delete-note = Delete note
edit-add-note = Add note
edit-raise-fret = Raise fret
edit-lower-fret = Lower fret
edit-set-technique = Set technique
