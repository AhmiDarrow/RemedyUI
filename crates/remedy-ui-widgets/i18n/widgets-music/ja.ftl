# 音楽ウィジェット: スクリーンリーダーが読み上げる内容と、元に戻す履歴の名前。
list-separator = {"、"}
pitch = { $accidental ->
    [sharp] { $letter }シャープ { $octave }
    [flat] { $letter }フラット { $octave }
    [double-sharp] { $letter }ダブルシャープ { $octave }
    [double-flat] { $letter }ダブルフラット { $octave }
   *[natural] { $letter } { $octave }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] 付点全音符
            [half] 付点2分音符
            [quarter] 付点4分音符
            [eighth] 付点8分音符
            [sixteenth] 付点16分音符
           *[thirty-second] 付点32分音符
        }
    [double-dotted]
        { $value ->
            [whole] 複付点全音符
            [half] 複付点2分音符
            [quarter] 複付点4分音符
            [eighth] 複付点8分音符
            [sixteenth] 複付点16分音符
           *[thirty-second] 複付点32分音符
        }
   *[none]
        { $value ->
            [whole] 全音符
            [half] 2分音符
            [quarter] 4分音符
            [eighth] 8分音符
            [sixteenth] 16分音符
           *[thirty-second] 32分音符
        }
}
event-note = { $pitch }、{ $length }
event-note-tied = { $pitch }、{ $length }、次の音とタイ
event-chord = { $pitches } の和音、{ $length }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] 付点全休符
            [half] 付点2分休符
            [quarter] 付点4分休符
            [eighth] 付点8分休符
            [sixteenth] 付点16分休符
           *[thirty-second] 付点32分休符
        }
    [double-dotted]
        { $value ->
            [whole] 複付点全休符
            [half] 複付点2分休符
            [quarter] 複付点4分休符
            [eighth] 複付点8分休符
            [sixteenth] 複付点16分休符
           *[thirty-second] 複付点32分休符
        }
   *[none]
        { $value ->
            [whole] 全休符
            [half] 2分休符
            [quarter] 4分休符
            [eighth] 8分休符
            [sixteenth] 16分休符
           *[thirty-second] 32分休符
        }
}
key-no-accidentals = 調号なし
key-sharps = シャープ { $count } 個
key-flats = フラット { $count } 個
staff-summary = { $clef ->
    [bass] ヘ音記号
    [alto] アルト記号
    [tenor] テノール記号
   *[treble] ト音記号
}、{ $key }、{ $measures } 小節
staff-summary-time = { $clef ->
    [bass] ヘ音記号
    [alto] アルト記号
    [tenor] テノール記号
   *[treble] ト音記号
}、{ $key }、{ $beats }/{ $unit } 拍子、{ $measures } 小節
staff-editable = { $summary }。編集可能: 英字キーで音符を入力、上下矢印キーで移動、数字キーで長さを設定。
staff-cursor = { $measure } 小節目、{ $beat } 拍目: { $event }
cursor-picked = { $what }、選択済み
cursor-marked = { $what }、マークあり
cursor-picked-marked = { $what }、選択済み、マークあり
chord-string-muted = { $string } 弦 ミュート
chord-string-open = { $string } 弦 開放
chord-string-fret = { $string } 弦 { $fret } フレット
chord-string-fret-finger = { $string } 弦 { $fret } フレット 指 { $finger }
chord-barre = { $fret } フレットで { $from } 弦から { $to } 弦までセーハ
chord-barre-finger = 指 { $finger } で { $fret } フレットの { $from } 弦から { $to } 弦までセーハ
chord-starts-at = ダイアグラムは { $fret } フレットから
chord-named = { $label }: { $name }
position-open = { $string } 弦 ({ $tuning })、開放、{ $note }
position-fret = { $string } 弦 ({ $tuning })、{ $fret } フレット、{ $note }
position-open-short = { $string } 弦、開放
position-fret-short = { $string } 弦、{ $fret } フレット
fretboard-hint = 矢印キーでネック上を移動、Enter キーで選択、Esc キーで選択を解除。
fretboard-hint-picked = 矢印キーでネック上を移動、Enter キーで選択、Esc キーで選択を解除。選択中: { $picked }。
fretboard-summary = { $strings } 弦、{ $frets } フレット、マーク { $marked } 個
technique = { $technique ->
    [hammer-on] ハンマリング・オン
    [pull-off] プリング・オフ
    [slide-up] スライド・アップ
    [slide-down] スライド・ダウン
    [bend-half] 半音チョーキング
    [bend-whole] 全音チョーキング
    [vibrato] ビブラート
    [harmonic] ハーモニクス
    [palm-mute] ブリッジミュート
    [dead] デッドノート
   *[tied] タイ
}
technique-bend = { $steps } 音チョーキング
tab-note-fret = { $string } 弦: { $fret } フレット
tab-note-open = { $string } 弦: 開放
tab-note-dead = { $string } 弦: デッドノート
tab-no-note = { $string } 弦: 音符なし
tab-note-technique = { $note }、{ $technique }
tab-cursor = { $measure } 小節目、{ $beat } 拍目、{ $note }
tab-summary = タブ譜、{ $strings } 弦、{ $measures } 小節
tab-editable = { $summary }。編集可能: フレット番号を入力、Delete キーで音符を削除。
edit-set-note = 音符を設定
edit-add-to-chord = 和音に音符を追加
edit-octave-up = 音符を1オクターブ上げる
edit-octave-down = 音符を1オクターブ下げる
edit-semitone-up = 音符を半音上げる
edit-semitone-down = 音符を半音下げる
edit-step-up = 音符を上へ移動
edit-step-down = 音符を下へ移動
edit-delete-measure = 小節を削除
edit-delete-event = イベントを削除
edit-delete-rest = 休符を削除
edit-make-rest = 休符に変更
edit-insert-rest = 休符を挿入
edit-insert-measure = 小節を挿入
edit-set-length = 長さを設定
edit-sharpen = シャープを付ける
edit-flatten = フラットを付ける
edit-make-natural = ナチュラルにする
edit-cycle-dots = 付点を切り替え
edit-shorten = 短くする
edit-lengthen = 長くする
edit-tie = タイ
edit-set-fret = フレットを設定
edit-delete-note = 音符を削除
edit-add-note = 音符を追加
edit-raise-fret = フレットを上げる
edit-lower-fret = フレットを下げる
edit-set-technique = 奏法を設定
