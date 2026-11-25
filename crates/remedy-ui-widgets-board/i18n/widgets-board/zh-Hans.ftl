# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = 撤销
redo = 重做
add-area = 添加区域
area-heading = 区域
area-title = 区域标题
area-title-hint = 标题（按 Enter 应用）
area-place = 第 { $column } 列，第 { $row } 行，宽 { $columns }，高 { $rows }
area-keys-hint = 方向键移动区域；按住 Shift 再按方向键可调整大小。
area-keys = 方向键移动区域；按住 Shift 再按方向键可调整大小
remove-area = 移除区域
widget-heading = 小组件
settings = 设置
widget-keys-hint = Alt 加上下方向键移动小组件；Alt 加左右方向键将其移到下一个区域。
widget-keys = Alt 加方向键移动小组件
remove-widget = 移除小组件
nothing-selected = 选择一个区域或小组件以进行更改。
add-widget = 添加小组件
widget-kind = 小组件类型

## The grid.
widget-name = { $name } 小组件
unknown-kind = 此看板使用了本应用没有的小组件（{ $kind }）。
area-named = { $name } 区域
area-numbered = 区域 { $number }
empty-area = 空区域：请添加小组件，或将小组件拖放到此处。
empty-board = 空看板
empty-board-editing = 此看板为空：请添加一个区域。
empty-board-viewing = 这里还没有内容。
one-widget = 此区域已有一个小组件：请为另一个小组件单独分配一个区域。

## Why a change was refused.
error-invalid = 不是看板：{ $why }
error-too-new = 此看板由更新的版本创建（格式 { $format }）
error-outside = 这样会超出看板边缘
error-overlap = 这样会覆盖区域“{ $area }”
error-missing = { $id } 不在看板上

## Setting editors.
time-date = { $label }：日期
time-time = { $label }：时间
time-not-set = 未设置
time-clear = 清除
choice-unavailable = { $value }（不可用）
list-item = { $label } { $number }
list-add = 添加到 { $label }
list-remove = 移除 { $label } { $number }
picture-source = { $label }：地址
picture-choose = 选择图片

## Scripted widgets.
stopped = 此小组件已停止：{ $why }
start-again = 重新开始

## The built-in widgets.
text-name = 文本
text-title = 标题
text-title-default = 欢迎
text-body = 文本
text-body-default = 和家里的大家打个招呼吧。
links-name = 链接
links-title = 标题
links-title-default = 链接
links-links = 链接（标签 | https://…，每行一个）
links-none = 还没有链接。
reactions-name = 表情回应
reactions-question = 问题
reactions-question-default = 大家好吗？
reactions-offered = 可选的表情回应

## The reactions component.
reaction-group = 表情回应
reaction-one = 1 个回应
reaction-many = { $count } 个回应
reaction-count = { $count } 个回应
reaction-spoken = { $emoji }，{ $count }
reaction-spoken-mine = { $emoji }，{ $count }，包括您的
