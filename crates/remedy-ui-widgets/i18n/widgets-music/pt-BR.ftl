# Widgets de música: o que os leitores de tela dizem e os nomes no histórico de desfazer.
list-separator = {", "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] dó sustenido { $octave }
            [D] ré sustenido { $octave }
            [E] mi sustenido { $octave }
            [F] fá sustenido { $octave }
            [G] sol sustenido { $octave }
            [A] lá sustenido { $octave }
           *[B] si sustenido { $octave }
        }
    [flat]
        { $letter ->
            [C] dó bemol { $octave }
            [D] ré bemol { $octave }
            [E] mi bemol { $octave }
            [F] fá bemol { $octave }
            [G] sol bemol { $octave }
            [A] lá bemol { $octave }
           *[B] si bemol { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] dó dobrado sustenido { $octave }
            [D] ré dobrado sustenido { $octave }
            [E] mi dobrado sustenido { $octave }
            [F] fá dobrado sustenido { $octave }
            [G] sol dobrado sustenido { $octave }
            [A] lá dobrado sustenido { $octave }
           *[B] si dobrado sustenido { $octave }
        }
    [double-flat]
        { $letter ->
            [C] dó dobrado bemol { $octave }
            [D] ré dobrado bemol { $octave }
            [E] mi dobrado bemol { $octave }
            [F] fá dobrado bemol { $octave }
            [G] sol dobrado bemol { $octave }
            [A] lá dobrado bemol { $octave }
           *[B] si dobrado bemol { $octave }
        }
   *[natural]
        { $letter ->
            [C] dó { $octave }
            [D] ré { $octave }
            [E] mi { $octave }
            [F] fá { $octave }
            [G] sol { $octave }
            [A] lá { $octave }
           *[B] si { $octave }
        }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] semibreve pontuada
            [half] mínima pontuada
            [quarter] semínima pontuada
            [eighth] colcheia pontuada
            [sixteenth] semicolcheia pontuada
           *[thirty-second] fusa pontuada
        }
    [double-dotted]
        { $value ->
            [whole] semibreve com duplo ponto
            [half] mínima com duplo ponto
            [quarter] semínima com duplo ponto
            [eighth] colcheia com duplo ponto
            [sixteenth] semicolcheia com duplo ponto
           *[thirty-second] fusa com duplo ponto
        }
   *[none]
        { $value ->
            [whole] semibreve
            [half] mínima
            [quarter] semínima
            [eighth] colcheia
            [sixteenth] semicolcheia
           *[thirty-second] fusa
        }
}
event-note = { $pitch }, { $length }
event-note-tied = { $pitch }, { $length }, ligada à próxima
event-chord = acorde ({ $length }) de { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] pausa de semibreve pontuada
            [half] pausa de mínima pontuada
            [quarter] pausa de semínima pontuada
            [eighth] pausa de colcheia pontuada
            [sixteenth] pausa de semicolcheia pontuada
           *[thirty-second] pausa de fusa pontuada
        }
    [double-dotted]
        { $value ->
            [whole] pausa de semibreve com duplo ponto
            [half] pausa de mínima com duplo ponto
            [quarter] pausa de semínima com duplo ponto
            [eighth] pausa de colcheia com duplo ponto
            [sixteenth] pausa de semicolcheia com duplo ponto
           *[thirty-second] pausa de fusa com duplo ponto
        }
   *[none]
        { $value ->
            [whole] pausa de semibreve
            [half] pausa de mínima
            [quarter] pausa de semínima
            [eighth] pausa de colcheia
            [sixteenth] pausa de semicolcheia
           *[thirty-second] pausa de fusa
        }
}
key-no-accidentals = sem acidentes
key-sharps = { $count ->
    [one] { $count } sustenido
    [many] { $count } de sustenidos
   *[other] { $count } sustenidos
}
key-flats = { $count ->
    [one] { $count } bemol
    [many] { $count } de bemóis
   *[other] { $count } bemóis
}
staff-summary = { $clef ->
    [bass] clave de fá
    [alto] clave de dó na 3ª linha
    [tenor] clave de dó na 4ª linha
   *[treble] clave de sol
}, { $key }, { $measures ->
    [one] { $measures } compasso
    [many] { $measures } de compassos
   *[other] { $measures } compassos
}
staff-summary-time = { $clef ->
    [bass] clave de fá
    [alto] clave de dó na 3ª linha
    [tenor] clave de dó na 4ª linha
   *[treble] clave de sol
}, { $key }, compasso { $beats }/{ $unit }, { $measures ->
    [one] { $measures } compasso
    [many] { $measures } de compassos
   *[other] { $measures } compassos
}
staff-editable = { $summary }. Editável: letras definem notas, Para cima e Para baixo as movem, dígitos definem a duração.
staff-cursor = compasso { $measure }, tempo { $beat }: { $event }
cursor-picked = { $what }, selecionado
cursor-marked = { $what }, marcado
cursor-picked-marked = { $what }, selecionado, marcado
chord-string-muted = corda { $string } abafada
chord-string-open = corda { $string } solta
chord-string-fret = corda { $string } casa { $fret }
chord-string-fret-finger = corda { $string } casa { $fret } dedo { $finger }
chord-barre = pestana na casa { $fret } da corda { $from } à { $to }
chord-barre-finger = pestana na casa { $fret } da corda { $from } à { $to } com o dedo { $finger }
chord-starts-at = o diagrama começa na casa { $fret }
chord-named = { $label }: { $name }
position-open = corda { $string } ({ $tuning }), solta, { $note }
position-fret = corda { $string } ({ $tuning }), casa { $fret }, { $note }
position-open-short = corda { $string }, solta
position-fret-short = corda { $string }, casa { $fret }
fretboard-hint = As setas percorrem o braço, Enter seleciona, Esc limpa a seleção.
fretboard-hint-picked = As setas percorrem o braço, Enter seleciona, Esc limpa a seleção. Selecionado: { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } corda
    [many] { $strings } de cordas
   *[other] { $strings } cordas
}, { $frets ->
    [one] { $frets } casa
    [many] { $frets } de casas
   *[other] { $frets } casas
}, { $marked } marcadas
technique = { $technique ->
    [hammer-on] hammer-on
    [pull-off] pull-off
    [slide-up] slide para cima
    [slide-down] slide para baixo
    [bend-half] bend de meio tom
    [bend-whole] bend de um tom
    [vibrato] vibrato
    [harmonic] harmônico
    [palm-mute] palm mute
    [dead] nota morta
   *[tied] ligada
}
technique-bend = { $steps ->
    [one] bend de { $steps } tom
    [many] bend de { $steps } de tons
   *[other] bend de { $steps } tons
}
tab-note-fret = corda { $string }: casa { $fret }
tab-note-open = corda { $string }: solta
tab-note-dead = corda { $string }: nota morta
tab-no-note = corda { $string }: sem nota
tab-note-technique = { $note }, { $technique }
tab-cursor = compasso { $measure }, tempo { $beat }, { $note }
tab-summary = Tablatura, { $strings ->
    [one] { $strings } corda
    [many] { $strings } de cordas
   *[other] { $strings } cordas
}, { $measures ->
    [one] { $measures } compasso
    [many] { $measures } de compassos
   *[other] { $measures } compassos
}
tab-editable = { $summary }. Editável: digite o número da casa; Delete remove a nota.
edit-set-note = Definir nota
edit-add-to-chord = Adicionar nota ao acorde
edit-octave-up = Subir nota uma oitava
edit-octave-down = Descer nota uma oitava
edit-semitone-up = Subir nota um semitom
edit-semitone-down = Descer nota um semitom
edit-step-up = Subir nota
edit-step-down = Descer nota
edit-delete-measure = Excluir compasso
edit-delete-event = Excluir evento
edit-delete-rest = Excluir pausa
edit-make-rest = Transformar em pausa
edit-insert-rest = Inserir pausa
edit-insert-measure = Inserir compasso
edit-set-length = Definir duração
edit-sharpen = Adicionar sustenido
edit-flatten = Adicionar bemol
edit-make-natural = Adicionar bequadro
edit-cycle-dots = Alternar pontos
edit-shorten = Encurtar
edit-lengthen = Alongar
edit-tie = Ligar
edit-set-fret = Definir casa
edit-delete-note = Excluir nota
edit-add-note = Adicionar nota
edit-raise-fret = Subir casa
edit-lower-fret = Descer casa
edit-set-technique = Definir técnica
