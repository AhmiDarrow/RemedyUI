# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = أدخل رقمًا
number-invalid = هذا ليس رقمًا
number-overflow = عدد الأرقام كبير جدًا
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = الحد الأدنى { $value }
number-at-most = الحد الأقصى { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA للنص الكبير
contrast-grade-fails = غير مطابق
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = التشبع والسطوع
color-area-value = التشبع { $saturation }%، السطوع { $brightness }%
color-hue = تدرج اللون
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = العتامة
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = الرمز السداسي العشري لـ { $label }
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = التباين { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = التباين { $ratio } إلى 1، { $grade }

## Calendar.
calendar-previous-month = الشهر السابق
calendar-next-month = الشهر التالي
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }، اليوم

## Forms.
# On the submit button and the status line while the save runs.
form-saving = جارٍ الحفظ…
form-fix-fields = { $count ->
    [zero] يُرجى تصحيح { $count } حقل للمتابعة
    [one] يُرجى تصحيح حقل واحد للمتابعة
    [two] يُرجى تصحيح حقلين للمتابعة
    [few] يُرجى تصحيح { $count } حقول للمتابعة
    [many] يُرجى تصحيح { $count } حقلًا للمتابعة
   *[other] يُرجى تصحيح { $count } حقل للمتابعة
}
form-saved = تم الحفظ
form-unsaved = توجد تغييرات غير محفوظة

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = اضغط على اختصار…
# Shown when no shortcut is set.
keys-none = لا يوجد
keys-recording = جارٍ التسجيل: اضغط على المفاتيح، أو Esc للإيقاف
keys-idle = انقل التركيز إلى هنا لتسجيل اختصار
