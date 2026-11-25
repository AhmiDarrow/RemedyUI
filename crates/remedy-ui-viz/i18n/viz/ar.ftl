bars-value =
    { $count ->
        [zero] لا أعمدة، الأعلى { $percent }٪
        [one] عمود واحد، الأعلى { $percent }٪
        [two] عمودان، الأعلى { $percent }٪
        [few] { $count } أعمدة، الأعلى { $percent }٪
        [many] { $count } عمودًا، الأعلى { $percent }٪
       *[other] { $count } عمود، الأعلى { $percent }٪
    }
meter-value-normal = { $percent }٪، عادي
meter-value-loud = { $percent }٪، مرتفع
meter-value-clipping = { $percent }٪، متشبّع
meter-threshold = عتبة { $label }
meter-percent = { $percent }٪
ring-speaking = يتحدث
ring-silent = صامت
ring-muted = الصوت مكتوم
sparkline-no-data = لا توجد بيانات
sparkline-latest = آخر قيمة { $value }
