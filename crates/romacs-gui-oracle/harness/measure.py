#!/usr/bin/env python3
"""Measure read-back quality of `romacs-gui-oracle read` over captured scenes.

usage: measure.py ORACLE_BIN SCENE_DIR CAPTURE_DIR EDITOR

Prints per-scene exact-text recovery and the distributions that set the
checker's tolerances: foreground/background colour error, glyph offsets,
style and underline agreement, cursor cell.
"""
import collections
import json
import subprocess
import sys
from pathlib import Path

oracle, scenes, caps, editor = sys.argv[1:5]


def hexrgb(s):
    return tuple(int(s[i:i + 2], 16) for i in (1, 3, 5))


def dist(a, b):
    return max(abs(x - y) for x, y in zip(hexrgb(a), hexrgb(b)))


fg_err = collections.Counter()
bg_err = collections.Counter()
offs = collections.Counter()
style = collections.Counter()
under = collections.Counter()
cursor = collections.Counter()
exact = 0
cells_ok = cells_all = 0
mismatches = []
low_margin = []
n = 0
classes = None
style_bad = []
fg_bad = []
for spec_path in sorted(Path(scenes).glob('*.json')):
    spec = json.loads(spec_path.read_text())
    cap = Path(caps) / editor / (spec_path.stem + '.png')
    p = subprocess.run([oracle, 'read', str(spec_path), str(cap)], capture_output=True,
                       text=True, check=True)
    g = json.loads(p.stdout)
    if classes is None:
        classes = json.loads(subprocess.run([oracle, 'equiv', str(spec_path)], capture_output=True,
                                            text=True, check=True).stdout)
    same = lambda a, b: a == b or any(a in k and b in k for k in classes)
    n += 1
    cols = g['cols']
    cell = lambda r, c: g['cells'][r * cols + c]
    rows = spec['expect']['rows']
    cur = spec['expect']['cursor']
    ok_scene = True
    for r in range(g['rows']):
        exp = rows[r]['text'] if r < len(rows) else ''
        got = ''.join(cell(r, c)['ch'] for c in range(cols)).rstrip()
        if len(got) != len(exp) or not all(same(a, b) for a, b in zip(got, exp)):
            ok_scene = False
            mismatches.append((spec_path.stem, r, exp, got))
        runs = rows[r]['runs'] if r < len(rows) else []
        for c in range(cols):
            ch = exp[c] if c < len(exp) else ' '
            ce = cell(r, c)
            cells_all += 1
            cells_ok += same(ce['ch'], ch)
            attrs = {}
            for run in runs:
                if run['start'] <= c < run['end']:
                    attrs.update(run)
            is_cur = (r, c) == (cur['row'], cur['col'])
            want_bg = spec['cursor_color'] if is_cur else attrs.get('bg', spec['frame']['bg'])
            want_fg = attrs.get('bg', spec['frame']['bg']) if is_cur else attrs.get('fg', spec['frame']['fg'])
            bg_err[dist(ce['bg'], want_bg)] += 1
            if is_cur:
                cursor['cursor cell bg ok' if dist(ce['bg'], want_bg) <= 2 else 'cursor cell bg BAD'] += 1
            elif dist(ce['bg'], spec['cursor_color']) <= 2:
                cursor['stray cursor-coloured cell'] += 1
            if ch != ' ' and ce['ch'] != ' ':
                fg_err[dist(ce['fg'], want_fg)] += 1
                if dist(ce['fg'], want_fg) > 24:
                    fg_bad.append((dist(ce['fg'], want_fg), spec_path.stem, r, c, ch, ce['style'], want_fg, ce['fg']))
                offs[(ce['dx'], ce['dy'])] += 1
                want_style = 'Bold' if attrs.get('bold') else 'Italic' if attrs.get('italic') else 'Regular'
                style[(want_style, ce['style'])] += 1
                if want_style != ce['style']:
                    style_bad.append((spec_path.stem, r, c, ch, want_style, ce['style'], ce['mass_ratio']))
                if ce['runner_up']:
                    low_margin.append((round(ce['score'] - ce['runner_up'][1], 3), ce['ch'],
                                       ce['runner_up'][0], spec_path.stem, r, c))
            under[(bool(attrs.get('underline')) and ch != ' ' or bool(attrs.get('underline')),
                   ce['underline'])] += 1
    exact += ok_scene

print('classes:', ''.join('/'.join(k) + ' ' for k in classes))
print('style misreads:', style_bad)
fg_bad.sort(reverse=True)
print('fg errors > 24:', len(fg_bad), fg_bad[:10])
print(f'editor={editor} scenes={n} exact-text(equivalence-aware)={exact}/{n} cells={cells_ok}/{cells_all}')
print('fg error (max channel) histogram:', sorted(fg_err.items()))
print('bg error histogram:', sorted(bg_err.items()))
print('glyph offsets (dx,dy):', sorted(offs.items()))
print('style (declared, read):', sorted(style.items()))
print('underline (declared, read):', sorted(under.items()))
print('cursor:', dict(cursor))
print('text mismatches:')
for m in mismatches:
    print('  ', m)
low_margin.sort()
print('smallest margins (score - runner-up):', low_margin[:12])
