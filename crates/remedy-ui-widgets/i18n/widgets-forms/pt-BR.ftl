# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Digite um número
number-invalid = Não é um número
number-overflow = Dígitos demais
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = No mínimo { $value }
number-at-most = No máximo { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA para texto grande
contrast-grade-fails = Não atende
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Saturação e brilho
color-area-value = saturação { $saturation }%, brilho { $brightness }%
color-hue = Matiz
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Opacidade
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = Código hexadecimal de { $label }
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Contraste { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Contraste de { $ratio } para 1, { $grade }

## Calendar.
calendar-previous-month = Mês anterior
calendar-next-month = Próximo mês
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, hoje

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Salvando…
form-fix-fields = { $count ->
    [one] Corrija { $count } campo para continuar
    [many] Corrija { $count } de campos para continuar
   *[other] Corrija { $count } campos para continuar
}
form-saved = Salvo
form-unsaved = Alterações não salvas

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Pressione um atalho…
# Shown when no shortcut is set.
keys-none = Nenhum
keys-recording = Gravando: pressione as teclas, Esc para parar
keys-idle = Selecione o campo para gravar um atalho
