# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first }和{ $second }
list-middle = { $first }、{ $rest }
list-end = { $first }和{ $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }；{ $rest }

## The kinds of customization: tab names and editor headings.
area-theme = 主题
area-shortcuts = 键盘快捷键
area-toolbars = 工具栏和菜单
area-text = 文本
area-layouts = 屏幕布局

## The settings page.
customize = 自定义
customize-status = 自定义状态
nothing-to-customize = 没有可自定义的内容
nothing-to-customize-hint = 此应用未启用自定义。
reset-everything = 全部重置
reset-everything-done = 所有内容都已恢复为应用的默认设置。撤销可以恢复。
# Also the undo history's name for resetting everything.
reset-all-customizations = 重置所有自定义

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = 重置
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = 重置{ $name }
# The undo history's name for changing one named thing ("Change Accent").
change-named = 更改{ $name }
# { $error } says why.
could-not-reset = 无法重置：{ $error }
not-saved = 未保存：{ $error }
could-not-change = 无法进行此更改：{ $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = 无法使用{ $area }（{ $file }）：{ $message }
notice-dismiss = 关闭
# The dismiss button's accessible name.
notice-dismiss-named = 关闭关于 { $file } 的通知
notice-not-utf8 = 该文件不是 UTF-8 文本
notice-could-not-save = 无法保存：{ $error }
notice-could-not-restore = 无法恢复：{ $detail }

## Errors.
error-not-enabled-theme = 此应用未启用主题自定义
error-not-enabled-shortcuts = 此应用未启用快捷键自定义
error-not-enabled-toolbars = 此应用未启用工具栏自定义
error-not-enabled-text = 此应用未启用文本自定义
error-not-enabled-layouts = 此应用未启用布局自定义
# { $path } is a file, { $detail } what the system said.
error-io = { $path }：{ $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord } 已被{ $commands }使用
error-refused = 已拒绝：{ $parts }
error-unknown-command = 没有命令 { $id }
error-unknown-slot = 没有工具栏 { $id }
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token } 不能保存这种类型的值

## Theme editor.
theme-editor = 主题编辑器
theme-status = 主题状态
theme-off = 主题自定义已关闭
theme-off-hint = 此应用不允许更改主题。
section-colors = 颜色
# The section of fonts and text sizes.
section-text = 文字
section-shape = 间距和形状
section-motion = 动画
reset-theme = 重置主题
# The button that opens a color's picker; its accessible name is edit-named.
edit = 编辑
edit-named = 编辑{ $name }
# Units after a number.
unit-px = 像素
unit-ms = 毫秒

## Design tokens, as people call them.
token-theme-name = 主题名称
token-color-bg = 背景
token-color-panel = 面板
token-color-card = 卡片
token-color-text = 文本
token-color-muted = 次要文本
token-color-accent = 强调色
token-color-on-accent = 强调色上的文本
token-color-danger = 危险
token-color-warn = 警告
token-color-ok = 成功
token-color-line = 线条
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = 颜色“{ $name }”
token-font = 字体
token-font-mono = 等宽字体
token-size-small = 小号文本大小
token-size-body = 正文大小
token-size-title = 标题大小
token-spacing-unit = 间距单位
token-control-height = 控件高度
token-radius-small = 小圆角半径
token-radius-medium = 中圆角半径
token-radius-large = 大圆角半径
token-stroke = 线条粗细
token-motion-quick = 快速动画
token-motion-standard = 标准动画
token-motion-emphasized = 强调动画
token-density = 密度
token-background-image = 背景图片
token-font-file = 字体文件“{ $name }”

