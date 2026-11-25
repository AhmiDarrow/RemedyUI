# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = { $agent } को अनुमति दें?
agent-asks-read = { $agent } { $how_long } यह देखने की अनुमति माँग रहा है कि यह विंडो क्या दिखाती है। पासवर्ड और सीक्रेट छिपे रहते हैं।
agent-asks-act = { $agent } { $how_long } इस विंडो को देखने और इस्तेमाल करने की अनुमति माँग रहा है। पासवर्ड और सीक्रेट छिपे रहते हैं।
agent-asks-confirm = { $agent } { $how_long } इस विंडो को देखने और इस्तेमाल करने की अनुमति माँग रहा है, हर कार्रवाई से पहले आपसे पूछकर। पासवर्ड और सीक्रेट छिपे रहते हैं।
agent-for-minutes = { $count ->
    [one] { $count } मिनट के लिए
   *[other] { $count } मिनट के लिए
}
agent-for-seconds = { $count ->
    [one] { $count } सेकंड के लिए
   *[other] { $count } सेकंड के लिए
}
agent-until-stopped = जब तक आप इसे न रोकें, तब तक

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = { $agents } इस विंडो को देख सकते हैं
agents-stop = रोकें

## Asking before an agent acts.
agent-asks = { $agent } पूछ रहा है
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = { $agent } को यह करने दें: { $command }?
# { $name } is the control the agent would use ("Send").
agent-let-use = { $agent } को “{ $name }” इस्तेमाल करने दें?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = { $name } को { $value } पर सेट करें
did-press = { $name } दबाएँ
did-raise = { $name } बढ़ाएँ
did-lower = { $name } घटाएँ
did-focus = { $name } पर जाएँ
did-expand = { $name } खोलें
did-collapse = { $name } बंद करें
did-show = { $name } दिखाएँ
