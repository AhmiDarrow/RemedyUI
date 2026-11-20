# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } और { $second }
list-middle = { $first }, { $rest }
list-end = { $first } और { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = थीम
area-shortcuts = कीबोर्ड शॉर्टकट
area-toolbars = टूलबार और मेनू
area-text = टेक्स्ट
area-layouts = स्क्रीन लेआउट

## The settings page.
customize = कस्टमाइज़ करें
customize-status = कस्टमाइज़ेशन की स्थिति
nothing-to-customize = कस्टमाइज़ करने के लिए कुछ नहीं है
nothing-to-customize-hint = इस ऐप ने कस्टमाइज़ेशन चालू नहीं किया है।
reset-everything = सब कुछ रीसेट करें
reset-everything-done = सब कुछ ऐप की डिफ़ॉल्ट सेटिंग पर वापस आ गया है। पहले जैसा करने के लिए 'पूर्ववत करें' का उपयोग करें।
# Also the undo history's name for resetting everything.
reset-all-customizations = सभी कस्टमाइज़ेशन रीसेट करें

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = रीसेट करें
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = { $name } रीसेट करें
# The undo history's name for changing one named thing ("Change Accent").
change-named = { $name } बदलें
# { $error } says why.
could-not-reset = रीसेट नहीं किया जा सका: { $error }
not-saved = सेव नहीं हुआ: { $error }
could-not-change = यह बदलाव नहीं किया जा सका: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = { $area } का उपयोग नहीं किया जा सका ({ $file }): { $message }
notice-dismiss = खारिज करें
# The dismiss button's accessible name.
notice-dismiss-named = { $file } से जुड़ी सूचना खारिज करें
notice-not-utf8 = यह फ़ाइल UTF-8 टेक्स्ट नहीं है
notice-could-not-save = सेव नहीं किया जा सका: { $error }
notice-could-not-restore = वापस नहीं लाया जा सका: { $detail }

## Errors.
error-not-enabled-theme = इस ऐप में थीम कस्टमाइज़ेशन चालू नहीं है
error-not-enabled-shortcuts = इस ऐप में शॉर्टकट कस्टमाइज़ेशन चालू नहीं है
error-not-enabled-toolbars = इस ऐप में टूलबार कस्टमाइज़ेशन चालू नहीं है
error-not-enabled-text = इस ऐप में टेक्स्ट कस्टमाइज़ेशन चालू नहीं है
error-not-enabled-layouts = इस ऐप में लेआउट कस्टमाइज़ेशन चालू नहीं है
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } का उपयोग { $commands } करता है
error-refused = अस्वीकार किया गया: { $parts }
error-unknown-command = { $id } नाम का कोई कमांड नहीं है
error-unknown-slot = { $id } नाम का कोई टूलबार नहीं है
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } में इस प्रकार का मान नहीं रखा जा सकता

## Theme editor.
theme-editor = थीम एडिटर
theme-status = थीम की स्थिति
theme-off = थीम कस्टमाइज़ेशन बंद है
theme-off-hint = यह ऐप अपनी थीम बदलने की अनुमति नहीं देता।
section-colors = रंग
# The section of fonts and text sizes.
section-text = फ़ॉन्ट और टेक्स्ट
section-shape = स्पेसिंग और आकार
section-motion = एनिमेशन
reset-theme = थीम रीसेट करें
# The button that opens a color's picker; its accessible name is edit-named.
edit = बदलें
edit-named = { $name } बदलें
# Units after a number.
unit-px = px
unit-ms = मि.से.

## Design tokens, as people call them.
token-theme-name = थीम का नाम
token-color-bg = बैकग्राउंड
token-color-panel = पैनल
token-color-card = कार्ड
token-color-text = टेक्स्ट
token-color-muted = हल्का टेक्स्ट
token-color-accent = एक्सेंट
token-color-on-accent = एक्सेंट पर टेक्स्ट
token-color-danger = खतरा
token-color-warn = चेतावनी
token-color-ok = सफलता
token-color-line = रेखाएँ
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = रंग “{ $name }”
token-font = फ़ॉन्ट
token-font-mono = मोनोस्पेस फ़ॉन्ट
token-size-small = छोटे टेक्स्ट का आकार
token-size-body = मुख्य टेक्स्ट का आकार
token-size-title = शीर्षक का आकार
token-spacing-unit = स्पेसिंग इकाई
token-control-height = कंट्रोल की ऊँचाई
token-radius-small = छोटा कोना त्रिज्या
token-radius-medium = मध्यम कोना त्रिज्या
token-radius-large = बड़ा कोना त्रिज्या
token-stroke = रेखा की मोटाई
token-motion-quick = तेज़ एनिमेशन
token-motion-standard = सामान्य एनिमेशन
token-motion-emphasized = ज़ोरदार एनिमेशन
token-density = सघनता
token-background-image = बैकग्राउंड इमेज
token-font-file = फ़ॉन्ट फ़ाइल “{ $name }”

