# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = Allow { $agent }?
agent-asks-read = { $agent } asks to see what this window shows { $how_long }. Passwords and secrets stay hidden.
agent-asks-act = { $agent } asks to see and use this window { $how_long }. Passwords and secrets stay hidden.
agent-asks-confirm = { $agent } asks to see this window and use it, asking you before each action { $how_long }. Passwords and secrets stay hidden.
agent-for-minutes = { $count ->
    [one] for { $count } minute
   *[other] for { $count } minutes
}
agent-for-seconds = { $count ->
    [one] for { $count } second
   *[other] for { $count } seconds
}
agent-until-stopped = until you stop it

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = { $agents } can see this window
agents-stop = Stop

## Asking before an agent acts.
agent-asks = { $agent } asks
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = Let { $agent } { $command }?
# { $name } is the control the agent would use ("Send").
agent-let-use = Let { $agent } use “{ $name }”?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = Set { $name } to { $value }
did-press = Press { $name }
did-raise = Raise { $name }
did-lower = Lower { $name }
did-focus = Move to { $name }
did-expand = Open { $name }
did-collapse = Close { $name }
did-show = Show { $name }
