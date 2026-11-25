# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } et { $second }
list-middle = { $first }, { $rest }
list-end = { $first } et { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first } ; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Thème
area-shortcuts = Raccourcis clavier
area-toolbars = Barres d’outils et menus
area-text = Textes
area-layouts = Dispositions d’écran

## The settings page.
customize = Personnaliser
customize-status = État de la personnalisation
nothing-to-customize = Rien à personnaliser
nothing-to-customize-hint = Cette application n’a pas activé la personnalisation.
reset-everything = Tout réinitialiser
reset-everything-done = Tout est revenu aux valeurs par défaut de l’application. Annuler permet de le récupérer.
# Also the undo history's name for resetting everything.
reset-all-customizations = Réinitialiser toutes les personnalisations

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Réinitialiser
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = Réinitialiser { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = Modifier { $name }
# { $error } says why.
could-not-reset = Impossible de réinitialiser : { $error }
not-saved = Non enregistré : { $error }
could-not-change = Impossible d’effectuer cette modification : { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = Impossible d’utiliser { $area } ({ $file }) : { $message }
notice-dismiss = Ignorer
# The dismiss button's accessible name.
notice-dismiss-named = Ignorer l’avis concernant { $file }
notice-not-utf8 = le fichier n’est pas du texte UTF-8
notice-could-not-save = impossible d’enregistrer : { $error }
notice-could-not-restore = impossible de restaurer : { $detail }

## Errors.
error-not-enabled-theme = La personnalisation du thème n’est pas activée dans cette application
error-not-enabled-shortcuts = La personnalisation des raccourcis n’est pas activée dans cette application
error-not-enabled-toolbars = La personnalisation des barres d’outils n’est pas activée dans cette application
error-not-enabled-text = La personnalisation des textes n’est pas activée dans cette application
error-not-enabled-layouts = La personnalisation des dispositions n’est pas activée dans cette application
# { $path } is a file, { $detail } what the system said.
error-io = { $path } : { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } est utilisé par { $commands }
error-refused = refusé : { $parts }
error-unknown-command = aucune commande { $id }
error-unknown-slot = aucune barre d’outils { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } n’accepte pas ce type de valeur

## Theme editor.
theme-editor = Éditeur de thème
theme-status = État du thème
theme-off = La personnalisation du thème est désactivée
theme-off-hint = Cette application ne permet pas de modifier son thème.
section-colors = Couleurs
# The section of fonts and text sizes.
section-text = Typographie
section-shape = Espacement et formes
section-motion = Animations
reset-theme = Réinitialiser le thème
# The button that opens a color's picker; its accessible name is edit-named.
edit = Modifier
edit-named = Modifier { $name }
# Units after a number.
unit-px = px
unit-ms = ms

## Design tokens, as people call them.
token-theme-name = Nom du thème
token-color-bg = Arrière-plan
token-color-panel = Panneaux
token-color-card = Cartes
token-color-text = Texte
token-color-muted = Texte atténué
token-color-accent = Couleur d’accentuation
token-color-on-accent = Texte sur l’accentuation
token-color-danger = Danger
token-color-warn = Avertissement
token-color-ok = Réussite
token-color-line = Traits
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Couleur « { $name } »
token-font = Police
token-font-mono = Police à chasse fixe
token-size-small = Taille du petit texte
token-size-body = Taille du texte courant
token-size-title = Taille des titres
token-spacing-unit = Unité d’espacement
token-control-height = Hauteur des commandes
token-radius-small = Petit rayon d’angle
token-radius-medium = Rayon d’angle moyen
token-radius-large = Grand rayon d’angle
token-stroke = Épaisseur des traits
token-motion-quick = Animation rapide
token-motion-standard = Animation standard
token-motion-emphasized = Animation accentuée
token-density = Densité
token-background-image = Image d’arrière-plan
token-font-file = Fichier de police « { $name } »

## How tightly controls are packed.
density-compact = Compacte
density-comfortable = Confortable
density-spacious = Aérée

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Contraste
contrast-all-pass = Toutes les paires de couleurs respectent le contraste WCAG.
# The contrast status read by screen readers.
contrast-status-all-pass = Toutes les paires de couleurs respectent le contraste WCAG
contrast-status-failing = { $count ->
    [one] { $count } paire de couleurs est sous le contraste WCAG
    [many] { $count } de paires de couleurs sont sous le contraste WCAG
   *[other] { $count } paires de couleurs sont sous le contraste WCAG
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair } : { NUMBER($ratio, minimumFractionDigits: 1) }:1, en dessous des { NUMBER($needed, minimumFractionDigits: 1) }:1 requis ({ $foreground } sur { $background })
contrast-pair-text-bg = Texte sur l’arrière-plan
contrast-pair-text-panel = Texte sur les panneaux
contrast-pair-text-card = Texte sur les cartes
contrast-pair-muted-bg = Texte atténué sur l’arrière-plan
contrast-pair-muted-panel = Texte atténué sur les panneaux
contrast-pair-muted-card = Texte atténué sur les cartes
contrast-pair-on-accent = Texte sur la couleur d’accentuation
contrast-pair-accent-bg = La couleur d’accentuation sur l’arrière-plan

## The preview: sample widgets in the theme being edited.
preview = Aperçu
preview-group = Aperçu du thème
preview-field = Champ d’exemple
preview-field-text = Texte dans un champ
preview-body = Le texte courant ressemble à ceci, à sa taille normale.
preview-muted = Texte atténué, pour les conseils et les détails.
preview-primary = Principal
preview-secondary = Secondaire
preview-danger = Danger
preview-switch = Un interrupteur
preview-progress = Progression
preview-link = Un lien
preview-error = Un message d’erreur.

## Sharing themes.
section-share = Partager
share-hint = Les thèmes sont des fichiers de design tokens que toute application Remedy peut lire.
theme-file = Fichier de thème
theme-file-placeholder = Un chemin comme themes/ocean.json
import-from-path = Importer depuis le chemin
export-to-path = Exporter vers le chemin
import-path-first = Saisissez d’abord le chemin du fichier de thème
export-path-first = Indiquez d’abord où enregistrer le thème
choose-theme = Choisir un thème à importer…
export-to-file = Exporter dans un fichier…
# File dialog titles and the name of their file type filter.
dialog-import-theme = Importer un thème
dialog-export-theme = Exporter le thème
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = theme
imported = Thème { $name } importé
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Thème { $name } importé, sans { $count } élément
    [many] Thème { $name } importé, sans { $count } d’éléments
   *[other] Thème { $name } importé, sans { $count } éléments
}
could-not-import = Impossible d’importer ce thème : { $error }
could-not-read = Impossible de lire { $path } : { $detail }
exported = Thème exporté vers { $path }
could-not-export = Impossible d’exporter le thème : { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = Ce thème contient plus que des couleurs et des tailles
theme-carries-more-detail = Ce thème contient plus que des couleurs et des tailles. Les éléments suivants restent désactivés, sauf si vous les activez :
allow-shaders = Autoriser les shaders (code exécuté par la carte graphique)
allow-links = Autoriser les liens externes (conservés, jamais téléchargés)
allow-files = Utiliser les images et polices qu’il fournit
import = Importer
import-without = Importer sans eux
cancel = Annuler
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token } : un shader ({ $detail })
refusal-link = { $token } : un lien externe ({ $detail })
refusal-file = { $token } : un fichier fourni par le thème ({ $detail })
theme-not-object = un fichier de thème est un objet JSON
# { $detail } is the JSON reader's message.
not-design-tokens = ce n’est pas un fichier de design tokens : { $detail }
history-import-theme = Importer le thème { $name }

## The theme's files: its background image and fonts.
section-files = Fichiers
files-hint = Les images s’affichent derrière chaque fenêtre, à sa taille ; les polices sont vérifiées avant d’être utilisées.
background-inline = Arrière-plan : une image intégrée au thème
# { $file } is the image's path or link.
background-file = Arrière-plan : { $file }
no-background = Aucune image d’arrière-plan
background-file-field = Fichier d’image d’arrière-plan
background-placeholder = Un chemin comme pictures/waves.png
use-image = Utiliser cette image
image-path-first = Saisissez d’abord le chemin de l’image
reset-background = Réinitialiser l’image d’arrière-plan
choose-image = Choisir une image…
dialog-choose-image = Choisir une image d’arrière-plan
filter-images = Images
no-fonts = Le thème ne fournit aucune police.
remove = Supprimer
remove-font-named = Supprimer la police { $family }
font-file = Fichier de police
font-file-placeholder = Un chemin comme fonts/Ocean-Regular.ttf
font-name = Nom de la police
font-name-placeholder = Le nom de la police, par exemple Ocean
add-font = Ajouter la police
font-first = Saisissez d’abord le chemin du fichier de police et son nom
history-change-background = Modifier l’image d’arrière-plan
history-remove-background = Supprimer l’image d’arrière-plan
history-add-font = Ajouter la police { $family }
history-remove-font = Supprimer la police { $family }
font-needs-name = une police a besoin d’un nom
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = impossible d’utiliser la police { $family } : { $detail }
background-not-used = impossible d’utiliser l’image d’arrière-plan : { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path } : { $detail }
asset-outside = les fichiers d’un thème se trouvent dans son dossier
asset-needs-file = importez le thème depuis son fichier pour que ses fichiers soient trouvés
asset-link = { $link } est un lien, et les liens ne sont pas téléchargés
not-an-image = ce n’est pas une image que cette application peut afficher : { $detail }
not-data-uri = ce n’est pas une URI data:
data-uri-comma = une URI data: comporte une virgule avant ses données
broken-escape = une séquence d’échappement % incorrecte
# { $char } is a character, in quotes.
not-base64 = { $char } n’est pas du base64

## Shortcut editor.
shortcut-editor = Éditeur de raccourcis
shortcut-status = État des raccourcis
shortcuts-off = La personnalisation des raccourcis est désactivée
shortcuts-off-hint = Cette application ne permet pas de modifier ses raccourcis.
search-commands = Rechercher des commandes
commands = Commandes
no-matching-command = Aucune commande correspondante
no-shortcut = Aucun raccourci
reset-all-shortcuts = Réinitialiser tous les raccourcis
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Ajouter un raccourci à { $command }
shortcut-remove-named = Retirer { $chord } de { $command }
reset-shortcuts-named = Réinitialiser les raccourcis de { $command }
shortcut-now-runs = { $chord } exécute désormais { $command }
shortcut-already-runs = { $chord } exécute déjà { $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } exécute déjà { $commands }.
shortcut-take = Utiliser { $chord } pour { $command }
shortcut-keep = Le garder pour { $commands }
history-change-shortcuts = Modifier les raccourcis de { $command }
history-add-shortcut = Ajouter { $chord } à { $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = Attribuer { $chord } à { $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = ignoré, le reste s’applique : { $parts }
shortcuts-clash-two = { $commands } répondraient tous deux à { $chord }
shortcuts-clash-many = { $commands } répondraient tous à { $chord }

## Toolbar editor.
toolbar-editor = Éditeur de barres d’outils
toolbar-status = État des barres d’outils
toolbars-off = La personnalisation des barres d’outils est désactivée
toolbars-off-hint = Cette application ne permet pas de modifier ses barres d’outils.
no-toolbars = Cette application n’a aucune barre d’outils modifiable.
reset-all-toolbars = Réinitialiser toutes les barres d’outils
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = ce n’est pas un fichier de barres d’outils : { $detail }
toolbars-not-object = un fichier de barres d’outils est un objet associant à chaque emplacement une liste d’identifiants de commandes
toolbar-slot-not-list = { $slot } : un emplacement est une liste d’identifiants de commandes comme ["file.save"]
toolbar-ids-not-text = { $slot } : les identifiants de commandes sont du texte

## Text editor.
text-editor = Éditeur de textes
text-status = État des textes
text-off = La personnalisation des textes est désactivée
text-off-hint = Cette application ne permet pas de modifier ses textes.
# { $folder } is a folder's path.
text-folder = Vos fichiers de messages se trouvent dans { $folder }
read-files-again = Relire les fichiers
search-text = Rechercher des textes
strings = Chaînes
no-matching-text = Aucun texte correspondant
reset-all-text = Réinitialiser tous les textes
could-not-change-text = Impossible de modifier ce texte : { $error }
could-not-reset-text = Impossible de réinitialiser ce texte : { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = Réinitialiser « { $text } »
change-text = Modifier « { $text } »
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Messages
search-messages = Rechercher des messages
no-matching-message = Aucun message correspondant
messages-hint = Messages Fluent : texte, espaces réservés comme {"{"} $name {"}"} et choix.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = Vérification de { $id }
text-skipped = ignoré : { $detail }
text-part-unread = une partie du fichier n’a pas pu être lue, le reste est utilisé : { $detail }

## Layout editor.
layout-editor = Éditeur de dispositions
layout-status = État des dispositions
layouts-off = La personnalisation des dispositions est désactivée
layouts-off-hint = Cette application ne permet pas de réorganiser ses écrans.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Vos dispositions se trouvent dans { $folder }
no-layouts = Aucun écran réorganisable n’est ouvert.
reset-all-layouts = Réinitialiser toutes les dispositions
layout-named = Disposition { $file }
layout-check = Vérification de la disposition
layout-ok = Aucun problème : la disposition peut être enregistrée.
layout-saved = Enregistré. L’écran affiche votre disposition.
save = Enregistrer
save-layout-named = Enregistrer la disposition { $file }
reset-layout-named = Réinitialiser la disposition { $file }
no-screen = aucun écran { $file } n’est affiché
