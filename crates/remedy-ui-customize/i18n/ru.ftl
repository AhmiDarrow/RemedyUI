# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first } и { $second }
list-middle = { $first }, { $rest }
list-end = { $first } и { $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }; { $rest }

## The kinds of customization: tab names and editor headings.
area-theme = Тема
area-shortcuts = Сочетания клавиш
area-toolbars = Панели инструментов и меню
area-text = Тексты
area-layouts = Макеты экранов

## The settings page.
customize = Настройка
customize-status = Состояние настройки
nothing-to-customize = Настраивать нечего
nothing-to-customize-hint = В этом приложении настройка не включена.
reset-everything = Сбросить всё
reset-everything-done = Для всего восстановлены значения приложения по умолчанию. Отменить сброс можно командой «Отменить».
# Also the undo history's name for resetting everything.
reset-all-customizations = Сбросить все настройки

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = Сбросить
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = Сбросить: { $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = Изменить: { $name }
# { $error } says why.
could-not-reset = Не удалось сбросить: { $error }
not-saved = Не сохранено: { $error }
could-not-change = Не удалось внести изменение: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = Не удалось применить раздел «{ $area }» ({ $file }): { $message }
notice-dismiss = Скрыть
# The dismiss button's accessible name.
notice-dismiss-named = Скрыть уведомление о файле { $file }
notice-not-utf8 = файл не является текстом в кодировке UTF-8
notice-could-not-save = не удалось сохранить: { $error }
notice-could-not-restore = не удалось восстановить: { $detail }

## Errors.
error-not-enabled-theme = Настройка темы в этом приложении не включена
error-not-enabled-shortcuts = Настройка сочетаний клавиш в этом приложении не включена
error-not-enabled-toolbars = Настройка панелей инструментов в этом приложении не включена
error-not-enabled-text = Настройка текстов в этом приложении не включена
error-not-enabled-layouts = Настройка макетов в этом приложении не включена
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } уже используется: { $commands }
error-refused = отклонено: { $parts }
error-unknown-command = нет команды { $id }
error-unknown-slot = нет панели инструментов { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } не принимает значения такого типа

## Theme editor.
theme-editor = Редактор темы
theme-status = Состояние темы
theme-off = Настройка темы отключена
theme-off-hint = Это приложение не позволяет менять тему.
section-colors = Цвета
# The section of fonts and text sizes.
section-text = Шрифты и текст
section-shape = Отступы и формы
section-motion = Анимация
reset-theme = Сбросить тему
# The button that opens a color's picker; its accessible name is edit-named.
edit = Изменить
edit-named = Изменить: { $name }
# Units after a number.
unit-px = пкс
unit-ms = мс

## Design tokens, as people call them.
token-theme-name = Название темы
token-color-bg = Фон
token-color-panel = Панели
token-color-card = Карточки
token-color-text = Текст
token-color-muted = Приглушённый текст
token-color-accent = Акцентный цвет
token-color-on-accent = Текст на акцентном цвете
token-color-danger = Опасность
token-color-warn = Предупреждение
token-color-ok = Успех
token-color-line = Линии
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = Цвет «{ $name }»
token-font = Шрифт
token-font-mono = Моноширинный шрифт
token-size-small = Размер мелкого текста
token-size-body = Размер основного текста
token-size-title = Размер заголовков
token-spacing-unit = Единица отступа
token-control-height = Высота элементов управления
token-radius-small = Малый радиус скругления
token-radius-medium = Средний радиус скругления
token-radius-large = Большой радиус скругления
token-stroke = Толщина линий
token-motion-quick = Быстрая анимация
token-motion-standard = Обычная анимация
token-motion-emphasized = Выразительная анимация
token-density = Плотность
token-background-image = Фоновое изображение
token-font-file = Файл шрифта «{ $name }»

