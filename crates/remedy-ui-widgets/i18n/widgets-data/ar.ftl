# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = الانتقال إلى الأحدث
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [zero] { $count } رسالة جديدة
    [one] رسالة جديدة
    [two] رسالتان جديدتان
    [few] { $count } رسائل جديدة
    [many] { $count } رسالة جديدة
   *[other] { $count } رسالة جديدة
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [zero] { $count } رسالة جديدة، الانتقال إلى الأحدث
    [one] رسالة جديدة واحدة، الانتقال إلى الأحدث
    [two] رسالتان جديدتان، الانتقال إلى الأحدث
    [few] { $count } رسائل جديدة، الانتقال إلى الأحدث
    [many] { $count } رسالة جديدة، الانتقال إلى الأحدث
   *[other] { $count } رسالة جديدة، الانتقال إلى الأحدث
}

## Icon grids.
grid-item-keys = تنتقل مفاتيح الأسهم بين العناصر؛ ويحدد مفتاحا المسافة وShift
grid-keys = يحدد Shift مع مفاتيح الأسهم نطاقًا؛ ويحدد Ctrl+A الكل
# Read with the grid's name: how many items it holds.
grid-count = { $count ->
    [zero] لا توجد عناصر
    [one] عنصر واحد
    [two] عنصران
    [few] { $count } عناصر
    [many] { $count } عنصرًا
   *[other] { $count } عنصر
}

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = تصفية { $label }
property-none = لا توجد خصائص مطابقة

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = تحرير { $column }
table-row-keys = تنتقل مفاتيح الأسهم بين الصفوف؛ ويحدد مفتاحا المسافة وShift
table-resize-keys = يغيّر Ctrl+السهم الأيسر وCtrl+السهم الأيمن على العنوان عرض العمود
table-move-keys = ينقل Alt+Shift+السهم الأيسر وAlt+Shift+السهم الأيمن العمود
# Read after a column heading's name.
table-sorted-ascending = مرتب تصاعديًا
table-sorted-descending = مرتب تنازليًا
table-not-sorted = غير مرتب
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [zero] لا توجد صفوف
    [one] صف واحد
    [two] صفّان
    [few] { $count } صفوف
    [many] { $count } صفًا
   *[other] { $count } صف
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = لا يمكن نقل عنصر إلى داخل نفسه
tree-not-folder = لا يمكن نقل العناصر إلا إلى مجلد
# Shown after an item whose children are loading.
tree-loading = جارٍ التحميل…
# The same, as a screen reader says it.
tree-loading-spoken = جارٍ التحميل
tree-keys = تنتقل مفاتيح الأسهم بين العناصر وتفتحها؛ وينقلها Ctrl+X ثم Ctrl+V
