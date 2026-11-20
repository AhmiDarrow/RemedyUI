# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = 请输入数字
number-invalid = 不是有效的数字
number-overflow = 位数过多
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = 不能小于 { $value }
number-at-most = 不能大于 { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA (大号文本)
contrast-grade-fails = 不达标
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = 饱和度和亮度
color-area-value = 饱和度 { $saturation }%，亮度 { $brightness }%
color-hue = 色相
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = 不透明度
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = { $label }的十六进制值
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = 对比度 { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = 对比度 { $ratio } 比 1，{ $grade }

## Calendar.
calendar-previous-month = 上个月
calendar-next-month = 下个月
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }，今天

## Forms.
# On the submit button and the status line while the save runs.
form-saving = 正在保存…
form-fix-fields = 请更正 { $count } 个字段后继续
form-saved = 已保存
form-unsaved = 有未保存的更改

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = 请按快捷键…
# Shown when no shortcut is set.
keys-none = 无
keys-recording = 正在录制：请按下按键，按 Esc 停止
keys-idle = 聚焦此处以录制快捷键
