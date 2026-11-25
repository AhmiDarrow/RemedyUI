# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } ও { $second }
list-middle = { $first }, { $rest }
list-end = { $first } ও { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = থিম
area-shortcuts = কীবোর্ড শর্টকাট
area-toolbars = টুলবার ও মেনু
area-text = টেক্সট
area-layouts = স্ক্রিন লেআউট

## The settings page.
customize = কাস্টমাইজ করুন
customize-status = কাস্টমাইজেশনের অবস্থা
nothing-to-customize = কাস্টমাইজ করার মতো কিছু নেই
nothing-to-customize-hint = এই অ্যাপে কাস্টমাইজেশন চালু করা হয়নি।
reset-everything = সবকিছু রিসেট করুন
reset-everything-done = সবকিছু অ্যাপের ডিফল্ট সেটিংয়ে ফিরে গেছে। 'পূর্বাবস্থায় ফেরান' দিয়ে আবার আগের মতো করা যাবে।
# Also the undo history's name for resetting everything.
reset-all-customizations = সব কাস্টমাইজেশন রিসেট করুন

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = রিসেট করুন
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = { $name } রিসেট করুন
# The undo history's name for changing one named thing ("Change Accent").
change-named = { $name } পরিবর্তন করুন
# { $error } says why.
could-not-reset = রিসেট করা যায়নি: { $error }
not-saved = সেভ হয়নি: { $error }
could-not-change = এই পরিবর্তন করা যায়নি: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = { $area } ব্যবহার করা যায়নি ({ $file }): { $message }
notice-dismiss = খারিজ করুন
# The dismiss button's accessible name.
notice-dismiss-named = { $file } সম্পর্কিত বিজ্ঞপ্তি খারিজ করুন
notice-not-utf8 = ফাইলটি UTF-8 টেক্সট নয়
notice-could-not-save = সেভ করা যায়নি: { $error }
notice-could-not-restore = ফিরিয়ে আনা যায়নি: { $detail }

## Errors.
error-not-enabled-theme = এই অ্যাপে থিম কাস্টমাইজেশন চালু নেই
error-not-enabled-shortcuts = এই অ্যাপে শর্টকাট কাস্টমাইজেশন চালু নেই
error-not-enabled-toolbars = এই অ্যাপে টুলবার কাস্টমাইজেশন চালু নেই
error-not-enabled-text = এই অ্যাপে টেক্সট কাস্টমাইজেশন চালু নেই
error-not-enabled-layouts = এই অ্যাপে লেআউট কাস্টমাইজেশন চালু নেই
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } ব্যবহার করছে { $commands }
error-refused = প্রত্যাখ্যাত: { $parts }
error-unknown-command = { $id } নামে কোনো কমান্ড নেই
error-unknown-slot = { $id } নামে কোনো টুলবার নেই
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } এই ধরনের মান রাখতে পারে না

## Theme editor.
theme-editor = থিম এডিটর
theme-status = থিমের অবস্থা
theme-off = থিম কাস্টমাইজেশন বন্ধ আছে
theme-off-hint = এই অ্যাপ তার থিম পরিবর্তন করতে দেয় না।
section-colors = রং
# The section of fonts and text sizes.
section-text = ফন্ট ও টেক্সট
section-shape = ব্যবধান ও আকৃতি
section-motion = অ্যানিমেশন
reset-theme = থিম রিসেট করুন
# The button that opens a color's picker; its accessible name is edit-named.
edit = সম্পাদনা
edit-named = { $name } সম্পাদনা করুন
# Units after a number.
unit-px = px
unit-ms = মি.সে.

## Design tokens, as people call them.
token-theme-name = থিমের নাম
token-color-bg = ব্যাকগ্রাউন্ড
token-color-panel = প্যানেল
token-color-card = কার্ড
token-color-text = টেক্সট
token-color-muted = হালকা টেক্সট
token-color-accent = অ্যাকসেন্ট
token-color-on-accent = অ্যাকসেন্টের উপর টেক্সট
token-color-danger = বিপদ
token-color-warn = সতর্কতা
token-color-ok = সাফল্য
token-color-line = রেখা
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = রং “{ $name }”
token-font = ফন্ট
token-font-mono = মনোস্পেস ফন্ট
token-size-small = ছোট টেক্সটের আকার
token-size-body = মূল টেক্সটের আকার
token-size-title = শিরোনামের আকার
token-spacing-unit = ব্যবধানের একক
token-control-height = কন্ট্রোলের উচ্চতা
token-radius-small = ছোট কোণের ব্যাসার্ধ
token-radius-medium = মাঝারি কোণের ব্যাসার্ধ
token-radius-large = বড় কোণের ব্যাসার্ধ
token-stroke = রেখার পুরুত্ব
token-motion-quick = দ্রুত অ্যানিমেশন
token-motion-standard = সাধারণ অ্যানিমেশন
token-motion-emphasized = জোরালো অ্যানিমেশন
token-density = ঘনত্ব
token-background-image = ব্যাকগ্রাউন্ড ছবি
token-font-file = ফন্ট ফাইল “{ $name }”

