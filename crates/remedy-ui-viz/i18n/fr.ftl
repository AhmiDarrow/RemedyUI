bars-value =
    { $count ->
        [one] { $count } barre, la plus haute à { $percent } %
        [many] { $count } barres, la plus haute à { $percent } %
       *[other] { $count } barres, la plus haute à { $percent } %
    }
meter-value-normal = { $percent } %, normal
meter-value-loud = { $percent } %, fort
meter-value-clipping = { $percent } %, saturation
meter-threshold = Seuil de { $label }
meter-percent = { $percent } %
ring-speaking = parle
ring-silent = silencieux
ring-muted = micro coupé
sparkline-no-data = aucune donnée
sparkline-latest = dernière valeur : { $value }
