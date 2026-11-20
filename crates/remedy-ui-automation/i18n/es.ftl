# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = ¿Permitir a { $agent }?
agent-asks-read = { $agent } pide ver lo que muestra esta ventana { $how_long }. Las contraseñas y los secretos permanecen ocultos.
agent-asks-act = { $agent } pide ver y usar esta ventana { $how_long }. Las contraseñas y los secretos permanecen ocultos.
agent-asks-confirm = { $agent } pide ver esta ventana y usarla, preguntándote antes de cada acción { $how_long }. Las contraseñas y los secretos permanecen ocultos.
agent-for-minutes = { $count ->
    [one] durante { $count } minuto
    [many] durante { $count } minutos
   *[other] durante { $count } minutos
}
agent-for-seconds = { $count ->
    [one] durante { $count } segundo
    [many] durante { $count } segundos
   *[other] durante { $count } segundos
}
agent-until-stopped = hasta que lo detengas

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = Pueden ver esta ventana: { $agents }
agents-stop = Detener

## Asking before an agent acts.
agent-asks = { $agent } pregunta
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = ¿Permitir que { $agent } ejecute «{ $command }»?
# { $name } is the control the agent would use ("Send").
agent-let-use = ¿Permitir que { $agent } use «{ $name }»?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = Establecer { $name } en { $value }
did-press = Pulsar { $name }
did-raise = Subir { $name }
did-lower = Bajar { $name }
did-focus = Mover a { $name }
did-expand = Abrir { $name }
did-collapse = Cerrar { $name }
did-show = Mostrar { $name }