## How tightly controls are packed.
density-compact = ঘন
density-comfortable = আরামদায়ক
density-spacious = প্রশস্ত

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = কনট্রাস্ট
contrast-all-pass = সব রঙের জোড়া WCAG কনট্রাস্ট পূরণ করে।
# The contrast status read by screen readers.
contrast-status-all-pass = সব রঙের জোড়া WCAG কনট্রাস্ট পূরণ করে
contrast-status-failing = { $count ->
    [one] { $count }টি রঙের জোড়া WCAG কনট্রাস্টের নিচে আছে
   *[other] { $count }টি রঙের জোড়া WCAG কনট্রাস্টের নিচে আছে
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, প্রয়োজনীয় { NUMBER($needed, minimumFractionDigits: 1) }:1-এর কম ({ $background }-এর উপর { $foreground })
contrast-pair-text-bg = ব্যাকগ্রাউন্ডের উপর টেক্সট
contrast-pair-text-panel = প্যানেলের উপর টেক্সট
contrast-pair-text-card = কার্ডের উপর টেক্সট
contrast-pair-muted-bg = ব্যাকগ্রাউন্ডের উপর হালকা টেক্সট
contrast-pair-muted-panel = প্যানেলের উপর হালকা টেক্সট
contrast-pair-muted-card = কার্ডের উপর হালকা টেক্সট
contrast-pair-on-accent = অ্যাকসেন্টের উপর টেক্সট
contrast-pair-accent-bg = ব্যাকগ্রাউন্ডের উপর অ্যাকসেন্ট

## The preview: sample widgets in the theme being edited.
preview = প্রিভিউ
preview-group = থিম প্রিভিউ
preview-field = নমুনা ফিল্ড
preview-field-text = ফিল্ডের মধ্যে টেক্সট
preview-body = মূল টেক্সট তার সাধারণ আকারে এভাবে দেখায়।
preview-muted = ইঙ্গিত ও বিবরণের জন্য হালকা টেক্সট।
preview-primary = প্রাথমিক
preview-secondary = মাধ্যমিক
preview-danger = বিপদ
preview-switch = একটি সুইচ
preview-progress = অগ্রগতি
preview-link = একটি লিংক
preview-error = একটি ত্রুটি বার্তা।

## Sharing themes.
section-share = শেয়ার করুন
share-hint = থিম হলো design tokens ফাইল, যা যেকোনো Remedy অ্যাপ পড়তে পারে।
theme-file = থিম ফাইল
theme-file-placeholder = themes/ocean.json-এর মতো একটি পাথ
import-from-path = পাথ থেকে ইমপোর্ট করুন
export-to-path = পাথে এক্সপোর্ট করুন
import-path-first = প্রথমে থিম ফাইলের পাথ লিখুন
export-path-first = প্রথমে লিখুন থিমটি কোথায় সেভ করবেন
choose-theme = ইমপোর্ট করার জন্য একটি থিম বেছে নিন…
export-to-file = একটি ফাইলে এক্সপোর্ট করুন…
# File dialog titles and the name of their file type filter.
dialog-import-theme = একটি থিম ইমপোর্ট করুন
dialog-export-theme = থিম এক্সপোর্ট করুন
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = theme
imported = থিম { $name } ইমপোর্ট করা হয়েছে
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] থিম { $name } ইমপোর্ট করা হয়েছে, { $count }টি অংশ বাদ দেওয়া হয়েছে
   *[other] থিম { $name } ইমপোর্ট করা হয়েছে, { $count }টি অংশ বাদ দেওয়া হয়েছে
}
could-not-import = ওই থিম ইমপোর্ট করা যায়নি: { $error }
could-not-read = { $path } পড়া যায়নি: { $detail }
exported = থিম { $path }-এ এক্সপোর্ট করা হয়েছে
could-not-export = থিম এক্সপোর্ট করা যায়নি: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = এই থিমে রং ও আকারের চেয়েও বেশি কিছু আছে
theme-carries-more-detail = এই থিমে রং ও আকারের চেয়েও বেশি কিছু আছে। আপনি চালু না করা পর্যন্ত এগুলো বন্ধ থাকবে:
allow-shaders = শেডার অনুমতি দিন (গ্রাফিক্স কার্ডে চলা কোড)
allow-links = বাইরের লিংক অনুমতি দিন (রাখা হয়, কখনো আনা হয় না)
allow-files = এর সাথে আসা ছবি ও ফন্ট ব্যবহার করুন
import = ইমপোর্ট করুন
import-without = এগুলো ছাড়া ইমপোর্ট করুন
cancel = বাতিল করুন
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: একটি শেডার ({ $detail })
refusal-link = { $token }: একটি বাইরের লিংক ({ $detail })
refusal-file = { $token }: থিমের সাথে আসা একটি ফাইল ({ $detail })
theme-not-object = থিম ফাইল একটি JSON অবজেক্ট হতে হবে
# { $detail } is the JSON reader's message.
not-design-tokens = এটি design tokens ফাইল নয়: { $detail }
history-import-theme = থিম { $name } ইমপোর্ট করুন

