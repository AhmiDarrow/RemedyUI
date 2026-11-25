# The form widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Esc, Umschalt…).

## Number fields: the error shown under the field while the typed text is not accepted.
number-empty = कोई संख्या दर्ज करें
number-invalid = यह संख्या नहीं है
number-overflow = बहुत अधिक अंक
# { $value } is the limit, already written the locale's way ("500.00").
number-at-least = कम से कम { $value }
number-at-most = अधिकतम { $value }

## Color picker.
# WCAG contrast grades: AAA and AA are the standard's level names.
contrast-grade-aaa = AAA
contrast-grade-aa = AA
# Passes AA only for large text.
contrast-grade-aa-large = बड़े टेक्स्ट के लिए AA
contrast-grade-fails = अनुपालन नहीं
# The 2D area: saturation across, brightness up (screen reader name and value).
color-area = संतृप्ति और चमक
color-area-value = संतृप्ति { $saturation }%, चमक { $brightness }%
color-hue = रंगत
# The hue in degrees.
color-hue-value = { $value }°
color-opacity = अपारदर्शिता
color-opacity-value = { $value }%
# The hex code field's name; { $label } is the picker's label ("Accent").
color-hex = { $label } का हेक्स कोड
# Shown under the picker: { $ratio } is like 4.5, { $grade } one of the grades above.
color-contrast = कंट्रास्ट { $ratio }:1 · { $grade }
# The same, as a screen reader says it.
color-contrast-spoken = कंट्रास्ट { $ratio } अनुपात 1, { $grade }

## Calendar.
calendar-previous-month = पिछला महीना
calendar-next-month = अगला महीना
# A day's screen reader name when it is today; { $date } is the long date.
calendar-today = { $date }, आज

## Forms.
# On the submit button and the status line while the save runs.
form-saving = सहेजा जा रहा है…
form-fix-fields = जारी रखने के लिए { $count } फ़ील्ड ठीक करें
form-saved = सहेजा गया
form-unsaved = बिना सहेजे गए बदलाव

## Shortcut recorder.
# Shown in the field while it waits for a key.
keys-prompt = कोई शॉर्टकट दबाएँ…
# Shown when no shortcut is set.
keys-none = कोई नहीं
keys-recording = रिकॉर्ड हो रहा है: कुंजियाँ दबाएँ, रोकने के लिए Esc दबाएँ
keys-idle = शॉर्टकट रिकॉर्ड करने के लिए इस पर फ़ोकस करें
