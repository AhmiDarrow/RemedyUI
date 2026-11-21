# Reference-shape fixtures

Fonts cut down to the paragraphs of `tests/reference_shapes.rs` (every character they use, with
all layout features), and `references.txt`, what HarfBuzz, GNU FriBidi and ICU4C make of those
paragraphs. `tools/shape_references.py` writes all four files; the same inputs give the same bytes.

| File | Cut from | Size |
|---|---|---|
| `NotoSansArabic-Subset.ttf` | Noto Sans Arabic 2.012 Regular, as `remedy-ui-fonts` bundles it | 13 KB |
| `NotoSansHebrew-Subset.ttf` | Noto Sans Hebrew 3.001 Regular, the Hebrew and Latin slices of `@fontsource/noto-sans-hebrew` 5.3.0 (npm) merged | 5 KB |
| `NotoSansSC-Subset.ttf` | Noto Sans SC 2.004 Regular, as `remedy-ui-fonts-cjk` bundles it | 17 KB |

```
pip install fonttools brotli uharfbuzz
npm pack @fontsource/noto-sans-hebrew@5.3.0 && tar xzf noto-sans-hebrew-5.3.0.tgz
python tools/shape_references.py package
```

The references were recorded with HarfBuzz through uharfbuzz 0.57.0, GNU FriBidi 1.0 and ICU4C 74.

Licensed under the SIL Open Font License 1.1 (`OFL.txt`). Noto Sans SC's Reserved Font Name is
"Source"; these subsets keep their Noto names.
