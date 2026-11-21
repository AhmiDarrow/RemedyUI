"""Build the reference-shape fixtures of `remedy-ui-text`: small OFL fonts cut down to the test
paragraphs, and what independent implementations say those paragraphs become.

    pip install fonttools brotli uharfbuzz
    npm pack @fontsource/noto-sans-hebrew@5.3.0 && tar xzf noto-sans-hebrew-5.3.0.tgz
    python tools/shape_references.py package

The references come from three libraries the engine does not use, so a test that passes means
two implementations agree, not that the engine agrees with itself:

- shaping (glyph ids, clusters, advances, offsets): HarfBuzz, through uharfbuzz, written in
  `hb-shape --no-glyph-names --cluster-level=0` form;
- bidi levels: GNU FriBidi (`libfribidi.so.0`), from which the visual order of runs is worked
  out by UAX #9 rule L2, the runs split where the script changes (UAX #24, from fontTools'
  Unicode data);
- line break opportunities: ICU4C (`libicuuc`), with the line width filled greedily from the
  HarfBuzz advances.

The fonts are subsets of Noto Sans Arabic (the face `remedy-ui-fonts` bundles), Noto Sans SC
(the face `remedy-ui-fonts-cjk` bundles) and Noto Sans Hebrew (Fontsource 5.3.0, its Hebrew
and Latin slices merged), each keeping only the characters of the paragraphs below, with
their layout tables. All three are under the SIL Open Font License 1.1.
"""

import ctypes
import ctypes.util
import glob
import os
import sys
import tempfile

import uharfbuzz as hb
from fontTools import subset, unicodedata
from fontTools.merge import Merger, Options
from fontTools.ttLib import TTFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from merge_font_slices import name  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "crates", "remedy-ui-text", "tests", "fixtures", "shaping")

# (name, font, language, paragraph direction, wrap width or None, text)
CASES = [
    # Joining forms, the lam-alef ligature, a mark (tanween) on its letter, Arabic-Indic
    # digits (an AN run inside the RTL paragraph) and Arabic punctuation.
    ("arabic", "arabic", "ar", "auto", None, "حفظ ٣ ملفات؟ لا يمكنك التراجع عن ذلك لاحقًا"),
    # Points (niqqud) and a shin dot stacked on their letters, European digits after Hebrew
    # (EN inside RTL), and a Latin word: three levels in one RTL paragraph.
    ("hebrew", "hebrew", "he", "auto", None, "שָׁלוֹם עוֹלָם, 123 קבצים ב-Remedy."),
    # Ideographs, a Latin digit between spaces, full-width punctuation.
    ("cjk", "cjk", "zh-Hans", "auto", None, "保存 3 个文件？您可以撤消此操作。"),
    # A left-to-right paragraph holding a Hebrew phrase whose number stays with it.
    ("bidi-ltr", "hebrew", "en", "ltr", None, "Open שלום עולם 42 now"),
    # The same pieces in a right-to-left paragraph.
    ("bidi-rtl", "hebrew", "he", "rtl", None, "שלום Open now עולם 42"),
    # Wrapping at spaces, Arabic.
    ("arabic-wrap", "arabic", "ar", "auto", 9000.0, "يمكنك حفظ الملفات الآن أو لاحقا، ولا يمكنك التراجع عن ذلك بعد الحفظ"),
    # Wrapping between ideographs, never before closing punctuation (the width puts a
    # full stop right at the edge).
    ("cjk-wrap", "cjk", "zh-Hans", "auto", None, "您可以撤消此操作。保存这些文件吗？可以。"),
]

FONTS = {
    "arabic": "NotoSansArabic-Subset.ttf",
    "hebrew": "NotoSansHebrew-Subset.ttf",
    "cjk": "NotoSansSC-Subset.ttf",
}


def plain(path, tmp, i):
    t = TTFont(path)
    t.flavor = None
    p = os.path.join(tmp, f"{i}.ttf")
    t.save(p)
    return p