## How tightly controls are packed.
density-compact = सघन
density-comfortable = आरामदायक
density-spacious = खुला

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = कंट्रास्ट
contrast-all-pass = सभी रंग जोड़े WCAG कंट्रास्ट को पूरा करते हैं।
# The contrast status read by screen readers.
contrast-status-all-pass = सभी रंग जोड़े WCAG कंट्रास्ट को पूरा करते हैं
contrast-status-failing = { $count ->
    [one] { $count } रंग जोड़ा WCAG कंट्रास्ट से कम है
   *[other] { $count } रंग जोड़े WCAG कंट्रास्ट से कम हैं
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, ज़रूरी { NUMBER($needed, minimumFractionDigits: 1) }:1 से कम ({ $background } पर { $foreground })
contrast-pair-text-bg = बैकग्राउंड पर टेक्स्ट
contrast-pair-text-panel = पैनल पर टेक्स्ट
contrast-pair-text-card = कार्ड पर टेक्स्ट
contrast-pair-muted-bg = बैकग्राउंड पर हल्का टेक्स्ट
contrast-pair-muted-panel = पैनल पर हल्का टेक्स्ट
contrast-pair-muted-card = कार्ड पर हल्का टेक्स्ट
contrast-pair-on-accent = एक्सेंट पर टेक्स्ट
contrast-pair-accent-bg = बैकग्राउंड पर एक्सेंट

## The preview: sample widgets in the theme being edited.
preview = प्रीव्यू
preview-group = थीम प्रीव्यू
preview-field = नमूना फ़ील्ड
preview-field-text = फ़ील्ड में टेक्स्ट
preview-body = मुख्य टेक्स्ट अपने सामान्य आकार में ऐसा दिखता है।
preview-muted = संकेतों और विवरणों के लिए हल्का टेक्स्ट।
preview-primary = प्राथमिक
preview-secondary = द्वितीयक
preview-danger = खतरा
preview-switch = एक स्विच
preview-progress = प्रगति
preview-link = एक लिंक
preview-error = एक त्रुटि संदेश।

## Sharing themes.
section-share = शेयर करें
share-hint = थीम design tokens फ़ाइलें होती हैं, जिन्हें हर Remedy ऐप पढ़ सकता है।
theme-file = थीम फ़ाइल
theme-file-placeholder = themes/ocean.json जैसा पाथ
import-from-path = पाथ से इंपोर्ट करें
export-to-path = पाथ पर एक्सपोर्ट करें
import-path-first = पहले थीम फ़ाइल का पाथ लिखें
export-path-first = पहले लिखें कि थीम कहाँ सेव करनी है
choose-theme = इंपोर्ट करने के लिए थीम चुनें…
export-to-file = फ़ाइल में एक्सपोर्ट करें…
# File dialog titles and the name of their file type filter.
dialog-import-theme = थीम इंपोर्ट करें
dialog-export-theme = थीम एक्सपोर्ट करें
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = theme
imported = थीम { $name } इंपोर्ट की गई
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] थीम { $name } इंपोर्ट की गई, { $count } हिस्सा छोड़ दिया गया
   *[other] थीम { $name } इंपोर्ट की गई, { $count } हिस्से छोड़ दिए गए
}
could-not-import = वह थीम इंपोर्ट नहीं की जा सकी: { $error }
could-not-read = { $path } पढ़ी नहीं जा सकी: { $detail }
exported = थीम { $path } पर एक्सपोर्ट की गई
could-not-export = थीम एक्सपोर्ट नहीं की जा सकी: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = इस थीम में रंगों और आकारों से ज़्यादा चीज़ें हैं
theme-carries-more-detail = इस थीम में रंगों और आकारों से ज़्यादा चीज़ें हैं। जब तक आप इन्हें चालू नहीं करते, ये बंद रहेंगी:
allow-shaders = शेडर की अनुमति दें (ग्राफ़िक्स कार्ड पर चलने वाला कोड)
allow-links = बाहरी लिंक की अनुमति दें (रखे जाते हैं, कभी फ़ेच नहीं किए जाते)
allow-files = इसके साथ आने वाली इमेज और फ़ॉन्ट का उपयोग करें
import = इंपोर्ट करें
import-without = इनके बिना इंपोर्ट करें
cancel = रद्द करें
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: एक शेडर ({ $detail })
refusal-link = { $token }: एक बाहरी लिंक ({ $detail })
refusal-file = { $token }: थीम के साथ आने वाली फ़ाइल ({ $detail })
theme-not-object = थीम फ़ाइल एक JSON ऑब्जेक्ट होनी चाहिए
# { $detail } is the JSON reader's message.
not-design-tokens = यह design tokens फ़ाइल नहीं है: { $detail }
history-import-theme = थीम { $name } इंपोर्ट करें

