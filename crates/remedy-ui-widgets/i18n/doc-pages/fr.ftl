## Pages.
page-number = Page { $page }
pages-list = Pages
page-view-value = Page { $page } sur { $count }, { $zoom } %

annotation-kind = { $kind ->
    [highlight] Surlignage
    [underline] Soulignement
    [strikeout] Texte barré
    [squiggly] Soulignement ondulé
    [drawing] Dessin
    [note] Note
    [text-box] Zone de texte
    [rectangle] Rectangle
    [ellipse] Ellipse
    [line] Ligne
    [arrow] Flèche
    [image] Image
   *[annotation] Annotation
}
annotation-quoting = { $what } : « { $text } »
annotation-with-comment = { $what }, commentaire : { $comment }
annotation-by = { $what }, par { $author }
annotation-selected = Sélectionné : { $description }
annotation-count = { $count ->
    [one] { $count } annotation :
    [many] { $count } d’annotations :
   *[other] { $count } annotations :
}

undo-add = { $kind ->
    [highlight] Ajouter un surlignage
    [underline] Ajouter un soulignement
    [strikeout] Ajouter un texte barré
    [squiggly] Ajouter un soulignement ondulé
    [drawing] Ajouter un dessin
    [note] Ajouter une note
    [text-box] Ajouter une zone de texte
    [rectangle] Ajouter un rectangle
    [ellipse] Ajouter une ellipse
    [line] Ajouter une ligne
    [arrow] Ajouter une flèche
    [image] Ajouter une image
   *[annotation] Ajouter une annotation
}
undo-change = { $kind ->
    [highlight] Modifier le surlignage
    [underline] Modifier le soulignement
    [strikeout] Modifier le texte barré
    [squiggly] Modifier le soulignement ondulé
    [drawing] Modifier le dessin
    [note] Modifier la note
    [text-box] Modifier la zone de texte
    [rectangle] Modifier le rectangle
    [ellipse] Modifier l’ellipse
    [line] Modifier la ligne
    [arrow] Modifier la flèche
    [image] Modifier l’image
   *[annotation] Modifier l’annotation
}
undo-delete = { $kind ->
    [highlight] Supprimer le surlignage
    [underline] Supprimer le soulignement
    [strikeout] Supprimer le texte barré
    [squiggly] Supprimer le soulignement ondulé
    [drawing] Supprimer le dessin
    [note] Supprimer la note
    [text-box] Supprimer la zone de texte
    [rectangle] Supprimer le rectangle
    [ellipse] Supprimer l’ellipse
    [line] Supprimer la ligne
    [arrow] Supprimer la flèche
    [image] Supprimer l’image
   *[annotation] Supprimer l’annotation
}
undo-move = { $kind ->
    [highlight] Déplacer le surlignage
    [underline] Déplacer le soulignement
    [strikeout] Déplacer le texte barré
    [squiggly] Déplacer le soulignement ondulé
    [drawing] Déplacer le dessin
    [note] Déplacer la note
    [text-box] Déplacer la zone de texte
    [rectangle] Déplacer le rectangle
    [ellipse] Déplacer l’ellipse
    [line] Déplacer la ligne
    [arrow] Déplacer la flèche
    [image] Déplacer l’image
   *[annotation] Déplacer l’annotation
}
