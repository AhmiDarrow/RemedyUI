"""Join a web font's slices back into one face, for the bundled script fallback faces.

Google Fonts (and Fontsource, which repackages it) serves large fonts such as Noto Sans SC as
about a hundred slices, each a subset of the same face for some characters. This joins the
slices of one weight back into a single face, names it plainly, and writes it as TrueType or
WOFF2 (by the output's extension).

    pip install fonttools brotli
    python tools/merge_font_slices.py <package>/files 400 NotoSansSC-Regular.woff2 "Noto Sans SC"

For a font that comes as one slice per script (Noto Sans Arabic), pass that slice's file
instead of a folder: it is renamed and converted only.
"""

import glob
import os
import sys
import tempfile

from fontTools.merge import Merger, Options
from fontTools.ttLib import TTFont


def slices(path, weight):
    if os.path.isfile(path):
        return [path]
    files = sorted(glob.glob(os.path.join(path, f"*-{weight}-normal.woff2")))
    # Numbered slices only: `noto-sans-sc-12-400-normal.woff2`.
    return [f for f in files if os.path.basename(f).split("-")[-3].isdigit()]


def name(font, family, weight):
    sub = "Regular" if weight == 400 else "Bold"
    table = font["name"]
    version = table.getDebugName(5) or ""
    for rec in list(table.names):
        if rec.nameID in (1, 2, 3, 4, 6, 16, 17, 21, 22, 25):
            table.removeNames(nameID=rec.nameID)
    ps = family.replace(" ", "") + "-" + sub
    for nid, val in [(1, family), (2, sub), (3, f"{version};{ps}"), (4, f"{family} {sub}"), (6, ps)]:
        table.setName(val, nid, 3, 1, 0x409)
        table.setName(val, nid, 1, 0, 0)
    font["OS/2"].usWeightClass = weight
    font["OS/2"].fsSelection = (font["OS/2"].fsSelection & ~0b1100001) | (0x20 if weight == 700 else 0x40)
    font["head"].macStyle = 1 if weight == 700 else 0


def main():
    src, weight, out, family = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    files = slices(src, weight)
    if not files:
        sys.exit(f"no slices of weight {weight} in {src}")
    with tempfile.TemporaryDirectory() as tmp:
        plain = []
        for i, f in enumerate(files):
            t = TTFont(f)
            t.flavor = None
            p = os.path.join(tmp, f"{i}.ttf")
            t.save(p)
            plain.append(p)
        if len(plain) == 1:
            font = TTFont(plain[0])
        else:
            # Vertical metrics differ per slice and are not needed for horizontal text.
            font = Merger(Options(drop_tables=["vhea", "vmtx", "VORG"])).merge(plain)
    name(font, family, weight)
    font.flavor = "woff2" if out.endswith(".woff2") else None
    font.save(out)
    print(f"{out}: {len(font.getBestCmap())} characters, {os.path.getsize(out)} bytes")


if __name__ == "__main__":
    main()
