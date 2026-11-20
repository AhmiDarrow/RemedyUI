# أدوات الموسيقى: ما تقرؤه قارئات الشاشة، وأسماء التعديلات في سجل التراجع.
list-separator = {"، "}
pitch = { $accidental ->
    [sharp]
        { $letter ->
            [C] دو دييز { $octave }
            [D] ري دييز { $octave }
            [E] مي دييز { $octave }
            [F] فا دييز { $octave }
            [G] صول دييز { $octave }
            [A] لا دييز { $octave }
           *[B] سي دييز { $octave }
        }
    [flat]
        { $letter ->
            [C] دو بيمول { $octave }
            [D] ري بيمول { $octave }
            [E] مي بيمول { $octave }
            [F] فا بيمول { $octave }
            [G] صول بيمول { $octave }
            [A] لا بيمول { $octave }
           *[B] سي بيمول { $octave }
        }
    [double-sharp]
        { $letter ->
            [C] دو دوبل دييز { $octave }
            [D] ري دوبل دييز { $octave }
            [E] مي دوبل دييز { $octave }
            [F] فا دوبل دييز { $octave }
            [G] صول دوبل دييز { $octave }
            [A] لا دوبل دييز { $octave }
           *[B] سي دوبل دييز { $octave }
        }
    [double-flat]
        { $letter ->
            [C] دو دوبل بيمول { $octave }
            [D] ري دوبل بيمول { $octave }
            [E] مي دوبل بيمول { $octave }
            [F] فا دوبل بيمول { $octave }
            [G] صول دوبل بيمول { $octave }
            [A] لا دوبل بيمول { $octave }
           *[B] سي دوبل بيمول { $octave }
        }
   *[natural]
        { $letter ->
            [C] دو { $octave }
            [D] ري { $octave }
            [E] مي { $octave }
            [F] فا { $octave }
            [G] صول { $octave }
            [A] لا { $octave }
           *[B] سي { $octave }
        }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] مستديرة منقوطة
            [half] بيضاء منقوطة
            [quarter] سوداء منقوطة
            [eighth] ذات سن منقوطة
            [sixteenth] ذات سنّين منقوطة
           *[thirty-second] ذات ثلاث سنون منقوطة
        }
    [double-dotted]
        { $value ->
            [whole] مستديرة بنقطتين
            [half] بيضاء بنقطتين
            [quarter] سوداء بنقطتين
            [eighth] ذات سن بنقطتين
            [sixteenth] ذات سنّين بنقطتين
           *[thirty-second] ذات ثلاث سنون بنقطتين
        }
   *[none]
        { $value ->
            [whole] مستديرة
            [half] بيضاء
            [quarter] سوداء
            [eighth] ذات سن
            [sixteenth] ذات سنّين
           *[thirty-second] ذات ثلاث سنون
        }
}
event-note = { $pitch }، { $length }
event-note-tied = { $pitch }، { $length }، مربوطة بالتالية
event-chord = تآلف ({ $length }) من { $pitches }
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] سكتة مستديرة منقوطة
            [half] سكتة بيضاء منقوطة
            [quarter] سكتة سوداء منقوطة
            [eighth] سكتة ذات سن منقوطة
            [sixteenth] سكتة ذات سنّين منقوطة
           *[thirty-second] سكتة ذات ثلاث سنون منقوطة
        }
    [double-dotted]
        { $value ->
            [whole] سكتة مستديرة بنقطتين
            [half] سكتة بيضاء بنقطتين
            [quarter] سكتة سوداء بنقطتين
            [eighth] سكتة ذات سن بنقطتين
            [sixteenth] سكتة ذات سنّين بنقطتين
           *[thirty-second] سكتة ذات ثلاث سنون بنقطتين
        }
   *[none]
        { $value ->
            [whole] سكتة مستديرة
            [half] سكتة بيضاء
            [quarter] سكتة سوداء
            [eighth] سكتة ذات سن
            [sixteenth] سكتة ذات سنّين
           *[thirty-second] سكتة ذات ثلاث سنون
        }
}
key-no-accidentals = بلا علامات تحويل
key-sharps = { $count ->
    [zero] { $count } دييز
    [one] دييز واحد
    [two] دييزان
    [few] { $count } دييزات
    [many] { $count } دييزًا
   *[other] { $count } دييز
}
key-flats = { $count ->
    [zero] { $count } بيمول
    [one] بيمول واحد
    [two] بيمولان
    [few] { $count } بيمولات
    [many] { $count } بيمولًا
   *[other] { $count } بيمول
}
staff-summary = { $clef ->
    [bass] مفتاح فا
    [alto] مفتاح دو على الخط الثالث
    [tenor] مفتاح دو على الخط الرابع
   *[treble] مفتاح صول
}، { $key }، { $measures ->
    [zero] لا مازورات
    [one] مازورة واحدة
    [two] مازورتان
    [few] { $measures } مازورات
    [many] { $measures } مازورة
   *[other] { $measures } مازورة
}
staff-summary-time = { $clef ->
    [bass] مفتاح فا
    [alto] مفتاح دو على الخط الثالث
    [tenor] مفتاح دو على الخط الرابع
   *[treble] مفتاح صول
}، { $key }، ميزان { $beats }/{ $unit }، { $measures ->
    [zero] لا مازورات
    [one] مازورة واحدة
    [two] مازورتان
    [few] { $measures } مازورات
    [many] { $measures } مازورة
   *[other] { $measures } مازورة
}
staff-editable = { $summary }. قابل للتحرير: الحروف تضع النوتات، والسهمان لأعلى ولأسفل يحرّكانها، والأرقام تحدد المدد.
staff-cursor = المازورة { $measure }، الضربة { $beat }: { $event }
cursor-picked = { $what }، محدد
cursor-marked = { $what }، معلَّم
cursor-picked-marked = { $what }، محدد، معلَّم
chord-string-muted = الوتر { $string } مكتوم
chord-string-open = الوتر { $string } مطلق
chord-string-fret = الوتر { $string } العتبة { $fret }
chord-string-fret-finger = الوتر { $string } العتبة { $fret } الإصبع { $finger }
chord-barre = باريه على العتبة { $fret } عبر الأوتار من { $from } إلى { $to }
chord-barre-finger = باريه على العتبة { $fret } عبر الأوتار من { $from } إلى { $to } بالإصبع { $finger }
chord-starts-at = يبدأ المخطط من العتبة { $fret }
chord-named = { $label }: { $name }
position-open = الوتر { $string } ({ $tuning })، مطلق، { $note }
position-fret = الوتر { $string } ({ $tuning })، العتبة { $fret }، { $note }
position-open-short = الوتر { $string }، مطلق
position-fret-short = الوتر { $string }، العتبة { $fret }
fretboard-hint = تتنقل مفاتيح الأسهم على زند الآلة، ويختار Enter، ويمسح Esc الاختيار.
fretboard-hint-picked = تتنقل مفاتيح الأسهم على زند الآلة، ويختار Enter، ويمسح Esc الاختيار. المحدد: { $picked }.
fretboard-summary = { $strings ->
    [zero] لا أوتار
    [one] وتر واحد
    [two] وتران
    [few] { $strings } أوتار
    [many] { $strings } وترًا
   *[other] { $strings } وتر
}، { $frets ->
    [zero] لا عتبات
    [one] عتبة واحدة
    [two] عتبتان
    [few] { $frets } عتبات
    [many] { $frets } عتبة
   *[other] { $frets } عتبة
}، { $marked } معلَّمة
technique = { $technique ->
    [hammer-on] هامر أون
    [pull-off] بول أوف
    [slide-up] انزلاق صاعد
    [slide-down] انزلاق هابط
    [bend-half] ثني نصف درجة
    [bend-whole] ثني درجة كاملة
    [vibrato] اهتزاز
    [harmonic] هارمونيك
    [palm-mute] كتم بكف اليد
    [dead] نغمة مكتومة
   *[tied] مربوطة
}
technique-bend = { $steps ->
    [zero] ثني { $steps } درجة
    [one] ثني درجة واحدة
    [two] ثني درجتين
    [few] ثني { $steps } درجات
    [many] ثني { $steps } درجة
   *[other] ثني { $steps } درجة
}
tab-note-fret = الوتر { $string }: العتبة { $fret }
tab-note-open = الوتر { $string }: مطلق
tab-note-dead = الوتر { $string }: نغمة مكتومة
tab-no-note = الوتر { $string }: لا نوتة
tab-note-technique = { $note }، { $technique }
tab-cursor = المازورة { $measure }، الضربة { $beat }، { $note }
tab-summary = تدوين التابلاتشر، { $strings ->
    [zero] لا أوتار
    [one] وتر واحد
    [two] وتران
    [few] { $strings } أوتار
    [many] { $strings } وترًا
   *[other] { $strings } وتر
}، { $measures ->
    [zero] لا مازورات
    [one] مازورة واحدة
    [two] مازورتان
    [few] { $measures } مازورات
    [many] { $measures } مازورة
   *[other] { $measures } مازورة
}
tab-editable = { $summary }. قابل للتحرير: اكتب رقم العتبة، ويحذف Delete النوتة.
edit-set-note = تعيين النوتة
edit-add-to-chord = إضافة نوتة إلى التآلف
edit-octave-up = رفع النوتة أوكتافًا
edit-octave-down = خفض النوتة أوكتافًا
edit-semitone-up = رفع النوتة نصف درجة
edit-semitone-down = خفض النوتة نصف درجة
edit-step-up = رفع النوتة
edit-step-down = خفض النوتة
edit-delete-measure = حذف المازورة
edit-delete-event = حذف الحدث
edit-delete-rest = حذف السكتة
edit-make-rest = تحويل إلى سكتة
edit-insert-rest = إدراج سكتة
edit-insert-measure = إدراج مازورة
edit-set-length = تعيين المدة
edit-sharpen = إضافة دييز
edit-flatten = إضافة بيمول
edit-make-natural = إضافة بيكار
edit-cycle-dots = تبديل النقاط
edit-shorten = تقصير
edit-lengthen = إطالة
edit-tie = ربط
edit-set-fret = تعيين العتبة
edit-delete-note = حذف النوتة
edit-add-note = إضافة نوتة
edit-raise-fret = رفع العتبة
edit-lower-fret = خفض العتبة
edit-set-technique = تعيين التقنية