## The theme's files: its background image and fonts.
section-files = फ़ाइलें
files-hint = इमेज हर विंडो के पीछे उसके आकार में दिखती हैं; फ़ॉन्ट उपयोग से पहले जाँचे जाते हैं।
background-inline = बैकग्राउंड: थीम के अंदर मौजूद इमेज
# { $file } is the image's path or link.
background-file = बैकग्राउंड: { $file }
no-background = कोई बैकग्राउंड इमेज नहीं
background-file-field = बैकग्राउंड इमेज फ़ाइल
background-placeholder = pictures/waves.png जैसा पाथ
use-image = यह इमेज इस्तेमाल करें
image-path-first = पहले इमेज का पाथ लिखें
reset-background = बैकग्राउंड इमेज रीसेट करें
choose-image = इमेज चुनें…
dialog-choose-image = बैकग्राउंड इमेज चुनें
filter-images = इमेज
no-fonts = इस थीम के साथ कोई फ़ॉन्ट नहीं है।
remove = हटाएँ
remove-font-named = फ़ॉन्ट { $family } हटाएँ
font-file = फ़ॉन्ट फ़ाइल
font-file-placeholder = fonts/Ocean-Regular.ttf जैसा पाथ
font-name = फ़ॉन्ट का नाम
font-name-placeholder = फ़ॉन्ट का नाम, जैसे Ocean
add-font = फ़ॉन्ट जोड़ें
font-first = पहले फ़ॉन्ट फ़ाइल का पाथ और उसका नाम लिखें
history-change-background = बैकग्राउंड इमेज बदलें
history-remove-background = बैकग्राउंड इमेज हटाएँ
history-add-font = फ़ॉन्ट { $family } जोड़ें
history-remove-font = फ़ॉन्ट { $family } हटाएँ
font-needs-name = फ़ॉन्ट का नाम होना ज़रूरी है
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = फ़ॉन्ट { $family } का उपयोग नहीं किया जा सका: { $detail }
background-not-used = बैकग्राउंड इमेज का उपयोग नहीं किया जा सका: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = थीम की फ़ाइलें उसके फ़ोल्डर के अंदर होनी चाहिए
asset-needs-file = थीम को उसकी फ़ाइल से इंपोर्ट करें, ताकि उसकी फ़ाइलें मिल सकें
asset-link = { $link } एक लिंक है, और लिंक फ़ेच नहीं किए जाते
not-an-image = यह ऐसी इमेज नहीं है जिसे यह ऐप दिखा सके: { $detail }
not-data-uri = यह data: URI नहीं है
data-uri-comma = data: URI में डेटा से पहले कॉमा होना चाहिए
broken-escape = गलत % एस्केप
# { $char } is a character, in quotes.
not-base64 = { $char } base64 नहीं है

