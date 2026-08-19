"""Give a bundled font a family name no machine will already have.

Cadenza ships its faces inside the binary so that it looks the same everywhere.
That is not enough on its own: Slint asks fontdb for a family by name, fontdb
gathers every face with that name — the system's copies were loaded first — and
picks the best match by weight, breaking a tie in favour of whichever it saw
first. So a listener who happens to have Cormorant Garamond installed would see
*their* copy, of whatever version, rather than the one this interface was drawn
against.

The fix is to ask for something only we have. This rewrites a font's `name`
table so the family becomes "Cadenza Serif", "Cadenza Sans" or "Cadenza Mono".
Nothing else about the file changes: not one outline, not one metric, not one
kerning pair. All three families are SIL OFL 1.1 with no Reserved Font Name, so
the rename is permitted; the licence ships beside the result, and each folder
carries a note saying what was done.

    python scripts/rename_font.py in.ttf out.ttf --family "Cadenza Serif" \
        --subfamily Italic

Run it again whenever a face is updated, rather than editing a binary by hand.
"""

import argparse
import struct

# Name IDs that carry a name rather than a description. Everything else in the
# table — the designer, the licence, the sample text — is left exactly as it is.
FAMILY = 1
SUBFAMILY = 2
UNIQUE_ID = 3
FULL_NAME = 4
POSTSCRIPT = 6
TYPO_FAMILY = 16
TYPO_SUBFAMILY = 17


def tables(data):
    """The table directory, as `tag -> bytes`."""
    count = struct.unpack(">H", data[4:6])[0]
    found = {}
    for index in range(count):
        entry = 12 + 16 * index
        tag = data[entry : entry + 4]
        offset, length = struct.unpack(">II", data[entry + 8 : entry + 16])
        found[tag] = data[offset : offset + length]
    return found


def checksum(data):
    padded = data + b"\0" * ((-len(data)) % 4)
    total = 0
    for index in range(0, len(padded), 4):
        total = (total + struct.unpack(">I", padded[index : index + 4])[0]) & 0xFFFFFFFF
    return total


def read_names(name_table):
    """Every record as `(platform, encoding, language, name id, bytes)`."""
    count, storage = struct.unpack(">HH", name_table[2:6])
    records = []
    for index in range(count):
        entry = 6 + 12 * index
        platform, encoding, language, name_id, length, offset = struct.unpack(
            ">HHHHHH", name_table[entry : entry + 12]
        )
        value = name_table[storage + offset : storage + offset + length]
        records.append((platform, encoding, language, name_id, value))
    return records


def write_names(records):
    """A format 0 name table. Identical strings are stored once."""
    storage = b""
    offsets = {}
    entries = b""
    for platform, encoding, language, name_id, value in records:
        if value not in offsets:
            offsets[value] = len(storage)
            storage += value
        entries += struct.pack(
            ">HHHHHH",
            platform,
            encoding,
            language,
            name_id,
            len(value),
            offsets[value],
        )
    header = struct.pack(">HHH", 0, len(records), 6 + 12 * len(records))
    return header + entries + storage


def encode(text, platform):
    return text.encode("utf-16-be") if platform == 3 else text.encode("latin-1")


def renamed(records, family, subfamily):
    """The same records, with every name that is a name replaced."""
    postscript = f"{family.replace(' ', '')}-{subfamily.replace(' ', '')}"
    full = family if subfamily == "Regular" else f"{family} {subfamily}"

    replacements = {
        FAMILY: family,
        SUBFAMILY: subfamily,
        UNIQUE_ID: f"{full}; Cadenza",
        FULL_NAME: full,
        POSTSCRIPT: postscript,
        # Dropped rather than rewritten: a typographic family is what makes a
        # weight hide behind the name of its family, which is the whole problem
        # being solved here. One file, one family, one name.
        TYPO_FAMILY: None,
        TYPO_SUBFAMILY: None,
    }

    out = []
    for platform, encoding, language, name_id, value in records:
        if name_id not in replacements:
            out.append((platform, encoding, language, name_id, value))
            continue
        text = replacements[name_id]
        if text is None:
            continue
        out.append((platform, encoding, language, name_id, encode(text, platform)))
    return out


def build(font, name_table):
    """A whole sfnt again, with tables in tag order and offsets recomputed."""
    font = dict(font)
    font[b"name"] = name_table

    tags = sorted(font)
    count = len(tags)
    selector = max(count.bit_length() - 1, 0)
    search = (1 << selector) * 16

    directory = b""
    body = b""
    offset = 12 + 16 * count
    for tag in tags:
        data = font[tag]
        directory += tag + struct.pack(">III", checksum(data), offset, len(data))
        padding = b"\0" * ((-len(data)) % 4)
        body += data + padding
        offset += len(data) + len(padding)

    header = struct.pack(">IHHHH", 0x00010000, count, search, selector, count * 16 - search)
    out = bytearray(header + directory + body)

    # `head.checkSumAdjustment` is a checksum of the whole file, so it can only
    # be written once the whole file exists — and it is computed with its own
    # four bytes zeroed.
    head = 12 + 16 * tags.index(b"head")
    head_offset = struct.unpack(">I", bytes(out[head + 8 : head + 12]))[0]
    out[head_offset + 8 : head_offset + 12] = b"\0\0\0\0"
    adjustment = (0xB1B0AFBA - checksum(bytes(out))) & 0xFFFFFFFF
    out[head_offset + 8 : head_offset + 12] = struct.pack(">I", adjustment)
    return bytes(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source")
    parser.add_argument("destination")
    parser.add_argument("--family", required=True)
    parser.add_argument("--subfamily", default="Regular")
    arguments = parser.parse_args()

    data = open(arguments.source, "rb").read()
    if data[:4] != b"\x00\x01\x00\x00":
        raise SystemExit(f"{arguments.source} is not a TrueType outline font")

    font = tables(data)
    records = renamed(read_names(font[b"name"]), arguments.family, arguments.subfamily)
    open(arguments.destination, "wb").write(build(font, write_names(records)))
    print(f"{arguments.destination}  {arguments.family} {arguments.subfamily}")


if __name__ == "__main__":
    main()
