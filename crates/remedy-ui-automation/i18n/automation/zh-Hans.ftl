# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = 允许 { $agent } 吗？
agent-asks-read = { $agent } 请求查看此窗口显示的内容，{ $how_long }。密码和机密信息保持隐藏。
agent-asks-act = { $agent } 请求查看并使用此窗口，{ $how_long }。密码和机密信息保持隐藏。
agent-asks-confirm = { $agent } 请求查看并使用此窗口，每次操作前都会征求你的同意，{ $how_long }。密码和机密信息保持隐藏。
agent-for-minutes = 时长 { $count } 分钟
agent-for-seconds = 时长 { $count } 秒
agent-until-stopped = 直到你将其停止

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = { $agents } 可以看到此窗口
agents-stop = 停止

## Asking before an agent acts.
agent-asks = { $agent } 请求
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = 允许 { $agent } 执行“{ $command }”吗？
# { $name } is the control the agent would use ("Send").
agent-let-use = 允许 { $agent } 使用“{ $name }”吗？

## What an agent did, for the undo history. { $name } is the control's name.
did-set = 将 { $name } 设为 { $value }
did-press = 按 { $name }
did-raise = 升高 { $name }
did-lower = 降低 { $name }
did-focus = 移到 { $name }
did-expand = 打开 { $name }
did-collapse = 关闭 { $name }
did-show = 显示 { $name }