## The theme's files: its background image and fonts.
section-files = ফাইল
files-hint = ছবি প্রতিটি উইন্ডোর পেছনে তার আকারে দেখানো হয়; ব্যবহারের আগে ফন্ট যাচাই করা হয়।
background-inline = ব্যাকগ্রাউন্ড: থিমের ভেতরে থাকা একটি ছবি
# { $file } is the image's path or link.
background-file = ব্যাকগ্রাউন্ড: { $file }
no-background = কোনো ব্যাকগ্রাউন্ড ছবি নেই
background-file-field = ব্যাকগ্রাউন্ড ছবির ফাইল
background-placeholder = pictures/waves.png-এর মতো একটি পাথ
use-image = এই ছবিটি ব্যবহার করুন
image-path-first = প্রথমে ছবির পাথ লিখুন
reset-background = ব্যাকগ্রাউন্ড ছবি রিসেট করুন
choose-image = একটি ছবি বেছে নিন…
dialog-choose-image = ব্যাকগ্রাউন্ড ছবি বেছে নিন
filter-images = ছবি
no-fonts = এই থিমের সাথে কোনো ফন্ট নেই।
remove = সরান
remove-font-named = ফন্ট { $family } সরান
font-file = ফন্ট ফাইল
font-file-placeholder = fonts/Ocean-Regular.ttf-এর মতো একটি পাথ
font-name = ফন্টের নাম
font-name-placeholder = ফন্টটির নাম, যেমন Ocean
add-font = ফন্ট যোগ করুন
font-first = প্রথমে ফন্ট ফাইলের পাথ ও তার নাম লিখুন
history-change-background = ব্যাকগ্রাউন্ড ছবি পরিবর্তন করুন
history-remove-background = ব্যাকগ্রাউন্ড ছবি সরান
history-add-font = ফন্ট { $family } যোগ করুন
history-remove-font = ফন্ট { $family } সরান
font-needs-name = ফন্টের একটি নাম দরকার
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = ফন্ট { $family } ব্যবহার করা যায়নি: { $detail }
background-not-used = ব্যাকগ্রাউন্ড ছবি ব্যবহার করা যায়নি: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = থিমের ফাইলগুলো তার ফোল্ডারের ভেতরে থাকতে হবে
asset-needs-file = থিমটি তার ফাইল থেকে ইমপোর্ট করুন, যাতে তার ফাইলগুলো খুঁজে পাওয়া যায়
asset-link = { $link } একটি লিংক, আর লিংক আনা হয় না
not-an-image = এটি এমন ছবি নয় যা এই অ্যাপ দেখাতে পারে: { $detail }
not-data-uri = এটি data: URI নয়
data-uri-comma = data: URI-তে ডেটার আগে একটি কমা থাকতে হবে
broken-escape = একটি ভুল % এস্কেপ
# { $char } is a character, in quotes.
not-base64 = { $char } base64 নয়

