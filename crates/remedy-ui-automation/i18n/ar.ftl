# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = هل تسمح لـ{ $agent }؟
agent-asks-read = يطلب { $agent } رؤية ما تعرضه هذه النافذة { $how_long }. تبقى كلمات المرور والأسرار مخفية.
agent-asks-act = يطلب { $agent } رؤية هذه النافذة واستخدامها { $how_long }. تبقى كلمات المرور والأسرار مخفية.
agent-asks-confirm = يطلب { $agent } رؤية هذه النافذة واستخدامها، مع سؤالك قبل كل إجراء، { $how_long }. تبقى كلمات المرور والأسرار مخفية.
agent-for-minutes = { $count ->
    [zero] لمدة { $count } دقيقة
    [one] لمدة { $count } دقيقة
    [two] لمدة { $count } دقيقتين
    [few] لمدة { $count } دقائق
    [many] لمدة { $count } دقيقة
   *[other] لمدة { $count } دقيقة
}
agent-for-seconds = { $count ->
    [zero] لمدة { $count } ثانية
    [one] لمدة { $count } ثانية
    [two] لمدة { $count } ثانيتين
    [few] لمدة { $count } ثوانٍ
    [many] لمدة { $count } ثانية
   *[other] لمدة { $count } ثانية
}
agent-until-stopped = حتى توقفه

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = يمكن لـ{ $agents } رؤية هذه النافذة
agents-stop = إيقاف

## Asking before an agent acts.
agent-asks = { $agent } يطلب
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = هل تسمح لـ{ $agent } بتنفيذ «{ $command }»؟
# { $name } is the control the agent would use ("Send").
agent-let-use = هل تسمح لـ{ $agent } باستخدام «{ $name }»؟

## What an agent did, for the undo history. { $name } is the control's name.
did-set = تعيين { $name } إلى { $value }
did-press = اضغط { $name }
did-raise = رفع { $name }
did-lower = خفض { $name }
did-focus = نقل إلى { $name }
did-expand = فتح { $name }
did-collapse = إغلاق { $name }
did-show = إظهار { $name }
