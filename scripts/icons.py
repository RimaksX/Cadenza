"""Make every icon one sharp pixel of line at the size it is drawn at.

    python scripts/icons.py            # report what would change
    python scripts/icons.py --write    # change it

Run after adding or replacing an icon, with its size added to SIZES. Two
things are set, both from the size the icon is drawn at:

The stroke. It is drawn in the icon's own 24-unit grid, so the same number is
a different line at every size; it is set to 24 / size so every icon's line is
one pixel on screen.

Where its straight lines stand.
A one-pixel line is sharp only when its centre falls on a pixel's middle. Each
icon is drawn at one known size (see SIZES), so its grid unit is size/24 pixels;
every value a horizontal or vertical line stands on is moved to the nearest
value that lands on a pixel centre, and the same value is moved everywhere else
in the drawing too, so what was joined stays joined.
"""
import glob
import io
import os
import re
import sys

SIZES = {}
for size, names in {
    12: ["queue", "trash", "chevron-down", "play-12"],
    14: ["search"],
    16: ["analytics", "equalizer", "house", "library", "playlist", "radio", "review",
         "settings", "circle", "circle-dot", "circle-question-mark", "dots", "dots-vertical",
         "repeat", "repeat-one", "shuffle", "chevron-up", "download", "play-16",
         "panel-left-open", "panel-left-close", "panel-right-open", "mic",
         "panel-right-close"],
    18: ["next", "previous", "gauge", "hourglass", "sliders-horizontal", "volume", "volume-muted"],
    20: ["heart", "heart-filled", "play", "pause"],
}.items():
    for name in names:
        SIZES[name] = size

NUMBER = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")
ARITY = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "T": 2, "A": 7, "Z": 0}


def parse(d):
    """Path data as (command, numbers) with every command absolute."""
    i, n = 0, len(d)
    out = []
    cx = cy = sx = sy = 0.0
    cmd = None

    def skip():
        nonlocal i
        while i < n and d[i] in " ,\t\n\r":
            i += 1

    def number():
        nonlocal i
        skip()
        m = NUMBER.match(d, i)
        if not m:
            raise ValueError("number expected at %d in %r" % (i, d))
        i = m.end()
        return float(m.group(0))

    def flag():
        nonlocal i
        skip()
        c = d[i]
        i += 1
        return float(c)

    while True:
        skip()
        if i >= n:
            break
        if d[i].isalpha():
            cmd = d[i]
            i += 1
            if cmd in "Zz":
                out.append(("Z", []))
                cx, cy = sx, sy
                continue
        up, rel = cmd.upper(), cmd.islower()
        if up == "A":
            rx, ry, rot = number(), number(), number()
            large, sweep = flag(), flag()
            x, y = number(), number()
            if rel:
                x, y = x + cx, y + cy
            out.append(("A", [rx, ry, rot, large, sweep, x, y]))
            cx, cy = x, y
            continue
        values = [number() for _ in range(ARITY[up])]
        if up == "H":
            x = values[0] + (cx if rel else 0)
            out.append(("H", [x]))
            cx = x
        elif up == "V":
            y = values[0] + (cy if rel else 0)
            out.append(("V", [y]))
            cy = y
        else:
            if rel:
                values = [v + (cx if k % 2 == 0 else cy) for k, v in enumerate(values)]
            out.append((up, values))
            cx, cy = values[-2], values[-1]
            if up == "M":
                sx, sy = cx, cy
                cmd = "l" if rel else "L"  # extra pairs after M are lines
    return out


def lines(segments):
    """The x values vertical lines stand on and the y values horizontal ones do."""
    xs, ys = set(), set()
    cx = cy = 0.0
    for up, v in segments:
        if up == "H":
            ys.add(cy)
            cx = v[0]
        elif up == "V":
            xs.add(cx)
            cy = v[0]
        elif up == "Z" or not v:
            continue
        else:
            if up == "L":
                if abs(v[0] - cx) < 1e-9:
                    xs.add(cx)
                if abs(v[1] - cy) < 1e-9:
                    ys.add(cy)
            cx, cy = v[-2], v[-1]
    return xs, ys


