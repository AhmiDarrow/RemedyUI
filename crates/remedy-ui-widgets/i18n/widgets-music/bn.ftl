# সংগীত উইজেট: স্ক্রিন রিডার যা পড়ে, এবং পূর্বাবস্থার ইতিহাসে নাম।
list-separator = {", "}
pitch = { $accidental ->
    [sharp] { $letter } শার্প { $octave }
    [flat] { $letter } ফ্ল্যাট { $octave }
    [double-sharp] { $letter } ডাবল শার্প { $octave }
    [double-flat] { $letter } ডাবল ফ্ল্যাট { $octave }
   *[natural] { $letter } { $octave }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] ডটেড হোল
            [half] ডটেড হাফ
            [quarter] ডটেড কোয়ার্টার
            [eighth] ডটেড এইটথ
            [sixteenth] ডটেড সিক্সটিনথ
           *[thirty-second] ডটেড থার্টি-সেকেন্ড
        }
    [double-dotted]
        { $value ->
            [whole] ডাবল ডটেড হোল
            [half] ডাবল ডটেড হাফ
            [quarter] ডাবল ডটেড কোয়ার্টার
            [eighth] ডাবল ডটেড এইটথ
            [sixteenth] ডাবল ডটেড সিক্সটিনথ
           *[thirty-second] ডাবল ডটেড থার্টি-সেকেন্ড
        }
   *[none]
        { $value ->
            [whole] হোল
            [half] হাফ
            [quarter] কোয়ার্টার
            [eighth] এইটথ
            [sixteenth] সিক্সটিনথ
           *[thirty-second] থার্টি-সেকেন্ড
        }
}
event-note = { $pitch } { $length } নোট
event-note-tied = { $pitch } { $length } নোট, পরের নোটের সাথে টাই করা
event-chord = { $pitches }-এর { $length } কর্ড
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] ডটেড হোল রেস্ট
            [half] ডটেড হাফ রেস্ট
            [quarter] ডটেড কোয়ার্টার রেস্ট
            [eighth] ডটেড এইটথ রেস্ট
            [sixteenth] ডটেড সিক্সটিনথ রেস্ট
           *[thirty-second] ডটেড থার্টি-সেকেন্ড রেস্ট
        }
    [double-dotted]
        { $value ->
            [whole] ডাবল ডটেড হোল রেস্ট
            [half] ডাবল ডটেড হাফ রেস্ট
            [quarter] ডাবল ডটেড কোয়ার্টার রেস্ট
            [eighth] ডাবল ডটেড এইটথ রেস্ট
            [sixteenth] ডাবল ডটেড সিক্সটিনথ রেস্ট
           *[thirty-second] ডাবল ডটেড থার্টি-সেকেন্ড রেস্ট
        }
   *[none]
        { $value ->
            [whole] হোল রেস্ট
            [half] হাফ রেস্ট
            [quarter] কোয়ার্টার রেস্ট
            [eighth] এইটথ রেস্ট
            [sixteenth] সিক্সটিনথ রেস্ট
           *[thirty-second] থার্টি-সেকেন্ড রেস্ট
        }
}
key-no-accidentals = কোনো শার্প বা ফ্ল্যাট নেই
key-sharps = { $count ->
    [one] { $count }টি শার্প
   *[other] { $count }টি শার্প
}
key-flats = { $count ->
    [one] { $count }টি ফ্ল্যাট
   *[other] { $count }টি ফ্ল্যাট
}
staff-summary = { $clef ->
    [bass] বেস ক্লেফ
    [alto] অল্টো ক্লেফ
    [tenor] টেনর ক্লেফ
   *[treble] ট্রেবল ক্লেফ
}, { $key }, { $measures ->
    [one] { $measures }টি বার
   *[other] { $measures }টি বার
}
staff-summary-time = { $clef ->
    [bass] বেস ক্লেফ
    [alto] অল্টো ক্লেফ
    [tenor] টেনর ক্লেফ
   *[treble] ট্রেবল ক্লেফ
}, { $key }, { $beats }/{ $unit } টাইম, { $measures ->
    [one] { $measures }টি বার
   *[other] { $measures }টি বার
}
staff-editable = { $summary }। সম্পাদনযোগ্য: অক্ষর দিয়ে নোট বসান, উপর ও নিচ তীর দিয়ে সরান, অঙ্ক দিয়ে দৈর্ঘ্য ঠিক করুন।
staff-cursor = বার { $measure }, বিট { $beat }: { $event }
cursor-picked = { $what }, নির্বাচিত
cursor-marked = { $what }, চিহ্নিত
cursor-picked-marked = { $what }, নির্বাচিত, চিহ্নিত
chord-string-muted = তার { $string } মিউট
chord-string-open = তার { $string } খোলা
chord-string-fret = তার { $string } ফ্রেট { $fret }
chord-string-fret-finger = তার { $string } ফ্রেট { $fret } আঙুল { $finger }
chord-barre = ফ্রেট { $fret }-এ তার { $from } থেকে { $to } পর্যন্ত ব্যারে
chord-barre-finger = ফ্রেট { $fret }-এ আঙুল { $finger } দিয়ে তার { $from } থেকে { $to } পর্যন্ত ব্যারে
chord-starts-at = ডায়াগ্রাম ফ্রেট { $fret } থেকে শুরু
chord-named = { $label }: { $name }
position-open = তার { $string } ({ $tuning }), খোলা, { $note }
position-fret = তার { $string } ({ $tuning }), ফ্রেট { $fret }, { $note }
position-open-short = তার { $string }, খোলা
position-fret-short = তার { $string }, ফ্রেট { $fret }
fretboard-hint = তীর কী দিয়ে নেকে সরান, Enter দিয়ে বেছে নিন, Esc দিয়ে নির্বাচন মুছুন।
fretboard-hint-picked = তীর কী দিয়ে নেকে সরান, Enter দিয়ে বেছে নিন, Esc দিয়ে নির্বাচন মুছুন। নির্বাচিত: { $picked }।
fretboard-summary = { $strings ->
    [one] { $strings }টি তার
   *[other] { $strings }টি তার
}, { $frets ->
    [one] { $frets }টি ফ্রেট
   *[other] { $frets }টি ফ্রেট
}, { $marked }টি চিহ্নিত
technique = { $technique ->
    [hammer-on] হ্যামার-অন
    [pull-off] পুল-অফ
    [slide-up] স্লাইড আপ
    [slide-down] স্লাইড ডাউন
    [bend-half] হাফ স্টেপ বেন্ড
    [bend-whole] হোল স্টেপ বেন্ড
    [vibrato] ভাইব্রেটো
    [harmonic] হারমোনিক
    [palm-mute] পাম মিউট
    [dead] ডেড নোট
   *[tied] টাই করা
}
technique-bend = { $steps ->
    [one] { $steps } স্টেপ বেন্ড
   *[other] { $steps } স্টেপ বেন্ড
}
tab-note-fret = তার { $string }: ফ্রেট { $fret }
tab-note-open = তার { $string }: খোলা
tab-note-dead = তার { $string }: ডেড নোট
tab-no-note = তার { $string }: কোনো নোট নেই
tab-note-technique = { $note }, { $technique }
tab-cursor = বার { $measure }, বিট { $beat }, { $note }
tab-summary = ট্যাবলেচার, { $strings ->
    [one] { $strings }টি তার
   *[other] { $strings }টি তার
}, { $measures ->
    [one] { $measures }টি বার
   *[other] { $measures }টি বার
}
tab-editable = { $summary }। সম্পাদনযোগ্য: ফ্রেট নম্বর টাইপ করুন, Delete নোট সরায়।
edit-set-note = নোট বসান
edit-add-to-chord = কর্ডে নোট যোগ করুন
edit-octave-up = নোট এক অক্টেভ উপরে নিন
edit-octave-down = নোট এক অক্টেভ নিচে নিন
edit-semitone-up = নোট এক সেমিটোন বাড়ান
edit-semitone-down = নোট এক সেমিটোন কমান
edit-step-up = নোট উপরে সরান
edit-step-down = নোট নিচে সরান
edit-delete-measure = বার মুছুন
edit-delete-event = ইভেন্ট মুছুন
edit-delete-rest = রেস্ট মুছুন
edit-make-rest = রেস্টে বদলান
edit-insert-rest = রেস্ট ঢোকান
edit-insert-measure = বার ঢোকান
edit-set-length = দৈর্ঘ্য ঠিক করুন
edit-sharpen = শার্প দিন
edit-flatten = ফ্ল্যাট দিন
edit-make-natural = ন্যাচারাল করুন
edit-cycle-dots = ডট বদলান
edit-shorten = ছোট করুন
edit-lengthen = লম্বা করুন
edit-tie = টাই করুন
edit-set-fret = ফ্রেট ঠিক করুন
edit-delete-note = নোট মুছুন
edit-add-note = নোট যোগ করুন
edit-raise-fret = ফ্রেট বাড়ান
edit-lower-fret = ফ্রেট কমান
edit-set-technique = কৌশল ঠিক করুন
