bars-value =
    { $count ->
        [one] { $count } Balken, höchster bei { $percent } %
       *[other] { $count } Balken, höchster bei { $percent } %
    }
meter-value-normal = { $percent } %, normal
meter-value-loud = { $percent } %, laut
meter-value-clipping = { $percent } %, übersteuert
meter-threshold = Schwellenwert für { $label }
meter-percent = { $percent } %
ring-speaking = spricht
ring-silent = still
ring-muted = stummgeschaltet
sparkline-no-data = keine Daten
sparkline-latest = neuester Wert: { $value }
