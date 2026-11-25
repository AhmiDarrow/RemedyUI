# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = Разрешить доступ для { $agent }?
agent-asks-read = { $agent } просит разрешения видеть содержимое этого окна { $how_long }. Пароли и секреты остаются скрытыми.
agent-asks-act = { $agent } просит разрешения видеть это окно и управлять им { $how_long }. Пароли и секреты остаются скрытыми.
agent-asks-confirm = { $agent } просит разрешения видеть это окно и управлять им, спрашивая вас перед каждым действием, { $how_long }. Пароли и секреты остаются скрытыми.
agent-for-minutes = { $count ->
    [one] на { $count } минуту
    [few] на { $count } минуты
    [many] на { $count } минут
   *[other] на { $count } минуты
}
agent-for-seconds = { $count ->
    [one] на { $count } секунду
    [few] на { $count } секунды
    [many] на { $count } секунд
   *[other] на { $count } секунды
}
agent-until-stopped = пока вы не остановите доступ

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = Это окно могут видеть: { $agents }
agents-stop = Остановить

## Asking before an agent acts.
agent-asks = Запрос от { $agent }
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = Разрешить { $agent } { $command }?
# { $name } is the control the agent would use ("Send").
agent-let-use = Разрешить { $agent } использовать «{ $name }»?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = Установить для «{ $name }» значение { $value }
did-press = Нажать «{ $name }»
did-raise = Увеличить: { $name }
did-lower = Уменьшить: { $name }
did-focus = Переместить в «{ $name }»
did-expand = Открыть { $name }
did-collapse = Закрыть { $name }
did-show = Показать { $name }
