"""Builds Remedy Symbols, the bundled fallback face for common interface symbols.

Every glyph is drawn here from geometry (lines, polygons, circles, arcs) on the metrics of
Remedy Sans UI: 1000 units per em, an 80-unit stem, symbols centred on the math axis at 315.
It covers the arrows, geometric shapes, stars, check marks and interface signs that apps use
as icons and that the Remedy text families do not have, so they render the same on every
platform, in a browser page and on systems without symbol fonts.

    python build.py            # writes ../fonts/RemedySymbols-Regular.ttf
    python build.py --preview  # also writes symbols-preview.png (needs Pillow)

Needs fontTools. The output is deterministic: the same script gives the same bytes.
"""

import math
import os
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UPM = 1000
AXIS = 315  # vertical centre of symbols, as Remedy Sans UI's arrows and operators
STEM = 80  # Remedy Sans UI Regular's stem
LINE = 70  # outlines of large shapes
ADV = 780  # default advance
FAMILY = "Remedy Symbols"
VERSION = "1.000"

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "fonts", "RemedySymbols-Regular.ttf")


# ---------------------------------------------------------------- geometry


def area(pts):
    return sum(x0 * y1 - x1 * y0 for (x0, y0), (x1, y1) in zip(pts, pts[1:] + pts[:1])) / 2


def oriented(pts, clockwise=True):
    """The polygon wound clockwise (filled outer) or counter-clockwise (hole)."""
    if (area(pts) < 0) != clockwise:
        return list(reversed(pts))
    return list(pts)


def line_intersection(p, d, q, e):
    """Where p + t*d meets q + s*e, or None when parallel."""
    den = d[0] * e[1] - d[1] * e[0]
    if abs(den) < 1e-9:
        return None
    t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / den
    return (p[0] + t * d[0], p[1] + t * d[1])


def unit(dx, dy):
    n = math.hypot(dx, dy)
    return (dx / n, dy / n)


def offset_closed(pts, dist):
    """Offsets a closed polygon by dist along its outward side (negative: inward), with
    mitered corners. The polygon must be clockwise."""
    pts = oriented(pts, True)
    n = len(pts)
    out = []
    for i in range(n):
        a, b, c = pts[i - 1], pts[i], pts[(i + 1) % n]
        d1 = unit(b[0] - a[0], b[1] - a[1])
        d2 = unit(c[0] - b[0], c[1] - b[1])
        # Clockwise in y-up: the outward normal of direction (dx, dy) is (-dy, dx).
        n1 = (-d1[1], d1[0])
        n2 = (-d2[1], d2[0])
        p1 = (a[0] + n1[0] * dist, a[1] + n1[1] * dist)
        p2 = (b[0] + n2[0] * dist, b[1] + n2[1] * dist)
        x = line_intersection(p1, d1, p2, d2)
        out.append(x if x else (b[0] + n1[0] * dist, b[1] + n1[1] * dist))
    return out


def stroke_open(pts, width):
    """The outline of an open polyline stroked `width` wide, butt ends, mitered joins."""
    h = width / 2
    left, right = [], []
    n = len(pts)
    for i in range(n):
        if i == 0:
            d = unit(pts[1][0] - pts[0][0], pts[1][1] - pts[0][1])
            nl = (-d[1], d[0])
            left.append((pts[0][0] + nl[0] * h, pts[0][1] + nl[1] * h))
            right.append((pts[0][0] - nl[0] * h, pts[0][1] - nl[1] * h))
        elif i == n - 1:
            d = unit(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1])
            nl = (-d[1], d[0])
            left.append((pts[i][0] + nl[0] * h, pts[i][1] + nl[1] * h))
            right.append((pts[i][0] - nl[0] * h, pts[i][1] - nl[1] * h))
        else:
            a, b, c = pts[i - 1], pts[i], pts[i + 1]
            d1 = unit(b[0] - a[0], b[1] - a[1])
            d2 = unit(c[0] - b[0], c[1] - b[1])
            n1 = (-d1[1], d1[0])
            n2 = (-d2[1], d2[0])
            for sign, side in ((1, left), (-1, right)):
                p1 = (a[0] + sign * n1[0] * h, a[1] + sign * n1[1] * h)
                p2 = (b[0] + sign * n2[0] * h, b[1] + sign * n2[1] * h)
                x = line_intersection(p1, d1, p2, d2)
                side.append(x if x else (b[0] + sign * n1[0] * h, b[1] + sign * n1[1] * h))
    return oriented(left + list(reversed(right)))