## Shortcut editor.
shortcut-editor = शॉर्टकट एडिटर
shortcut-status = शॉर्टकट की स्थिति
shortcuts-off = शॉर्टकट कस्टमाइज़ेशन बंद है
shortcuts-off-hint = यह ऐप अपने शॉर्टकट बदलने की अनुमति नहीं देता।
search-commands = कमांड खोजें
commands = कमांड
no-matching-command = कोई मिलता-जुलता कमांड नहीं
no-shortcut = कोई शॉर्टकट नहीं
reset-all-shortcuts = सभी शॉर्टकट रीसेट करें
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = { $command } में शॉर्टकट जोड़ें
shortcut-remove-named = { $command } से { $chord } हटाएँ
reset-shortcuts-named = { $command } के शॉर्टकट रीसेट करें
shortcut-now-runs = { $chord } अब { $command } चलाता है
shortcut-already-runs = { $chord } पहले से { $commands } चलाता है
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } पहले से { $commands } चलाता है।
shortcut-take = { $command } के लिए { $chord } का उपयोग करें
shortcut-keep = इसे { $commands } के लिए रखें
history-change-shortcuts = { $command } के शॉर्टकट बदलें
history-add-shortcut = { $command } में { $chord } जोड़ें
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = { $chord } को { $command } को दें
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = छोड़ दिया गया, बाकी लागू है: { $parts }
shortcuts-clash-two = { $commands } दोनों { $chord } पर प्रतिक्रिया देंगे
shortcuts-clash-many = { $commands } सभी { $chord } पर प्रतिक्रिया देंगे

## Toolbar editor.
toolbar-editor = टूलबार एडिटर
toolbar-status = टूलबार की स्थिति
toolbars-off = टूलबार कस्टमाइज़ेशन बंद है
toolbars-off-hint = यह ऐप अपने टूलबार बदलने की अनुमति नहीं देता।
no-toolbars = इस ऐप में बदलने लायक कोई टूलबार नहीं है।
reset-all-toolbars = सभी टूलबार रीसेट करें
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = यह टूलबार फ़ाइल नहीं है: { $detail }
toolbars-not-object = टूलबार फ़ाइल एक ऑब्जेक्ट होती है जो हर स्लॉट को कमांड ID की सूची से जोड़ती है
toolbar-slot-not-list = { $slot }: स्लॉट कमांड ID की सूची होता है, जैसे ["file.save"]
toolbar-ids-not-text = { $slot }: कमांड ID टेक्स्ट होने चाहिए

## Text editor.
text-editor = टेक्स्ट एडिटर
text-status = टेक्स्ट की स्थिति
text-off = टेक्स्ट कस्टमाइज़ेशन बंद है
text-off-hint = यह ऐप अपना टेक्स्ट बदलने की अनुमति नहीं देता।
# { $folder } is a folder's path.
text-folder = आपकी मैसेज फ़ाइलें { $folder } में हैं
read-files-again = फ़ाइलें फिर से पढ़ें
search-text = टेक्स्ट खोजें
strings = स्ट्रिंग
no-matching-text = कोई मिलता-जुलता टेक्स्ट नहीं
reset-all-text = सभी टेक्स्ट रीसेट करें
could-not-change-text = वह टेक्स्ट बदला नहीं जा सका: { $error }
could-not-reset-text = वह टेक्स्ट रीसेट नहीं किया जा सका: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = “{ $text }” रीसेट करें
change-text = “{ $text }” बदलें
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = मैसेज
search-messages = मैसेज खोजें
no-matching-message = कोई मिलता-जुलता मैसेज नहीं
messages-hint = Fluent मैसेज: टेक्स्ट, {"{"} $name {"}"} जैसे प्लेसहोल्डर, और विकल्प।
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = { $id } की जाँच
text-skipped = छोड़ दिया गया: { $detail }
text-part-unread = फ़ाइल का एक हिस्सा पढ़ा नहीं जा सका, बाकी का उपयोग किया गया है: { $detail }

## Layout editor.
layout-editor = लेआउट एडिटर
layout-status = लेआउट की स्थिति
layouts-off = लेआउट कस्टमाइज़ेशन बंद है
layouts-off-hint = यह ऐप अपनी स्क्रीन को फिर से व्यवस्थित करने की अनुमति नहीं देता।
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = आपके लेआउट { $folder } में हैं
no-layouts = ऐसी कोई स्क्रीन खुली नहीं है जिसे फिर से व्यवस्थित किया जा सके।
reset-all-layouts = सभी लेआउट रीसेट करें
layout-named = लेआउट { $file }
layout-check = लेआउट की जाँच
layout-ok = कोई समस्या नहीं: लेआउट सेव किया जा सकता है।
layout-saved = सेव हो गया। स्क्रीन पर आपका लेआउट दिख रहा है।
save = सेव करें
save-layout-named = लेआउट { $file } सेव करें
reset-layout-named = लेआउट { $file } रीसेट करें
no-screen = { $file } नाम की कोई स्क्रीन नहीं दिख रही है
