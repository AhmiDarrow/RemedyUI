bars-value =
    { $count ->
        [one] { $count } barra, la más alta al { $percent } %
        [many] { $count } barras, la más alta al { $percent } %
       *[other] { $count } barras, la más alta al { $percent } %
    }
meter-value-normal = { $percent } %, normal
meter-value-loud = { $percent } %, alto
meter-value-clipping = { $percent } %, saturado
meter-threshold = Umbral de { $label }
meter-percent = { $percent } %
ring-speaking = hablando
ring-silent = en silencio
ring-muted = silenciado
sparkline-no-data = sin datos
sparkline-latest = último valor: { $value }