def snap(value, unit):
    pixels = value * unit
    return (round(pixels - 0.5) + 0.5) / unit


def fmt(v):
    s = ("%.3f" % v).rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def moved(value, table):
    for old, new in table.items():
        if abs(value - old) < 1e-6:
            return new
    return value


def emit(segments, xmap, ymap):
    parts = []
    for up, v in segments:
        if up == "Z":
            parts.append("Z")
        elif up == "H":
            parts.append("H" + fmt(moved(v[0], xmap)))
        elif up == "V":
            parts.append("V" + fmt(moved(v[0], ymap)))
        elif up == "A":
            rx, ry, rot, large, sweep, x, y = v
            parts.append("A%s %s %s %d %d %s %s" % (fmt(rx), fmt(ry), fmt(rot), large, sweep,
                                                   fmt(moved(x, xmap)), fmt(moved(y, ymap))))
        else:
            pts = [fmt(moved(val, xmap if k % 2 == 0 else ymap)) for k, val in enumerate(v)]
            parts.append(up + " ".join(pts))
    return "".join(parts)


def hint(path):
    name = os.path.basename(path)[:-4]
    unit = SIZES[name] / 24
    svg = io.open(path, encoding="utf-8").read()
    svg = re.sub(r'stroke-width="[^"]*"', 'stroke-width="%s"' % fmt(round(1 / unit, 3)), svg)
    svg = svg.replace(" preview-icon", "")

    # every line value in the whole drawing, so one value moves one way everywhere
    xs, ys = set(), set()
    paths = re.findall(r'<path d="([^"]+)"', svg)
    parsed = {d: parse(d) for d in paths}
    for segs in parsed.values():
        x, y = lines(segs)
        xs |= x
        ys |= y
    for rect in re.findall(r"<rect ([^>]+)>", svg):
        a = dict(re.findall(r'(\w+)="([^"]+)"', rect))
        x, y, w, h = (float(a.get(k, 0)) for k in ("x", "y", "width", "height"))
        xs |= {x, x + w}
        ys |= {y, y + h}
    xmap = {v: snap(v, unit) for v in xs}
    ymap = {v: snap(v, unit) for v in ys}

    for d, segs in parsed.items():
        svg = svg.replace('<path d="%s"' % d, '<path d="%s"' % emit(segs, xmap, ymap), 1)

    def rect_fix(m):
        a = dict(re.findall(r'(\w+)="([^"]+)"', m.group(1)))
        x, y, w, h = (float(a.get(k, 0)) for k in ("x", "y", "width", "height"))
        nx, ny = moved(x, xmap), moved(y, ymap)
        a["x"], a["y"] = fmt(nx), fmt(ny)
        a["width"], a["height"] = fmt(moved(x + w, xmap) - nx), fmt(moved(y + h, ymap) - ny)
        return "<rect " + " ".join('%s="%s"' % kv for kv in a.items()) + "/>"

    svg = re.sub(r"<rect ([^>]*?)/?>", rect_fix, svg)
    shift = max([abs(a - b) for a, b in list(xmap.items()) + list(ymap.items())] or [0])
    return svg, len(xmap) + len(ymap), shift


write = "--write" in sys.argv
for path in sorted(glob.glob("resources/ui/icons/*.svg")):
    svg, count, shift = hint(path)
    print("%-22s %2d lines moved at most %.2f units" % (os.path.basename(path)[:-4], count, shift))
    if svg != io.open(path, encoding="utf-8").read():
        print("    changes")
    if write:
        io.open(path, "w", encoding="utf-8", newline="").write(svg)
print("WRITTEN" if write else "DRY RUN")
