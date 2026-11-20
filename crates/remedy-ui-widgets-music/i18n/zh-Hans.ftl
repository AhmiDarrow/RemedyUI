# 音乐控件：屏幕阅读器朗读的内容，以及撤销历史中的名称。
list-separator = {"、"}
pitch = { $accidental ->
    [sharp] 升{ $letter } { $octave }
    [flat] 降{ $letter } { $octave }
    [double-sharp] 重升{ $letter } { $octave }
    [double-flat] 重降{ $letter } { $octave }
   *[natural] { $letter } { $octave }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] 附点全音符
            [half] 附点二分音符
            [quarter] 附点四分音符
            [eighth] 附点八分音符
            [sixteenth] 附点十六分音符
           *[thirty-second] 附点三十二分音符
        }
    [double-dotted]
        { $value ->
            [whole] 复附点全音符
            [half] 复附点二分音符
            [quarter] 复附点四分音符
            [eighth] 复附点八分音符
            [sixteenth] 复附点十六分音符
           *[thirty-second] 复附点三十二分音符
        }
   *[none]
        { $value ->
            [whole] 全音符
            [half] 二分音符
            [quarter] 四分音符
            [eighth] 八分音符
            [sixteenth] 十六分音符
           *[thirty-second] 三十二分音符
        }
}
event-note = { $pitch }，{ $length }
event-note-tied = { $pitch }，{ $length }，延音连至下一音符
event-chord = { $pitches } 和弦，{ $length }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] 附点全休止符
            [half] 附点二分休止符
            [quarter] 附点四分休止符
            [eighth] 附点八分休止符
            [sixteenth] 附点十六分休止符
           *[thirty-second] 附点三十二分休止符
        }
    [double-dotted]
        { $value ->
            [whole] 复附点全休止符
            [half] 复附点二分休止符
            [quarter] 复附点四分休止符
            [eighth] 复附点八分休止符
            [sixteenth] 复附点十六分休止符
           *[thirty-second] 复附点三十二分休止符
        }
   *[none]
        { $value ->
            [whole] 全休止符
            [half] 二分休止符
            [quarter] 四分休止符
            [eighth] 八分休止符
            [sixteenth] 十六分休止符
           *[thirty-second] 三十二分休止符
        }
}
key-no-accidentals = 无升降号
key-sharps = { $count } 个升号
key-flats = { $count } 个降号
staff-summary = { $clef ->
    [bass] 低音谱号
    [alto] 中音谱号
    [tenor] 次中音谱号
   *[treble] 高音谱号
}，{ $key }，{ $measures } 个小节
staff-summary-time = { $clef ->
    [bass] 低音谱号
    [alto] 中音谱号
    [tenor] 次中音谱号
   *[treble] 高音谱号
}，{ $key }，{ $beats }/{ $unit } 拍，{ $measures } 个小节
staff-editable = { $summary }。可编辑：字母键设置音符，上下方向键移动音符，数字键设置时值。
staff-cursor = 第 { $measure } 小节，第 { $beat } 拍：{ $event }
cursor-picked = { $what }，已选定
cursor-marked = { $what }，已标记
cursor-picked-marked = { $what }，已选定，已标记
chord-string-muted = 第 { $string } 弦不弹
chord-string-open = 第 { $string } 弦空弦
chord-string-fret = 第 { $string } 弦第 { $fret } 品
chord-string-fret-finger = 第 { $string } 弦第 { $fret } 品（手指 { $finger }）
chord-barre = 在第 { $fret } 品横按第 { $from } 至第 { $to } 弦
chord-barre-finger = 在第 { $fret } 品用手指 { $finger } 横按第 { $from } 至第 { $to } 弦
chord-starts-at = 和弦图从第 { $fret } 品开始
chord-named = { $label }：{ $name }
position-open = 第 { $string } 弦（{ $tuning }），空弦，{ $note }
position-fret = 第 { $string } 弦（{ $tuning }），第 { $fret } 品，{ $note }
position-open-short = 第 { $string } 弦，空弦
position-fret-short = 第 { $string } 弦，第 { $fret } 品
fretboard-hint = 方向键在指板上移动，Enter 键选定，Esc 键清除选定。
fretboard-hint-picked = 方向键在指板上移动，Enter 键选定，Esc 键清除选定。已选定：{ $picked }。
fretboard-summary = { $strings } 根弦，{ $frets } 个品，{ $marked } 处已标记
technique = { $technique ->
    [hammer-on] 击弦
    [pull-off] 勾弦
    [slide-up] 上滑音
    [slide-down] 下滑音
    [bend-half] 推弦半音
    [bend-whole] 推弦全音
    [vibrato] 揉弦
    [harmonic] 泛音
    [palm-mute] 掌闷音
    [dead] 闷音
   *[tied] 延音
}
technique-bend = 推弦 { $steps } 个全音
tab-note-fret = 第 { $string } 弦：第 { $fret } 品
tab-note-open = 第 { $string } 弦：空弦
tab-note-dead = 第 { $string } 弦：闷音
tab-no-note = 第 { $string } 弦：无音符
tab-note-technique = { $note }，{ $technique }
tab-cursor = 第 { $measure } 小节，第 { $beat } 拍，{ $note }
tab-summary = TAB 谱，{ $strings } 根弦，{ $measures } 个小节
tab-editable = { $summary }。可编辑：输入品位数字，按 Delete 键删除音符。
edit-set-note = 设置音符
edit-add-to-chord = 向和弦添加音符
edit-octave-up = 音符升高八度
edit-octave-down = 音符降低八度
edit-semitone-up = 音符升高半音
edit-semitone-down = 音符降低半音
edit-step-up = 音符上移
edit-step-down = 音符下移
edit-delete-measure = 删除小节
edit-delete-event = 删除事件
edit-delete-rest = 删除休止符
edit-make-rest = 改为休止符
edit-insert-rest = 插入休止符
edit-insert-measure = 插入小节
edit-set-length = 设置时值
edit-sharpen = 加升号
edit-flatten = 加降号
edit-make-natural = 加还原号
edit-cycle-dots = 切换附点
edit-shorten = 缩短
edit-lengthen = 延长
edit-tie = 加延音线
edit-set-fret = 设置品位
edit-delete-note = 删除音符
edit-add-note = 添加音符
edit-raise-fret = 升高品位
edit-lower-fret = 降低品位
edit-set-technique = 设置技巧
