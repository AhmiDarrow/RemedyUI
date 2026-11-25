# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = { $agent } erlauben?
agent-asks-read = { $agent } bittet darum, zu sehen, was dieses Fenster anzeigt ({ $how_long }). Passwörter und Geheimnisse bleiben verborgen.
agent-asks-act = { $agent } bittet darum, dieses Fenster zu sehen und zu bedienen ({ $how_long }). Passwörter und Geheimnisse bleiben verborgen.
agent-asks-confirm = { $agent } bittet darum, dieses Fenster zu sehen und zu bedienen und dich vor jeder Aktion zu fragen ({ $how_long }). Passwörter und Geheimnisse bleiben verborgen.
agent-for-minutes = { $count ->
    [one] für { $count } Minute
   *[other] für { $count } Minuten
}
agent-for-seconds = { $count ->
    [one] für { $count } Sekunde
   *[other] für { $count } Sekunden
}
agent-until-stopped = bis du es beendest

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = Dieses Fenster ist sichtbar für { $agents }
agents-stop = Stopp

## Asking before an agent acts.
agent-asks = { $agent } fragt an
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = { $agent } { $command } lassen?
# { $name } is the control the agent would use ("Send").
agent-let-use = { $agent } „{ $name }“ verwenden lassen?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = { $name } auf { $value } setzen
did-press = { $name } drücken
did-raise = { $name } erhöhen
did-lower = { $name } verringern
did-focus = Fokus auf { $name } setzen
did-expand = { $name } öffnen
did-collapse = { $name } schließen
did-show = { $name } anzeigen
