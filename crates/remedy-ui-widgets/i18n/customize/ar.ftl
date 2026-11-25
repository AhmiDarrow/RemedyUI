# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } و{ $second }
list-middle = { $first }، { $rest }
list-end = { $first } و{ $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }؛ { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = السمة
area-shortcuts = اختصارات لوحة المفاتيح
area-toolbars = أشرطة الأدوات والقوائم
area-text = النصوص
area-layouts = تخطيطات الشاشات

## The settings page.
customize = تخصيص
customize-status = حالة التخصيص
nothing-to-customize = لا يوجد ما يمكن تخصيصه
nothing-to-customize-hint = لم يفعّل هذا التطبيق التخصيص.
reset-everything = إعادة تعيين كل شيء
reset-everything-done = عاد كل شيء إلى الإعدادات الافتراضية للتطبيق. يمكنك استعادته بالتراجع.
# Also the undo history's name for resetting everything.
reset-all-customizations = إعادة تعيين كل التخصيصات

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = إعادة تعيين
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = إعادة تعيين { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = تغيير { $name }
# { $error } says why.
could-not-reset = تعذّرت إعادة التعيين: { $error }
not-saved = لم يتم الحفظ: { $error }
could-not-change = تعذّر إجراء هذا التغيير: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = تعذّر استخدام { $area } ({ $file }): { $message }
notice-dismiss = تجاهل
# The dismiss button's accessible name.
notice-dismiss-named = تجاهل الإشعار المتعلق بالملف { $file }
notice-not-utf8 = الملف ليس نصًا بترميز UTF-8
notice-could-not-save = تعذّر الحفظ: { $error }
notice-could-not-restore = تعذّرت الاستعادة: { $detail }

## Errors.
error-not-enabled-theme = تخصيص السمة غير مفعّل في هذا التطبيق
error-not-enabled-shortcuts = تخصيص اختصارات لوحة المفاتيح غير مفعّل في هذا التطبيق
error-not-enabled-toolbars = تخصيص أشرطة الأدوات غير مفعّل في هذا التطبيق
error-not-enabled-text = تخصيص النصوص غير مفعّل في هذا التطبيق
error-not-enabled-layouts = تخصيص التخطيطات غير مفعّل في هذا التطبيق
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = يستخدم { $commands } الاختصار { $chord }
error-refused = مرفوض: { $parts }
error-unknown-command = لا يوجد أمر باسم { $id }
error-unknown-slot = لا يوجد شريط أدوات باسم { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = لا يقبل { $token } هذا النوع من القيم

## Theme editor.
theme-editor = محرر السمة
theme-status = حالة السمة
theme-off = تخصيص السمة متوقف
theme-off-hint = لا يسمح هذا التطبيق بتغيير سمته.
section-colors = الألوان
# The section of fonts and text sizes.
section-text = الخطوط والنصوص
section-shape = المسافات والأشكال
section-motion = الحركة
reset-theme = إعادة تعيين السمة
# The button that opens a color's picker; its accessible name is edit-named.
edit = تعديل
edit-named = تعديل { $name }
# Units after a number.
unit-px = بكسل
unit-ms = مللي ثانية

## Design tokens, as people call them.
token-theme-name = اسم السمة
token-color-bg = الخلفية
token-color-panel = اللوحات
token-color-card = البطاقات
token-color-text = النص
token-color-muted = النص الخافت
token-color-accent = اللون المميز
token-color-on-accent = النص على اللون المميز
token-color-danger = الخطر
token-color-warn = التحذير
token-color-ok = النجاح
token-color-line = الخطوط الفاصلة
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = اللون «{ $name }»
token-font = الخط
token-font-mono = الخط ثابت العرض
token-size-small = حجم النص الصغير
token-size-body = حجم نص المتن
token-size-title = حجم العنوان
token-spacing-unit = وحدة المسافات
token-control-height = ارتفاع عناصر التحكم
token-radius-small = نصف قطر الزاوية الصغير
token-radius-medium = نصف قطر الزاوية المتوسط
token-radius-large = نصف قطر الزاوية الكبير
token-stroke = سُمك الخط
token-motion-quick = الحركة السريعة
token-motion-standard = الحركة العادية
token-motion-emphasized = الحركة البارزة
token-density = الكثافة
token-background-image = صورة الخلفية
token-font-file = ملف الخط «{ $name }»

## How tightly controls are packed.
density-compact = مضغوطة
density-comfortable = مريحة
density-spacious = واسعة

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = التباين
contrast-all-pass = جميع أزواج الألوان تستوفي تباين WCAG.
# The contrast status read by screen readers.
contrast-status-all-pass = جميع أزواج الألوان تستوفي تباين WCAG
contrast-status-failing = { $count ->
    [zero] لا توجد أزواج ألوان دون تباين WCAG
    [one] زوج ألوان واحد دون تباين WCAG
    [two] زوجا ألوان دون تباين WCAG
    [few] { $count } أزواج ألوان دون تباين WCAG
    [many] { $count } زوجًا من الألوان دون تباين WCAG
   *[other] { $count } زوج ألوان دون تباين WCAG
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: النسبة { NUMBER($ratio, minimumFractionDigits: 1) }:1، وهي أقل من النسبة المطلوبة { NUMBER($needed, minimumFractionDigits: 1) }:1 ({ $foreground } على { $background })
contrast-pair-text-bg = النص على الخلفية
contrast-pair-text-panel = النص على اللوحات
contrast-pair-text-card = النص على البطاقات
contrast-pair-muted-bg = النص الخافت على الخلفية
contrast-pair-muted-panel = النص الخافت على اللوحات
contrast-pair-muted-card = النص الخافت على البطاقات
contrast-pair-on-accent = النص على اللون المميز
contrast-pair-accent-bg = اللون المميز على الخلفية

## The preview: sample widgets in the theme being edited.
preview = معاينة
preview-group = معاينة السمة
preview-field = حقل نموذجي
preview-field-text = نص داخل حقل
preview-body = يظهر نص المتن هكذا بالحجم العادي.
preview-muted = نص خافت للتلميحات والتفاصيل.
preview-primary = أساسي
preview-secondary = ثانوي
preview-danger = خطر
preview-switch = مفتاح تبديل
preview-progress = التقدّم
preview-link = رابط
preview-error = رسالة خطأ.

## Sharing themes.
section-share = المشاركة
share-hint = السمات ملفات رموز تصميم (design tokens) يمكن لأي تطبيق Remedy قراءتها.
theme-file = ملف السمة
theme-file-placeholder = مسار مثل themes/ocean.json
import-from-path = استيراد من المسار
export-to-path = تصدير إلى المسار
import-path-first = اكتب مسار ملف السمة أولًا
export-path-first = اكتب مكان حفظ السمة أولًا
choose-theme = اختيار سمة لاستيرادها…
export-to-file = تصدير إلى ملف…
# File dialog titles and the name of their file type filter.
dialog-import-theme = استيراد سمة
dialog-export-theme = تصدير السمة
filter-design-tokens = رموز التصميم
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = سمة
imported = تم استيراد السمة { $name }
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [zero] تم استيراد السمة { $name } دون استبعاد أي جزء
    [one] تم استيراد السمة { $name } مع استبعاد جزء واحد
    [two] تم استيراد السمة { $name } مع استبعاد جزأين
    [few] تم استيراد السمة { $name } مع استبعاد { $count } أجزاء
    [many] تم استيراد السمة { $name } مع استبعاد { $count } جزءًا
   *[other] تم استيراد السمة { $name } مع استبعاد { $count } جزء
}
could-not-import = تعذّر استيراد هذه السمة: { $error }
could-not-read = تعذّرت قراءة { $path }: { $detail }
exported = تم تصدير السمة إلى { $path }
could-not-export = تعذّر تصدير السمة: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = تحمل هذه السمة أكثر من الألوان والأحجام
theme-carries-more-detail = تحمل هذه السمة أكثر من الألوان والأحجام. تبقى العناصر التالية معطّلة ما لم تفعّلها:
allow-shaders = السماح بالمظلِّلات (شيفرة تعمل على بطاقة الرسوميات)
allow-links = السماح بالروابط الخارجية (تُحفظ ولا يتم جلبها أبدًا)
allow-files = استخدام الصور والخطوط المرفقة بها
import = استيراد
import-without = استيراد من دونها
cancel = إلغاء
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: مظلِّل ({ $detail })
refusal-link = { $token }: رابط خارجي ({ $detail })
refusal-file = { $token }: ملف مرفق بالسمة ({ $detail })
theme-not-object = ملف السمة يجب أن يكون كائن JSON
# { $detail } is the JSON reader's message.
not-design-tokens = ليس ملف رموز تصميم: { $detail }
history-import-theme = استيراد السمة { $name }

## The theme's files: its background image and fonts.
section-files = الملفات
files-hint = تظهر الصور خلف كل نافذة بحجمها، ويتم فحص الخطوط قبل استخدامها.
background-inline = الخلفية: صورة مضمّنة في السمة
# { $file } is the image's path or link.
background-file = الخلفية: { $file }
no-background = لا توجد صورة خلفية
background-file-field = ملف صورة الخلفية
background-placeholder = مسار مثل pictures/waves.png
use-image = استخدام هذه الصورة
image-path-first = اكتب مسار الصورة أولًا
reset-background = إعادة تعيين صورة الخلفية
choose-image = اختيار صورة…
dialog-choose-image = اختيار صورة الخلفية
filter-images = الصور
no-fonts = لا تتضمن السمة أي خطوط.
remove = إزالة
remove-font-named = إزالة الخط { $family }
font-file = ملف الخط
font-file-placeholder = مسار مثل fonts/Ocean-Regular.ttf
font-name = اسم الخط
font-name-placeholder = اسم الخط، مثل Ocean
add-font = إضافة خط
font-first = اكتب مسار ملف الخط واسمه أولًا
history-change-background = تغيير صورة الخلفية
history-remove-background = إزالة صورة الخلفية
history-add-font = إضافة الخط { $family }
history-remove-font = إزالة الخط { $family }
font-needs-name = يحتاج الخط إلى اسم
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = تعذّر استخدام الخط { $family }: { $detail }
background-not-used = تعذّر استخدام صورة الخلفية: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = يجب أن تكون ملفات السمة داخل مجلدها
asset-needs-file = استورد السمة من ملفها حتى يمكن العثور على ملفاتها
asset-link = { $link } رابط، ولا يتم جلب الروابط
not-an-image = ليست صورة يمكن لهذا التطبيق عرضها: { $detail }
not-data-uri = ليس معرّف data: URI
data-uri-comma = يجب أن يحتوي معرّف data: URI على فاصلة قبل بياناته
broken-escape = تسلسل هروب % غير صالح
# { $char } is a character, in quotes.
not-base64 = { $char } ليس من رموز Base64

## Shortcut editor.
shortcut-editor = محرر اختصارات لوحة المفاتيح
shortcut-status = حالة الاختصارات
shortcuts-off = تخصيص الاختصارات متوقف
shortcuts-off-hint = لا يسمح هذا التطبيق بتغيير اختصاراته.
search-commands = البحث في الأوامر
commands = الأوامر
no-matching-command = لا يوجد أمر مطابق
no-shortcut = لا يوجد اختصار
reset-all-shortcuts = إعادة تعيين كل الاختصارات
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = إضافة اختصار إلى { $command }
shortcut-remove-named = إزالة { $chord } من { $command }
reset-shortcuts-named = إعادة تعيين اختصارات { $command }
shortcut-now-runs = أصبح { $chord } يشغّل { $command }
shortcut-already-runs = { $chord } يشغّل { $commands } بالفعل
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } يشغّل { $commands } بالفعل.
shortcut-take = استخدام { $chord } للأمر { $command }
shortcut-keep = إبقاؤه للأمر { $commands }
history-change-shortcuts = تغيير اختصارات { $command }
history-add-shortcut = إضافة { $chord } إلى { $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = منح { $chord } للأمر { $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = تم استبعادها، ويُطبَّق الباقي: { $parts }
shortcuts-clash-two = سيستجيب كلٌّ من { $commands } إلى { $chord }
shortcuts-clash-many = ستستجيب جميع الأوامر { $commands } إلى { $chord }

## Toolbar editor.
toolbar-editor = محرر أشرطة الأدوات
toolbar-status = حالة أشرطة الأدوات
toolbars-off = تخصيص أشرطة الأدوات متوقف
toolbars-off-hint = لا يسمح هذا التطبيق بتغيير أشرطة أدواته.
no-toolbars = لا يحتوي هذا التطبيق على أشرطة أدوات يمكن تغييرها.
reset-all-toolbars = إعادة تعيين كل أشرطة الأدوات
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = ليس ملف أشرطة أدوات: { $detail }
toolbars-not-object = ملف أشرطة الأدوات كائن يربط كل خانة بقائمة من معرّفات الأوامر
toolbar-slot-not-list = { $slot }: الخانة قائمة من معرّفات الأوامر مثل ["file.save"]
toolbar-ids-not-text = { $slot }: معرّفات الأوامر نصوص

## Text editor.
text-editor = محرر النصوص
text-status = حالة النصوص
text-off = تخصيص النصوص متوقف
text-off-hint = لا يسمح هذا التطبيق بتغيير نصوصه.
# { $folder } is a folder's path.
text-folder = ملفات الرسائل الخاصة بك موجودة في { $folder }
read-files-again = إعادة قراءة الملفات
search-text = البحث في النصوص
strings = السلاسل النصية
no-matching-text = لا يوجد نص مطابق
reset-all-text = إعادة تعيين كل النصوص
could-not-change-text = تعذّر تغيير هذا النص: { $error }
could-not-reset-text = تعذّرت إعادة تعيين هذا النص: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = إعادة تعيين «{ $text }»
change-text = تغيير «{ $text }»
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = الرسائل
search-messages = البحث في الرسائل
no-matching-message = لا توجد رسالة مطابقة
messages-hint = رسائل Fluent: نصوص، وعناصر نائبة مثل {"{"} $name {"}"}، وخيارات.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = فحص { $id }
text-skipped = تم تخطيه: { $detail }
text-part-unread = تعذّرت قراءة جزء من الملف، ويُستخدم الباقي: { $detail }

## Layout editor.
layout-editor = محرر التخطيطات
layout-status = حالة التخطيطات
layouts-off = تخصيص التخطيطات متوقف
layouts-off-hint = لا يسمح هذا التطبيق بإعادة ترتيب شاشاته.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = تخطيطاتك موجودة في { $folder }
no-layouts = لا توجد شاشة مفتوحة يمكن إعادة ترتيبها.
reset-all-layouts = إعادة تعيين كل التخطيطات
layout-named = التخطيط { $file }
layout-check = فحص التخطيط
layout-ok = لا توجد مشكلات: يمكن حفظ التخطيط.
layout-saved = تم الحفظ. تعرض الشاشة تخطيطك الآن.
save = حفظ
save-layout-named = حفظ التخطيط { $file }
reset-layout-named = إعادة تعيين التخطيط { $file }
no-screen = لا تُعرض أي شاشة باسم { $file }
