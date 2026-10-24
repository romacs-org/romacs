#!/usr/bin/env python3
"""Generate scene specs for oracle validation.

`corpus.py OUTDIR` writes 20 deterministic scenes (seeded): 3-4 rows each of
mixed Latin and Greek words, foreground colours, bold/italic/underline runs,
background runs, and the cursor at a varying position.  Rows stay inside one
line of the frame, so wrapping is exercised by its own hand-written scene.

The `buffer` half drives the harness; the `expect` half is derived here from
the same declaration, by rules that do not depend on any renderer: one
column per character, rows split at newlines, point -> (row, col).
"""
import json
import random
import sys
from pathlib import Path

FONT = {
    "family": "Liberation Mono",
    "pixel_size": 20,
    "regular": "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
    "bold": "/usr/share/fonts/liberation/LiberationMono-Bold.ttf",
    "italic": "/usr/share/fonts/liberation/LiberationMono-Italic.ttf",
}
LATIN = ["Hello", "world", "Emacs", "buffer", "window", "frame", "glyph", "cursor",
         "Quick", "brown", "fox", "jumps", "over", "lazy", "dog", "R2D2", "x=42;",
         "(car)", "[0-9]+", "a&b", "#if", "@user", "{ok}", "50%", "q?", "C:\\tmp"]
GREEK = ["Ωμέγα", "καλημέρα", "κόσμε", "Αθήνα", "λόγος", "ψυχή", "ζωή", "φως",
         "ΣΟΦΙΑ", "θάλασσα", "ήλιος", "δέντρο", "ξένος", "Χάος", "ΔΕΛΤΑ", "βιβλίο"]
PALETTE = ["#ff0000", "#0000ff", "#008000", "#a020f0", "#b8860b", "#ff8c00",
           "#2e8b57", "#8b0000", "#1e90ff", "#696969"]
BGS = ["#ffff00", "#e0e0ff", "#ffe4e1", "#d0ffd0"]


def scene(i, rng):
    rows = []
    segs = []
    for _ in range(rng.randint(3, 4)):
        row_segs = []
        width = 0
        while True:
            word = rng.choice(LATIN if rng.random() < 0.5 else GREEK)
            sep = " " if row_segs else ""
            if width + len(sep) + len(word) > 36:
                break
            if sep:
                row_segs.append({"text": sep})
                width += 1
            seg = {"text": word}
            r = rng.random()
            if r < 0.35:
                seg["fg"] = rng.choice(PALETTE)
            if rng.random() < 0.15:
                seg["bold"] = True
            elif rng.random() < 0.12:
                seg["italic"] = True
            if rng.random() < 0.10:
                seg["underline"] = True
            if rng.random() < 0.08:
                seg["bg"] = rng.choice(BGS)
            row_segs.append(seg)
            width += len(word)
        rows.append(row_segs)
    for k, row in enumerate(rows):
        segs.extend(row)
        segs.append({"text": "\n"})
    # Expected rows and runs, by column.
    exp_rows = []
    for row in rows:
        text = ""
        runs = []
        for seg in row:
            start = len(text)
            text += seg["text"]
            attrs = {k: v for k, v in seg.items() if k != "text"}
            if attrs:
                runs.append(dict(start=start, end=len(text), **attrs))
        exp_rows.append({"text": text, "runs": runs})
    # Point: a random position on a random row, possibly end of line.
    prow = rng.randrange(len(exp_rows))
    pcol = rng.randint(0, len(exp_rows[prow]["text"]))
    point = 1 + sum(len(r["text"]) + 1 for r in exp_rows[:prow]) + pcol
    return {
        "name": f"corpus-{i:02d}",
        "frame": {"width": 480, "height": 168, "bg": "#ffffff", "fg": "#000000"},
        "font": FONT,
        "cursor_color": "#00a000",
        "buffer": segs,
        "point": point,
        "expect": {"origin": [0, 0], "rows": exp_rows,
                   "cursor": {"row": prow, "col": pcol}},
    }


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(107)
    for i in range(20):
        (out / f"corpus-{i:02d}.json").write_text(
            json.dumps(scene(i, rng), ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
