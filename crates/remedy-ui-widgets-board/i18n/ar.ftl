# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = تراجع
redo = إعادة
add-area = إضافة منطقة
area-heading = المنطقة
area-title = عنوان المنطقة
area-title-hint = العنوان (اضغط Enter للتطبيق)
area-place = العمود { $column }، الصف { $row }، العرض { $columns }، الارتفاع { $rows }
area-keys-hint = تنقلها مفاتيح الأسهم؛ ومع Shift تغيّر حجمها.
area-keys = تنقلها مفاتيح الأسهم، ومع Shift تغيّر مفاتيح الأسهم حجمها
remove-area = إزالة المنطقة
widget-heading = الأداة
settings = الإعدادات
widget-keys-hint = يؤدي الضغط على Alt مع السهمين لأعلى ولأسفل إلى نقلها، ومع السهمين لليمين ولليسار إلى نقلها إلى المنطقة التالية.
widget-keys = يؤدي الضغط على Alt مع مفاتيح الأسهم إلى نقلها
remove-widget = إزالة الأداة
nothing-selected = اختر منطقة أو أداة لتغييرها.
add-widget = إضافة أداة
widget-kind = نوع الأداة

## The grid.
widget-name = أداة { $name }
unknown-kind = تستخدم هذه اللوحة أداة غير موجودة في هذا التطبيق ({ $kind }).
area-named = منطقة { $name }
area-numbered = المنطقة { $number }
empty-area = منطقة فارغة: أضف أداة أو أفلت واحدة هنا.
empty-board = لوحة فارغة
empty-board-editing = هذه اللوحة فارغة: أضف منطقة.
empty-board-viewing = لا يوجد شيء هنا بعد.
one-widget = تحتوي هذه المنطقة على أداة بالفعل: خصّص للأداة الأخرى منطقة خاصة بها.

## Why a change was refused.
error-invalid = هذه ليست لوحة: { $why }
error-too-new = أُنشئت هذه اللوحة بإصدار أحدث (التنسيق { $format })
error-outside = سيتجاوز ذلك حافة اللوحة
error-overlap = سيغطي ذلك المنطقة { $area }
error-missing = { $id } غير موجود على اللوحة

## Setting editors.
time-date = { $label }: التاريخ
time-time = { $label }: الوقت
time-not-set = غير محدد
time-clear = مسح
choice-unavailable = { $value } (غير متاح)
list-item = { $label } { $number }
list-add = إضافة إلى { $label }
list-remove = إزالة { $label } { $number }
picture-source = { $label }: العنوان
picture-choose = اختيار صورة

## Scripted widgets.
stopped = توقفت هذه الأداة: { $why }
start-again = البدء من جديد

## The built-in widgets.
text-name = نص
text-title = العنوان
text-title-default = مرحبًا
text-body = النص
text-body-default = ألقِ التحية على أهل البيت.
links-name = روابط
links-title = العنوان
links-title-default = روابط
links-links = الروابط (التسمية | https://…، رابط في كل سطر)
links-none = لا توجد روابط بعد.
reactions-name = تفاعلات
reactions-question = السؤال
reactions-question-default = كيف حال الجميع؟
reactions-offered = التفاعلات المتاحة

## The reactions component.
reaction-group = التفاعلات
reaction-one = تفاعل واحد
reaction-many = عدد التفاعلات: { $count }
reaction-count = { $count ->
    [zero] لا تفاعلات
    [one] تفاعل واحد
    [two] تفاعلان
    [few] { $count } تفاعلات
    [many] { $count } تفاعلًا
   *[other] { $count } تفاعل
}
reaction-spoken = { $emoji }، { $count }
reaction-spoken-mine = { $emoji }، { $count }، بما في ذلك تفاعلك
