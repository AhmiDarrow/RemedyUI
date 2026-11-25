# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = Desfazer
redo = Refazer
add-area = Adicionar área
area-heading = Área
area-title = Título da área
area-title-hint = Título (Enter para aplicar)
area-place = Coluna { $column }, linha { $row }, largura { $columns }, altura { $rows }
area-keys-hint = As setas a movem; Shift com as setas a redimensiona.
area-keys = As teclas de seta a movem; Shift com as teclas de seta a redimensiona
remove-area = Remover área
widget-heading = Widget
settings = Configurações
widget-keys-hint = Alt com Para cima e Para baixo o move; Alt com Esquerda e Direita o move para a próxima área.
widget-keys = Alt com as teclas de seta o move
remove-widget = Remover widget
nothing-selected = Selecione uma área ou um widget para alterá-lo.
add-widget = Adicionar widget
widget-kind = Tipo de widget

## The grid.
widget-name = Widget { $name }
unknown-kind = Este quadro usa um widget que este aplicativo não tem ({ $kind }).
area-named = Área { $name }
area-numbered = Área { $number }
empty-area = Área vazia: adicione um widget ou solte um aqui.
empty-board = Quadro vazio
empty-board-editing = Este quadro está vazio: adicione uma área.
empty-board-viewing = Nada por aqui ainda.
one-widget = Esta área já contém um widget: dê ao outro uma área própria.

## Why a change was refused.
error-invalid = não é um quadro: { $why }
error-too-new = este quadro foi criado por uma versão mais recente (formato { $format })
error-outside = isso passaria da borda do quadro
error-overlap = isso cobriria a área { $area }
error-missing = { $id } não está no quadro

## Setting editors.
time-date = { $label }: data
time-time = { $label }: hora
time-not-set = Não definido
time-clear = Limpar
choice-unavailable = { $value } (indisponível)
list-item = { $label } { $number }
list-add = Adicionar a { $label }
list-remove = Remover { $label } { $number }
picture-source = { $label }: endereço
picture-choose = Escolher imagem

## Scripted widgets.
stopped = Este widget parou: { $why }
start-again = Iniciar de novo

## The built-in widgets.
text-name = Texto
text-title = Título
text-title-default = Boas-vindas
text-body = Texto
text-body-default = Diga olá para a casa.
links-name = Links
links-title = Título
links-title-default = Links
links-links = Links (Rótulo | https://…, um por linha)
links-none = Nenhum link ainda.
reactions-name = Reações
reactions-question = Pergunta
reactions-question-default = Como vocês estão?
reactions-offered = Reações oferecidas

## The reactions component.
reaction-group = Reações
reaction-one = 1 reação
reaction-many = { $count } reações
reaction-count = { $count ->
    [one] { $count } reação
    [many] { $count } de reações
   *[other] { $count } reações
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, incluindo a sua
