"""Renders Cadenza's window into a bitmap, without touching the screen.

    python scripts/shoot.py out.png [--section equaliser] [--wait 6]

Why a script and not a screenshot: `PrintWindow` asks the window to draw itself
into a device context we own. Nothing is captured from the desktop, so the
result does not depend on what is in front, whether the machine is locked, or
where the pointer happens to be. It is the window's own drawing, which is the
only thing worth checking.

What it cannot show is everything that needs a live pointer — hover, drag,
focus, the click itself. Those stay unverified and are said to be.

`--section` rewrites the shell's default page for the length of one render and
puts it back afterwards, because navigating would need a click.
"""

import argparse
import ctypes
import ctypes.wintypes as wintypes
import io
import os
import re
import struct
import subprocess
import sys
import time

TITLE = "Cadenza"
SHELL = "crates/ui/slint/components/AppShell.slint"
SECTION = re.compile(r'(property <string> section: ")([a-z]+)(";)')

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32

# PrintWindow with PW_RENDERFULLCONTENT: without it a window drawn by a GPU
# backend comes back blank.
PW_RENDERFULLCONTENT = 2


def find_window(pid, deadline):
    """The top-level window belonging to `pid`, once it has one."""
    found = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def visit(handle, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(handle, ctypes.byref(owner))
        if owner.value != pid or not user32.IsWindowVisible(handle):
            return True
        length = user32.GetWindowTextLengthW(handle)
        text = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(handle, text, length + 1)
        if TITLE in text.value:
            found.append(handle)
            return False
        return True

    while time.time() < deadline:
        user32.EnumWindows(visit, 0)
        if found:
            return found[0]
        time.sleep(0.25)
    return None


def capture(handle):
    """The window's own pixels, as (width, height, BGRA rows top-down)."""
    rect = wintypes.RECT()
    user32.GetWindowRect(handle, ctypes.byref(rect))
    width, height = rect.right - rect.left, rect.bottom - rect.top

    window_dc = user32.GetWindowDC(handle)
    memory_dc = gdi32.CreateCompatibleDC(window_dc)
    bitmap = gdi32.CreateCompatibleBitmap(window_dc, width, height)
    gdi32.SelectObject(memory_dc, bitmap)

    if not user32.PrintWindow(handle, memory_dc, PW_RENDERFULLCONTENT):
        raise RuntimeError("the window refused to draw itself")

    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [
            ("biSize", wintypes.DWORD),
            ("biWidth", ctypes.c_long),
            ("biHeight", ctypes.c_long),
            ("biPlanes", wintypes.WORD),
            ("biBitCount", wintypes.WORD),
            ("biCompression", wintypes.DWORD),
            ("biSizeImage", wintypes.DWORD),
            ("biXPelsPerMeter", ctypes.c_long),
            ("biYPelsPerMeter", ctypes.c_long),
            ("biClrUsed", wintypes.DWORD),
            ("biClrImportant", wintypes.DWORD),
        ]

    header = BITMAPINFOHEADER()
    header.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    header.biWidth = width
    # Negative height asks for the rows the way everything else reads them:
    # top first.
    header.biHeight = -height
    header.biPlanes = 1
    header.biBitCount = 32
    header.biCompression = 0

    pixels = ctypes.create_string_buffer(width * height * 4)
    gdi32.GetDIBits(memory_dc, bitmap, 0, height, pixels, ctypes.byref(header), 0)

    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(memory_dc)
    user32.ReleaseDC(handle, window_dc)

    return width, height, pixels.raw


def write_png(path, width, height, bgra):
    """Writes a PNG without a library, because one line of zlib is enough."""
    import zlib

    rows = bytearray()
    for y in range(height):
        rows.append(0)  # no filter on this row
        start = y * width * 4
        for x in range(width):
            b, g, r, _ = bgra[start + x * 4 : start + x * 4 + 4]
            rows += bytes((r, g, b))

    def chunk(kind, payload):
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    with io.open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n")
        out.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        out.write(chunk(b"IDAT", zlib.compress(bytes(rows), 6)))
        out.write(chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("output")
    parser.add_argument("--section", help="the page to open the window on")
    parser.add_argument("--wait", type=float, default=8.0)
    arguments = parser.parse_args()

    original = None
    if arguments.section:
        original = io.open(SHELL, encoding="utf-8").read()
        patched, changed = SECTION.subn(
            lambda m: m.group(1) + arguments.section + m.group(3), original, count=1
        )
        # Counted rather than compared: asking for the page that is already the
        # default rewrites it to itself, and a file that came back identical is
        # not a file that could not be found.
        if not changed:
            sys.exit("could not find the shell's default section to change")
        io.open(SHELL, "w", encoding="utf-8", newline="\n").write(patched)

    try:
        subprocess.run(
            ["cargo", "build", "-q", "-p", "cadenza-app"], check=True, shell=False
        )
        binary = os.path.join("target", "debug", "cadenza.exe")
        application = subprocess.Popen([binary])
        try:
            handle = find_window(application.pid, time.time() + arguments.wait)
            if handle is None:
                sys.exit("the window never appeared")
            # A moment more: the first frame is drawn before the library is.
            time.sleep(1.5)
            width, height, pixels = capture(handle)
            write_png(arguments.output, width, height, pixels)
            sys.stdout.write("%s  %dx%d\n" % (arguments.output, width, height))
        finally:
            application.terminate()
            application.wait(timeout=10)
    finally:
        if original is not None:
            io.open(SHELL, "w", encoding="utf-8", newline="\n").write(original)


if __name__ == "__main__":
    main()
