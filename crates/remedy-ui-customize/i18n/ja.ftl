# The Customize page's own words: the theme, shortcut, toolbar, text and layout editors, their
# notices and their undo history. Placeholders ({ $name }) must stay; their order may change.
# File names, paths, command ids, shortcuts (Ctrl+S) and technical details ({ $detail }) are
# shown as they are.

## Lists, built from the end: "a, b and c" is list-middle(a, list-end(b, c)).
list-two = { $first }と{ $second }
list-middle = { $first }、{ $rest }
list-end = { $first }と{ $last }
# Joins whole sentences about separate things ("Ctrl+S is used by Save; F3 is used by Find").
clauses = { $first }。{ $rest }

## The kinds of customization: tab names and editor headings.
area-theme = テーマ
area-shortcuts = キーボードショートカット
area-toolbars = ツールバーとメニュー
area-text = テキスト
area-layouts = 画面レイアウト

## The settings page.
customize = カスタマイズ
customize-status = カスタマイズの状態
nothing-to-customize = カスタマイズできる項目はありません
nothing-to-customize-hint = このアプリではカスタマイズが有効になっていません。
reset-everything = すべてリセット
reset-everything-done = すべてアプリの既定値に戻しました。元に戻すと復元できます。
# Also the undo history's name for resetting everything.
reset-all-customizations = すべてのカスタマイズをリセット

## Buttons and status lines every editor shares.
# The visible text of every reset button; screen readers hear what it resets.
reset = リセット
# A reset button's accessible name, and the undo history's: { $name } is what it resets
# ("Body text size", "Main toolbar", a file name, a message id).
reset-named = { $name }をリセット
# The undo history's name for changing one named thing ("Change Accent").
change-named = { $name }を変更
# { $error } says why.
could-not-reset = リセットできませんでした: { $error }
not-saved = 保存されていません: { $error }
could-not-change = 変更できませんでした: { $error }

## Notices: a customization file that could not be used.
# { $area } is the kind ("Theme"), { $file } the file name, { $message } what went wrong.
notice-row = { $area }を使用できませんでした（{ $file }）: { $message }
notice-dismiss = 閉じる
# The dismiss button's accessible name.
notice-dismiss-named = { $file }に関するお知らせを閉じる
notice-not-utf8 = ファイルが UTF-8 テキストではありません
notice-could-not-save = 保存できませんでした: { $error }
notice-could-not-restore = 復元できませんでした: { $detail }

## Errors.
error-not-enabled-theme = このアプリではテーマのカスタマイズが有効になっていません
error-not-enabled-shortcuts = このアプリではショートカットのカスタマイズが有効になっていません
error-not-enabled-toolbars = このアプリではツールバーのカスタマイズが有効になっていません
error-not-enabled-text = このアプリではテキストのカスタマイズが有効になっていません
error-not-enabled-layouts = このアプリではレイアウトのカスタマイズが有効になっていません
# { $path } is a file, { $detail } what the system said.
error-io = { $path }: { $detail }
# { $chord } is a shortcut, { $commands } the commands that use it.
error-clash = { $chord }は{ $commands }で使用されています
error-refused = 拒否されました: { $parts }
error-unknown-command = コマンド { $id } はありません
error-unknown-slot = ツールバー { $id } はありません
# { $token } is a design token's path, such as color.accent.
error-wrong-kind = { $token }はその種類の値を保持できません

## Theme editor.
theme-editor = テーマエディター
theme-status = テーマの状態
theme-off = テーマのカスタマイズはオフです
theme-off-hint = このアプリではテーマを変更できません。
section-colors = 色
# The section of fonts and text sizes.
section-text = 文字
section-shape = 間隔と形状
section-motion = アニメーション
reset-theme = テーマをリセット
# The button that opens a color's picker; its accessible name is edit-named.
edit = 編集
edit-named = { $name }を編集
# Units after a number.
unit-px = px
unit-ms = ミリ秒

