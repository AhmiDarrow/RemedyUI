# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = একটি সংখ্যা লিখুন
number-invalid = এটি কোনো সংখ্যা নয়
number-overflow = অঙ্ক খুব বেশি
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = কমপক্ষে { $value }
number-at-most = সর্বোচ্চ { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = বড় টেক্সটের জন্য AA
contrast-grade-fails = মানদণ্ড পূরণ করে না
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = স্যাচুরেশন ও উজ্জ্বলতা
color-area-value = স্যাচুরেশন { $saturation }%, উজ্জ্বলতা { $brightness }%
color-hue = হিউ
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = অস্বচ্ছতা
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = { $label }-এর হেক্স কোড
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = কনট্রাস্ট { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = কনট্রাস্ট { $ratio } অনুপাত 1, { $grade }

## Calendar.
calendar-previous-month = আগের মাস
calendar-next-month = পরের মাস
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, আজ

## Forms.
# On the submit button and the status line while the save runs.
form-saving = সংরক্ষণ করা হচ্ছে…
form-fix-fields = চালিয়ে যেতে { $count }টি ফিল্ড ঠিক করুন
form-saved = সংরক্ষিত হয়েছে
form-unsaved = অসংরক্ষিত পরিবর্তন

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = একটি শর্টকাট চাপুন…
# Shown when no shortcut is set.
keys-none = কোনোটি নয়
keys-recording = রেকর্ড হচ্ছে: কীগুলি চাপুন, থামাতে Esc চাপুন
keys-idle = শর্টকাট রেকর্ড করতে এখানে ফোকাস করুন