def segment(x0, y0, x1, y1, width=STEM):
    return stroke_open([(x0, y0), (x1, y1)], width)


def rect(x0, y0, x1, y1):
    return oriented([(x0, y0), (x0, y1), (x1, y1), (x1, y0)])


def rotate(pts, cx, cy, deg):
    a = math.radians(deg)
    ca, sa = math.cos(a), math.sin(a)
    return [(cx + (x - cx) * ca - (y - cy) * sa, cy + (x - cx) * sa + (y - cy) * ca) for x, y in pts]


def mirror_x(pts, cx):
    return oriented([(2 * cx - x, y) for x, y in pts])


def regular(cx, cy, r, sides, start_deg=90):
    return oriented(
        [
            (cx + r * math.cos(math.radians(start_deg + 360 * i / sides)), cy + r * math.sin(math.radians(start_deg + 360 * i / sides)))
            for i in range(sides)
        ]
    )


def star(cx, cy, outer, inner, points, start_deg=90):
    pts = []
    for i in range(points * 2):
        r = outer if i % 2 == 0 else inner
        a = math.radians(start_deg + 180 * i / points)
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return oriented(pts)


def arc_points(cx, cy, r, a0, a1, steps=40):
    return [
        (cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / steps)), cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / steps)))
        for i in range(steps + 1)
    ]


class Circle:
    """A circle as eight quadratic arcs; `hole` winds it the other way."""

    def __init__(self, cx, cy, r, hole=False):
        self.cx, self.cy, self.r, self.hole = cx, cy, r, hole


class Curved:
    """A closed contour of alternating on- and off-curve points, starting on-curve."""

    def __init__(self, pts, hole=False):
        self.pts, self.hole = pts, hole


def ring(cx, cy, r, width):
    """A circle outline whose stroke is centred on radius r."""
    return [Circle(cx, cy, r + width / 2), Circle(cx, cy, r - width / 2, hole=True)]


def outline(pts, width=LINE):
    """A closed polygon drawn as an outline `width` thick, the stroke inside the shape."""
    return [oriented(pts), oriented(offset_closed(pts, -width), clockwise=False)]


def chevron(tx, ty, dx, dy, arm, width=STEM):
    """An open arrowhead whose outer point is at (tx, ty), pointing along (dx, dy)."""
    d = unit(dx, dy)
    # The mitred tip pokes out width/sqrt(2) beyond the centre line's corner.
    inset = width / math.sqrt(2)
    cx, cy = tx - d[0] * inset, ty - d[1] * inset
    back = (-d[0], -d[1])
    a1 = rotate([(cx + back[0] * arm, cy + back[1] * arm)], cx, cy, 45)[0]
    a2 = rotate([(cx + back[0] * arm, cy + back[1] * arm)], cx, cy, -45)[0]
    return stroke_open([a1, (cx, cy), a2], width)


def arrow(x0, y0, x1, y1, head_end=True, head_start=False, arm=210, width=STEM):
    d = unit(x1 - x0, y1 - y0)
    shorten = width * 0.5
    sx0, sy0 = (x0 + d[0] * shorten, y0 + d[1] * shorten) if head_start else (x0, y0)
    sx1, sy1 = (x1 - d[0] * shorten, y1 - d[1] * shorten) if head_end else (x1, y1)
    parts = [segment(sx0, sy0, sx1, sy1, width)]
    if head_end:
        parts.append(chevron(x1, y1, d[0], d[1], arm, width))
    if head_start:
        parts.append(chevron(x0, y0, -d[0], -d[1], arm, width))
    return parts


