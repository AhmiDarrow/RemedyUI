# Font sources

Noto Sans SC and Noto Sans JP 2.004 (the Google Fonts builds of Noto Sans CJK), from the Fontsource packages `@fontsource/noto-sans-sc` 5.3.1 and `@fontsource/noto-sans-jp` 5.3.1 (npm). Google Fonts serves each as about a hundred slices by character frequency; `tools/merge_font_slices.py` joins the 400 and 700 slices back into one face each and writes them as WOFF2 (the text engine unpacks WOFF2 once):

```
python tools/merge_font_slices.py <package>/files 400 NotoSansSC-Regular.woff2 "Noto Sans SC"
```

| Face | Characters | Covers |
|---|---|---|
| Noto Sans SC | 13,525 | every hanzi of GB 2312, CJK punctuation, kana, full-width forms |
| Noto Sans JP | 16,591 | every kanji of JIS X 0208, kana, CJK punctuation, full-width forms |

Characters outside these sets (rare names, historic forms) come from the system's fonts where the app enables `system-fonts`.

Licensed under the SIL Open Font License 1.1 (`OFL-NotoSansCJK.txt`). The Reserved Font Name is "Source"; these faces keep their Noto names.
