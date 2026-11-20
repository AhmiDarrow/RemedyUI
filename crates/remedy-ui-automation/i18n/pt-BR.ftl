# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = Permitir { $agent }?
agent-asks-read = { $agent } pede para ver o que esta janela mostra { $how_long }. Senhas e segredos continuam ocultos.
agent-asks-act = { $agent } pede para ver e usar esta janela { $how_long }. Senhas e segredos continuam ocultos.
agent-asks-confirm = { $agent } pede para ver esta janela e usá-la, perguntando a você antes de cada ação, { $how_long }. Senhas e segredos continuam ocultos.
agent-for-minutes = { $count ->
    [one] por { $count } minuto
    [many] por { $count } minutos
   *[other] por { $count } minutos
}
agent-for-seconds = { $count ->
    [one] por { $count } segundo
    [many] por { $count } segundos
   *[other] por { $count } segundos
}
agent-until-stopped = até você interromper

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = Esta janela está visível para { $agents }
agents-stop = Parar

## Asking before an agent acts.
agent-asks = { $agent } pergunta
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = Permitir que { $agent } execute “{ $command }”?
# { $name } is the control the agent would use ("Send").
agent-let-use = Permitir que { $agent } use “{ $name }”?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = Definir { $name } como { $value }
did-press = Pressionar { $name }
did-raise = Aumentar { $name }
did-lower = Diminuir { $name }
did-focus = Mover para { $name }
did-expand = Abrir { $name }
did-collapse = Fechar { $name }
did-show = Mostrar { $name }
