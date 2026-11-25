# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = Отменить
redo = Повторить
add-area = Добавить область
area-heading = Область
area-title = Название области
area-title-hint = Название (Enter — применить)
area-place = Столбец { $column }, строка { $row }, ширина { $columns }, высота { $rows }
area-keys-hint = Стрелки перемещают её; Shift со стрелками меняет её размер.
area-keys = Клавиши со стрелками перемещают её; Shift с клавишами со стрелками меняет её размер
remove-area = Удалить область
widget-heading = Виджет
settings = Настройки
widget-keys-hint = Alt со стрелками вверх и вниз перемещает его; Alt со стрелками влево и вправо перемещает его в следующую область.
widget-keys = Alt с клавишами со стрелками перемещает его
remove-widget = Удалить виджет
nothing-selected = Выберите область или виджет, чтобы изменить их.
add-widget = Добавить виджет
widget-kind = Тип виджета

## The grid.
widget-name = Виджет «{ $name }»
unknown-kind = На этой доске есть виджет, которого нет в этом приложении ({ $kind }).
area-named = Область «{ $name }»
area-numbered = Область { $number }
empty-area = Пустая область: добавьте виджет или перетащите его сюда.
empty-board = Пустая доска
empty-board-editing = Эта доска пуста: добавьте область.
empty-board-viewing = Здесь пока ничего нет.
one-widget = В этой области уже есть виджет: выделите другому отдельную область.

## Why a change was refused.
error-invalid = это не доска: { $why }
error-too-new = эта доска создана в более новой версии (формат { $format })
error-outside = это выйдет за край доски
error-overlap = это закроет область «{ $area }»
error-missing = { $id } нет на доске

## Setting editors.
time-date = { $label }: дата
time-time = { $label }: время
time-not-set = Не задано
time-clear = Очистить
choice-unavailable = { $value } (недоступно)
list-item = { $label } { $number }
list-add = Добавить в «{ $label }»
list-remove = Удалить { $label } { $number }
picture-source = { $label }: адрес
picture-choose = Выбрать изображение

## Scripted widgets.
stopped = Этот виджет остановился: { $why }
start-again = Запустить снова

## The built-in widgets.
text-name = Текст
text-title = Заголовок
text-title-default = Добро пожаловать
text-body = Текст
text-body-default = Поздоровайтесь с домом.
links-name = Ссылки
links-title = Заголовок
links-title-default = Ссылки
links-links = Ссылки (Подпись | https://…, по одной в строке)
links-none = Ссылок пока нет.
reactions-name = Реакции
reactions-question = Вопрос
reactions-question-default = Как у всех дела?
reactions-offered = Доступные реакции

## The reactions component.
reaction-group = Реакции
reaction-one = 1 реакция
reaction-many = Реакций: { $count }
reaction-count = { $count ->
    [one] { $count } реакция
    [few] { $count } реакции
    [many] { $count } реакций
   *[other] { $count } реакции
}
reaction-spoken = { $emoji }, { $count }
reaction-spoken-mine = { $emoji }, { $count }, включая вашу
