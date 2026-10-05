#!/usr/bin/env python3
"""Builds the demo folder of the screenshots from demo.json.

    scripts/screenshots/make-demo.py <empty folder>

Files are sparse, so the folder takes almost no space; the screenshots show
apparent sizes. An entry with "text" is written with that content instead;
"{date-N}" in it becomes the time N seconds ago (ISO 8601, UTC), as in the
backups' Info.plist. An entry with "image" is a real PNG of that width and
height and pattern, for the similar images report: the same pattern at
other sizes looks alike to it. Ages are relative to now, so the date colors stay the same.
On macOS, setting a time before the creation date also moves the creation
date back, so each entry gets its creation time first and then its
modification time.
"""
import json, math, os, re, struct, sys, time, zlib

root = sys.argv[1]
spec = json.load(open(os.path.join(os.path.dirname(__file__), "demo.json")))
now = time.time()
dirs = []


def stamp(path, e):
    for age in (e["created"], e["age"]):
        os.utime(path, (now - age, now - age))

def pixel(pattern, fx, fy):
    """The color at (fx, fy) in 0..1: broad shapes, like a photo."""
    if pattern == "beach":
        inside = (fx - 0.5) ** 2 + (fy - 0.4) ** 2 < 0.04
        return (int(255 * fx), int(255 * (1 - fy)), 230 if inside else 40)
    if pattern == "portrait":
        return (200 if fx < 0.5 else 60, int(128 + 100 * math.sin(fy * 3)), int(255 * fy))
    v = 250 if (int(fx * 8) + int(fy * 8)) % 2 == 0 else 10
    return (v, 255 - v, v // 2)


def png(path, w, h, pattern):
    """An uncompressed PNG (zlib level 0), so it is large like a photo."""
    rows = b"".join(
        b"\0" + bytes(c for x in range(w) for c in pixel(pattern, x / w, y / h))
        for y in range(h)
    )
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(rows, 0)))
        f.write(chunk(b"IEND", b""))


for e in spec["entries"]:
    p = os.path.join(root, e["path"])
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if "link" in e:
        os.symlink(e["link"], p)
        t = now - e["age"]
        os.utime(p, (t, t), follow_symlinks=False)
    elif e.get("dir"):
        os.makedirs(p, exist_ok=True)
        dirs.append((p, e))
    elif "image" in e:
        i = e["image"]
        png(p, i["w"], i["h"], i["pattern"])
        stamp(p, e)
    elif "text" in e:
        text = re.sub(
            r"\{date-(\d+)\}",
            lambda m: time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now - int(m.group(1)))),
            e["text"],
        )
        with open(p, "w") as f:
            f.write(text)
        stamp(p, e)
    else:
        with open(p, "wb") as f:
            f.truncate(e["size"])
        stamp(p, e)
# Folders last, deepest first: creating their contents changed their times.
for p, e in sorted(dirs, key=lambda d: -len(d[0])):
    stamp(p, e)
