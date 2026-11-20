# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Enter a number
number-invalid = Not a number
number-overflow = Too many digits
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = At least { $value }
number-at-most = At most { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA large text
contrast-grade-fails = Fails
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Saturation and brightness
color-area-value = saturation { $saturation }%, brightness { $brightness }%
color-hue = Hue
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Opacity
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = { $label } hex
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Contrast { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Contrast { $ratio } to 1, { $grade }

## Calendar.
calendar-previous-month = Previous month
calendar-next-month = Next month
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, today

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Saving…
form-fix-fields = { $count ->
    [one] Fix { $count } field to continue
   *[other] Fix { $count } fields to continue
}
form-saved = Saved
form-unsaved = Unsaved changes

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Press a shortcut…
# Shown when no shortcut is set.
keys-none = None
keys-recording = Recording: press the keys, Escape to stop
keys-idle = Focus to record a shortcut
