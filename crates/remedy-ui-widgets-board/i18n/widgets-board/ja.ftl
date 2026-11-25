# The board's own words (see en.ftl for what each message is). Placeholders ({ $name })
# must stay; their order may change.

## The editing panel.
undo = 元に戻す
redo = やり直す
add-area = エリアを追加
area-heading = エリア
area-title = エリアのタイトル
area-title-hint = タイトル（Enter で適用）
area-place = 列 { $column }、行 { $row }、幅 { $columns }、高さ { $rows }
area-keys-hint = 矢印キーで移動、Shift と矢印キーでサイズを変更します。
area-keys = 矢印キーで移動、Shift と矢印キーでサイズを変更します
remove-area = エリアを削除
widget-heading = ウィジェット
settings = 設定
widget-keys-hint = Alt と上下キーで移動、Alt と左右キーで隣のエリアに移動します。
widget-keys = Alt と矢印キーで移動します
remove-widget = ウィジェットを削除
nothing-selected = 変更するエリアまたはウィジェットを選択してください。
add-widget = ウィジェットを追加
widget-kind = ウィジェットの種類

## The grid.
widget-name = { $name } ウィジェット
unknown-kind = このボードには、このアプリにないウィジェット（{ $kind }）が使われています。
area-named = { $name } エリア
area-numbered = エリア { $number }
empty-area = 空のエリア：ウィジェットを追加するか、ここにドロップしてください。
empty-board = 空のボード
empty-board-editing = このボードは空です。エリアを追加してください。
empty-board-viewing = まだ何もありません。
one-widget = このエリアにはすでにウィジェットがあります。もう一方には別のエリアを用意してください。

## Why a change was refused.
error-invalid = ボードではありません：{ $why }
error-too-new = このボードは新しいバージョンで作成されています（形式 { $format }）
error-outside = ボードの端からはみ出します
error-overlap = エリア「{ $area }」に重なります
error-missing = { $id } はボード上にありません

## Setting editors.
time-date = { $label }：日付
time-time = { $label }：時刻
time-not-set = 未設定
time-clear = クリア
choice-unavailable = { $value }（利用できません）
list-item = { $label } { $number }
list-add = { $label } に追加
list-remove = { $label } { $number } を削除
picture-source = { $label }：アドレス
picture-choose = 画像を選択

## Scripted widgets.
stopped = このウィジェットは停止しました：{ $why }
start-again = 再開

## The built-in widgets.
text-name = テキスト
text-title = タイトル
text-title-default = ようこそ
text-body = テキスト
text-body-default = 家のみんなにあいさつしましょう。
links-name = リンク
links-title = タイトル
links-title-default = リンク
links-links = リンク（ラベル | https://…、1 行に 1 つ）
links-none = リンクはまだありません。
reactions-name = リアクション
reactions-question = 質問
reactions-question-default = みなさん、調子はどうですか？
reactions-offered = 選べるリアクション

## The reactions component.
reaction-group = リアクション
reaction-one = リアクション 1 件
reaction-many = リアクション { $count } 件
reaction-count = リアクション { $count } 件
reaction-spoken = { $emoji }、{ $count }
reaction-spoken-mine = { $emoji }、{ $count }（あなたのものを含む）
