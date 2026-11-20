## Страницы.
page-number = Страница { $page }
pages-list = Страницы
page-view-value = Страница { $page } из { $count }, { $zoom } %

annotation-kind = { $kind ->
    [highlight] Выделение
    [underline] Подчёркивание
    [strikeout] Зачёркивание
    [squiggly] Волнистое подчёркивание
    [drawing] Рисунок
    [note] Заметка
    [text-box] Текстовое поле
    [rectangle] Прямоугольник
    [ellipse] Эллипс
    [line] Линия
    [arrow] Стрелка
   *[annotation] Аннотация
}
annotation-quoting = { $what }: «{ $text }»
annotation-with-comment = { $what }, комментарий: { $comment }
annotation-by = { $what }, автор: { $author }
annotation-selected = Выбрано: { $description }
annotation-count = { $count ->
    [one] { $count } аннотация:
    [few] { $count } аннотации:
    [many] { $count } аннотаций:
   *[other] { $count } аннотации:
}

undo-add = { $kind ->
    [highlight] Добавить выделение
    [underline] Добавить подчёркивание
    [strikeout] Добавить зачёркивание
    [squiggly] Добавить волнистое подчёркивание
    [drawing] Добавить рисунок
    [note] Добавить заметку
    [text-box] Добавить текстовое поле
    [rectangle] Добавить прямоугольник
    [ellipse] Добавить эллипс
    [line] Добавить линию
    [arrow] Добавить стрелку
   *[annotation] Добавить аннотацию
}
undo-change = { $kind ->
    [highlight] Изменить выделение
    [underline] Изменить подчёркивание
    [strikeout] Изменить зачёркивание
    [squiggly] Изменить волнистое подчёркивание
    [drawing] Изменить рисунок
    [note] Изменить заметку
    [text-box] Изменить текстовое поле
    [rectangle] Изменить прямоугольник
    [ellipse] Изменить эллипс
    [line] Изменить линию
    [arrow] Изменить стрелку
   *[annotation] Изменить аннотацию
}
undo-delete = { $kind ->
    [highlight] Удалить выделение
    [underline] Удалить подчёркивание
    [strikeout] Удалить зачёркивание
    [squiggly] Удалить волнистое подчёркивание
    [drawing] Удалить рисунок
    [note] Удалить заметку
    [text-box] Удалить текстовое поле
    [rectangle] Удалить прямоугольник
    [ellipse] Удалить эллипс
    [line] Удалить линию
    [arrow] Удалить стрелку
   *[annotation] Удалить аннотацию
}
undo-move = { $kind ->
    [highlight] Переместить выделение
    [underline] Переместить подчёркивание
    [strikeout] Переместить зачёркивание
    [squiggly] Переместить волнистое подчёркивание
    [drawing] Переместить рисунок
    [note] Переместить заметку
    [text-box] Переместить текстовое поле
    [rectangle] Переместить прямоугольник
    [ellipse] Переместить эллипс
    [line] Переместить линию
    [arrow] Переместить стрелку
   *[annotation] Переместить аннотацию
}
