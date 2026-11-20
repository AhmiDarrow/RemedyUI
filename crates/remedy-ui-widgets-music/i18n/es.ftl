# Widgets de música: lo que dicen los lectores de pantalla y los nombres del historial de deshacer.
list-separator = {", "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] do sostenido { $octave }
            [D] re sostenido { $octave }
            [E] mi sostenido { $octave }
            [F] fa sostenido { $octave }
            [G] sol sostenido { $octave }
            [A] la sostenido { $octave }
           *[B] si sostenido { $octave }
        }
    [flat]
        { $letter ->
            [C] do bemol { $octave }
            [D] re bemol { $octave }
            [E] mi bemol { $octave }
            [F] fa bemol { $octave }
            [G] sol bemol { $octave }
            [A] la bemol { $octave }
           *[B] si bemol { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] do doble sostenido { $octave }
            [D] re doble sostenido { $octave }
            [E] mi doble sostenido { $octave }
            [F] fa doble sostenido { $octave }
            [G] sol doble sostenido { $octave }
            [A] la doble sostenido { $octave }
           *[B] si doble sostenido { $octave }
        }
    [double-flat]
        { $letter ->
            [C] do doble bemol { $octave }
            [D] re doble bemol { $octave }
            [E] mi doble bemol { $octave }
            [F] fa doble bemol { $octave }
            [G] sol doble bemol { $octave }
            [A] la doble bemol { $octave }
           *[B] si doble bemol { $octave }
        }
   *[natural]
        { $letter ->
            [C] do { $octave }
            [D] re { $octave }
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
            [whole] redonda con puntillo
            [half] blanca con puntillo
            [quarter] negra con puntillo
            [eighth] corchea con puntillo
            [sixteenth] semicorchea con puntillo
           *[thirty-second] fusa con puntillo
        }
    [double-dotted]
        { $value ->
            [whole] redonda con doble puntillo
            [half] blanca con doble puntillo
            [quarter] negra con doble puntillo
            [eighth] corchea con doble puntillo
            [sixteenth] semicorchea con doble puntillo
           *[thirty-second] fusa con doble puntillo
        }
   *[none]
        { $value ->
            [whole] redonda
            [half] blanca
            [quarter] negra
            [eighth] corchea
            [sixteenth] semicorchea
           *[thirty-second] fusa
        }
}
event-note = { $pitch }, { $length }
event-note-tied = { $pitch }, { $length }, ligada a la siguiente
event-chord = acorde ({ $length }) de { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] silencio de redonda con puntillo
            [half] silencio de blanca con puntillo
            [quarter] silencio de negra con puntillo
            [eighth] silencio de corchea con puntillo
            [sixteenth] silencio de semicorchea con puntillo
           *[thirty-second] silencio de fusa con puntillo
        }
    [double-dotted]
        { $value ->
            [whole] silencio de redonda con doble puntillo
            [half] silencio de blanca con doble puntillo
            [quarter] silencio de negra con doble puntillo
            [eighth] silencio de corchea con doble puntillo
            [sixteenth] silencio de semicorchea con doble puntillo
           *[thirty-second] silencio de fusa con doble puntillo
        }
   *[none]
        { $value ->
            [whole] silencio de redonda
            [half] silencio de blanca
            [quarter] silencio de negra
            [eighth] silencio de corchea
            [sixteenth] silencio de semicorchea
           *[thirty-second] silencio de fusa
        }
}
key-no-accidentals = sin alteraciones
key-sharps = { $count ->
    [one] { $count } sostenido
    [many] { $count } de sostenidos
   *[other] { $count } sostenidos
}
key-flats = { $count ->
    [one] { $count } bemol
    [many] { $count } de bemoles
   *[other] { $count } bemoles
}
staff-summary = { $clef ->
    [bass] clave de fa
    [alto] clave de do en tercera
    [tenor] clave de do en cuarta
   *[treble] clave de sol
}, { $key }, { $measures ->
    [one] { $measures } compás
    [many] { $measures } de compases
   *[other] { $measures } compases
}
staff-summary-time = { $clef ->
    [bass] clave de fa
    [alto] clave de do en tercera
    [tenor] clave de do en cuarta
   *[treble] clave de sol
}, { $key }, compás de { $beats }/{ $unit }, { $measures ->
    [one] { $measures } compás
    [many] { $measures } de compases
   *[other] { $measures } compases
}
staff-editable = { $summary }. Editable: las letras ponen notas, Arriba y Abajo las mueven, los números fijan la duración.
staff-cursor = compás { $measure }, tiempo { $beat }: { $event }
cursor-picked = { $what }, seleccionado
cursor-marked = { $what }, marcado
cursor-picked-marked = { $what }, seleccionado, marcado
chord-string-muted = cuerda { $string } silenciada
chord-string-open = cuerda { $string } al aire
chord-string-fret = cuerda { $string } traste { $fret }
chord-string-fret-finger = cuerda { $string } traste { $fret } dedo { $finger }
chord-barre = cejilla en el traste { $fret } de la cuerda { $from } a la { $to }
chord-barre-finger = cejilla en el traste { $fret } de la cuerda { $from } a la { $to } con el dedo { $finger }
chord-starts-at = el diagrama empieza en el traste { $fret }
chord-named = { $label }: { $name }
position-open = cuerda { $string } ({ $tuning }), al aire, { $note }
position-fret = cuerda { $string } ({ $tuning }), traste { $fret }, { $note }
position-open-short = cuerda { $string }, al aire
position-fret-short = cuerda { $string }, traste { $fret }
fretboard-hint = Las flechas recorren el mástil, Intro selecciona, Esc borra la selección.
fretboard-hint-picked = Las flechas recorren el mástil, Intro selecciona, Esc borra la selección. Seleccionado: { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } cuerda
    [many] { $strings } de cuerdas
   *[other] { $strings } cuerdas
}, { $frets ->
    [one] { $frets } traste
    [many] { $frets } de trastes
   *[other] { $frets } trastes
}, { $marked } marcadas
technique = { $technique ->
    [hammer-on] hammer-on
    [pull-off] pull-off
    [slide-up] slide ascendente
    [slide-down] slide descendente
    [bend-half] bend de medio tono
    [bend-whole] bend de un tono
    [vibrato] vibrato
    [harmonic] armónico
    [palm-mute] palm mute
    [dead] nota muerta
   *[tied] ligada
}
technique-bend = { $steps ->
    [one] bend de { $steps } tono
    [many] bend de { $steps } de tonos
   *[other] bend de { $steps } tonos
}
tab-note-fret = cuerda { $string }: traste { $fret }
tab-note-open = cuerda { $string }: al aire
tab-note-dead = cuerda { $string }: nota muerta
tab-no-note = cuerda { $string }: sin nota
tab-note-technique = { $note }, { $technique }
tab-cursor = compás { $measure }, tiempo { $beat }, { $note }
tab-summary = Tablatura, { $strings ->
    [one] { $strings } cuerda
    [many] { $strings } de cuerdas
   *[other] { $strings } cuerdas
}, { $measures ->
    [one] { $measures } compás
    [many] { $measures } de compases
   *[other] { $measures } compases
}
tab-editable = { $summary }. Editable: escriba un número de traste; Supr elimina la nota.
edit-set-note = Poner nota
edit-add-to-chord = Añadir nota al acorde
edit-octave-up = Subir nota una octava
edit-octave-down = Bajar nota una octava
edit-semitone-up = Subir nota un semitono
edit-semitone-down = Bajar nota un semitono
edit-step-up = Subir nota
edit-step-down = Bajar nota
edit-delete-measure = Eliminar compás
edit-delete-event = Eliminar evento
edit-delete-rest = Eliminar silencio
edit-make-rest = Convertir en silencio
edit-insert-rest = Insertar silencio
edit-insert-measure = Insertar compás
edit-set-length = Cambiar duración
edit-sharpen = Añadir sostenido
edit-flatten = Añadir bemol
edit-make-natural = Poner becuadro
edit-cycle-dots = Cambiar puntillos
edit-shorten = Acortar
edit-lengthen = Alargar
edit-tie = Ligar
edit-set-fret = Poner traste
edit-delete-note = Eliminar nota
edit-add-note = Añadir nota
edit-raise-fret = Subir traste
edit-lower-fret = Bajar traste
edit-set-technique = Poner técnica
