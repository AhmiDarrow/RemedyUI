# संगीत विजेट: स्क्रीन रीडर क्या पढ़ते हैं, और पूर्ववत इतिहास में नाम।
list-separator = {", "}
pitch = { $accidental ->
    [sharp] { $letter } शार्प { $octave }
    [flat] { $letter } फ़्लैट { $octave }
    [double-sharp] { $letter } डबल शार्प { $octave }
    [double-flat] { $letter } डबल फ़्लैट { $octave }
   *[natural] { $letter } { $octave }
}
note-length = { $dots ->
    [dotted]
        { $value ->
            [whole] डॉटेड होल
            [half] डॉटेड हाफ़
            [quarter] डॉटेड क्वार्टर
            [eighth] डॉटेड एट्थ
            [sixteenth] डॉटेड सिक्सटींथ
           *[thirty-second] डॉटेड थर्टी-सेकंड
        }
    [double-dotted]
        { $value ->
            [whole] डबल डॉटेड होल
            [half] डबल डॉटेड हाफ़
            [quarter] डबल डॉटेड क्वार्टर
            [eighth] डबल डॉटेड एट्थ
            [sixteenth] डबल डॉटेड सिक्सटींथ
           *[thirty-second] डबल डॉटेड थर्टी-सेकंड
        }
   *[none]
        { $value ->
            [whole] होल
            [half] हाफ़
            [quarter] क्वार्टर
            [eighth] एट्थ
            [sixteenth] सिक्सटींथ
           *[thirty-second] थर्टी-सेकंड
        }
}
event-note = { $pitch } { $length } नोट
event-note-tied = { $pitch } { $length } नोट, अगले नोट से टाई किया हुआ
event-chord = { $pitches } का { $length } कॉर्ड
event-rest = { $dots ->
    [dotted]
        { $value ->
            [whole] डॉटेड होल रेस्ट
            [half] डॉटेड हाफ़ रेस्ट
            [quarter] डॉटेड क्वार्टर रेस्ट
            [eighth] डॉटेड एट्थ रेस्ट
            [sixteenth] डॉटेड सिक्सटींथ रेस्ट
           *[thirty-second] डॉटेड थर्टी-सेकंड रेस्ट
        }
    [double-dotted]
        { $value ->
            [whole] डबल डॉटेड होल रेस्ट
            [half] डबल डॉटेड हाफ़ रेस्ट
            [quarter] डबल डॉटेड क्वार्टर रेस्ट
            [eighth] डबल डॉटेड एट्थ रेस्ट
            [sixteenth] डबल डॉटेड सिक्सटींथ रेस्ट
           *[thirty-second] डबल डॉटेड थर्टी-सेकंड रेस्ट
        }
   *[none]
        { $value ->
            [whole] होल रेस्ट
            [half] हाफ़ रेस्ट
            [quarter] क्वार्टर रेस्ट
            [eighth] एट्थ रेस्ट
            [sixteenth] सिक्सटींथ रेस्ट
           *[thirty-second] थर्टी-सेकंड रेस्ट
        }
}
key-no-accidentals = कोई शार्प या फ़्लैट नहीं
key-sharps = { $count ->
    [one] { $count } शार्प
   *[other] { $count } शार्प
}
key-flats = { $count ->
    [one] { $count } फ़्लैट
   *[other] { $count } फ़्लैट
}
staff-summary = { $clef ->
    [bass] बेस क्लेफ़
    [alto] ऑल्टो क्लेफ़
    [tenor] टेनर क्लेफ़
   *[treble] ट्रेबल क्लेफ़
}, { $key }, { $measures ->
    [one] { $measures } बार
   *[other] { $measures } बार
}
staff-summary-time = { $clef ->
    [bass] बेस क्लेफ़
    [alto] ऑल्टो क्लेफ़
    [tenor] टेनर क्लेफ़
   *[treble] ट्रेबल क्लेफ़
}, { $key }, { $beats }/{ $unit } टाइम, { $measures ->
    [one] { $measures } बार
   *[other] { $measures } बार
}
staff-editable = { $summary }. संपादन योग्य: अक्षर नोट सेट करते हैं, ऊपर और नीचे तीर उन्हें खिसकाते हैं, अंक अवधि सेट करते हैं।
staff-cursor = बार { $measure }, बीट { $beat }: { $event }
cursor-picked = { $what }, चुना गया
cursor-marked = { $what }, चिह्नित
cursor-picked-marked = { $what }, चुना गया, चिह्नित
chord-string-muted = तार { $string } म्यूट
chord-string-open = तार { $string } खुला
chord-string-fret = तार { $string } फ़्रेट { $fret }
chord-string-fret-finger = तार { $string } फ़्रेट { $fret } उंगली { $finger }
chord-barre = फ़्रेट { $fret } पर तार { $from } से { $to } तक बैरे
chord-barre-finger = फ़्रेट { $fret } पर तार { $from } से { $to } तक उंगली { $finger } से बैरे
chord-starts-at = डायग्राम फ़्रेट { $fret } से शुरू होता है
chord-named = { $label }: { $name }
position-open = तार { $string } ({ $tuning }), खुला, { $note }
position-fret = तार { $string } ({ $tuning }), फ़्रेट { $fret }, { $note }
position-open-short = तार { $string }, खुला
position-fret-short = तार { $string }, फ़्रेट { $fret }
fretboard-hint = तीर कुंजियाँ नेक पर घुमाती हैं, Enter चुनता है, Esc चयन हटाता है।
fretboard-hint-picked = तीर कुंजियाँ नेक पर घुमाती हैं, Enter चुनता है, Esc चयन हटाता है। चुना गया: { $picked }।
fretboard-summary = { $strings ->
    [one] { $strings } तार
   *[other] { $strings } तार
}, { $frets ->
    [one] { $frets } फ़्रेट
   *[other] { $frets } फ़्रेट
}, { $marked } चिह्नित
technique = { $technique ->
    [hammer-on] हैमर-ऑन
    [pull-off] पुल-ऑफ़
    [slide-up] स्लाइड अप
    [slide-down] स्लाइड डाउन
    [bend-half] आधे स्टेप का बेंड
    [bend-whole] पूरे स्टेप का बेंड
    [vibrato] वाइब्रेटो
    [harmonic] हार्मोनिक
    [palm-mute] पाम म्यूट
    [dead] डेड नोट
   *[tied] टाई किया हुआ
}
technique-bend = { $steps ->
    [one] { $steps } स्टेप का बेंड
   *[other] { $steps } स्टेप का बेंड
}
tab-note-fret = तार { $string }: फ़्रेट { $fret }
tab-note-open = तार { $string }: खुला
tab-note-dead = तार { $string }: डेड नोट
tab-no-note = तार { $string }: कोई नोट नहीं
tab-note-technique = { $note }, { $technique }
tab-cursor = बार { $measure }, बीट { $beat }, { $note }
tab-summary = टैबलेचर, { $strings ->
    [one] { $strings } तार
   *[other] { $strings } तार
}, { $measures ->
    [one] { $measures } बार
   *[other] { $measures } बार
}
tab-editable = { $summary }. संपादन योग्य: फ़्रेट संख्या टाइप करें, Delete नोट हटाता है।
edit-set-note = नोट सेट करें
edit-add-to-chord = कॉर्ड में नोट जोड़ें
edit-octave-up = नोट एक ऑक्टेव ऊपर करें
edit-octave-down = नोट एक ऑक्टेव नीचे करें
edit-semitone-up = नोट एक सेमीटोन ऊपर करें
edit-semitone-down = नोट एक सेमीटोन नीचे करें
edit-step-up = नोट ऊपर करें
edit-step-down = नोट नीचे करें
edit-delete-measure = बार हटाएँ
edit-delete-event = इवेंट हटाएँ
edit-delete-rest = रेस्ट हटाएँ
edit-make-rest = रेस्ट बनाएँ
edit-insert-rest = रेस्ट डालें
edit-insert-measure = बार डालें
edit-set-length = अवधि सेट करें
edit-sharpen = शार्प लगाएँ
edit-flatten = फ़्लैट लगाएँ
edit-make-natural = नेचुरल करें
edit-cycle-dots = डॉट बदलें
edit-shorten = छोटा करें
edit-lengthen = लंबा करें
edit-tie = टाई करें
edit-set-fret = फ़्रेट सेट करें
edit-delete-note = नोट हटाएँ
edit-add-note = नोट जोड़ें
edit-raise-fret = फ़्रेट बढ़ाएँ
edit-lower-fret = फ़्रेट घटाएँ
edit-set-technique = तकनीक सेट करें
