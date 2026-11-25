# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = Введите число
number-invalid = Это не число
number-overflow = Слишком много цифр
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = Не меньше { $value }
number-at-most = Не больше { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA для крупного текста
contrast-grade-fails = Не соответствует
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = Насыщенность и яркость
color-area-value = насыщенность { $saturation } %, яркость { $brightness } %
color-hue = Оттенок
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = Непрозрачность
color-opacity-value = { $value } %
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = Шестнадцатеричный код ({ $label })
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = Контраст { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = Контраст { $ratio } к 1, { $grade }

## Calendar.
calendar-previous-month = Предыдущий месяц
calendar-next-month = Следующий месяц
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, сегодня

## Forms.
# On the submit button and the status line while the save runs.
form-saving = Сохранение…
form-fix-fields = { $count ->
    [one] Исправьте { $count } поле, чтобы продолжить
    [few] Исправьте { $count } поля, чтобы продолжить
    [many] Исправьте { $count } полей, чтобы продолжить
   *[other] Исправьте { $count } поля, чтобы продолжить
}
form-saved = Сохранено
form-unsaved = Несохранённые изменения

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = Нажмите сочетание клавиш…
# Shown when no shortcut is set.
keys-none = Нет
keys-recording = Запись: нажмите клавиши; Esc — остановить
keys-idle = Выберите поле, чтобы записать сочетание клавиш
