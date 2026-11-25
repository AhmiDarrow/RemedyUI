# Музыкальные виджеты: что говорят программы чтения с экрана, и названия в журнале отмены.
list-separator = {", "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] до-диез { $octave }
            [D] ре-диез { $octave }
            [E] ми-диез { $octave }
            [F] фа-диез { $octave }
            [G] соль-диез { $octave }
            [A] ля-диез { $octave }
           *[B] си-диез { $octave }
        }
    [flat]
        { $letter ->
            [C] до-бемоль { $octave }
            [D] ре-бемоль { $octave }
            [E] ми-бемоль { $octave }
            [F] фа-бемоль { $octave }
            [G] соль-бемоль { $octave }
            [A] ля-бемоль { $octave }
           *[B] си-бемоль { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] до-дубль-диез { $octave }
            [D] ре-дубль-диез { $octave }
            [E] ми-дубль-диез { $octave }
            [F] фа-дубль-диез { $octave }
            [G] соль-дубль-диез { $octave }
            [A] ля-дубль-диез { $octave }
           *[B] си-дубль-диез { $octave }
        }
    [double-flat]
        { $letter ->
            [C] до-дубль-бемоль { $octave }
            [D] ре-дубль-бемоль { $octave }
            [E] ми-дубль-бемоль { $octave }
            [F] фа-дубль-бемоль { $octave }
            [G] соль-дубль-бемоль { $octave }
            [A] ля-дубль-бемоль { $octave }
           *[B] си-дубль-бемоль { $octave }
        }
   *[natural]
        { $letter ->
            [C] до { $octave }
            [D] ре { $octave }
            [E] ми { $octave }
            [F] фа { $octave }
            [G] соль { $octave }
            [A] ля { $octave }
           *[B] си { $octave }
        }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] целая нота с точкой
            [half] половинная нота с точкой
            [quarter] четвертная нота с точкой
            [eighth] восьмая нота с точкой
            [sixteenth] шестнадцатая нота с точкой
           *[thirty-second] тридцать вторая нота с точкой
        }
    [double-dotted]
        { $value ->
            [whole] целая нота с двумя точками
            [half] половинная нота с двумя точками
            [quarter] четвертная нота с двумя точками
            [eighth] восьмая нота с двумя точками
            [sixteenth] шестнадцатая нота с двумя точками
           *[thirty-second] тридцать вторая нота с двумя точками
        }
   *[none]
        { $value ->
            [whole] целая нота
            [half] половинная нота
            [quarter] четвертная нота
            [eighth] восьмая нота
            [sixteenth] шестнадцатая нота
           *[thirty-second] тридцать вторая нота
        }
}
event-note = { $pitch }, { $length }
event-note-tied = { $pitch }, { $length }, залигована со следующей
event-chord = аккорд из { $pitches }, длительность: { $length }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] целая пауза с точкой
            [half] половинная пауза с точкой
            [quarter] четвертная пауза с точкой
            [eighth] восьмая пауза с точкой
            [sixteenth] шестнадцатая пауза с точкой
           *[thirty-second] тридцать вторая пауза с точкой
        }
    [double-dotted]
        { $value ->
            [whole] целая пауза с двумя точками
            [half] половинная пауза с двумя точками
            [quarter] четвертная пауза с двумя точками
            [eighth] восьмая пауза с двумя точками
            [sixteenth] шестнадцатая пауза с двумя точками
           *[thirty-second] тридцать вторая пауза с двумя точками
        }
   *[none]
        { $value ->
            [whole] целая пауза
            [half] половинная пауза
            [quarter] четвертная пауза
            [eighth] восьмая пауза
            [sixteenth] шестнадцатая пауза
           *[thirty-second] тридцать вторая пауза
        }
}
key-no-accidentals = без знаков при ключе
key-sharps = { $count ->
    [one] { $count } диез
    [few] { $count } диеза
    [many] { $count } диезов
   *[other] { $count } диеза
}
key-flats = { $count ->
    [one] { $count } бемоль
    [few] { $count } бемоля
    [many] { $count } бемолей
   *[other] { $count } бемоля
}
staff-summary = { $clef ->
    [bass] басовый ключ
    [alto] альтовый ключ
    [tenor] теноровый ключ
   *[treble] скрипичный ключ
}, { $key }, { $measures ->
    [one] { $measures } такт
    [few] { $measures } такта
    [many] { $measures } тактов
   *[other] { $measures } такта
}
staff-summary-time = { $clef ->
    [bass] басовый ключ
    [alto] альтовый ключ
    [tenor] теноровый ключ
   *[treble] скрипичный ключ
}, { $key }, размер { $beats }/{ $unit }, { $measures ->
    [one] { $measures } такт
    [few] { $measures } такта
    [many] { $measures } тактов
   *[other] { $measures } такта
}
staff-editable = { $summary }. Можно редактировать: буквы ставят ноты, стрелки вверх и вниз сдвигают их, цифры задают длительность.
staff-cursor = такт { $measure }, доля { $beat }: { $event }
cursor-picked = { $what }, выбрано
cursor-marked = { $what }, отмечено
cursor-picked-marked = { $what }, выбрано, отмечено
chord-string-muted = струна { $string } заглушена
chord-string-open = струна { $string } открытая
chord-string-fret = струна { $string } на { $fret } ладу
chord-string-fret-finger = струна { $string } на { $fret } ладу пальцем { $finger }
chord-barre = баррэ на { $fret } ладу на струнах с { $from } по { $to }
chord-barre-finger = баррэ пальцем { $finger } на { $fret } ладу на струнах с { $from } по { $to }
chord-starts-at = диаграмма начинается с { $fret } лада
chord-named = { $label }: { $name }
position-open = струна { $string } ({ $tuning }), открытая, { $note }
position-fret = струна { $string } ({ $tuning }), лад { $fret }, { $note }
position-open-short = струна { $string }, открытая
position-fret-short = струна { $string }, лад { $fret }
fretboard-hint = Стрелки перемещают по грифу, Enter выбирает, Esc сбрасывает выбор.
fretboard-hint-picked = Стрелки перемещают по грифу, Enter выбирает, Esc сбрасывает выбор. Выбрано: { $picked }.
fretboard-summary = { $strings ->
    [one] { $strings } струна
    [few] { $strings } струны
    [many] { $strings } струн
   *[other] { $strings } струны
}, { $frets ->
    [one] { $frets } лад
    [few] { $frets } лада
    [many] { $frets } ладов
   *[other] { $frets } лада
}, отмечено: { $marked }
technique = { $technique ->
    [hammer-on] хаммер-он
    [pull-off] пулл-офф
    [slide-up] слайд вверх
    [slide-down] слайд вниз
    [bend-half] подтяжка на полтона
    [bend-whole] подтяжка на тон
    [vibrato] вибрато
    [harmonic] флажолет
    [palm-mute] глушение ладонью
    [dead] мёртвая нота
   *[tied] залиговано
}
technique-bend = { $steps ->
    [one] подтяжка на { $steps } тон
    [few] подтяжка на { $steps } тона
    [many] подтяжка на { $steps } тонов
   *[other] подтяжка на { $steps } тона
}
tab-note-fret = струна { $string }: лад { $fret }
tab-note-open = струна { $string }: открытая
tab-note-dead = струна { $string }: мёртвая нота
tab-no-note = струна { $string }: нет ноты
tab-note-technique = { $note }, { $technique }
tab-cursor = такт { $measure }, доля { $beat }, { $note }
tab-summary = Табулатура, { $strings ->
    [one] { $strings } струна
    [few] { $strings } струны
    [many] { $strings } струн
   *[other] { $strings } струны
}, { $measures ->
    [one] { $measures } такт
    [few] { $measures } такта
    [many] { $measures } тактов
   *[other] { $measures } такта
}
tab-editable = { $summary }. Можно редактировать: введите номер лада, Delete удаляет ноту.
edit-set-note = Изменить ноту
edit-add-to-chord = Добавить ноту в аккорд
edit-octave-up = Сдвинуть ноту на октаву вверх
edit-octave-down = Сдвинуть ноту на октаву вниз
edit-semitone-up = Повысить ноту на полутон
edit-semitone-down = Понизить ноту на полутон
edit-step-up = Сдвинуть ноту вверх
edit-step-down = Сдвинуть ноту вниз
edit-delete-measure = Удалить такт
edit-delete-event = Удалить элемент
edit-delete-rest = Удалить паузу
edit-make-rest = Заменить паузой
edit-insert-rest = Вставить паузу
edit-insert-measure = Вставить такт
edit-set-length = Изменить длительность
edit-sharpen = Поставить диез
edit-flatten = Поставить бемоль
edit-make-natural = Поставить бекар
edit-cycle-dots = Переключить точки
edit-shorten = Укоротить
edit-lengthen = Удлинить
edit-tie = Залиговать
edit-set-fret = Изменить лад
edit-delete-note = Удалить ноту
edit-add-note = Добавить ноту
edit-raise-fret = Повысить лад
edit-lower-fret = Понизить лад
edit-set-technique = Изменить приём
