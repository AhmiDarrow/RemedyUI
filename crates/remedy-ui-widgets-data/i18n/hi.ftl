# The data widgets' own words. Placeholders ({ $name }) must stay; their order may change.
# Keyboard hints name keys as the keyboard prints them in the language (Strg, Umschalt…).

## Chat lists: the button that appears while reading older messages.
chat-jump-latest = नवीनतम पर जाएँ
# Shown on the button when messages arrived below.
chat-new-count = { $count ->
    [one] { $count } नया
   *[other] { $count } नए
}
# What a screen reader says for that button.
chat-new-spoken = { $count ->
    [one] { $count } नया संदेश, नवीनतम पर जाएँ
   *[other] { $count } नए संदेश, नवीनतम पर जाएँ
}

## Icon grids.
grid-item-keys = तीर कुंजियों से आइटम के बीच जाएँ; Space और Shift से चुनें
grid-keys = Shift के साथ तीर कुंजियाँ दबाने पर एक श्रेणी चुनी जाती है; Ctrl+A से सब कुछ चुना जाता है
# Read with the grid's name: how many items it holds.
grid-count = { $count } आइटम

## Property grids.
# The filter box; { $label } is the grid's name ("Properties").
property-filter = { $label } फ़िल्टर करें
property-none = कोई मेल खाती प्रॉपर्टी नहीं

## Tables.
# The field that edits a cell; { $column } is the column's title.
table-edit = { $column } संपादित करें
table-row-keys = तीर कुंजियों से पंक्तियों के बीच जाएँ; Space और Shift से चुनें
table-resize-keys = शीर्षक पर Ctrl+बायाँ तीर और Ctrl+दायाँ तीर से कॉलम की चौड़ाई बदलती है
table-move-keys = Alt+Shift+बायाँ तीर और Alt+Shift+दायाँ तीर से कॉलम खिसकता है
# Read after a column heading's name.
table-sorted-ascending = आरोही क्रम में
table-sorted-descending = अवरोही क्रम में
table-not-sorted = क्रमबद्ध नहीं
# Read with the table's name: how many rows it holds.
table-count = { $count ->
    [one] { $count } पंक्ति
   *[other] { $count } पंक्तियाँ
}

## Tree views.
# Why a move (by dragging, or cut and paste) did not happen.
tree-into-itself = किसी आइटम को उसी के अंदर नहीं ले जाया जा सकता
tree-not-folder = आइटम केवल किसी फ़ोल्डर में ले जाए जा सकते हैं
# Shown after an item whose children are loading.
tree-loading = लोड हो रहा है…
# The same, as a screen reader says it.
tree-loading-spoken = लोड हो रहा है
tree-keys = तीर कुंजियों से आइटम के बीच जाएँ और उन्हें खोलें; Ctrl+X और फिर Ctrl+V से उन्हें ले जाएँ