def build_fonts(hebrew_package):
    texts = {k: "".join(c[5] for c in CASES if c[1] == k) for k in FONTS}
    with tempfile.TemporaryDirectory() as tmp:
        sources = {
            "arabic": os.path.join(ROOT, "crates", "remedy-ui-fonts", "fonts", "NotoSansArabic-Regular.ttf"),
            "cjk": plain(os.path.join(ROOT, "crates", "remedy-ui-fonts-cjk", "fonts", "NotoSansSC-Regular.woff2"), tmp, 0),
        }
        slices = [
            plain(f, tmp, 10 + i)
            for i, f in enumerate(
                sorted(glob.glob(os.path.join(hebrew_package, "files", "noto-sans-hebrew-*-400-normal.woff2")))
            )
            if os.path.basename(f).split("-")[3] in ("hebrew", "latin")
        ]
        merged = Merger(Options(drop_tables=["vhea", "vmtx", "VORG"])).merge(slices)
        merged["head"].created = TTFont(slices[0])["head"].created
        # Fontsource's slices are named after the variable font's first instance.
        name(merged, "Noto Sans Hebrew", 400)
        sources["hebrew"] = os.path.join(tmp, "hebrew.ttf")
        merged.save(sources["hebrew"])
        for key, src in sources.items():
            opts = subset.Options()
            opts.layout_features = ["*"]
            opts.name_IDs = ["*"]
            opts.notdef_outline = True
            opts.drop_tables += ["vhea", "vmtx", "VORG", "DSIG"]
            font = TTFont(src)
            s = subset.Subsetter(opts)
            s.populate(text=texts[key] + " ")
            s.subset(font)
            out = os.path.join(OUT, FONTS[key])
            # Keep the source's dates, so the same inputs give the same bytes.
            font.recalcTimestamp = False
            font["head"].modified = font["head"].created
            font.save(out)
            missing = {c for c in texts[key] if ord(c) not in font.getBestCmap()}
            assert not missing, f"{key}: {missing}"
            print(f"{out}: {len(font.getGlyphOrder())} glyphs, {os.path.getsize(out)} bytes")


def fribidi_levels(text, direction):
    lib = ctypes.CDLL(ctypes.util.find_library("fribidi") or "libfribidi.so.0")
    n = len(text)
    chars = (ctypes.c_uint32 * n)(*[ord(c) for c in text])
    types = (ctypes.c_uint32 * n)()
    brackets = (ctypes.c_uint32 * n)()
    levels = (ctypes.c_int8 * n)()
    lib.fribidi_get_bidi_types(chars, n, types)
    lib.fribidi_get_bracket_types(chars, n, types, brackets)
    # FRIBIDI_PAR_ON (auto), FRIBIDI_PAR_LTR, FRIBIDI_PAR_RTL.
    base = ctypes.c_uint32({"auto": 0x40, "ltr": 0x110, "rtl": 0x111}[direction])
    lib.fribidi_get_par_embedding_levels_ex.restype = ctypes.c_int8
    assert lib.fribidi_get_par_embedding_levels_ex(types, brackets, n, ctypes.byref(base), levels) > 0
    return list(levels)