## How tightly controls are packed.
density-compact = 紧凑
density-comfortable = 舒适
density-spacious = 宽松

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = 对比度
contrast-all-pass = 所有颜色组合都符合 WCAG 对比度要求。
# The contrast status read by screen readers.
contrast-status-all-pass = 所有颜色组合都符合 WCAG 对比度要求
contrast-status-failing = { $count ->
   *[other] { $count } 组颜色低于 WCAG 对比度要求
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }：{ NUMBER($ratio, minimumFractionDigits: 1) }:1，低于所需的 { NUMBER($needed, minimumFractionDigits: 1) }:1（{ $background }上的{ $foreground }）
contrast-pair-text-bg = 背景上的文本
contrast-pair-text-panel = 面板上的文本
contrast-pair-text-card = 卡片上的文本
contrast-pair-muted-bg = 背景上的次要文本
contrast-pair-muted-panel = 面板上的次要文本
contrast-pair-muted-card = 卡片上的次要文本
contrast-pair-on-accent = 强调色上的文本
contrast-pair-accent-bg = 背景上的强调色

## The preview: sample widgets in the theme being edited.
preview = 预览
preview-group = 主题预览
preview-field = 示例字段
preview-field-text = 字段中的文本
preview-body = 正文以标准大小显示时是这样的。
preview-muted = 次要文本，用于提示和详细信息。
preview-primary = 主要
preview-secondary = 次要
preview-danger = 危险
preview-switch = 开关
preview-progress = 进度
preview-link = 链接
preview-error = 错误消息。

## Sharing themes.
section-share = 共享
share-hint = 主题是任何 Remedy 应用都能读取的设计令牌（design tokens）文件。
theme-file = 主题文件
theme-file-placeholder = 路径，例如 themes/ocean.json
import-from-path = 从路径导入
export-to-path = 导出到路径
import-path-first = 请先输入主题文件的路径
export-path-first = 请先输入主题的保存位置
choose-theme = 选择要导入的主题…
export-to-file = 导出到文件…
# File dialog titles and the name of their file type filter.
dialog-import-theme = 导入主题
dialog-export-theme = 导出主题
filter-design-tokens = 设计令牌
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = 主题
imported = 已导入主题“{ $name }”
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
   *[other] 已导入主题“{ $name }”，省略了 { $count } 项
}
could-not-import = 无法导入该主题：{ $error }
could-not-read = 无法读取 { $path }：{ $detail }
exported = 已将主题导出到 { $path }
could-not-export = 无法导出主题：{ $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = 此主题包含的不只是颜色和大小
theme-carries-more-detail = 此主题包含的不只是颜色和大小。除非你开启，以下内容将保持关闭：
allow-shaders = 允许着色器（在显卡上运行的代码）
allow-links = 允许外部链接（会保留，但从不获取）
allow-files = 使用其中附带的图片和字体
import = 导入
import-without = 不含这些内容导入
cancel = 取消
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }：着色器（{ $detail }）
refusal-link = { $token }：外部链接（{ $detail }）
refusal-file = { $token }：主题附带的文件（{ $detail }）
theme-not-object = 主题文件必须是 JSON 对象
# { $detail } is the JSON reader's message.
not-design-tokens = 不是设计令牌文件：{ $detail }
history-import-theme = 导入主题“{ $name }”

## The theme's files: its background image and fonts.
section-files = 文件
files-hint = 图片按窗口大小显示在每个窗口后面；字体在使用前会经过检查。
background-inline = 背景：主题内嵌的图片
# { $file } is the image's path or link.
background-file = 背景：{ $file }
no-background = 无背景图片
background-file-field = 背景图片文件
background-placeholder = 路径，例如 pictures/waves.png
use-image = 使用此图片
image-path-first = 请先输入图片的路径
reset-background = 重置背景图片
choose-image = 选择图片…
dialog-choose-image = 选择背景图片
filter-images = 图片
no-fonts = 此主题没有附带字体。
remove = 移除
remove-font-named = 移除字体 { $family }
font-file = 字体文件
font-file-placeholder = 路径，例如 fonts/Ocean-Regular.ttf
font-name = 字体名称
font-name-placeholder = 字体的名称，例如 Ocean
add-font = 添加字体
font-first = 请先输入字体文件的路径及其名称
history-change-background = 更改背景图片
history-remove-background = 移除背景图片
history-add-font = 添加字体 { $family }
history-remove-font = 移除字体 { $family }
font-needs-name = 字体需要名称
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = 无法使用字体 { $family }：{ $detail }
background-not-used = 无法使用背景图片：{ $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }：{ $detail }
asset-outside = 主题的文件必须位于其文件夹内
asset-needs-file = 请从主题文件导入主题，以便找到它的文件
asset-link = { $link } 是链接，链接不会被获取
not-an-image = 不是此应用可以显示的图片：{ $detail }
not-data-uri = 不是 data: URI
data-uri-comma = data: URI 的数据前需要有逗号
broken-escape = % 转义序列无效
# { $char } is a character, in quotes.
not-base64 = { $char } 不是 Base64 字符