## Design tokens, as people call them.
token-theme-name = テーマ名
token-color-bg = 背景
token-color-panel = パネル
token-color-card = カード
token-color-text = テキスト
token-color-muted = 控えめなテキスト
token-color-accent = アクセント
token-color-on-accent = アクセント上のテキスト
token-color-danger = 危険
token-color-warn = 警告
token-color-ok = 成功
token-color-line = 線
# A color the app defines; { $name } is its name in the theme file.
token-color-custom = 色「{ $name }」
token-font = フォント
token-font-mono = 等幅フォント
token-size-small = 小さい文字のサイズ
token-size-body = 本文のサイズ
token-size-title = タイトルのサイズ
token-spacing-unit = 間隔の単位
token-control-height = コントロールの高さ
token-radius-small = 角の半径（小）
token-radius-medium = 角の半径（中）
token-radius-large = 角の半径（大）
token-stroke = 線の太さ
token-motion-quick = 速いアニメーション
token-motion-standard = 標準のアニメーション
token-motion-emphasized = 強調したアニメーション
token-density = 密度
token-background-image = 背景画像
token-font-file = フォントファイル「{ $name }」

## How tightly controls are packed.
density-compact = コンパクト
density-comfortable = 標準
density-spacious = ゆったり

## Contrast (WCAG is the Web Content Accessibility Guidelines).
contrast = コントラスト
contrast-all-pass = すべての色の組み合わせが WCAG のコントラスト基準を満たしています。
# The contrast status read by screen readers.
contrast-status-all-pass = すべての色の組み合わせが WCAG のコントラスト基準を満たしています
contrast-status-failing = { $count ->
   *[other] { $count } 組の色が WCAG のコントラスト基準を下回っています
}
# { $pair } is a pair below (contrast-pair-…), { $ratio } and { $needed } contrast ratios
# ("2.1:1", "4.5:1"), { $foreground } and { $background } the two colors' names.
contrast-warning = { $pair }: { NUMBER($ratio, minimumFractionDigits: 1) }:1（必要な { NUMBER($needed, minimumFractionDigits: 1) }:1 を下回っています。{ $background }上の{ $foreground }）
contrast-pair-text-bg = 背景上のテキスト
contrast-pair-text-panel = パネル上のテキスト
contrast-pair-text-card = カード上のテキスト
contrast-pair-muted-bg = 背景上の控えめなテキスト
contrast-pair-muted-panel = パネル上の控えめなテキスト
contrast-pair-muted-card = カード上の控えめなテキスト
contrast-pair-on-accent = アクセント上のテキスト
contrast-pair-accent-bg = 背景上のアクセント

## The preview: sample widgets in the theme being edited.
preview = プレビュー
preview-group = テーマのプレビュー
preview-field = サンプルのフィールド
preview-field-text = フィールド内のテキスト
preview-body = 本文は標準のサイズでこのように表示されます。
preview-muted = ヒントや詳細に使う控えめなテキストです。
preview-primary = プライマリ
preview-secondary = セカンダリ
preview-danger = 危険
preview-switch = スイッチ
preview-progress = 進行状況
preview-link = リンク
preview-error = エラーメッセージです。

