# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = 数値を入力してください
number-invalid = 数値ではありません
number-overflow = 桁数が多すぎます
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = { $value } 以上にしてください
number-at-most = { $value } 以下にしてください

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = AA (大きな文字)
contrast-grade-fails = 不適合
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = 彩度と明度
color-area-value = 彩度 { $saturation }%、明度 { $brightness }%
color-hue = 色相
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = 不透明度
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = { $label } の 16 進コード
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = コントラスト { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = コントラスト { $ratio } 対 1、{ $grade }

## Calendar.
calendar-previous-month = 前の月
calendar-next-month = 次の月
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }、今日

## Forms.
# On the submit button and the status line while the save runs.
form-saving = 保存中…
form-fix-fields = 続行するには { $count } 個の項目を修正してください
form-saved = 保存しました
form-unsaved = 未保存の変更があります

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = ショートカットを押してください…
# Shown when no shortcut is set.
keys-none = なし
keys-recording = 記録中: キーを押してください (Esc で停止)
keys-idle = フォーカスするとショートカットを記録できます
