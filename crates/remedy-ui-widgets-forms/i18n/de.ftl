# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Geben Sie eine Zahl ein
number-invalid = Keine Zahl
number-overflow = Zu viele Ziffern
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = Mindestens { $value }
number-at-most = Höchstens { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA für großen Text
contrast-grade-fails = Nicht bestanden
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Sättigung und Helligkeit
color-area-value = Sättigung { $saturation } %, Helligkeit { $brightness } %
color-hue = Farbton
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Deckkraft
color-opacity-value = { $value } %
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = Hexwert für { $label }
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Kontrast { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Kontrast { $ratio } zu 1, { $grade }

## Calendar.
calendar-previous-month = Vorheriger Monat
calendar-next-month = Nächster Monat
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, heute

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Wird gespeichert…
form-fix-fields = { $count ->
    [one] Korrigieren Sie { $count } Feld, um fortzufahren
   *[other] Korrigieren Sie { $count } Felder, um fortzufahren
}
form-saved = Gespeichert
form-unsaved = Nicht gespeicherte Änderungen

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Tastenkombination drücken…
# Shown when no shortcut is set.
keys-none = Keine
keys-recording = Aufnahme: Tasten drücken, Esc zum Beenden
keys-idle = Fokussieren, um eine Tastenkombination aufzunehmen