## Sharing themes.
section-share = 共有
share-hint = テーマは、どの Remedy アプリでも読み込めるデザイントークン（design tokens）ファイルです。
theme-file = テーマファイル
theme-file-placeholder = themes/ocean.json などのパス
import-from-path = パスからインポート
export-to-path = パスにエクスポート
import-path-first = 先にテーマファイルのパスを入力してください
export-path-first = 先にテーマの保存先を入力してください
choose-theme = インポートするテーマを選択…
export-to-file = ファイルにエクスポート…
# File dialog titles and the name of their file type filter.
dialog-import-theme = テーマのインポート
dialog-export-theme = テーマのエクスポート
filter-design-tokens = デザイントークン
# The suggested file name of an exported theme without a name (".json" is added).
export-default-name = テーマ
imported = テーマ「{ $name }」をインポートしました
# { $count } parts of the theme (shaders, links, files) were not imported.
imported-leaving-out = { $count ->
   *[other] テーマ「{ $name }」をインポートしました（{ $count } 個の項目を除外）
}
could-not-import = このテーマをインポートできませんでした: { $error }
could-not-read = { $path } を読み込めませんでした: { $detail }
exported = テーマを { $path } にエクスポートしました
could-not-export = テーマをエクスポートできませんでした: { $error }
# The question shown when a shared theme brings shaders, links or files.
theme-carries-more = このテーマには色やサイズ以外のものが含まれています
theme-carries-more-detail = このテーマには色やサイズ以外のものが含まれています。次の項目は、オンにしない限りオフのままです:
allow-shaders = シェーダーを許可する（グラフィックスカードで実行されるコード）
allow-links = 外部リンクを許可する（保持されますが、取得はされません）
allow-files = テーマに含まれる画像とフォントを使用する
import = インポート
import-without = それらを除いてインポート
cancel = キャンセル
# What a shared theme carries: { $token } is where in the file, { $detail } what it holds.
refusal-shader = { $token }: シェーダー（{ $detail }）
refusal-link = { $token }: 外部リンク（{ $detail }）
refusal-file = { $token }: テーマに含まれるファイル（{ $detail }）
theme-not-object = テーマファイルは JSON オブジェクトである必要があります
# { $detail } is the JSON reader's message.
not-design-tokens = デザイントークンファイルではありません: { $detail }
history-import-theme = テーマ「{ $name }」をインポート

## The theme's files: its background image and fonts.
section-files = ファイル
files-hint = 画像は各ウィンドウの背後にウィンドウの大きさで表示されます。フォントは使用前にチェックされます。
background-inline = 背景: テーマに埋め込まれた画像
# { $file } is the image's path or link.
background-file = 背景: { $file }
no-background = 背景画像なし
background-file-field = 背景画像ファイル
background-placeholder = pictures/waves.png などのパス
use-image = この画像を使用
image-path-first = 先に画像のパスを入力してください
reset-background = 背景画像をリセット
choose-image = 画像を選択…
dialog-choose-image = 背景画像の選択
filter-images = 画像
no-fonts = このテーマにはフォントが含まれていません。
remove = 削除
remove-font-named = フォント { $family } を削除
font-file = フォントファイル
font-file-placeholder = fonts/Ocean-Regular.ttf などのパス
font-name = フォント名
font-name-placeholder = フォントの名前（例: Ocean）
add-font = フォントを追加
font-first = 先にフォントファイルのパスと名前を入力してください
history-change-background = 背景画像を変更
history-remove-background = 背景画像を削除
history-add-font = フォント { $family } を追加
history-remove-font = フォント { $family } を削除
font-needs-name = フォントには名前が必要です
# A theme file names something that cannot be used; { $detail } says why.
font-not-used = フォント { $family } を使用できませんでした: { $detail }
background-not-used = 背景画像を使用できませんでした: { $detail }
# { $path } is where in the theme file, { $detail } what is wrong.
asset-problem = { $path }: { $detail }
asset-outside = テーマのファイルはテーマのフォルダー内にある必要があります
asset-needs-file = ファイルが見つかるよう、テーマはそのファイルからインポートしてください
asset-link = { $link } はリンクです。リンクは取得されません
not-an-image = このアプリで表示できる画像ではありません: { $detail }
not-data-uri = data: URI ではありません
data-uri-comma = data: URI ではデータの前にカンマが必要です
broken-escape = % エスケープが正しくありません
# { $char } is a character, in quotes.
not-base64 = { $char } は Base64 の文字ではありません

