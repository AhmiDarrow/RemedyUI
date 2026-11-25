# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = পূর্বাবস্থায় ফেরান
redo = আবার করুন
add-area = এলাকা যোগ করুন
area-heading = এলাকা
area-title = এলাকার শিরোনাম
area-title-hint = শিরোনাম (প্রয়োগ করতে Enter চাপুন)
area-place = কলাম { $column }, সারি { $row }, প্রস্থ { $columns }, উচ্চতা { $rows }
area-keys-hint = তীর কী এটিকে সরায়; Shift-এর সঙ্গে তীর কী এর আকার বদলায়।
area-keys = তীর কীগুলো এটিকে সরায়; Shift-এর সঙ্গে তীর কীগুলো এর আকার বদলায়
remove-area = এলাকা সরান
widget-heading = উইজেট
settings = সেটিংস
widget-keys-hint = Alt-এর সঙ্গে উপর ও নিচ এটিকে সরায়; Alt-এর সঙ্গে বাম ও ডান এটিকে পরের এলাকায় নিয়ে যায়।
widget-keys = Alt-এর সঙ্গে তীর কীগুলো এটিকে সরায়
remove-widget = উইজেট সরান
nothing-selected = পরিবর্তন করতে একটি এলাকা বা উইজেট বেছে নিন।
add-widget = উইজেট যোগ করুন
widget-kind = উইজেটের ধরন

## The grid.
widget-name = { $name } উইজেট
unknown-kind = এই বোর্ডে এমন একটি উইজেট আছে যা এই অ্যাপে নেই ({ $kind })।
area-named = { $name } এলাকা
area-numbered = এলাকা { $number }
empty-area = খালি এলাকা: একটি উইজেট যোগ করুন বা এখানে ছেড়ে দিন।
empty-board = খালি বোর্ড
empty-board-editing = এই বোর্ডটি খালি: একটি এলাকা যোগ করুন।
empty-board-viewing = এখানে এখনও কিছু নেই।
one-widget = এই এলাকায় ইতিমধ্যে একটি উইজেট আছে: অন্যটিকে আলাদা একটি এলাকা দিন।

## Why a change was refused.
error-invalid = এটি বোর্ড নয়: { $why }
error-too-new = এই বোর্ডটি একটি নতুন সংস্করণে তৈরি (ফরম্যাট { $format })
error-outside = এতে এটি বোর্ডের কিনারার বাইরে চলে যাবে
error-overlap = এতে { $area } এলাকাটি ঢেকে যাবে
error-missing = { $id } বোর্ডে নেই

## Setting editors.
time-date = { $label }: তারিখ
time-time = { $label }: সময়
time-not-set = সেট করা নেই
time-clear = মুছুন
choice-unavailable = { $value } (উপলভ্য নয়)
list-item = { $label } { $number }
list-add = { $label }-এ যোগ করুন
list-remove = { $label } { $number } সরান
picture-source = { $label }: ঠিকানা
picture-choose = ছবি বেছে নিন

## Scripted widgets.
stopped = এই উইজেটটি থেমে গেছে: { $why }
start-again = আবার শুরু করুন

## The built-in widgets.
text-name = লেখা
text-title = শিরোনাম
text-title-default = স্বাগতম
text-body = লেখা
text-body-default = বাড়ির সবাইকে নমস্কার জানান।
links-name = লিংক
links-title = শিরোনাম
links-title-default = লিংক
links-links = লিংক (লেবেল | https://…, প্রতি লাইনে একটি)
links-none = এখনও কোনো লিংক নেই।
reactions-name = প্রতিক্রিয়া
reactions-question = প্রশ্ন
reactions-question-default = সবাই কেমন আছেন?
reactions-offered = যে প্রতিক্রিয়াগুলো দেওয়া যাবে

## The reactions component.
reaction-group = প্রতিক্রিয়া
reaction-one = একটি প্রতিক্রিয়া
reaction-many = { $count }টি প্রতিক্রিয়া
reaction-count = { $count ->
    [one] { $count }টি প্রতিক্রিয়া
   *[other] { $count }টি প্রতিক্রিয়া
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, আপনারটি সহ
