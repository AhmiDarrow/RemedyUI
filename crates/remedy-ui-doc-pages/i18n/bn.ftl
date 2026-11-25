## পৃষ্ঠা।
page-number = পৃষ্ঠা { $page }
pages-list = পৃষ্ঠাগুলো
page-view-value = { $count }টির মধ্যে পৃষ্ঠা { $page }, { $zoom }%

annotation-kind = { $kind ->
    [highlight] হাইলাইট
    [underline] আন্ডারলাইন
    [strikeout] স্ট্রাইকআউট
    [squiggly] ঢেউখেলানো আন্ডারলাইন
    [drawing] অঙ্কন
    [note] নোট
    [text-box] টেক্সট বক্স
    [rectangle] আয়তক্ষেত্র
    [ellipse] উপবৃত্ত
    [line] রেখা
    [arrow] তীর
    [image] ছবি
   *[annotation] টীকা
}
annotation-quoting = { $what }: “{ $text }”
annotation-with-comment = { $what }, মন্তব্য: { $comment }
annotation-by = { $what }, { $author }-এর
annotation-selected = নির্বাচিত: { $description }
annotation-count = { $count ->
    [one] { $count }টি টীকা:
   *[other] { $count }টি টীকা:
}

undo-add = { $kind ->
    [highlight] হাইলাইট যোগ করুন
    [underline] আন্ডারলাইন যোগ করুন
    [strikeout] স্ট্রাইকআউট যোগ করুন
    [squiggly] ঢেউখেলানো আন্ডারলাইন যোগ করুন
    [drawing] অঙ্কন যোগ করুন
    [note] নোট যোগ করুন
    [text-box] টেক্সট বক্স যোগ করুন
    [rectangle] আয়তক্ষেত্র যোগ করুন
    [ellipse] উপবৃত্ত যোগ করুন
    [line] রেখা যোগ করুন
    [arrow] তীর যোগ করুন
    [image] ছবি যোগ করুন
   *[annotation] টীকা যোগ করুন
}
undo-change = { $kind ->
    [highlight] হাইলাইট পরিবর্তন করুন
    [underline] আন্ডারলাইন পরিবর্তন করুন
    [strikeout] স্ট্রাইকআউট পরিবর্তন করুন
    [squiggly] ঢেউখেলানো আন্ডারলাইন পরিবর্তন করুন
    [drawing] অঙ্কন পরিবর্তন করুন
    [note] নোট পরিবর্তন করুন
    [text-box] টেক্সট বক্স পরিবর্তন করুন
    [rectangle] আয়তক্ষেত্র পরিবর্তন করুন
    [ellipse] উপবৃত্ত পরিবর্তন করুন
    [line] রেখা পরিবর্তন করুন
    [arrow] তীর পরিবর্তন করুন
    [image] ছবি পরিবর্তন করুন
   *[annotation] টীকা পরিবর্তন করুন
}
undo-delete = { $kind ->
    [highlight] হাইলাইট মুছুন
    [underline] আন্ডারলাইন মুছুন
    [strikeout] স্ট্রাইকআউট মুছুন
    [squiggly] ঢেউখেলানো আন্ডারলাইন মুছুন
    [drawing] অঙ্কন মুছুন
    [note] নোট মুছুন
    [text-box] টেক্সট বক্স মুছুন
    [rectangle] আয়তক্ষেত্র মুছুন
    [ellipse] উপবৃত্ত মুছুন
    [line] রেখা মুছুন
    [arrow] তীর মুছুন
    [image] ছবি মুছুন
   *[annotation] টীকা মুছুন
}
undo-move = { $kind ->
    [highlight] হাইলাইট সরান
    [underline] আন্ডারলাইন সরান
    [strikeout] স্ট্রাইকআউট সরান
    [squiggly] ঢেউখেলানো আন্ডারলাইন সরান
    [drawing] অঙ্কন সরান
    [note] নোট সরান
    [text-box] টেক্সট বক্স সরান
    [rectangle] আয়তক্ষেত্র সরান
    [ellipse] উপবৃত্ত সরান
    [line] রেখা সরান
    [arrow] তীর সরান
    [image] ছবি সরান
   *[annotation] টীকা সরান
}
