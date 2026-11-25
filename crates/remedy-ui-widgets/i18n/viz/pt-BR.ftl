bars-value =
    { $count ->
        [one] { $count } barra, a mais alta em { $percent }%
        [many] { $count } barras, a mais alta em { $percent }%
       *[other] { $count } barras, a mais alta em { $percent }%
    }
meter-value-normal = { $percent }%, normal
meter-value-loud = { $percent }%, alto
meter-value-clipping = { $percent }%, saturado
meter-threshold = Limite de { $label }
meter-percent = { $percent }%
ring-speaking = falando
ring-silent = em silêncio
ring-muted = sem áudio
sparkline-no-data = sem dados
sparkline-latest = último valor: { $value }