## Shortcut editor.
shortcut-editor = ショートカットエディター
shortcut-status = ショートカットの状態
shortcuts-off = ショートカットのカスタマイズはオフです
shortcuts-off-hint = このアプリではショートカットを変更できません。
search-commands = コマンドを検索
commands = コマンド
no-matching-command = 一致するコマンドはありません
no-shortcut = ショートカットなし
reset-all-shortcuts = すべてのショートカットをリセット
# { $chord } is a shortcut such as Ctrl+S, { $command } a command's name, { $commands } a list.
shortcut-add-named = { $command }にショートカットを追加
shortcut-remove-named = { $command }から{ $chord }を削除
reset-shortcuts-named = { $command }のショートカットをリセット
shortcut-now-runs = { $chord }で{ $command }を実行するようになりました
shortcut-already-runs = { $chord }はすでに{ $commands }を実行します
# The question shown when a recorded shortcut is another command's.
shortcut-clash = { $chord }はすでに{ $commands }を実行します。
shortcut-take = { $chord }を{ $command }に使用
shortcut-keep = { $commands }のままにする
history-change-shortcuts = { $command }のショートカットを変更
history-add-shortcut = { $command }に{ $chord }を追加
# Adding a shortcut another command had: that command loses it.
history-give-shortcut = { $chord }を{ $command }に割り当て
# A shortcuts file gives a shortcut to commands that would clash: { $parts } says which.
shortcuts-left-out = 除外しました（残りは適用されます）: { $parts }
shortcuts-clash-two = { $commands }の両方が{ $chord }に反応してしまいます
shortcuts-clash-many = { $commands }のすべてが{ $chord }に反応してしまいます

## Toolbar editor.
toolbar-editor = ツールバーエディター
toolbar-status = ツールバーの状態
toolbars-off = ツールバーのカスタマイズはオフです
toolbars-off-hint = このアプリではツールバーを変更できません。
no-toolbars = このアプリには変更できるツールバーがありません。
reset-all-toolbars = すべてのツールバーをリセット
# { $detail } is the JSON reader's message; { $slot } a toolbar's id.
not-toolbars-file = ツールバーファイルではありません: { $detail }
toolbars-not-object = ツールバーファイルは、各スロットをコマンド ID のリストに対応付けるオブジェクトです
toolbar-slot-not-list = { $slot }: スロットは ["file.save"] のようなコマンド ID のリストです
toolbar-ids-not-text = { $slot }: コマンド ID はテキストである必要があります

## Text editor.
text-editor = テキストエディター
text-status = テキストの状態
text-off = テキストのカスタマイズはオフです
text-off-hint = このアプリではテキストを変更できません。
# { $folder } is a folder's path.
text-folder = メッセージファイルは { $folder } にあります
read-files-again = ファイルを再読み込み
search-text = テキストを検索
strings = 文字列
no-matching-text = 一致するテキストはありません
reset-all-text = すべてのテキストをリセット
could-not-change-text = このテキストを変更できませんでした: { $error }
could-not-reset-text = このテキストをリセットできませんでした: { $error }
# A text of the app, quoted: { $text } is the app's own text.
reset-text = 「{ $text }」をリセット
change-text = 「{ $text }」を変更
# Fluent messages (Fluent is the message format) the person can rewrite.
messages = メッセージ
search-messages = メッセージを検索
no-matching-message = 一致するメッセージはありません
messages-hint = Fluent メッセージ: テキスト、{"{"} $name {"}"} などのプレースホルダー、選択肢。
# The status that says whether a rewritten message reads; { $id } is the message's id.
message-check = { $id } のチェック
text-skipped = スキップしました: { $detail }
text-part-unread = ファイルの一部を読み込めませんでした（残りは使用されます）: { $detail }

## Layout editor.
layout-editor = レイアウトエディター
layout-status = レイアウトの状態
layouts-off = レイアウトのカスタマイズはオフです
layouts-off-hint = このアプリでは画面を並べ替えられません。
# { $folder } is a folder's path; { $file } a screen's file name, such as settings.rui.
layouts-folder = レイアウトは { $folder } にあります
no-layouts = 並べ替えられる画面は開いていません。
reset-all-layouts = すべてのレイアウトをリセット
layout-named = レイアウト { $file }
layout-check = レイアウトのチェック
layout-ok = 問題はありません。レイアウトを保存できます。
layout-saved = 保存しました。画面にあなたのレイアウトが表示されています。
save = 保存
save-layout-named = レイアウト { $file } を保存
reset-layout-named = レイアウト { $file } をリセット
no-screen = 画面 { $file } は表示されていません