def icu_breaks(text, locale):
    """Line break opportunities as byte offsets (UAX #14 with ICU's tailoring)."""
    path = ctypes.util.find_library("icuuc") or "libicuuc.so"
    lib = ctypes.CDLL(path)
    ver = next(v for v in range(60, 100) if hasattr(lib, f"ubrk_open_{v}"))
    f = lambda name: getattr(lib, f"{name}_{ver}")
    u16 = text.encode("utf-16-le")
    buf = ctypes.create_string_buffer(u16, len(u16) + 2)
    status = ctypes.c_int(0)
    bi = f("ubrk_open")(2, locale.encode(), buf, len(u16) // 2, ctypes.byref(status))
    assert status.value <= 0, status.value
    units = []
    f("ubrk_first")(bi)
    while (p := f("ubrk_next")(bi)) != -1:
        units.append(p)
    f("ubrk_close")(bi)
    return [len(u16[: 2 * u].decode("utf-16-le").encode()) for u in units]


def shape(font, text, start, end, rtl, lang, script=None):
    buf = hb.Buffer()
    data = text.encode()
    # The whole paragraph as context, the run as the item, as an engine shapes it.
    buf.add_utf8(data, item_offset=start, item_length=end - start)
    buf.guess_segment_properties()
    if script:
        buf.script = script
    buf.direction = "rtl" if rtl else "ltr"
    buf.language = lang
    buf.cluster_level = hb.BufferClusterLevel.MONOTONE_GRAPHEMES
    hb.shape(font, buf)
    return list(zip(buf.glyph_infos, buf.glyph_positions))


def serialize(glyphs):
    return "[" + "|".join(f"{i.codepoint}={i.cluster}@{p.x_offset},{p.y_offset}+{p.x_advance}" for i, p in glyphs) + "]"


def scripts_of(text):
    """Each character's script, Common and Inherited ones taking the script before them (or
    after, at the start), as UAX #24 script runs are usually resolved."""
    raw = [unicodedata.script(c) for c in text]
    out, last = [], None
    for s in raw:
        if s not in ("Zyyy", "Zinh"):
            last = s
        out.append(last)
    first = next((s for s in out if s), "Zyyy")
    return [s or first for s in out]


def runs_of(levels, text):
    """Maximal spans of one level and one script, as byte ranges with their level."""
    out = []
    pos = 0
    for c, lvl, script in zip(text, levels, scripts_of(text)):
        n = len(c.encode())
        if out and out[-1][2] == lvl and out[-1][3] == script:
            out[-1][1] += n
        else:
            out.append([pos, pos + n, lvl, script])
        pos += n
    return [tuple(r) for r in out]


def visual(runs):
    """UAX #9 L2: reverse every maximal sequence at or above each level, highest first."""
    order = list(runs)
    top = max(r[2] for r in order)
    low = min(r[2] for r in order if r[2] % 2) if any(r[2] % 2 for r in order) else top + 1
    for k in range(top, low - 1, -1):
        i = 0
        while i < len(order):
            if order[i][2] >= k:
                j = i
                while j < len(order) and order[j][2] >= k:
                    j += 1
                order[i:j] = order[i:j][::-1]
                i = j
            else:
                i += 1
    return order


def wrap(text, glyphs, breaks, width):
    """Greedy filling: each line takes the most text up to a break opportunity whose width,
    trailing spaces left out, fits."""
    adv = {}
    for i, p in glyphs:
        adv[i.cluster] = adv.get(i.cluster, 0) + p.x_advance
    data = text.encode()

    def w(a, b):
        while b > a and data[b - 1 : b] == b" ":
            b -= 1
        return sum(v for k, v in adv.items() if a <= k < b)

    lines, start = [], 0
    while start < len(data):
        fit = [b for b in breaks if b > start and w(start, b) <= width]
        end = fit[-1] if fit else next(b for b in breaks if b > start)
        lines.append((start, end))
        start = end
    return lines


def main():
    hebrew_package = sys.argv[1]
    os.makedirs(OUT, exist_ok=True)
    build_fonts(hebrew_package)
    out = [
        "# Reference shapes for tests/reference_shapes.rs, written by tools/shape_references.py.",
        f"# HarfBuzz {hb.__version__} (uharfbuzz), GNU FriBidi, ICU4C; do not edit by hand.",
        "# Glyphs are in hb-shape form, visual order: [glyph=cluster@x_offset,y_offset+x_advance|...],",
        "# in font units, clusters as byte offsets into the paragraph.",
    ]
    for name, key, lang, direction, width, text in CASES:
        blob = hb.Blob.from_file_path(os.path.join(OUT, FONTS[key]))
        font = hb.Font(hb.Face(blob))
        upem = font.face.upem
        levels = fribidi_levels(text, direction)
        runs = runs_of(levels, text)
        breaks = icu_breaks(text, lang)
        out += ["", f"case {name}", f"font {FONTS[key]}", f"lang {lang}", f"dir {direction}", f"upem {upem}", f"text {text}"]
        if name == "cjk-wrap":
            # A width that ends exactly at the full stop: a break before it would fit the
            # ideograph before, so only the rule against it keeps them together.
            g = shape(font, text, 0, len(text.encode()), False, lang)
            stop = text.encode().index("。".encode())
            width = sum(p.x_advance for i, p in g if i.cluster < stop) - 1
        if width is None:
            for s, e, lvl, script in visual(runs):
                glyphs = serialize(shape(font, text, s, e, lvl % 2 == 1, lang, script))
                out.append(f"run {s} {e} {'rtl' if lvl % 2 else 'ltr'} {glyphs}")
        else:
            assert len({r[2] for r in runs}) == 1, f"{name}: wrap cases keep one level"
            g = shape(font, text, 0, len(text.encode()), runs[0][2] % 2 == 1, lang)
            out.append(f"width {width:g}")
            for a, b in wrap(text, g, breaks, width):
                out.append(f"line {a} {b}")
    path = os.path.join(OUT, "references.txt")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print(path)


if __name__ == "__main__":
    main()
