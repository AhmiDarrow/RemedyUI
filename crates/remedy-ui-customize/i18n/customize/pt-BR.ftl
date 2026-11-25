# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } e { $second }
list-middle = { $first }, { $rest }
list-end = { $first } e { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Tema
area-shortcuts = Atalhos de teclado
area-toolbars = Barras de ferramentas e menus
area-text = Textos
area-layouts = Layouts de tela

## The settings page.
customize = Personalizar
customize-status = Status da personalização
nothing-to-customize = Nada para personalizar
nothing-to-customize-hint = Este aplicativo não ativou a personalização.
reset-everything = Redefinir tudo
reset-everything-done = Tudo voltou aos padrões do aplicativo. Desfazer traz de volta.
# Also the undo history's name for resetting everything.
reset-all-customizations = Redefinir todas as personalizações

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Redefinir
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = Redefinir { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = Alterar { $name }
# { $error } says why.
could-not-reset = Não foi possível redefinir: { $error }
not-saved = Não salvo: { $error }
could-not-change = Não foi possível fazer essa alteração: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = Não foi possível usar { $area } ({ $file }): { $message }
notice-dismiss = Dispensar
# The dismiss button's accessible name.
notice-dismiss-named = Dispensar o aviso sobre { $file }
notice-not-utf8 = o arquivo não é texto UTF-8
notice-could-not-save = não foi possível salvar: { $error }
notice-could-not-restore = não foi possível restaurar: { $detail }

## Errors.
error-not-enabled-theme = A personalização do tema não está ativada neste aplicativo
error-not-enabled-shortcuts = A personalização dos atalhos não está ativada neste aplicativo
error-not-enabled-toolbars = A personalização das barras de ferramentas não está ativada neste aplicativo
error-not-enabled-text = A personalização dos textos não está ativada neste aplicativo
error-not-enabled-layouts = A personalização dos layouts não está ativada neste aplicativo
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } é usado por { $commands }
error-refused = recusado: { $parts }
error-unknown-command = nenhum comando { $id }
error-unknown-slot = nenhuma barra de ferramentas { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } não aceita esse tipo de valor

## Theme editor.
theme-editor = Editor de tema
theme-status = Status do tema
theme-off = A personalização do tema está desativada
theme-off-hint = Este aplicativo não permite alterar o tema.
section-colors = Cores
# The section of fonts and text sizes.
section-text = Tipografia
section-shape = Espaçamento e formas
section-motion = Movimento
reset-theme = Redefinir tema
# The button that opens a color's picker; its accessible name is edit-named.
edit = Editar
edit-named = Editar { $name }
# Units after a number.
unit-px = px
unit-ms = ms

## Design tokens, as people call them.
token-theme-name = Nome do tema
token-color-bg = Fundo
token-color-panel = Painéis
token-color-card = Cartões
token-color-text = Texto
token-color-muted = Texto atenuado
token-color-accent = Destaque
token-color-on-accent = Texto sobre o destaque
token-color-danger = Perigo
token-color-warn = Aviso
token-color-ok = Sucesso
token-color-line = Linhas
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Cor “{ $name }”
token-font = Fonte
token-font-mono = Fonte monoespaçada
token-size-small = Tamanho do texto pequeno
token-size-body = Tamanho do texto principal
token-size-title = Tamanho do título
token-spacing-unit = Unidade de espaçamento
token-control-height = Altura dos controles
token-radius-small = Raio de canto pequeno
token-radius-medium = Raio de canto médio
token-radius-large = Raio de canto grande
token-stroke = Espessura da linha
token-motion-quick = Movimento rápido
token-motion-standard = Movimento padrão
token-motion-emphasized = Movimento enfatizado
token-density = Densidade
token-background-image = Imagem de fundo
token-font-file = Arquivo de fonte “{ $name }”

## How tightly controls are packed.
density-compact = Compacta
density-comfortable = Confortável
density-spacious = Espaçosa

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Contraste
contrast-all-pass = Todos os pares de cores atendem ao contraste WCAG.
# The contrast status read by screen readers.
contrast-status-all-pass = Todos os pares de cores atendem ao contraste WCAG
contrast-status-failing = { $count ->
    [one] { $count } par de cores está abaixo do contraste WCAG
    [many] { $count } de pares de cores estão abaixo do contraste WCAG
   *[other] { $count } pares de cores estão abaixo do contraste WCAG
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, abaixo dos { NUMBER($needed, minimumFractionDigits: 1) }:1 necessários ({ $foreground } sobre { $background })
contrast-pair-text-bg = Texto sobre o fundo
contrast-pair-text-panel = Texto sobre painéis
contrast-pair-text-card = Texto sobre cartões
contrast-pair-muted-bg = Texto atenuado sobre o fundo
contrast-pair-muted-panel = Texto atenuado sobre painéis
contrast-pair-muted-card = Texto atenuado sobre cartões
contrast-pair-on-accent = Texto sobre o destaque
contrast-pair-accent-bg = O destaque sobre o fundo

## The preview: sample widgets in the theme being edited.
preview = Pré-visualização
preview-group = Pré-visualização do tema
preview-field = Campo de exemplo
preview-field-text = Texto em um campo
preview-body = O texto principal fica assim, no tamanho normal.
preview-muted = Texto atenuado, para dicas e detalhes.
preview-primary = Principal
preview-secondary = Secundário
preview-danger = Perigo
preview-switch = Um interruptor
preview-progress = Progresso
preview-link = Um link
preview-error = Uma mensagem de erro.

## Sharing themes.
section-share = Compartilhar
share-hint = Temas são arquivos de design tokens que qualquer aplicativo Remedy consegue ler.
theme-file = Arquivo de tema
theme-file-placeholder = Um caminho como themes/ocean.json
import-from-path = Importar do caminho
export-to-path = Exportar para o caminho
import-path-first = Digite primeiro o caminho do arquivo de tema
export-path-first = Digite primeiro onde salvar o tema
choose-theme = Escolher um tema para importar…
export-to-file = Exportar para um arquivo…
# File dialog titles and the name of their file type filter.
dialog-import-theme = Importar um tema
dialog-export-theme = Exportar o tema
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = tema
imported = Tema { $name } importado
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Tema { $name } importado, sem { $count } item
    [many] Tema { $name } importado, sem { $count } de itens
   *[other] Tema { $name } importado, sem { $count } itens
}
could-not-import = Não foi possível importar esse tema: { $error }
could-not-read = Não foi possível ler { $path }: { $detail }
exported = Tema exportado para { $path }
could-not-export = Não foi possível exportar o tema: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = Este tema traz mais do que cores e tamanhos
theme-carries-more-detail = Este tema traz mais do que cores e tamanhos. Os itens a seguir ficam desativados, a menos que você os ative:
allow-shaders = Permitir shaders (código executado na placa de vídeo)
allow-links = Permitir links externos (mantidos, nunca baixados)
allow-files = Usar as imagens e fontes que ele traz
import = Importar
import-without = Importar sem eles
cancel = Cancelar
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: um shader ({ $detail })
refusal-link = { $token }: um link externo ({ $detail })
refusal-file = { $token }: um arquivo que o tema traz ({ $detail })
theme-not-object = um arquivo de tema é um objeto JSON
# { $detail } is the JSON reader's message.
not-design-tokens = não é um arquivo de design tokens: { $detail }
history-import-theme = Importar o tema { $name }

## The theme's files: its background image and fonts.
section-files = Arquivos
files-hint = As imagens aparecem atrás de cada janela, no tamanho dela; as fontes são verificadas antes do uso.
background-inline = Fundo: uma imagem dentro do tema
# { $file } is the image's path or link.
background-file = Fundo: { $file }
no-background = Sem imagem de fundo
background-file-field = Arquivo da imagem de fundo
background-placeholder = Um caminho como pictures/waves.png
use-image = Usar esta imagem
image-path-first = Digite primeiro o caminho da imagem
reset-background = Redefinir imagem de fundo
choose-image = Escolher uma imagem…
dialog-choose-image = Escolher uma imagem de fundo
filter-images = Imagens
no-fonts = O tema não traz fontes.
remove = Remover
remove-font-named = Remover a fonte { $family }
font-file = Arquivo de fonte
font-file-placeholder = Um caminho como fonts/Ocean-Regular.ttf
font-name = Nome da fonte
font-name-placeholder = Como a fonte se chama, por exemplo Ocean
add-font = Adicionar fonte
font-first = Digite primeiro o caminho do arquivo de fonte e o nome dela
history-change-background = Alterar a imagem de fundo
history-remove-background = Remover a imagem de fundo
history-add-font = Adicionar a fonte { $family }
history-remove-font = Remover a fonte { $family }
font-needs-name = uma fonte precisa de um nome
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = não foi possível usar a fonte { $family }: { $detail }
background-not-used = não foi possível usar a imagem de fundo: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = os arquivos de um tema ficam dentro da pasta dele
asset-needs-file = importe o tema a partir do arquivo dele para que os arquivos possam ser encontrados
asset-link = { $link } é um link, e links não são baixados
not-an-image = não é uma imagem que este aplicativo consiga exibir: { $detail }
not-data-uri = não é um URI data:
data-uri-comma = um URI data: tem uma vírgula antes dos dados
broken-escape = uma sequência de escape % inválida
# { $char } is a character, in quotes.
not-base64 = { $char } não é base64

## Shortcut editor.
shortcut-editor = Editor de atalhos
shortcut-status = Status dos atalhos
shortcuts-off = A personalização dos atalhos está desativada
shortcuts-off-hint = Este aplicativo não permite alterar os atalhos.
search-commands = Pesquisar comandos
commands = Comandos
no-matching-command = Nenhum comando correspondente
no-shortcut = Nenhum atalho
reset-all-shortcuts = Redefinir todos os atalhos
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Adicionar um atalho a { $command }
shortcut-remove-named = Remover { $chord } de { $command }
reset-shortcuts-named = Redefinir os atalhos de { $command }
shortcut-now-runs = { $chord } agora executa { $command }
shortcut-already-runs = { $chord } já executa { $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } já executa { $commands }.
shortcut-take = Usar { $chord } para { $command }
shortcut-keep = Manter para { $commands }
history-change-shortcuts = Alterar os atalhos de { $command }
history-add-shortcut = Adicionar { $chord } a { $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = Atribuir { $chord } a { $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = ignorado, o restante se aplica: { $parts }
shortcuts-clash-two = { $commands } responderiam ambos a { $chord }
shortcuts-clash-many = { $commands } responderiam todos a { $chord }

## Toolbar editor.
toolbar-editor = Editor de barras de ferramentas
toolbar-status = Status das barras de ferramentas
toolbars-off = A personalização das barras de ferramentas está desativada
toolbars-off-hint = Este aplicativo não permite alterar as barras de ferramentas.
no-toolbars = Este aplicativo não tem barras de ferramentas que possam ser alteradas.
reset-all-toolbars = Redefinir todas as barras de ferramentas
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = não é um arquivo de barras de ferramentas: { $detail }
toolbars-not-object = um arquivo de barras de ferramentas é um objeto que associa cada posição a uma lista de IDs de comandos
toolbar-slot-not-list = { $slot }: uma posição é uma lista de IDs de comandos como ["file.save"]
toolbar-ids-not-text = { $slot }: os IDs de comandos são texto

## Text editor.
text-editor = Editor de textos
text-status = Status dos textos
text-off = A personalização dos textos está desativada
text-off-hint = Este aplicativo não permite alterar os textos.
# { $folder } is a folder's path.
text-folder = Seus arquivos de mensagens estão em { $folder }
read-files-again = Ler os arquivos novamente
search-text = Pesquisar textos
strings = Strings
no-matching-text = Nenhum texto correspondente
reset-all-text = Redefinir todos os textos
could-not-change-text = Não foi possível alterar esse texto: { $error }
could-not-reset-text = Não foi possível redefinir esse texto: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = Redefinir “{ $text }”
change-text = Alterar “{ $text }”
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Mensagens
search-messages = Pesquisar mensagens
no-matching-message = Nenhuma mensagem correspondente
messages-hint = Mensagens do Fluent: texto, marcadores como {"{"} $name {"}"} e escolhas.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = Verificação de { $id }
text-skipped = ignorado: { $detail }
text-part-unread = parte do arquivo não pôde ser lida, o restante é usado: { $detail }

## Layout editor.
layout-editor = Editor de layouts
layout-status = Status dos layouts
layouts-off = A personalização dos layouts está desativada
layouts-off-hint = Este aplicativo não permite reorganizar as telas.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Seus layouts estão em { $folder }
no-layouts = Nenhuma tela que possa ser reorganizada está aberta.
reset-all-layouts = Redefinir todos os layouts
layout-named = Layout { $file }
layout-check = Verificação do layout
layout-ok = Nenhum problema: o layout pode ser salvo.
layout-saved = Salvo. A tela mostra o seu layout.
save = Salvar
save-layout-named = Salvar o layout { $file }
reset-layout-named = Redefinir o layout { $file }
no-screen = nenhuma tela { $file } está sendo exibida
