"""Every gap on the library screen, measured ink to ink.

    python scripts/shoot.py shot.png
    python scripts/measure_gaps.py shot.png

Reports the distance between the *ink* of one thing and the ink of the next,
which is not what the markup says: a glyph has side bearings and a lucide icon
fills 20 of its 24 units, so a stated 12 arrives on screen as 15. `audit_ui.py`
checks the numbers; this checks what reaches the glass.
"""

import sys
from PIL import Image

if len(sys.argv) < 2:
    raise SystemExit("usage: python scripts/measure_gaps.py <screenshot.png>")

im = Image.open(sys.argv[1]).convert("RGB")
px = im.load()
W, H = im.size

LADDER = {4: "space-1", 8: "space-2", 12: "space-3", 16: "space-4", 24: "space-5", 32: "space-6"}


def ink_columns(x0, x1, y0, y1, thresh=22):
    base = min(sum(px[x, y]) for x in range(x0, x1) for y in range(y0, y1))
    return [x for x in range(x0, x1)
            if any(sum(px[x, y]) - base > thresh for y in range(y0, y1))]


def ink_rows(x0, x1, y0, y1, thresh=22):
    base = min(sum(px[x, y]) for x in range(x0, x1) for y in range(y0, y1))
    return [y for y in range(y0, y1)
            if any(sum(px[x, y]) - base > thresh for x in range(x0, x1))]


def runs(values, gap=5):
    out, start, prev = [], None, None
    for v in values:
        if start is None:
            start = v
        elif v - prev > gap:
            out.append((start, prev))
            start = v
        prev = v
    if start is not None:
        out.append((start, prev))
    return out


def verdict(gap):
    if gap in LADDER:
        return "on the ladder (%s)" % LADDER[gap]
    near = min(LADDER, key=lambda t: abs(t - gap))
    return "OFF by %+d from %s %d" % (gap - near, LADDER[near], near)


def report(name, gap):
    print("  %-42s %4d px   %s" % (name, gap, verdict(gap)))


print("\n=== sidebar, horizontal ===")
icon = runs(ink_columns(20, 50, 194, 212), gap=3)
label = runs(ink_columns(50, 140, 194, 212), gap=8)
report("nav icon ink -> label ink", label[0][0] - icon[-1][1] - 1)

grp = runs(ink_columns(20, 140, 164, 180), gap=8)
report("group label ink starts at x", grp[0][0])
report("nav icon ink starts at x", icon[0][0])

print("\n=== sidebar, vertical ===")
items = runs(ink_rows(50, 140, 186, 470), gap=6)
for a, b in zip(items, items[1:]):
    report("nav item ink -> next item ink", b[0] - a[1] - 1)
    break
grp_rows = runs(ink_rows(20, 140, 160, 215), gap=6)
report("group label ink -> first item ink", grp_rows[1][0] - grp_rows[0][1] - 1)

print("\n=== the head ===")
bands = runs(ink_rows(296, 900, 50, 200), gap=6)
names = ["heading", "summary line", "controls"]
for i, (a, b) in enumerate(zip(bands, bands[1:])):
    label_ = "%s ink -> %s ink" % (names[i] if i < len(names) else "band %d" % i,
                                   names[i + 1] if i + 1 < len(names) else "next band")
    report(label_, b[0] - a[1] - 1)

print("\n=== a library row ===")
row_top, row_bottom = 210, 270
cells = runs(ink_columns(296, 1130, row_top, row_bottom), gap=14)
labels = ["sleeve", "title", "artist", "album", "time"]
for i, (a, b) in enumerate(zip(cells, cells[1:])):
    left = labels[i] if i < len(labels) else "cell %d" % i
    right = labels[i + 1] if i + 1 < len(labels) else "next"
    report("%s ink -> %s ink" % (left, right), b[0] - a[1] - 1)
report("page inset -> sleeve ink", cells[0][0] - 296)
report("time ink -> content edge", 1124 - cells[-1][1])

print("\n=== the player bar ===")
blocks = runs(ink_columns(30, 340, 690, 730), gap=14)
report("cover ink -> title ink", blocks[1][0] - blocks[0][1] - 1)
report("title ink -> heart ink", blocks[-1][0] - blocks[-2][1] - 1)

transport = runs(ink_columns(470, 720, 690, 712), gap=10)
for a, b in zip(transport, transport[1:]):
    report("transport glyph -> glyph", b[0] - a[1] - 1)
