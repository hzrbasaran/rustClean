#!/usr/bin/env python3
"""Runs rustclean in a pseudo-terminal, sends keys, and writes the final
screen as an SVG. Used by make-shots.sh; needs `pyte`.

    svgshot.py <out.svg> <keys> <command…>

Keys are comma-separated: characters, or ENTER, DOWN, UP, LEFT, RIGHT, TAB,
ESC, SPACE, BACKSPACE, QM (`?`), SLEEPn (wait n seconds). Environment: ROWS,
COLS, WAIT (seconds after a key), START (seconds before the first key),
LIGHT=1 (a white terminal), REPLACE ("prefix=>text||…": shortens paths).
"""
import os, pty, sys, time, select, fcntl, termios, struct, html
import pyte
ROWS, COLS = int(os.environ.get("ROWS", 32)), int(os.environ.get("COLS", 120))
WAIT = float(os.environ.get("WAIT", 1.2))
out_path, keys, args = sys.argv[1], sys.argv[2], sys.argv[3:]
REPLACE = [tuple(p.split("=>", 1)) for p in os.environ.get("REPLACE", "").split("||") if "=>" in p]
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv(args[0], args)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
screen = pyte.Screen(COLS, ROWS); stream = pyte.ByteStream(screen)
def pump(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try: stream.feed(os.read(fd, 1 << 20))
            except OSError: return
pump(float(os.environ.get("START", 3)))
names = {"ENTER": b"\r", "DOWN": b"\x1b[B", "UP": b"\x1b[A", "LEFT": b"\x1b[D", "RIGHT": b"\x1b[C",
         "TAB": b"\t", "ESC": b"\x1b", "SPACE": b" ", "BACKSPACE": b"\x7f", "QM": b"?"}
for k in [k for k in keys.split(",") if k]:
    if k.startswith("SLEEP"):
        pump(float(k[5:])); continue
    os.write(fd, names.get(k, k.encode())); pump(WAIT)
os.write(fd, b"q"); pump(0.3)

# A dark theme for the 16 named colors; 256-color/hex values pass through.
NAMED = {"black": "#1e1e2e", "red": "#f38ba8", "green": "#a6e3a1", "yellow": "#f9e2af", "blue": "#89b4fa",
         "magenta": "#cba6f7", "cyan": "#94e2d5", "white": "#cdd6f4", "brightblack": "#585b70",
         "brightred": "#f38ba8", "brightgreen": "#a6e3a1", "brightyellow": "#f9e2af", "brightblue": "#89b4fa",
         "brightmagenta": "#f5c2e7", "brightcyan": "#89dceb", "brightwhite": "#ffffff", "brown": "#f9e2af"}
BG, FG = "#11111b", "#cdd6f4"
# LIGHT=1: a white terminal (for the light theme), with xterm's named colors.
if os.environ.get("LIGHT"):
    BG, FG = "#ffffff", "#1d1d1f"
    NAMED.update({"black": "#000000", "red": "#cd0000", "green": "#00a000", "yellow": "#a07000",
                  "blue": "#0000ee", "magenta": "#cd00cd", "cyan": "#008b8b", "white": "#e5e5e5",
                  "brightblack": "#7f7f7f", "brightwhite": "#ffffff"})
def color(c, default):
    if c == "default": return default
    if c in NAMED: return NAMED[c]
    if len(c) == 6: return "#" + c
    return default
CW, CH = 9.0, 19.0
w, h = COLS * CW, ROWS * CH
parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{w+24:.0f}" height="{h+24:.0f}" viewBox="0 0 {w+24:.0f} {h+24:.0f}">',
         f'<rect width="100%" height="100%" rx="8" fill="{BG}"/>',
         '<g transform="translate(12,12)" font-family="SFMono-Regular,Menlo,Consolas,monospace" font-size="15">']
def style(c):
    bg = color(c.bg, BG) if not c.reverse else color(c.fg, FG)
    fg = color(c.fg, FG) if not c.reverse else color(c.bg, BG)
    return (bg, fg, c.bold)
for y in range(ROWS):
    row = screen.buffer[y]
    cells = [(row[x].data, style(row[x])) for x in range(COLS)]
    # One character per cell (the second half of a wide character is empty),
    # so positions in `line` are positions in `cells`.
    line = "".join(c[0] or "\0" for c in cells)
    # Shorten long paths, shifting the rest of the row left.
    # REPLACE: "prefix=>text" replaces from prefix up to the next "  ", "│" or
    # the end of the line (paths are often cut off by the terminal width).
    for a, b in REPLACE:
        i = line.find(a)
        if i >= 0:
            ends = [j for j in (line.find("  ", i), line.find("│", i)) if j > i]
            end = min(ends) if ends else COLS
            st = cells[i][1]
            cells = cells[:i] + [(ch, st) for ch in b] + cells[end:]
            cells += [(" ", cells[-1][1])] * (COLS - len(cells))
            line = "".join(c[0] or "\0" for c in cells)
    x = 0
    while x < COLS:
        st = cells[x][1]
        x2 = x + 1
        while x2 < COLS and cells[x2][1] == st:
            x2 += 1
        bg, fg, bold = st
        text = "".join(c[0] for c in cells[x:x2])
        if bg != BG:
            parts.append(f'<rect x="{x*CW:.1f}" y="{y*CH:.1f}" width="{(x2-x)*CW:.1f}" height="{CH:.1f}" fill="{bg}"/>')
        if text.strip():
            weight = ' font-weight="bold"' if bold else ''
            parts.append(f'<text x="{x*CW:.1f}" y="{y*CH+14:.1f}" fill="{fg}"{weight} xml:space="preserve" textLength="{(x2-x)*CW:.1f}" lengthAdjust="spacingAndGlyphs">{html.escape(text)}</text>')
        x = x2
parts.append("</g></svg>")
open(out_path, "w").write("\n".join(parts))
print("wrote", out_path)
