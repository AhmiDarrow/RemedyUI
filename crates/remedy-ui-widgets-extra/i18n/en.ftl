# The extra widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language.

## Carousel: pages that slide. { $page } is the page's number, { $count } how many there are.
# Keyboard hint for the carousel's viewport.
carousel-keys = Arrow keys on the carousel, or the previous and next buttons
# Read by screen readers for the page shown.
carousel-page-of = Page { $page } of { $count }
carousel-previous = Previous page
carousel-next = Next page
# A dot under the carousel that turns to its page.
carousel-go-to-page = Go to page { $page } of { $count }
# The autoplay button's text.
carousel-pause = Pause
carousel-play = Play
# The autoplay button's screen reader name.
carousel-pause-autoplay = Pause automatic page changes
carousel-play-autoplay = Play automatic page changes

## QR codes.
# Read by screen readers. { $level } is the error correction letter (L, M, Q or H).
qr-described = QR code, version { $version }, error correction { $level }
# Shown instead of the code.
qr-too-long = This text is too long for a QR code at this error correction level
qr-bad-options = The QR code options are out of range

## Star ratings, read by screen readers. { $value } may have a decimal (2.5).
rating-value-stars =
    { $max ->
        [one] { $value } of { $max } star
       *[other] { $value } of { $max } stars
    }
# With the app's own unit ("hearts"), which is not translated.
rating-value = { $value } of { $max } { $unit }

## Step-by-step flows (setup wizards).
stepper-back = Back
stepper-next = Next
stepper-skip = Skip
stepper-finish = Finish
# The progress bar's name. { $label } is the flow's name ("Setup").
stepper-progress = { $label } progress
# A step's marker, read by screen readers. { $title } is the step's title.
stepper-marker-completed = Step { $step }: { $title }, completed
stepper-marker-current = Step { $step }: { $title }, current
stepper-marker-not-started = Step { $step }: { $title }, not started
stepper-marker-locked = Step { $step }: { $title }, locked
# Read by screen readers when the step changes.
stepper-step-of-titled = Step { $step } of { $count }: { $title }
# Shown above the step.
stepper-step-of = Step { $step } of { $count }

## Color swatch grids, read by screen readers where the cursor is. { $name } is the color's
## name ("Moss"), { $hex } its code (#5B7F3A).
swatch-cursor = { $name }, { $hex }, row { $row } column { $column }
swatch-cursor-selected = { $name }, { $hex }, row { $row } column { $column }, selected