def double_arrow(x0, x1, y, head_end=True, head_start=False):
    """⇒-style: two thin shafts closed by a wide open head."""
    gap, w, arm = 100, 60, 300
    parts = []
    s0 = x0 + (gap if head_start else 0)
    s1 = x1 - (gap if head_end else 0)
    for oy in (gap, -gap):
        parts.append(rect(s0, y + oy - w / 2, s1, y + oy + w / 2))
    if head_end:
        parts.append(chevron(x1, y, 1, 0, arm, LINE))
    if head_start:
        parts.append(chevron(x0, y, -1, 0, arm, LINE))
    return parts


def circle_arrow(clockwise):
    cx, cy, r = ADV / 2, AXIS, 230
    # Open at the top; a solid head on the end the arrow travels towards, pointing across
    # the gap.
    a0, a1 = (60, 60 - 290) if clockwise else (120, 120 + 290)
    pts = arc_points(cx, cy, r, a0, a1, 48)
    parts = [stroke_open(pts, LINE)]
    ex, ey = pts[-1]
    px, py = pts[-2]
    d = unit(ex - px, ey - py)
    n = (-d[1], d[0])
    half, ahead, behind = 115, 150, 10
    base = (ex - d[0] * behind, ey - d[1] * behind)
    parts.append(
        oriented(
            [
                (base[0] + n[0] * half, base[1] + n[1] * half),
                (ex + d[0] * ahead, ey + d[1] * ahead),
                (base[0] - n[0] * half, base[1] - n[1] * half),
            ]
        )
    )
    return parts


def triangle(cx, cy, w, h, direction):
    """A triangle w wide (across its point) and h long, pointing up/down/left/right."""
    pts = [(cx - w / 2, cy - h / 2), (cx + w / 2, cy - h / 2), (cx, cy + h / 2)]
    deg = {"up": 0, "left": 90, "down": 180, "right": -90}[direction]
    return oriented(rotate(pts, cx, cy, deg))


def check(heavy):
    w = 130 if heavy else STEM
    return [stroke_open([(110, 330), (300, 110), (680, 580)], w)]


def cross(x0, y0, x1, y1, w):
    return [segment(x0, y0, x1, y1, w), segment(x0, y1, x1, y0, w)]


def ballot(heavy):
    w = 130 if heavy else STEM
    return [segment(170, 30, 620, 600, w), segment(140, 560, 640, 60, w)]


def four_point(cx, cy, r, k):
    """✦: four points joined by concave curves, k sets how far the curves reach out."""
    return Curved(
        [
            (cx, cy + r), (cx + k, cy + k),
            (cx + r, cy), (cx + k, cy - k),
            (cx, cy - r), (cx - k, cy - k),
            (cx - r, cy), (cx - k, cy + k),
        ]
    )


def gear(cx, cy):
    teeth, outer, root = 8, 330, 250
    pts = []
    period = 360 / teeth
    for i in range(teeth):
        a = 90 + i * period
        for deg, r in ((a - 13, root), (a - 8, outer), (a + 8, outer), (a + 13, root)):
            pts.append((cx + r * math.cos(math.radians(deg)), cy + r * math.sin(math.radians(deg))))
    return [oriented(pts), Circle(cx, cy, 110, hole=True)]


def command(cx, cy):
    w, s, r = 60, 110, 90
    parts = [
        rect(cx - s - w / 2, cy - s - r, cx - s + w / 2, cy + s + r),
        rect(cx + s - w / 2, cy - s - r, cx + s + w / 2, cy + s + r),
        rect(cx - s - r, cy - s - w / 2, cx + s + r, cy - s + w / 2),
        rect(cx - s - r, cy + s - w / 2, cx + s + r, cy + s + w / 2),
    ]
    for ox in (-1, 1):
        for oy in (-1, 1):
            parts += ring(cx + ox * (s + r), cy + oy * (s + r), r, w)
    return parts


