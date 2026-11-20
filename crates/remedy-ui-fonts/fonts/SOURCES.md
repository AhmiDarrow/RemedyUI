# Font sources

The hinted TrueType builds, copied from each family's own repository. To update a family, rebuild it there (`python build.py && python check.py`), then copy its `fonts/*.ttf` here.

| Family | Repository | Commit |
|---|---|---|
| Remedy Sans UI | https://github.com/AhmiDarrow/RemedySansUI | 50d038e |
| Remedy Sans | https://github.com/AhmiDarrow/RemedySans | 9844a11 |
| AhmiPrint | https://github.com/AhmiDarrow/AhmiPrint | 451c495 |
| AhmiPixel | https://github.com/AhmiDarrow/AhmiPixelFont | 620cb94 |

Remedy Symbols is drawn in this repository: `symbols/build.py` builds `RemedySymbols-Regular.ttf` from geometry on Remedy Sans UI's metrics (`python build.py`, needs fontTools; `--preview` also writes a PNG of every glyph). To add a symbol, draw it there and rebuild.

All are licensed under the SIL Open Font License 1.1 (`OFL-*.txt`).
