bars-value =
    { $count ->
        [one] { $count } столбец, максимум { $percent }%
        [few] { $count } столбца, максимум { $percent }%
        [many] { $count } столбцов, максимум { $percent }%
       *[other] { $count } столбца, максимум { $percent }%
    }
meter-value-normal = { $percent }%, норма
meter-value-loud = { $percent }%, громко
meter-value-clipping = { $percent }%, перегрузка
meter-threshold = Порог: { $label }
meter-percent = { $percent }%
ring-speaking = говорит
ring-silent = молчит
ring-muted = микрофон выключен
sparkline-no-data = нет данных
sparkline-latest = последнее значение: { $value }