def warning():
    tri = [(30, -40), (390, 650), (750, -40)]
    parts = outline(tri, 64)
    parts.append(rect(352, 175, 428, 390))
    parts.append(Circle(390, 95, 40))
    return parts


def search():
    cx, cy, r = 450, 400, 180
    parts = ring(cx, cy, r, LINE)
    a = math.radians(225)
    sx, sy = cx + (r + LINE / 2 - 10) * math.cos(a), cy + (r + LINE / 2 - 10) * math.sin(a)
    parts.append(segment(sx, sy, 110, 60, STEM + 10))
    return parts


def house():
    pts = [(150, 0), (150, 380), (390, 620), (630, 380), (630, 0)]
    return [oriented(offset_closed(pts, LINE / 2)), oriented(offset_closed(pts, -LINE / 2), clockwise=False)]


def bars(n_bars, cx, cy, width, thick, gap):
    total = n_bars * thick + (n_bars - 1) * gap
    y = cy - total / 2
    parts = []
    for _ in range(n_bars):
        parts.append(rect(cx - width / 2, y, cx + width / 2, y + thick))
        y += thick + gap
    return parts


C = ADV / 2


def glyphs():
    """(codepoint, name, advance, parts)."""
    g = []

    def add(cp, name, parts, adv=ADV):
        # One shape (a circle, a curve or a polygon, which is a list of points) or a list of them.
        single = not isinstance(parts, list) or isinstance(parts[0], tuple)
        g.append((cp, name, adv, [parts] if single else parts))

    # Arrows, on Remedy Sans UI's: 80-unit stroke, open 45° heads, centred on the axis.
    add(0x2190, "arrowleft", arrow(700, AXIS, 42, AXIS), 770)
    add(0x2191, "arrowup", arrow(315, -10, 315, 700), 630)
    add(0x2192, "arrowright", arrow(70, AXIS, 728, AXIS), 770)
    add(0x2193, "arrowdown", arrow(315, 690, 315, -20), 630)
    add(0x2194, "arrowboth", arrow(70, AXIS, 830, AXIS, head_start=True), 900)
    add(0x2195, "arrowupdn", arrow(315, -40, 315, 680, head_start=True), 630)
    add(0x2196, "arrowupleft", arrow(640, -10, 110, 520), 750)
    add(0x2197, "arrowupright", arrow(110, -10, 640, 520), 750)
    add(0x2198, "arrowdownright", arrow(110, 640, 640, 110), 750)
    add(0x2199, "arrowdownleft", arrow(640, 640, 110, 110), 750)
    add(0x21BA, "circlearrowleft", circle_arrow(False))
    add(0x21BB, "circlearrowright", circle_arrow(True))
    add(0x21C4, "arrowrightoverleft", arrow(70, AXIS + 140, 728, AXIS + 140, arm=160) + arrow(728, AXIS - 140, 70, AXIS - 140, arm=160), 800)
    add(0x21C5, "arrowupleftofdown", arrow(200, -20, 200, 660, arm=160) + arrow(500, 660, 500, -20, arm=160), 700)
    add(0x21C6, "arrowleftoverright", arrow(728, AXIS + 140, 70, AXIS + 140, arm=160) + arrow(70, AXIS - 140, 728, AXIS - 140, arm=160), 800)
    add(0x21D0, "arrowdblleft", [mirror_x(p, 400) if isinstance(p, list) else p for p in double_arrow(70, 730, AXIS)], 800)
    add(0x21D2, "arrowdblright", double_arrow(70, 730, AXIS), 800)
    add(0x21D4, "arrowdblboth", double_arrow(70, 930, AXIS, head_start=True), 1000)
    add(0x21E4, "arrowtabbarleft", arrow(720, AXIS, 140, AXIS) + [rect(70, AXIS - 230, 70 + STEM, AXIS + 230)], 800)
    add(0x21E5, "arrowtabbarright", arrow(80, AXIS, 660, AXIS) + [rect(730 - STEM, AXIS - 230, 730, AXIS + 230)], 800)
    add(0x27F5, "longarrowleft", arrow(1110, AXIS, 42, AXIS), 1180)
    add(0x27F6, "longarrowright", arrow(70, AXIS, 1138, AXIS), 1180)
    add(0x27F7, "longarrowboth", arrow(70, AXIS, 1110, AXIS, head_start=True), 1180)

    # Geometric shapes.
    add(0x25A0, "blacksquare", rect(C - 300, AXIS - 300, C + 300, AXIS + 300))
    add(0x25A1, "whitesquare", outline(rect(C - 300, AXIS - 300, C + 300, AXIS + 300)))
    add(0x25AA, "blacksmallsquare", rect(C - 170, AXIS - 170, C + 170, AXIS + 170))
    add(0x25AB, "whitesmallsquare", outline(rect(C - 170, AXIS - 170, C + 170, AXIS + 170), 60))
    add(0x25AC, "blackrectangle", rect(C - 320, AXIS - 150, C + 320, AXIS + 150))
    add(0x25AD, "whiterectangle", outline(rect(C - 320, AXIS - 150, C + 320, AXIS + 150)))
    big = (640, 600)
    small = (340, 320)
    for cp, name, size, d, filled in (
        (0x25B2, "blackuptriangle", big, "up", True),
        (0x25B3, "whiteuptriangle", big, "up", False),
        (0x25B4, "blackupsmalltriangle", small, "up", True),
        (0x25B5, "whiteupsmalltriangle", small, "up", False),
        (0x25B6, "blackrighttriangle", big, "right", True),
        (0x25B7, "whiterighttriangle", big, "right", False),
        (0x25B8, "blackrightsmalltriangle", small, "right", True),
        (0x25B9, "whiterightsmalltriangle", small, "right", False),
        (0x25BC, "blackdowntriangle", big, "down", True),
        (0x25BD, "whitedowntriangle", big, "down", False),
        (0x25BE, "blackdownsmalltriangle", small, "down", True),
        (0x25BF, "whitedownsmalltriangle", small, "down", False),
        (0x25C0, "blacklefttriangle", big, "left", True),
        (0x25C1, "whitelefttriangle", big, "left", False),
        (0x25C2, "blackleftsmalltriangle", small, "left", True),
        (0x25C3, "whiteleftsmalltriangle", small, "left", False),
    ):
        t = triangle(C, AXIS, size[0], size[1], d)
        add(cp, name, t if filled else outline(t, LINE if size == big else 56))
    add(0x25BA, "blackrightpointer", triangle(C, AXIS, 400, 600, "right"))
    add(0x25C4, "blackleftpointer", triangle(C, AXIS, 400, 600, "left"))
    diamond = lambda r: oriented([(C, AXIS + r), (C + r, AXIS), (C, AXIS - r), (C - r, AXIS)])
    add(0x25C6, "blackdiamond", diamond(330))
    add(0x25C7, "whitediamond", outline(diamond(330), 64))
    add(0x25C8, "whitediamondcontainingblack", outline(diamond(330), 64) + [diamond(150)])
    add(0x25C9, "fisheye", ring(C, AXIS, 300 - LINE / 2, LINE) + [Circle(C, AXIS, 140)])
    add(0x25CB, "whitecircle", ring(C, AXIS, 300 - LINE / 2, LINE))
    add(0x25CE, "bullseye", ring(C, AXIS, 300 - LINE / 2, LINE) + ring(C, AXIS, 150, LINE))
    add(0x25CF, "blackcircle", Circle(C, AXIS, 300))
    add(0x25E6, "whitebullet", ring(210, AXIS, 85, 56), 420)
    add(0x25EF, "largecircle", ring(C + 60, AXIS, 370 - LINE / 2, LINE), 900)

    # Stars and sparkles.
    add(0x2605, "blackstar", star(C, AXIS - 15, 345, 140, 5))
    add(0x2606, "whitestar", outline(star(C, AXIS - 15, 345, 140, 5), 56))
    sparkle = four_point(C, AXIS, 340, 40)
    add(0x2726, "blackfourpointedstar", sparkle)
    hollow = four_point(C, AXIS, 190, 12)
    hollow.hole = True
    add(0x2727, "whitefourpointedstar", [sparkle, hollow])

    # Check marks and crosses.
    add(0x2713, "checkmark", check(False))
    add(0x2714, "heavycheckmark", check(True))
    add(0x2715, "multiplicationx", cross(150, 65, 630, 565, STEM))
    add(0x2716, "heavymultiplicationx", cross(150, 65, 630, 565, 130))
    add(0x2717, "ballotx", ballot(False))
    add(0x2718, "heavyballotx", ballot(True))

    # Interface signs.
    add(0x26A0, "warningsign", warning())
    add(0x2630, "trigramheaven", bars(3, C, AXIS, 580, STEM, 110))
    add(0x2261, "equivalence", bars(3, C, AXIS, 480, 64, 100))
    add(0x2315, "telephonerecorder", search())
    add(0x2318, "placeofinterest", command(C, AXIS))
    add(0x2302, "house", house())
    add(0x22EE, "verticalellipsis", [Circle(210, AXIS + d, 48) for d in (-220, 0, 220)], 420)
    add(0x22EF, "midlineellipsis", [Circle(x, AXIS, 48) for x in (190, 390, 590)])
    add(0x2699, "gear", gear(C, AXIS))
    add(0x2023, "triangularbullet", triangle(210, AXIS, 260, 230, "right"), 420)
    add(0x2043, "hyphenbullet", rect(110, AXIS - 40, 330, AXIS + 40), 440)

    # Media controls.
    add(0x23F4, "blackmediumlefttriangle", triangle(C, AXIS, 440, 400, "left"))
    add(0x23F5, "blackmediumrighttriangle", triangle(C, AXIS, 440, 400, "right"))
    add(0x23F6, "blackmediumuptriangle", triangle(C, AXIS, 440, 400, "up"))
    add(0x23F7, "blackmediumdowntriangle", triangle(C, AXIS, 440, 400, "down"))
    add(0x23F8, "pause", [rect(C - 190, AXIS - 260, C - 60, AXIS + 260), rect(C + 60, AXIS - 260, C + 190, AXIS + 260)])
    add(0x23F9, "stop", rect(C - 240, AXIS - 240, C + 240, AXIS + 240))
    add(0x23FA, "record", Circle(C, AXIS, 260))
    add(0x23CF, "eject", [triangle(C, AXIS + 120, 520, 300, "up"), rect(C - 260, AXIS - 260, C + 260, AXIS - 160)])
    return g


