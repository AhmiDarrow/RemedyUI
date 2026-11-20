# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Saisissez un nombre
number-invalid = Ce n’est pas un nombre
number-overflow = Trop de chiffres
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = Au moins { $value }
number-at-most = Au plus { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA pour le texte de grande taille
contrast-grade-fails = Non conforme
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Saturation et luminosité
color-area-value = saturation { $saturation } %, luminosité { $brightness } %
color-hue = Teinte
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Opacité
color-opacity-value = { $value } %
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = Code hexadécimal de { $label }
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Contraste { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Contraste de { $ratio } pour 1, { $grade }

## Calendar.
calendar-previous-month = Mois précédent
calendar-next-month = Mois suivant
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, aujourd’hui

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Enregistrement…
form-fix-fields = { $count ->
    [one] Corrigez { $count } champ pour continuer
    [many] Corrigez { $count } de champs pour continuer
   *[other] Corrigez { $count } champs pour continuer
}
form-saved = Enregistré
form-unsaved = Modifications non enregistrées

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Appuyez sur un raccourci…
# Shown when no shortcut is set.
keys-none = Aucun
keys-recording = Enregistrement : appuyez sur les touches, Échap pour arrêter
keys-idle = Activez le champ pour enregistrer un raccourci