## Shortcut editor.
shortcut-editor = 快捷键编辑器
shortcut-status = 快捷键状态
shortcuts-off = 快捷键自定义已关闭
shortcuts-off-hint = 此应用不允许更改快捷键。
search-commands = 搜索命令
commands = 命令
no-matching-command = 没有匹配的命令
no-shortcut = 无快捷键
reset-all-shortcuts = 重置所有快捷键
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = 为{ $command }添加快捷键
shortcut-remove-named = 从{ $command }中移除 { $chord }
reset-shortcuts-named = 重置{ $command }的快捷键
shortcut-now-runs = 现在 { $chord } 运行{ $command }
shortcut-already-runs = { $chord } 已用于运行{ $commands }
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord } 已用于运行{ $commands }。
shortcut-take = 将 { $chord } 用于{ $command }
shortcut-keep = 保留给{ $commands }
history-change-shortcuts = 更改{ $command }的快捷键
history-add-shortcut = 将 { $chord } 添加到{ $command }
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = 将 { $chord } 分配给{ $command }
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = 已省略，其余部分已应用：{ $parts }
shortcuts-clash-two = { $commands }这两个命令都会响应 { $chord }
shortcuts-clash-many = { $commands }都会响应 { $chord }

## Toolbar editor.
toolbar-editor = 工具栏编辑器
toolbar-status = 工具栏状态
toolbars-off = 工具栏自定义已关闭
toolbars-off-hint = 此应用不允许更改工具栏。
no-toolbars = 此应用没有可以更改的工具栏。
reset-all-toolbars = 重置所有工具栏
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = 不是工具栏文件：{ $detail }
toolbars-not-object = 工具栏文件是一个将每个位置映射到命令 ID 列表的对象
toolbar-slot-not-list = { $slot }：位置是命令 ID 的列表，例如 ["file.save"]
toolbar-ids-not-text = { $slot }：命令 ID 必须是文本

## Text editor.
text-editor = 文本编辑器
text-status = 文本状态
text-off = 文本自定义已关闭
text-off-hint = 此应用不允许更改文本。
# { $folder } is a folder's path.
text-folder = 你的消息文件位于 { $folder }
read-files-again = 重新读取文件
search-text = 搜索文本
strings = 字符串
no-matching-text = 没有匹配的文本
reset-all-text = 重置所有文本
could-not-change-text = 无法更改该文本：{ $error }
could-not-reset-text = 无法重置该文本：{ $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = 重置“{ $text }”
change-text = 更改“{ $text }”
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = 消息
search-messages = 搜索消息
no-matching-message = 没有匹配的消息
messages-hint = Fluent 消息：文本、{"{"} $name {"}"} 等占位符以及选项。
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = { $id } 检查
text-skipped = 已跳过：{ $detail }
text-part-unread = 文件的一部分无法读取，其余部分已使用：{ $detail }

## Layout editor.
layout-editor = 布局编辑器
layout-status = 布局状态
layouts-off = 布局自定义已关闭
layouts-off-hint = 此应用不允许重新排列其屏幕。
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = 你的布局位于 { $folder }
no-layouts = 没有打开可重新排列的屏幕。
reset-all-layouts = 重置所有布局
layout-named = 布局 { $file }
layout-check = 布局检查
layout-ok = 没有问题：可以保存布局。
layout-saved = 已保存。屏幕正在显示你的布局。
save = 保存
save-layout-named = 保存布局 { $file }
reset-layout-named = 重置布局 { $file }
no-screen = 未显示屏幕 { $file }
