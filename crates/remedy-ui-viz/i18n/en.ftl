# The visualization widgets' words, read by screen readers. Placeholders ({ $name }) must
# stay; their order may change.

## Bar groups (a spectrum, a set of levels). { $percent } is a whole number from 0 to 100.
bars-value =
    { $count ->
        [one] { $count } bar, highest { $percent } percent
       *[other] { $count } bars, highest { $percent } percent
    }

## Level meters: the level and its zone.
meter-value-normal = { $percent } percent, normal
# Close to the limit.
meter-value-loud = { $percent } percent, loud
# At the limit: the sound is being cut.
meter-value-clipping = { $percent } percent, clipping
# The draggable threshold's name. { $label } is the meter's name ("Microphone").
meter-threshold = { $label } threshold
# The threshold's value.
meter-percent = { $percent } percent

## Speaking rings around a person's picture.
ring-speaking = speaking
ring-silent = silent
# The person's microphone is off.
ring-muted = muted

## Sparklines (small line charts).
sparkline-no-data = no data
# { $value } is the newest value (0.40).
sparkline-latest = latest { $value }