## How tightly controls are packed.
density-compact = Компактная
density-comfortable = Удобная
density-spacious = Просторная

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = Контрастность
contrast-all-pass = Все пары цветов соответствуют контрастности WCAG.
# The contrast status read by screen readers.
contrast-status-all-pass = Все пары цветов соответствуют контрастности WCAG
contrast-status-failing = { $count ->
    [one] { $count } пара цветов не достигает контрастности WCAG
    [few] { $count } пары цветов не достигают контрастности WCAG
    [many] { $count } пар цветов не достигают контрастности WCAG
   *[other] { $count } пары цветов не достигают контрастности WCAG
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1, ниже требуемых { NUMBER($needed, minimumFractionDigits: 1) }:1 ({ $foreground } на фоне «{ $background }»)
contrast-pair-text-bg = Текст на фоне
contrast-pair-text-panel = Текст на панелях
contrast-pair-text-card = Текст на карточках
contrast-pair-muted-bg = Приглушённый текст на фоне
contrast-pair-muted-panel = Приглушённый текст на панелях
contrast-pair-muted-card = Приглушённый текст на карточках
contrast-pair-on-accent = Текст на акцентном цвете
contrast-pair-accent-bg = Акцентный цвет на фоне

## The preview: sample widgets in the theme being edited.
preview = Предварительный просмотр
preview-group = Предварительный просмотр темы
preview-field = Пример поля
preview-field-text = Текст в поле
preview-body = Так выглядит основной текст обычного размера.
preview-muted = Приглушённый текст для подсказок и подробностей.
preview-primary = Основная
preview-secondary = Дополнительная
preview-danger = Опасная
preview-switch = Переключатель
preview-progress = Ход выполнения
preview-link = Ссылка
preview-error = Сообщение об ошибке.

## Sharing themes.
section-share = Обмен
share-hint = Темы — это файлы design tokens, которые может прочитать любое приложение Remedy.
theme-file = Файл темы
theme-file-placeholder = Путь, например themes/ocean.json
import-from-path = Импортировать по пути
export-to-path = Экспортировать по пути
import-path-first = Сначала укажите путь к файлу темы
export-path-first = Сначала укажите, куда сохранить тему
choose-theme = Выбрать тему для импорта…
export-to-file = Экспортировать в файл…
# File dialog titles and the name of their file type filter.
dialog-import-theme = Импорт темы
dialog-export-theme = Экспорт темы
filter-design-tokens = Design tokens
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = тема
imported = Тема «{ $name }» импортирована
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
    [one] Тема «{ $name }» импортирована, пропущен { $count } элемент
    [few] Тема «{ $name }» импортирована, пропущено { $count } элемента
    [many] Тема «{ $name }» импортирована, пропущено { $count } элементов
   *[other] Тема «{ $name }» импортирована, пропущено { $count } элемента
}
could-not-import = Не удалось импортировать эту тему: { $error }
could-not-read = Не удалось прочитать { $path }: { $detail }
exported = Тема экспортирована в { $path }
could-not-export = Не удалось экспортировать тему: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = Эта тема содержит не только цвета и размеры
theme-carries-more-detail = Эта тема содержит не только цвета и размеры. Следующее останется отключённым, пока вы это не включите:
allow-shaders = Разрешить шейдеры (код, выполняемый на видеокарте)
allow-links = Разрешить внешние ссылки (сохраняются, но никогда не загружаются)
allow-files = Использовать изображения и шрифты из темы
import = Импортировать
import-without = Импортировать без них
cancel = Отмена
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: шейдер ({ $detail })
refusal-link = { $token }: внешняя ссылка ({ $detail })
refusal-file = { $token }: файл из темы ({ $detail })
theme-not-object = файл темы должен быть объектом JSON
# { $detail } is the JSON reader's message.
not-design-tokens = это не файл design tokens: { $detail }
history-import-theme = Импорт темы «{ $name }»

## The theme's files: its background image and fonts.
section-files = Файлы
files-hint = Изображения показываются за содержимым каждого окна по его размеру; шрифты проверяются перед использованием.
background-inline = Фон: изображение внутри темы
# { $file } is the image's path or link.
background-file = Фон: { $file }
no-background = Нет фонового изображения
background-file-field = Файл фонового изображения
background-placeholder = Путь, например pictures/waves.png
use-image = Использовать это изображение
image-path-first = Сначала укажите путь к изображению
reset-background = Сбросить фоновое изображение
choose-image = Выбрать изображение…
dialog-choose-image = Выбор фонового изображения
filter-images = Изображения
no-fonts = В теме нет шрифтов.
remove = Удалить
remove-font-named = Удалить шрифт { $family }
font-file = Файл шрифта
font-file-placeholder = Путь, например fonts/Ocean-Regular.ttf
font-name = Название шрифта
font-name-placeholder = Как называется шрифт, например Ocean
add-font = Добавить шрифт
font-first = Сначала укажите путь к файлу шрифта и его название
history-change-background = Изменение фонового изображения
history-remove-background = Удаление фонового изображения
history-add-font = Добавление шрифта { $family }
history-remove-font = Удаление шрифта { $family }
font-needs-name = у шрифта должно быть название
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = не удалось использовать шрифт { $family }: { $detail }
background-not-used = не удалось использовать фоновое изображение: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = файлы темы должны находиться в её папке
asset-needs-file = импортируйте тему из её файла, чтобы можно было найти её файлы
asset-link = { $link } — это ссылка, а ссылки не загружаются
not-an-image = это не изображение, которое может показать это приложение: { $detail }
not-data-uri = это не URI data:
data-uri-comma = в URI data: перед данными должна стоять запятая
broken-escape = неверная escape-последовательность %
# { $char } is a character, in quotes.
not-base64 = { $char } не является символом base64

