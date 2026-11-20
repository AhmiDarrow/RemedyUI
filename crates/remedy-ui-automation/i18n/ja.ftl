# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = { $agent } を許可しますか？
agent-asks-read = { $agent } が、{ $how_long }このウィンドウの表示内容を見ることを求めています。パスワードや秘密情報は非表示のままです。
agent-asks-act = { $agent } が、{ $how_long }このウィンドウを見て操作することを求めています。パスワードや秘密情報は非表示のままです。
agent-asks-confirm = { $agent } が、{ $how_long }このウィンドウを見て、操作のたびにあなたに確認しながら操作することを求めています。パスワードや秘密情報は非表示のままです。
agent-for-minutes = { $count } 分間
agent-for-seconds = { $count } 秒間
agent-until-stopped = 停止するまで

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = { $agents } がこのウィンドウを見ることができます
agents-stop = 停止

## Asking before an agent acts.
agent-asks = { $agent } からの要求
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = { $agent } に次の操作を許可しますか？：{ $command }
# { $name } is the control the agent would use ("Send").
agent-let-use = { $agent } に「{ $name }」の使用を許可しますか？

## What an agent did, for the undo history. { $name } is the control's name.
did-set = { $name } を { $value } に設定
did-press = { $name } を押す
did-raise = { $name } を上げる
did-lower = { $name } を下げる
did-focus = { $name } に移動
did-expand = { $name } を開く
did-collapse = { $name } を閉じる
did-show = { $name } を表示
