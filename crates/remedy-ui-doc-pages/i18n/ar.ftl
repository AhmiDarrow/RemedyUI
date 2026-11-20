## الصفحات.
page-number = الصفحة { $page }
pages-list = الصفحات
page-view-value = الصفحة { $page } من { $count }، { $zoom }٪

annotation-kind = { $kind ->
    [highlight] تمييز
    [underline] تسطير
    [strikeout] شطب
    [squiggly] تسطير متموج
    [drawing] رسم
    [note] ملاحظة
    [text-box] مربع نص
    [rectangle] مستطيل
    [ellipse] قطع ناقص
    [line] خط
    [arrow] سهم
   *[annotation] تعليق توضيحي
}
annotation-quoting = { $what }: «{ $text }»
annotation-with-comment = { $what }، تعليق: { $comment }
annotation-by = { $what }، بواسطة { $author }
annotation-selected = محدد: { $description }
annotation-count = { $count ->
    [zero] { $count } تعليقات توضيحية:
    [one] تعليق توضيحي واحد:
    [two] تعليقان توضيحيان:
    [few] { $count } تعليقات توضيحية:
    [many] { $count } تعليقًا توضيحيًا:
   *[other] { $count } تعليق توضيحي:
}

undo-add = { $kind ->
    [highlight] إضافة تمييز
    [underline] إضافة تسطير
    [strikeout] إضافة شطب
    [squiggly] إضافة تسطير متموج
    [drawing] إضافة رسم
    [note] إضافة ملاحظة
    [text-box] إضافة مربع نص
    [rectangle] إضافة مستطيل
    [ellipse] إضافة قطع ناقص
    [line] إضافة خط
    [arrow] إضافة سهم
   *[annotation] إضافة تعليق توضيحي
}
undo-change = { $kind ->
    [highlight] تغيير التمييز
    [underline] تغيير التسطير
    [strikeout] تغيير الشطب
    [squiggly] تغيير التسطير المتموج
    [drawing] تغيير الرسم
    [note] تغيير الملاحظة
    [text-box] تغيير مربع النص
    [rectangle] تغيير المستطيل
    [ellipse] تغيير القطع الناقص
    [line] تغيير الخط
    [arrow] تغيير السهم
   *[annotation] تغيير التعليق التوضيحي
}
undo-delete = { $kind ->
    [highlight] حذف التمييز
    [underline] حذف التسطير
    [strikeout] حذف الشطب
    [squiggly] حذف التسطير المتموج
    [drawing] حذف الرسم
    [note] حذف الملاحظة
    [text-box] حذف مربع النص
    [rectangle] حذف المستطيل
    [ellipse] حذف القطع الناقص
    [line] حذف الخط
    [arrow] حذف السهم
   *[annotation] حذف التعليق التوضيحي
}
undo-move = { $kind ->
    [highlight] نقل التمييز
    [underline] نقل التسطير
    [strikeout] نقل الشطب
    [squiggly] نقل التسطير المتموج
    [drawing] نقل الرسم
    [note] نقل الملاحظة
    [text-box] نقل مربع النص
    [rectangle] نقل المستطيل
    [ellipse] نقل القطع الناقص
    [line] نقل الخط
    [arrow] نقل السهم
   *[annotation] نقل التعليق التوضيحي
}
