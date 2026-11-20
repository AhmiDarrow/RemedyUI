# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = { $agent }-কে অনুমতি দেবেন?
agent-asks-read = { $agent } { $how_long } এই উইন্ডোতে যা দেখায় তা দেখতে চাইছে। পাসওয়ার্ড ও গোপন তথ্য লুকানো থাকে।
agent-asks-act = { $agent } { $how_long } এই উইন্ডোটি দেখতে ও ব্যবহার করতে চাইছে। পাসওয়ার্ড ও গোপন তথ্য লুকানো থাকে।
agent-asks-confirm = { $agent } { $how_long } এই উইন্ডোটি দেখতে ও ব্যবহার করতে চাইছে, প্রতিটি কাজের আগে আপনাকে জিজ্ঞেস করে। পাসওয়ার্ড ও গোপন তথ্য লুকানো থাকে।
agent-for-minutes = { $count ->
    [one] { $count } মিনিটের জন্য
   *[other] { $count } মিনিটের জন্য
}
agent-for-seconds = { $count ->
    [one] { $count } সেকেন্ডের জন্য
   *[other] { $count } সেকেন্ডের জন্য
}
agent-until-stopped = আপনি বন্ধ না করা পর্যন্ত

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = { $agents } এই উইন্ডোটি দেখতে পারে
agents-stop = বন্ধ করুন

## Asking before an agent acts.
agent-asks = { $agent } অনুরোধ করছে
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = { $agent }-কে { $command } করতে দেবেন?
# { $name } is the control the agent would use ("Send").
agent-let-use = { $agent }-কে “{ $name }” ব্যবহার করতে দেবেন?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = { $name } { $value }-এ সেট করুন
did-press = { $name } চাপুন
did-raise = { $name } ওঠান
did-lower = { $name } নামান
did-focus = { $name }-এ সরান
did-expand = { $name } খুলুন
did-collapse = { $name } বন্ধ করুন
did-show = { $name } দেখান
