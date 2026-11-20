bars-value =
    { $count ->
        [one] { $count } बार, सबसे ऊँचा { $percent } प्रतिशत
       *[other] { $count } बार, सबसे ऊँचा { $percent } प्रतिशत
    }
meter-value-normal = { $percent } प्रतिशत, सामान्य
meter-value-loud = { $percent } प्रतिशत, तेज़
meter-value-clipping = { $percent } प्रतिशत, क्लिपिंग
meter-threshold = { $label } की सीमा
meter-percent = { $percent } प्रतिशत
ring-speaking = बोल रहे हैं
ring-silent = चुप
ring-muted = म्यूट
sparkline-no-data = कोई डेटा नहीं
sparkline-latest = नवीनतम { $value }
