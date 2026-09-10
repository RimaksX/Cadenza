"""Five questions asked of every line of Slint markup.

Run from the repository root:

    python scripts/audit_ui.py

Each check exists because the defect it names actually shipped and had to be
found by eye first:

1. A line of type with no box of its own. A text asks for a fractional height
   from its font's metrics; a layout centring that fraction puts the line
   between two pixel rows, and the letters are then rasterised at visibly
   different heights.

2. A box too small for the type in it. Slint does not clip a line that will not
   fit — it drops it. A button lost its label this way, twice.

3. A boxed line standing directly in a horizontal layout. A child with a height
   of its own cannot stretch, so the layout stands it at the top of the row
   instead of the middle, beside whatever it was meant to line up with.

4. A halving that was never rounded. `(parent - self) / 2` is the interface's
   most common expression and lands on half a pixel whenever the two differ by
   an odd number.

5. Any odd length. The rule in `theme/tokens.slint` is that every length is
   even, so that halving one is whole arithmetic and nobody has to remember
   check 4 at all.

Exit status is 1 when anything is found, so this can be a gate.
"""

import glob
import io
import re
import sys

TOKENS = "crates/ui/slint/theme/tokens.slint"


def scale():
    """The type scale, read from the theme rather than copied out of it.

    Copied, it drifted: the table here still named three sizes that had been
    removed and put the title two pixels short, so check 2 was measuring
    against numbers nobody used any more — and a box too small for a 12px
    label, which is the defect that ate a button's text twice, passed.
    """
    source = io.open(TOKENS, encoding="utf-8").read()
    return {
        "Theme.%s" % name: float(size)
        for name, size in re.findall(
            r"out property <length> (text-[\w-]+):\s*(?:root\.type-size\()?([\d.]+)",
            source,
        )
    }


# The type scale, so a `font-size: Theme.text-body` can be measured too.
SIZES = scale()

# A line box below this multiple of the font size is a dropped line, not a
# tight one.
MINIMUM_LINE = 1.35


def size_of(expression):
    expression = expression.strip()
    literal = re.fullmatch(r"([\d.]+)px", expression)
    return float(literal.group(1)) if literal else SIZES.get(expression)


def design_length(expression):
    """The length as designed, whatever form it is written in.

    Every length in the markup goes through `Theme.px()` so that it can be
 scaled and snapped back onto a whole pixel. The rules
    below are about the *design* value inside that call — the number somebody
    chose — so this reads it out of either form and reports nothing for an
    expression it cannot measure.
    """
    expression = expression.strip()
    literal = re.fullmatch(r"(\d+)px", expression)
    if literal:
        return int(literal.group(1))
    scaled = re.fullmatch(r"Theme\.px\((\d+)\)", expression)
    return int(scaled.group(1)) if scaled else None


def audit(path):
    findings = []
    stack, text = [], None

    for number, line in enumerate(io.open(path, encoding="utf-8").read().splitlines(), 1):
        statement = line.strip()

        if re.search(r"\) / 2;", statement) and "round(" not in statement \
                and "self.width / 2" not in statement:
            findings.append((number, "halved without rounding"))

        for match in re.finditer(
            r"\b(width|height|padding[\w-]*|spacing|border-radius):\s*"
            r"(\d+px|Theme\.px\(\d+\))",
            statement,
        ):
            value = design_length(match.group(2))
            if value is not None and value % 2 and value != 1:
                findings.append((number, "odd length %dpx" % value))

        if re.search(r"\bText \{", statement):
            text = {
                "line": number,
                "parent": stack[-1] if stack else "none",
                "size": None,
                "box": None,
                "wraps": False,
            }
            stack.append("text")
            continue
        if re.search(r"(HorizontalLayout|VerticalLayout|GridLayout) \{", statement):
            stack.append("row" if "Horizontal" in statement else "column")
            continue
        if statement.endswith("{"):
            stack.append("other")
            continue

        if text is not None:
            if statement.startswith("font-size:"):
                asked = statement.split(":", 1)[1].rstrip(";").strip()
                text["size"] = size_of(asked)
                # A name the theme does not have is the drift that made this
                # script quietly stop checking. Unmeasurable is a finding.
                if text["size"] is None:
                    findings.append((number, "type size %s is not in the theme" % asked))
            elif statement.startswith("height:"):
                text["box"] = statement
            elif statement.startswith("wrap:"):
                text["wraps"] = True

        if statement.startswith("}"):
            if stack and stack[-1] == "text" and text:
                size, box = text["size"], text["box"]
                if size and box:
                    fixed = design_length(box.split(":", 1)[1].rstrip(";"))
                    if fixed is not None and fixed < size * MINIMUM_LINE:
                        findings.append(
                            (text["line"], "box %dpx too small for %spx type"
                             % (fixed, size))
                        )
                if size and not box and not text["wraps"]:
                    findings.append((text["line"], "no line box for %spx type" % size))
                if box and text["parent"] == "row":
                    findings.append(
                        (text["line"], "boxed text standing in a horizontal layout")
                    )
                text = None
            if stack:
                stack.pop()

    return findings


def main():
    total = 0
    for path in sorted(glob.glob("crates/ui/slint/**/*.slint", recursive=True)):
        for number, what in audit(path):
            sys.stdout.write("%s:%d  %s\n" % (path.replace("\\", "/"), number, what))
            total += 1

    sys.stdout.write("%d finding%s\n" % (total, "" if total == 1 else "s"))
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
