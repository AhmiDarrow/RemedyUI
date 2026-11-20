bars-value =
    { $count ->
        [one] { $count }টি বার, সর্বোচ্চ { $percent } শতাংশ
       *[other] { $count }টি বার, সর্বোচ্চ { $percent } শতাংশ
    }
meter-value-normal = { $percent } শতাংশ, স্বাভাবিক
meter-value-loud = { $percent } শতাংশ, জোরে
meter-value-clipping = { $percent } শতাংশ, ক্লিপিং
meter-threshold = { $label }-এর সীমা
meter-percent = { $percent } শতাংশ
ring-speaking = কথা বলছেন
ring-silent = নীরব
ring-muted = মিউট করা
sparkline-no-data = কোনো ডেটা নেই
sparkline-latest = সর্বশেষ { $value }