# ---------------------------------------------------------------- font


def draw(pen, part):
    if isinstance(part, Circle):
        k = 1 / math.cos(math.radians(22.5))
        angles = [i * 45 for i in range(8)]
        if not part.hole:
            angles = [-a for a in angles]  # clockwise
        pts = []
        for a in angles:
            pts.append((part.cx + part.r * math.cos(math.radians(a)), part.cy + part.r * math.sin(math.radians(a))))
        step = -45 if not part.hole else 45
        pen.moveTo(rnd(pts[0]))
        for i, a in enumerate(angles):
            mid = a + step / 2
            off = (part.cx + part.r * k * math.cos(math.radians(mid)), part.cy + part.r * k * math.sin(math.radians(mid)))
            pen.qCurveTo(rnd(off), rnd(pts[(i + 1) % 8]))
        pen.closePath()
    elif isinstance(part, Curved):
        pts = part.pts
        ons = pts[0::2]
        offs = pts[1::2]
        # Wind it: clockwise for a shape, counter-clockwise for a hole.
        if (area(ons) < 0) == part.hole:
            ons = [ons[0]] + list(reversed(ons[1:]))
            offs = list(reversed(offs))
        pen.moveTo(rnd(ons[0]))
        for i in range(len(ons)):
            pen.qCurveTo(rnd(offs[i]), rnd(ons[(i + 1) % len(ons)]))
        pen.closePath()
    else:
        pts = [rnd(p) for p in part]
        dedup = [p for i, p in enumerate(pts) if p != pts[i - 1]]
        pen.moveTo(dedup[0])
        for p in dedup[1:]:
            pen.lineTo(p)
        pen.closePath()


