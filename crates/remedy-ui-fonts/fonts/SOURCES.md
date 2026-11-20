# Font sources

The hinted TrueType builds, copied from each family's own repository. To update a family, rebuild it there (`python build.py && python check.py`), then copy its `fonts/*.ttf` here.

| Family | Repository | Commit |
|---|---|---|
| Remedy Sans UI | https://github.com/AhmiDarrow/RemedySansUI | 50d038e |
| Remedy Sans | https://github.com/AhmiDarrow/RemedySans | 023154e |
| AhmiPrint | https://github.com/AhmiDarrow/AhmiPrint | dd5e94e |
| AhmiPixel | https://github.com/AhmiDarrow/AhmiPixelFont | 620cb94 |

Remedy Symbols is drawn in this repository: `symbols/build.py` builds `RemedySymbols-Regular.ttf` from geometry on Remedy Sans UI's metrics (`python build.py`, needs fontTools; `--preview` also writes a PNG of every glyph). To add a symbol, draw it there and rebuild.

The Noto script faces are the Google Fonts builds of Noto Sans Arabic 2.012, Noto Sans Devanagari 2.006 and Noto Sans Bengali 3.011 (https://github.com/notofonts), from the Fontsource packages `@fontsource/noto-sans-arabic`, `-devanagari` and `-bengali` 5.3.0 (npm): each script's own slice (the letters, marks, digits and punctuation of that script, with every shaping feature), as TrueType, Regular and Bold. Latin and the other scripts come from Remedy Sans UI. `OFL-NotoSans.txt` carries their copyright notices.

All are licensed under the SIL Open Font License 1.1 (`OFL-*.txt`).
