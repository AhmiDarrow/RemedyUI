# The agent access prompts and what agents did. Placeholders ({ $name }) must stay; their order may change.

## Asking for access. { $agent } is the agent's name; { $how_long } one of the durations below.
agent-allow = Autoriser { $agent } ?
agent-asks-read = { $agent } demande à voir ce qu'affiche cette fenêtre { $how_long }. Les mots de passe et les secrets restent masqués.
agent-asks-act = { $agent } demande à voir et à utiliser cette fenêtre { $how_long }. Les mots de passe et les secrets restent masqués.
agent-asks-confirm = { $agent } demande à voir cette fenêtre et à l'utiliser, en vous demandant l'autorisation avant chaque action, { $how_long }. Les mots de passe et les secrets restent masqués.
agent-for-minutes = { $count ->
    [one] pendant { $count } minute
    [many] pendant { $count } minutes
   *[other] pendant { $count } minutes
}
agent-for-seconds = { $count ->
    [one] pendant { $count } seconde
    [many] pendant { $count } secondes
   *[other] pendant { $count } secondes
}
agent-until-stopped = jusqu'à ce que vous l'arrêtiez

## The indicator while agents have access. { $agents } lists their names.
agents-can-see = Cette fenêtre est visible par { $agents }
agents-stop = Arrêter

## Asking before an agent acts.
agent-asks = { $agent } demande
# { $command } is the command's label, lowercased ("delete the draft").
agent-let-command = Autoriser { $agent } à { $command } ?
# { $name } is the control the agent would use ("Send").
agent-let-use = Autoriser { $agent } à utiliser « { $name } » ?

## What an agent did, for the undo history. { $name } is the control's name.
did-set = Régler { $name } sur { $value }
did-press = Appuyer sur { $name }
did-raise = Augmenter { $name }
did-lower = Diminuer { $name }
did-focus = Déplacer vers { $name }
did-expand = Ouvrir { $name }
did-collapse = Fermer { $name }
did-show = Afficher { $name }
