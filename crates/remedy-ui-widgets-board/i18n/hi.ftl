# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = पूर्ववत करें
redo = फिर से करें
add-area = क्षेत्र जोड़ें
area-heading = क्षेत्र
area-title = क्षेत्र का शीर्षक
area-title-hint = शीर्षक (लागू करने के लिए Enter दबाएँ)
area-place = कॉलम { $column }, पंक्ति { $row }, चौड़ाई { $columns }, ऊँचाई { $rows }
area-keys-hint = तीर कुंजियाँ इसे खिसकाती हैं; Shift के साथ तीर कुंजियाँ इसका आकार बदलती हैं।
area-keys = तीर कुंजियाँ इसे खिसकाती हैं; Shift के साथ तीर कुंजियाँ इसका आकार बदलती हैं
remove-area = क्षेत्र हटाएँ
widget-heading = विजेट
settings = सेटिंग्स
widget-keys-hint = Alt के साथ ऊपर और नीचे इसे खिसकाते हैं; Alt के साथ बाएँ और दाएँ इसे अगले क्षेत्र में ले जाते हैं।
widget-keys = Alt के साथ तीर कुंजियाँ इसे खिसकाती हैं
remove-widget = विजेट हटाएँ
nothing-selected = बदलने के लिए कोई क्षेत्र या विजेट चुनें।
add-widget = विजेट जोड़ें
widget-kind = विजेट का प्रकार

## The grid.
widget-name = { $name } विजेट
unknown-kind = यह बोर्ड ऐसा विजेट उपयोग करता है जो इस ऐप में नहीं है ({ $kind })।
area-named = { $name } क्षेत्र
area-numbered = क्षेत्र { $number }
empty-area = खाली क्षेत्र: कोई विजेट जोड़ें या यहाँ छोड़ें।
empty-board = खाली बोर्ड
empty-board-editing = यह बोर्ड खाली है: कोई क्षेत्र जोड़ें।
empty-board-viewing = यहाँ अभी कुछ नहीं है।
one-widget = इस क्षेत्र में पहले से एक विजेट है: दूसरे विजेट को उसका अपना क्षेत्र दें।

## Why a change was refused.
error-invalid = यह बोर्ड नहीं है: { $why }
error-too-new = यह बोर्ड किसी नए संस्करण से बनाया गया है (फ़ॉर्मैट { $format })
error-outside = इससे यह बोर्ड के किनारे से बाहर चला जाएगा
error-overlap = इससे क्षेत्र { $area } ढक जाएगा
error-missing = { $id } बोर्ड पर नहीं है

## Setting editors.
time-date = { $label }: तारीख
time-time = { $label }: समय
time-not-set = सेट नहीं है
time-clear = साफ़ करें
choice-unavailable = { $value } (उपलब्ध नहीं)
list-item = { $label } { $number }
list-add = { $label } में जोड़ें
list-remove = { $label } { $number } हटाएँ
picture-source = { $label }: पता
picture-choose = चित्र चुनें

## Scripted widgets.
stopped = यह विजेट रुक गया: { $why }
start-again = फिर से शुरू करें

## The built-in widgets.
text-name = टेक्स्ट
text-title = शीर्षक
text-title-default = स्वागत है
text-body = टेक्स्ट
text-body-default = घर के सभी लोगों को नमस्ते कहें।
links-name = लिंक
links-title = शीर्षक
links-title-default = लिंक
links-links = लिंक (लेबल | https://…, हर पंक्ति में एक)
links-none = अभी कोई लिंक नहीं है।
reactions-name = प्रतिक्रियाएँ
reactions-question = प्रश्न
reactions-question-default = सब कैसे हैं?
reactions-offered = दी जाने वाली प्रतिक्रियाएँ

## The reactions component.
reaction-group = प्रतिक्रियाएँ
reaction-one = 1 प्रतिक्रिया
reaction-many = { $count } प्रतिक्रियाएँ
reaction-count = { $count ->
    [one] { $count } प्रतिक्रिया
   *[other] { $count } प्रतिक्रियाएँ
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, आपकी प्रतिक्रिया सहित