## Shortcut editor.
shortcut-editor = শর্টকাট এডিটর
shortcut-status = শর্টকাটের অবস্থা
shortcuts-off = শর্টকাট কাস্টমাইজেশন বন্ধ আছে
shortcuts-off-hint = এই অ্যাপ তার শর্টকাট পরিবর্তন করতে দেয় না।
search-commands = কমান্ড খুঁজুন
commands = কমান্ড
no-matching-command = মেলে এমন কোনো কমান্ড নেই
no-shortcut = কোনো শর্টকাট নেই
reset-all-shortcuts = সব শর্টকাট রিসেট করুন
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = { $command }-এ একটি শর্টকাট যোগ করুন
shortcut-remove-named = { $command } থেকে { $chord } সরান
reset-shortcuts-named = { $command }-এর শর্টকাট রিসেট করুন
shortcut-now-runs = { $chord } এখন { $command } চালায়
shortcut-already-runs = { $chord } ইতিমধ্যে { $commands } চালায়
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } ইতিমধ্যে { $commands } চালায়।
shortcut-take = { $command }-এর জন্য { $chord } ব্যবহার করুন
shortcut-keep = { $commands }-এর জন্য রাখুন
history-change-shortcuts = { $command }-এর শর্টকাট পরিবর্তন করুন
history-add-shortcut = { $command }-এ { $chord } যোগ করুন
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = { $chord } { $command }-কে দিন
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = বাদ দেওয়া হয়েছে, বাকিটা প্রযোজ্য: { $parts }
shortcuts-clash-two = { $commands } দুটিই { $chord }-এ সাড়া দেবে
shortcuts-clash-many = { $commands } সবগুলোই { $chord }-এ সাড়া দেবে

## Toolbar editor.
toolbar-editor = টুলবার এডিটর
toolbar-status = টুলবারের অবস্থা
toolbars-off = টুলবার কাস্টমাইজেশন বন্ধ আছে
toolbars-off-hint = এই অ্যাপ তার টুলবার পরিবর্তন করতে দেয় না।
no-toolbars = এই অ্যাপে পরিবর্তন করার মতো কোনো টুলবার নেই।
reset-all-toolbars = সব টুলবার রিসেট করুন
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = এটি টুলবার ফাইল নয়: { $detail }
toolbars-not-object = টুলবার ফাইল এমন একটি অবজেক্ট যা প্রতিটি স্লটকে কমান্ড আইডির তালিকার সাথে যুক্ত করে
toolbar-slot-not-list = { $slot }: স্লট হলো কমান্ড আইডির তালিকা, যেমন ["file.save"]
toolbar-ids-not-text = { $slot }: কমান্ড আইডি টেক্সট হতে হবে

## Text editor.
text-editor = টেক্সট এডিটর
text-status = টেক্সটের অবস্থা
text-off = টেক্সট কাস্টমাইজেশন বন্ধ আছে
text-off-hint = এই অ্যাপ তার টেক্সট পরিবর্তন করতে দেয় না।
# { $folder } is a folder's path.
text-folder = আপনার মেসেজ ফাইলগুলো { $folder }-এ আছে
read-files-again = ফাইলগুলো আবার পড়ুন
search-text = টেক্সট খুঁজুন
strings = স্ট্রিং
no-matching-text = মেলে এমন কোনো টেক্সট নেই
reset-all-text = সব টেক্সট রিসেট করুন
could-not-change-text = ওই টেক্সট পরিবর্তন করা যায়নি: { $error }
could-not-reset-text = ওই টেক্সট রিসেট করা যায়নি: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = “{ $text }” রিসেট করুন
change-text = “{ $text }” পরিবর্তন করুন
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = মেসেজ
search-messages = মেসেজ খুঁজুন
no-matching-message = মেলে এমন কোনো মেসেজ নেই
messages-hint = Fluent মেসেজ: টেক্সট, {"{"} $name {"}"}-এর মতো প্লেসহোল্ডার, এবং বিকল্প।
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = { $id } যাচাই
text-skipped = বাদ দেওয়া হয়েছে: { $detail }
text-part-unread = ফাইলের একটি অংশ পড়া যায়নি, বাকিটা ব্যবহার করা হচ্ছে: { $detail }

## Layout editor.
layout-editor = লেআউট এডিটর
layout-status = লেআউটের অবস্থা
layouts-off = লেআউট কাস্টমাইজেশন বন্ধ আছে
layouts-off-hint = এই অ্যাপ তার স্ক্রিনগুলো নতুন করে সাজাতে দেয় না।
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = আপনার লেআউটগুলো { $folder }-এ আছে
no-layouts = নতুন করে সাজানো যায় এমন কোনো স্ক্রিন খোলা নেই।
reset-all-layouts = সব লেআউট রিসেট করুন
layout-named = লেআউট { $file }
layout-check = লেআউট যাচাই
layout-ok = কোনো সমস্যা নেই: লেআউটটি সেভ করা যাবে।
layout-saved = সেভ হয়েছে। স্ক্রিনে আপনার লেআউট দেখানো হচ্ছে।
save = সেভ করুন
save-layout-named = লেআউট { $file } সেভ করুন
reset-layout-named = লেআউট { $file } রিসেট করুন
no-screen = { $file } নামে কোনো স্ক্রিন দেখানো হচ্ছে না
