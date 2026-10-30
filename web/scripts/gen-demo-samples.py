#!/usr/bin/env python3
"""Generate synthetic demo-precheck samples (geometric mock documents).

No real document images, no fonts, no people: flat shapes only. The
filenames and the demo page both say synthetic. Outputs land in
web/static/demo/ next to the other committed static assets.
"""
import struct
import sys
import zlib
from pathlib import Path

W, H = 480, 640


def new_canvas(rgb):
    return [[rgb] * W for _ in range(H)]


def rect(px, x0, y0, x1, y1, rgb):
    for y in range(max(0, y0), min(H, y1)):
        row = px[y]
        for x in range(max(0, x0), min(W, x1)):
            row[x] = rgb


def circle(px, cx, cy, r, rgb):
    for y in range(max(0, cy - r), min(H, cy + r)):
        for x in range(max(0, cx - r), min(W, cx + r)):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
                px[y][x] = rgb


def wash(px, amount):
    for y in range(H):
        row = px[y]
        for x in range(W):
            r, g, b = row[x]
            row[x] = (
                min(255, r + amount),
                min(255, g + amount),
                min(255, b + amount),
            )


def darken(px, amount):
    for y in range(H):
        row = px[y]
        for x in range(W):
            r, g, b = row[x]
            row[x] = (
                max(0, r - amount),
                max(0, g - amount),
                max(0, b - amount),
            )


def write_png(path, px):
    raw = b"".join(b"\x00" + b"".join(bytes(p) for p in row) for row in px)

    def chunk(typ, data):
        c = typ + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))

    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(png)


def base_doc():
    px = new_canvas((245, 243, 238))
    rect(px, 40, 40, W - 40, H - 40, (255, 255, 255))  # page
    rect(px, 40, 40, W - 40, 44, (11, 74, 117))  # top rule
    rect(px, 70, 90, W - 70, 116, (200, 210, 220))  # header bar
    y = 150
    while y < 400:  # text lines
        rect(px, 70, y, W - 70 - ((y // 30) % 3) * 60, y + 10, (210, 215, 222))
        y += 30
    rect(px, 70, 430, 190, 550, (170, 190, 205))  # photo box
    circle(px, 130, 490, 34, (140, 160, 180))  # photo placeholder head
    circle(px, W - 150, 490, 52, (31, 110, 163))  # seal
    circle(px, W - 150, 490, 40, (255, 255, 255))
    circle(px, W - 150, 490, 28, (31, 110, 163))
    return px


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("web/static/demo")
    out.mkdir(parents=True, exist_ok=True)

    write_png(out / "syn-doc-clean.png", base_doc())

    dark = base_doc()
    darken(dark, 90)  # underexposed proxy
    rect(dark, 0, 0, W, 120, (20, 20, 20))  # cropped top (missing header)
    rect(dark, 70, 90, W - 70, 116, (20, 20, 20))
    write_png(out / "syn-doc-dark.png", dark)

    washed = base_doc()
    wash(washed, 60)  # low-contrast proxy
    rect(washed, W - 210, 438, W - 90, 542, (245, 243, 238))  # seal obscured
    write_png(out / "syn-doc-wash.png", washed)

    for f in sorted(out.glob("syn-doc-*.png")):
        print(f, f.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
