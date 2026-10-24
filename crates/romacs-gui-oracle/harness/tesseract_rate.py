#!/usr/bin/env python3
"""Exact-text recovery of Tesseract on the same captures (GUI-FLIP §6 (ii)).

usage: tesseract_rate.py SCENE_DIR CAPTURE_DIR EDITOR TESSDATA [SCALE]

Runs `tesseract IMG - -l ell+eng --psm 6` (optionally after an ffmpeg
lanczos upscale) and counts scenes whose every non-blank row is recovered
exactly.  Comparison is equivalence-aware with the same Latin/Greek
homoglyph classes the oracle uses, so Tesseract is not penalised for
choosing `A` over `Α`.
"""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

scenes, caps, editor, tessdata = sys.argv[1:5]
scale = int(sys.argv[5]) if len(sys.argv) > 5 else 1
PAIRS = "AΑ BΒ EΕ HΗ IΙ KΚ MΜ NΝ OΟ PΡ TΤ XΧ YΥ ZΖ oο"
canon = {}
for p in PAIRS.split():
    canon[p[1]] = p[0]
norm = lambda s: ''.join(canon.get(c, c) for c in s)
env = dict(os.environ, TESSDATA_PREFIX=tessdata)
exact = 0
rows_ok = rows_all = 0
n = 0
for spec_path in sorted(Path(scenes).glob('*.json')):
    spec = json.loads(spec_path.read_text())
    img = Path(caps) / editor / (spec_path.stem + '.png')
    with tempfile.TemporaryDirectory() as td:
        src = img
        if scale > 1:
            src = Path(td) / 'up.png'
            subprocess.run(['ffmpeg', '-loglevel', 'error', '-i', str(img), '-vf',
                            f'scale=iw*{scale}:ih*{scale}:flags=lanczos', str(src)], check=True)
        out = subprocess.run(['tesseract', str(src), '-', '-l', 'ell+eng', '--psm', '6'],
                             capture_output=True, text=True, env=env, check=True).stdout
    got = [l.rstrip() for l in out.splitlines() if l.strip()]
    exp = [r['text'] for r in spec['expect']['rows']]
    n += 1
    ok = len(got) == len(exp) and all(norm(a) == norm(b) for a, b in zip(got, exp))
    exact += ok
    rows_all += len(exp)
    rows_ok += sum(norm(a) == norm(b) for a, b in zip(got, exp))
    if not ok and n <= 3:
        print('  example', spec_path.stem, list(zip(exp, got))[:2])
print(f'tesseract editor={editor} scale={scale} exact-text={exact}/{n} rows={rows_ok}/{rows_all}')