def rnd(p):
    return (int(round(p[0])), int(round(p[1])))


def notdef():
    pen = TTGlyphPen(None)
    for part in outline(rect(60, 0, 440, 700), 50):
        draw(pen, part)
    return pen.glyph(), 500


def build():
    order = [".notdef", "space"]
    cmap = {0x20: "space"}
    glyf = {}
    metrics = {}
    g, adv = notdef()
    glyf[".notdef"] = g
    metrics[".notdef"] = (adv, 60)
    glyf["space"] = TTGlyphPen(None).glyph()
    metrics["space"] = (260, 0)
    for cp, name, adv, parts in glyphs():
        assert name not in glyf, name
        pen = TTGlyphPen(None)
        for part in parts:
            draw(pen, part)
        glyph = pen.glyph()
        glyph.recalcBounds(None)
        order.append(name)
        cmap[cp] = name
        glyf[name] = glyph
        metrics[name] = (adv, getattr(glyph, "xMin", 0))

    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyf)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=1020, descent=-310)
    fb.setupNameTable(
        {
            "copyright": "Copyright 2026 Ahmi Darrow",
            "familyName": FAMILY,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"{FAMILY} Regular {VERSION}",
            "fullName": f"{FAMILY} Regular",
            "version": f"Version {VERSION}",
            "psName": "RemedySymbols-Regular",
            "licenseDescription": "This Font Software is licensed under the SIL Open Font License, Version 1.1. "
            "This license is available with a FAQ at: https://openfontlicense.org",
            "licenseInfoURL": "https://openfontlicense.org",
        }
    )
    fb.setupOS2(
        version=4,
        usWeightClass=400,
        fsType=0,
        achVendID="NONE",
        sTypoAscender=1020,
        sTypoDescender=-310,
        sTypoLineGap=0,
        usWinAscent=1075,
        usWinDescent=310,
        sxHeight=490,
        sCapHeight=700,
        fsSelection=0x40 | 0x80,  # REGULAR, USE_TYPO_METRICS
    )
    fb.setupPost()
    fb.setupHead(unitsPerEm=UPM, fontRevision=float(VERSION), created=0, modified=0)
    fb.font["head"].flags |= 1 << 3  # integer ppem, as hinted text expects
    fb.save(OUT)
    return fb.font, cmap


def preview(font_path, cmap):
    from PIL import Image, ImageDraw, ImageFont

    cps = sorted(c for c in cmap if c != 0x20)
    size, cols = 64, 16
    rows = (len(cps) + cols - 1) // cols
    img = Image.new("L", (cols * size + 20, rows * (size + 24) + 20), 255)
    d = ImageDraw.Draw(img)
    f = ImageFont.truetype(font_path, 44)
    label = ImageFont.load_default()
    for i, cp in enumerate(cps):
        x, y = 10 + (i % cols) * size, 10 + (i // cols) * (size + 24)
        d.text((x + 8, y + 4), chr(cp), font=f, fill=0)
        d.text((x + 4, y + size), "%04X" % cp, font=label, fill=120)
    out = os.path.join(HERE, "symbols-preview.png")
    img.save(out)
    print("preview:", out)


if __name__ == "__main__":
    font, cmap = build()
    print(f"{OUT}: {len(cmap) - 1} symbols, {os.path.getsize(OUT)} bytes")
    if "--preview" in sys.argv:
        preview(OUT, cmap)
