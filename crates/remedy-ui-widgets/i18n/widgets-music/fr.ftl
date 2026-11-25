# Widgets de musique : ce que disent les lecteurs d’écran, et les noms dans l’historique d’annulation.
list-separator = {", "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] do dièse { $octave }
            [D] ré dièse { $octave }
            [E] mi dièse { $octave }
            [F] fa dièse { $octave }
            [G] sol dièse { $octave }
            [A] la dièse { $octave }
           *[B] si dièse { $octave }
        }
    [flat]
        { $letter ->
            [C] do bémol { $octave }
            [D] ré bémol { $octave }
            [E] mi bémol { $octave }
            [F] fa bémol { $octave }
            [G] sol bémol { $octave }
            [A] la bémol { $octave }
           *[B] si bémol { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] do double dièse { $octave }
            [D] ré double dièse { $octave }
            [E] mi double dièse { $octave }
            [F] fa double dièse { $octave }
            [G] sol double dièse { $octave }
            [A] la double dièse { $octave }
           *[B] si double dièse { $octave }
        }
    [double-flat]
        { $letter ->
            [C] do double bémol { $octave }
            [D] ré double bémol { $octave }
            [E] mi double bémol { $octave }
            [F] fa double bémol { $octave }
            [G] sol double bémol { $octave }
            [A] la double bémol { $octave }
           *[B] si double bémol { $octave }
        }
   *[natural]
        { $letter ->
            [C] do { $octave }
            [D] ré { $octave }
            [E] mi { $octave }
            [F] fa { $octave }
            [G] sol { $octave }
            [A] la { $octave }
           *[B] si { $octave }
        }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] ronde pointée
            [half] blanche pointée
            [quarter] noire pointée
            [eighth] croche pointée
            [sixteenth] double croche pointée
           *[thirty-second] triple croche pointée
        }
    [double-dotted]
        { $value ->
            [whole] ronde doublement pointée
            [half] blanche doublement pointée
            [quarter] noire doublement pointée
            [eighth] croche doublement pointée
            [sixteenth] double croche doublement pointée
           *[thirty-second] triple croche doublement pointée
        }
   *[none]
        { $value ->
            [whole] ronde
            [half] blanche
            [quarter] noire
            [eighth] croche
            [sixteenth] double croche
           *[thirty-second] triple croche
        }
}
event-note = { $pitch }, { $length }
event-note-tied = { $pitch }, { $length }, liée à la suivante
event-chord = accord ({ $length }) de { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] pause pointée
            [half] demi-pause pointée
            [quarter] soupir pointé
            [eighth] demi-soupir pointé
            [sixteenth] quart de soupir pointé
           *[thirty-second] huitième de soupir pointé
        }
    [double-dotted]
        { $value ->
            [whole] pause doublement pointée
            [half] demi-pause doublement pointée
            [quarter] soupir doublement pointé
            [eighth] demi-soupir doublement pointé
            [sixteenth] quart de soupir doublement pointé
           *[thirty-second] huitième de soupir doublement pointé
        }
   *[none]
        { $value ->
            [whole] pause
            [half] demi-pause
            [quarter] soupir
            [eighth] demi-soupir
            [sixteenth] quart de soupir
           *[thirty-second] huitième de soupir
        }
}
key-no-accidentals = aucune altération
key-sharps = { $count ->
    [one] { $count } dièse
    [many] { $count } de dièses
   *[other] { $count } dièses
}
key-flats = { $count ->
    [one] { $count } bémol
    [many] { $count } de bémols
   *[other] { $count } bémols
}
staff-summary = { $clef ->
    [bass] clé de fa
    [alto] clé d’ut 3e ligne
    [tenor] clé d’ut 4e ligne
   *[treble] clé de sol
}, { $key }, { $measures ->
    [one] { $measures } mesure
    [many] { $measures } de mesures
   *[other] { $measures } mesures
}
staff-summary-time = { $clef ->
    [bass] clé de fa
    [alto] clé d’ut 3e ligne
    [tenor] clé d’ut 4e ligne
   *[treble] clé de sol
}, { $key }, mesure à { $beats }/{ $unit }, { $measures ->
    [one] { $measures } mesure
    [many] { $measures } de mesures
   *[other] { $measures } mesures
}
staff-editable = { $summary }. Modifiable : les lettres placent les notes, Haut et Bas les déplacent, les chiffres fixent les durées.
staff-cursor = mesure { $measure }, temps { $beat } : { $event }
cursor-picked = { $what }, sélectionné
cursor-marked = { $what }, marqué
cursor-picked-marked = { $what }, sélectionné, marqué
chord-string-muted = corde { $string } non jouée
chord-string-open = corde { $string } à vide
chord-string-fret = corde { $string } case { $fret }
chord-string-fret-finger = corde { $string } case { $fret } doigt { $finger }
chord-barre = barré en case { $fret } des cordes { $from } à { $to }
chord-barre-finger = barré en case { $fret } des cordes { $from } à { $to } avec le doigt { $finger }
chord-starts-at = le diagramme commence à la case { $fret }
chord-named = { $label } : { $name }
position-open = corde { $string } ({ $tuning }), à vide, { $note }
position-fret = corde { $string } ({ $tuning }), case { $fret }, { $note }
position-open-short = corde { $string }, à vide
position-fret-short = corde { $string }, case { $fret }
fretboard-hint = Les flèches parcourent le manche, Entrée sélectionne, Échap efface la sélection.
fretboard-hint-picked = Les flèches parcourent le manche, Entrée sélectionne, Échap efface la sélection. Sélection : { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } corde
    [many] { $strings } de cordes
   *[other] { $strings } cordes
}, { $frets ->
    [one] { $frets } case
    [many] { $frets } de cases
   *[other] { $frets } cases
}, { $marked } marquées
technique = { $technique ->
    [hammer-on] hammer-on
    [pull-off] pull-off
    [slide-up] glissé vers le haut
    [slide-down] glissé vers le bas
    [bend-half] tiré d’un demi-ton
    [bend-whole] tiré d’un ton
    [vibrato] vibrato
    [harmonic] harmonique
    [palm-mute] étouffé de la paume
    [dead] note morte
   *[tied] liée
}
technique-bend = { $steps ->
    [one] tiré de { $steps } ton
    [many] tiré de { $steps } de tons
   *[other] tiré de { $steps } tons
}
tab-note-fret = corde { $string } : case { $fret }
tab-note-open = corde { $string } : à vide
tab-note-dead = corde { $string } : note morte
tab-no-note = corde { $string } : aucune note
tab-note-technique = { $note }, { $technique }
tab-cursor = mesure { $measure }, temps { $beat }, { $note }
tab-summary = Tablature, { $strings ->
    [one] { $strings } corde
    [many] { $strings } de cordes
   *[other] { $strings } cordes
}, { $measures ->
    [one] { $measures } mesure
    [many] { $measures } de mesures
   *[other] { $measures } mesures
}
tab-editable = { $summary }. Modifiable : tapez un numéro de case, Suppr supprime la note.
edit-set-note = Définir la note
edit-add-to-chord = Ajouter la note à l’accord
edit-octave-up = Monter la note d’une octave
edit-octave-down = Descendre la note d’une octave
edit-semitone-up = Hausser la note d’un demi-ton
edit-semitone-down = Baisser la note d’un demi-ton
edit-step-up = Monter la note
edit-step-down = Descendre la note
edit-delete-measure = Supprimer la mesure
edit-delete-event = Supprimer l’élément
edit-delete-rest = Supprimer le silence
edit-make-rest = Changer en silence
edit-insert-rest = Insérer un silence
edit-insert-measure = Insérer une mesure
edit-set-length = Définir la durée
edit-sharpen = Ajouter un dièse
edit-flatten = Ajouter un bémol
edit-make-natural = Ajouter un bécarre
edit-cycle-dots = Changer les points
edit-shorten = Raccourcir
edit-lengthen = Allonger
edit-tie = Lier
edit-set-fret = Définir la case
edit-delete-note = Supprimer la note
edit-add-note = Ajouter une note
edit-raise-fret = Monter la case
edit-lower-fret = Descendre la case
edit-set-technique = Définir la technique
