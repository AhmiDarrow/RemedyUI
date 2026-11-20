# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Introduzca un número
number-invalid = No es un número
number-overflow = Demasiados dígitos
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = Como mínimo { $value }
number-at-most = Como máximo { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA para texto grande
contrast-grade-fails = No cumple
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Saturación y brillo
color-area-value = saturación { $saturation } %, brillo { $brightness } %
color-hue = Tono
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Opacidad
color-opacity-value = { $value } %
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = Código hexadecimal de { $label }
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Contraste { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Contraste de { $ratio } a 1, { $grade }

## Calendar.
calendar-previous-month = Mes anterior
calendar-next-month = Mes siguiente
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, hoy

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Guardando…
form-fix-fields = { $count ->
    [one] Corrija { $count } campo para continuar
    [many] Corrija { $count } de campos para continuar
   *[other] Corrija { $count } campos para continuar
}
form-saved = Guardado
form-unsaved = Cambios sin guardar

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Pulse un atajo…
# Shown when no shortcut is set.
keys-none = Ninguno
keys-recording = Grabando: pulse las teclas; Esc para detener
keys-idle = Seleccione el campo para grabar un atajo