## Shortcut editor.
shortcut-editor = Редактор сочетаний клавиш
shortcut-status = Состояние сочетаний клавиш
shortcuts-off = Настройка сочетаний клавиш отключена
shortcuts-off-hint = Это приложение не позволяет менять сочетания клавиш.
search-commands = Поиск команд
commands = Команды
no-matching-command = Подходящих команд нет
no-shortcut = Нет сочетания клавиш
reset-all-shortcuts = Сбросить все сочетания клавиш
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = Добавить сочетание клавиш для команды «{ $command }»
shortcut-remove-named = Удалить { $chord } у команды «{ $command }»
reset-shortcuts-named = Сбросить сочетания клавиш для команды «{ $command }»
shortcut-now-runs = { $chord } теперь выполняет команду «{ $command }»
shortcut-already-runs = { $chord } уже выполняет: { $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } уже выполняет: { $commands }.
shortcut-take = Использовать { $chord } для команды «{ $command }»
shortcut-keep = Оставить для: { $commands }
history-change-shortcuts = Изменение сочетаний клавиш для команды «{ $command }»
history-add-shortcut = Добавление { $chord } для команды «{ $command }»
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = Передача { $chord } команде «{ $command }»
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = пропущено, остальное применено: { $parts }
shortcuts-clash-two = { $commands } обе реагировали бы на { $chord }
shortcuts-clash-many = { $commands } все реагировали бы на { $chord }

## Toolbar editor.
toolbar-editor = Редактор панелей инструментов
toolbar-status = Состояние панелей инструментов
toolbars-off = Настройка панелей инструментов отключена
toolbars-off-hint = Это приложение не позволяет менять панели инструментов.
no-toolbars = В этом приложении нет панелей инструментов, которые можно изменить.
reset-all-toolbars = Сбросить все панели инструментов
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = это не файл панелей инструментов: { $detail }
toolbars-not-object = файл панелей инструментов должен быть объектом, сопоставляющим каждому месту список идентификаторов команд
toolbar-slot-not-list = { $slot }: место — это список идентификаторов команд, например ["file.save"]
toolbar-ids-not-text = { $slot }: идентификаторы команд должны быть текстом

## Text editor.
text-editor = Редактор текстов
text-status = Состояние текстов
text-off = Настройка текстов отключена
text-off-hint = Это приложение не позволяет менять тексты.
# { $folder } is a folder's path.
text-folder = Ваши файлы сообщений находятся в папке { $folder }
read-files-again = Перечитать файлы
search-text = Поиск текста
strings = Строки
no-matching-text = Подходящего текста нет
reset-all-text = Сбросить все тексты
could-not-change-text = Не удалось изменить этот текст: { $error }
could-not-reset-text = Не удалось сбросить этот текст: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = Сбросить «{ $text }»
change-text = Изменение «{ $text }»
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = Сообщения
search-messages = Поиск сообщений
no-matching-message = Подходящих сообщений нет
messages-hint = Сообщения Fluent: текст, заполнители вроде {"{"} $name {"}"} и варианты.
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = Проверка { $id }
text-skipped = пропущено: { $detail }
text-part-unread = часть файла не удалось прочитать, используется остальное: { $detail }

## Layout editor.
layout-editor = Редактор макетов
layout-status = Состояние макетов
layouts-off = Настройка макетов отключена
layouts-off-hint = Это приложение не позволяет перестраивать свои экраны.
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = Ваши макеты находятся в папке { $folder }
no-layouts = Нет открытых экранов, которые можно перестроить.
reset-all-layouts = Сбросить все макеты
layout-named = Макет { $file }
layout-check = Проверка макета
layout-ok = Проблем нет: макет можно сохранить.
layout-saved = Сохранено. На экране показан ваш макет.
save = Сохранить
save-layout-named = Сохранить макет { $file }
reset-layout-named = Сбросить макет { $file }
no-screen = экран { $file } не показан
