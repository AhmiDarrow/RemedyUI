## पेज।
page-number = पेज { $page }
pages-list = पेज
page-view-value = { $count } में से पेज { $page }, { $zoom }%

annotation-kind = { $kind ->
    [highlight] हाइलाइट
    [underline] रेखांकन
    [strikeout] स्ट्राइकआउट
    [squiggly] लहरदार रेखांकन
    [drawing] ड्रॉइंग
    [note] नोट
    [text-box] टेक्स्ट बॉक्स
    [rectangle] आयत
    [ellipse] दीर्घवृत्त
    [line] रेखा
    [arrow] तीर
    [image] चित्र
   *[annotation] एनोटेशन
}
annotation-quoting = { $what }: “{ $text }”
annotation-with-comment = { $what }, टिप्पणी: { $comment }
annotation-by = { $what }, { $author } द्वारा
annotation-selected = चयनित: { $description }
annotation-count = { $count ->
    [one] { $count } एनोटेशन:
   *[other] { $count } एनोटेशन:
}

undo-add = { $kind ->
    [highlight] हाइलाइट जोड़ें
    [underline] रेखांकन जोड़ें
    [strikeout] स्ट्राइकआउट जोड़ें
    [squiggly] लहरदार रेखांकन जोड़ें
    [drawing] ड्रॉइंग जोड़ें
    [note] नोट जोड़ें
    [text-box] टेक्स्ट बॉक्स जोड़ें
    [rectangle] आयत जोड़ें
    [ellipse] दीर्घवृत्त जोड़ें
    [line] रेखा जोड़ें
    [arrow] तीर जोड़ें
    [image] चित्र जोड़ें
   *[annotation] एनोटेशन जोड़ें
}
undo-change = { $kind ->
    [highlight] हाइलाइट बदलें
    [underline] रेखांकन बदलें
    [strikeout] स्ट्राइकआउट बदलें
    [squiggly] लहरदार रेखांकन बदलें
    [drawing] ड्रॉइंग बदलें
    [note] नोट बदलें
    [text-box] टेक्स्ट बॉक्स बदलें
    [rectangle] आयत बदलें
    [ellipse] दीर्घवृत्त बदलें
    [line] रेखा बदलें
    [arrow] तीर बदलें
    [image] चित्र बदलें
   *[annotation] एनोटेशन बदलें
}
undo-delete = { $kind ->
    [highlight] हाइलाइट हटाएँ
    [underline] रेखांकन हटाएँ
    [strikeout] स्ट्राइकआउट हटाएँ
    [squiggly] लहरदार रेखांकन हटाएँ
    [drawing] ड्रॉइंग हटाएँ
    [note] नोट हटाएँ
    [text-box] टेक्स्ट बॉक्स हटाएँ
    [rectangle] आयत हटाएँ
    [ellipse] दीर्घवृत्त हटाएँ
    [line] रेखा हटाएँ
    [arrow] तीर हटाएँ
    [image] चित्र हटाएँ
   *[annotation] एनोटेशन हटाएँ
}
undo-move = { $kind ->
    [highlight] हाइलाइट खिसकाएँ
    [underline] रेखांकन खिसकाएँ
    [strikeout] स्ट्राइकआउट खिसकाएँ
    [squiggly] लहरदार रेखांकन खिसकाएँ
    [drawing] ड्रॉइंग खिसकाएँ
    [note] नोट खिसकाएँ
    [text-box] टेक्स्ट बॉक्स खिसकाएँ
    [rectangle] आयत खिसकाएँ
    [ellipse] दीर्घवृत्त खिसकाएँ
    [line] रेखा खिसकाएँ
    [arrow] तीर खिसकाएँ
    [image] चित्र ले जाएँ
   *[annotation] एनोटेशन खिसकाएँ
}
