# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = Annuler
redo = Rétablir
add-area = Ajouter une zone
area-heading = Zone
area-title = Titre de la zone
area-title-hint = Titre (Entrée pour appliquer)
area-place = Colonne { $column }, ligne { $row }, largeur { $columns }, hauteur { $rows }
area-keys-hint = Les flèches la déplacent ; Maj avec les flèches la redimensionne.
area-keys = Les touches fléchées la déplacent ; Maj avec les touches fléchées la redimensionne
remove-area = Supprimer la zone
widget-heading = Widget
settings = Paramètres
widget-keys-hint = Alt avec Haut et Bas le déplace ; Alt avec Gauche et Droite le déplace vers la zone suivante.
widget-keys = Alt avec les touches fléchées le déplace
remove-widget = Supprimer le widget
nothing-selected = Sélectionnez une zone ou un widget pour le modifier.
add-widget = Ajouter un widget
widget-kind = Type de widget

## The grid.
widget-name = Widget { $name }
unknown-kind = Ce tableau utilise un widget que cette application ne possède pas ({ $kind }).
area-named = Zone { $name }
area-numbered = Zone { $number }
empty-area = Zone vide : ajoutez un widget ou déposez-en un ici.
empty-board = Tableau vide
empty-board-editing = Ce tableau est vide : ajoutez une zone.
empty-board-viewing = Rien ici pour l’instant.
one-widget = Cette zone contient déjà un widget : donnez à l’autre une zone à lui.

## Why a change was refused.
error-invalid = ce n’est pas un tableau : { $why }
error-too-new = ce tableau a été créé par une version plus récente (format { $format })
error-outside = cela dépasserait le bord du tableau
error-overlap = cela recouvrirait la zone { $area }
error-missing = { $id } n’est pas sur le tableau

## Setting editors.
time-date = { $label } : date
time-time = { $label } : heure
time-not-set = Non défini
time-clear = Effacer
choice-unavailable = { $value } (non disponible)
list-item = { $label } { $number }
list-add = Ajouter à { $label }
list-remove = Supprimer { $label } { $number }
picture-source = { $label } : adresse
picture-choose = Choisir une image

## Scripted widgets.
stopped = Ce widget s’est arrêté : { $why }
start-again = Redémarrer

## The built-in widgets.
text-name = Texte
text-title = Titre
text-title-default = Bienvenue
text-body = Texte
text-body-default = Dites bonjour à la maison.
links-name = Liens
links-title = Titre
links-title-default = Liens
links-links = Liens (Libellé | https://…, un par ligne)
links-none = Aucun lien pour l’instant.
reactions-name = Réactions
reactions-question = Question
reactions-question-default = Comment allez-vous tous ?
reactions-offered = Réactions proposées

## The reactions component.
reaction-group = Réactions
reaction-one = 1 réaction
reaction-many = { $count } réactions
reaction-count = { $count ->
    [one] { $count } réaction
    [many] { $count } de réactions
   *[other] { $count } réactions
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, dont la vôtre
