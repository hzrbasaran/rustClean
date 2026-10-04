#!/usr/bin/env python3
"""Builds the demo folder of the screenshots from demo.json.

    scripts/screenshots/make-demo.py <empty folder>

Files are sparse, so the folder takes almost no space; the screenshots show
apparent sizes. Ages are relative to now, so the date colors stay the same.
On macOS, setting a time before the creation date also moves the creation
date back, so each entry gets its creation time first and then its
modification time.
"""
import json, os, sys, time

root = sys.argv[1]
spec = json.load(open(os.path.join(os.path.dirname(__file__), "demo.json")))
now = time.time()
dirs = []


def stamp(path, e):
    for age in (e["created"], e["age"]):
        os.utime(path, (now - age, now - age))

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
    else:
        with open(p, "wb") as f:
            f.truncate(e["size"])
        stamp(p, e)
# Folders last, deepest first: creating their contents changed their times.
for p, e in sorted(dirs, key=lambda d: -len(d[0])):
    stamp(p, e)
