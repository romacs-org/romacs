#!/usr/bin/env python3
"""Prove the compositor checks in livecheck.py can fail. For every recorded
step, re-run check_step against its own screenshot (must pass) and against
mutated evidence (each must fail):
  blank          a black frame of the same size
  stale:<step>   the screenshot of a different step (01 and 02 are the same
                 window selection with different scroll positions)
  shifted-300px  the right screenshot sampled 300px off
  empty-bands    own screenshot with every mode line repainted in its own
                 background colour (bands coloured, text gone)
  empty-text     own screenshot with every text area painted black (bands
                 intact, buffer text gone)
  right-blank    only the right-hand windows' text erased
  digits-erased  the left window's text erased right of column 4 in its
                 top half (row labels kept, numbers gone)
  rows-rolled    the left window's top half rolled down one row: same
                 occupancy, every row shows its neighbour's text
  band-dashes    every mode line repainted with one bright dash
  swap-12        every text glyph '1' and '2' exchanged, pixels copied from
                 genuine cells of the other digit (shapes stay consistent)
  swap-sa-band   every mode-line 's' and 'a' exchanged the same way
  r-to-o         every text 'r' painted as an 'o'
  corner-blank   the top-right corner (where notifications appear) erased
  stars-dashes   every mode-line '*' drawn as that line's '-' (modified flag
                 hidden), and dashes-stars the reverse
  dash-dot       every mode-line '-' drawn as a centred dot, and star-plus
                 every '*' as a plus (round 5: both passed check_flags)
  text-swap-08   every text '0' and '8' exchanged with genuine cells
  en-dash        every mode-line '-' drawn as the font's en dash, and l-as-I
                 every 'l' as its 'I' (round 6: the en dash passed)
(the last fifteen were the reviewer's counterexamples to earlier checkers)

    mutate_check.py live/default
"""
import json, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import livecheck


GENUINE_MASK = False   # notifications drawn in the step's own screenshot


def under_mask(g):
    """A cell check_step does not judge in the genuine screenshot."""
    mx, my, mw, mh = livecheck.NOTIFICATION_MASK
    return GENUINE_MASK and mx <= g['x'] < mx + mw and my <= g['y'] < my + mh


def swap_cells(png, glyphs, roles, chars, out):
    """Repaint every glyph in `chars` with a genuine on-screen sample of its
    replacement character in the same colour (the reviewer's probe3)."""
    w, h = map(int, subprocess.check_output(['magick', 'identify', '-format', '%w %h', str(png)], text=True).split())
    raw = subprocess.check_output(['magick', str(png), '-depth', '8', 'rgb:-'])
    buf, n = bytearray(raw), 0
    for key, members in livecheck.regions(glyphs).items():
        if key[0] not in roles:
            continue
        samples = {}
        for g in members:
            samples.setdefault((g['char'], g['foreground']), g)
        for g in members:
            s = samples.get((chars.get(g['char']), g['foreground']))
            # A cell under a drawn notification is not judged, so changing it
            # proves nothing; only judged cells are mutated.
            if s is None or under_mask(g):
                continue
            x, y, sx, sy = map(int, (g['x'], g['y'], s['x'], s['y']))
            for row in range(25):
                a, b = ((y + row) * w + x) * 3, ((sy + row) * w + sx) * 3
                buf[a:a + 30] = raw[b:b + 30]
            n += 1
    subprocess.run(['magick', '-size', f'{w}x{h}', '-depth', '8', 'rgb:-', str(out)], input=bytes(buf), check=True)
    return n


def draw_glyph(png, glyphs, char, substitute, out):
    """Redraw every mode-line CHAR cell as SUBSTITUTE, rasterised from the
    font the way livecheck's references are, in the cell's own colours."""
    ref = livecheck.reference(substitute)
    alpha = [row[livecheck.REF_X:livecheck.REF_X + 10] for row in ref[livecheck.REF_TOP:livecheck.REF_TOP + 25]]
    return draw_cells(png, glyphs, char, lambda dx, dy: alpha[dy][dx], out)


def draw_cells(png, glyphs, char, lit, out):
    """Redraw every mode-line CHAR cell as the pixel pattern lit(dx, dy) in
    the cell's own colours (round 5's counterexamples: a centred dot for '-',
    a plus for '*'). Returns the number of RGB bytes changed (asserted > 0,
    so a mutant that fails to apply cannot pass as a killed one)."""
    w, h = map(int, subprocess.check_output(['magick', 'identify', '-format', '%w %h', str(png)], text=True).split())
    raw = subprocess.check_output(['magick', str(png), '-depth', '8', 'rgb:-'])
    buf = bytearray(raw)
    for g in glyphs:
        if g['role'] != 'ModeLine' or g['char'] != char or under_mask(g):
            continue
        x, y = int(g['x']), int(g['y'])
        bg, fg = bytes.fromhex(g['background'].lstrip('#')), bytes.fromhex(g['foreground'].lstrip('#'))
        for dy in range(25):
            for dx in range(10):
                at = ((y + dy) * w + x + dx) * 3
                k = float(lit(dx, dy))
                buf[at:at + 3] = bytes(round(f * k + g * (1 - k)) for f, g in zip(fg, bg))
    subprocess.run(['magick', '-size', f'{w}x{h}', '-depth', '8', 'rgb:-', str(out)], input=bytes(buf), check=True)
    return sum(a != b for a, b in zip(raw, buf))

d = Path(sys.argv[1]).resolve()
records = json.loads((d / 'results.json').read_text())
by = {r['step']: r for r in records}
stale = {'00-startup': '01-scrolled-to-point', '01-scrolled-to-point': '02-scrolled-back-to-top',
         '02-scrolled-back-to-top': '01-scrolled-to-point', '03-other-window': '02-scrolled-back-to-top',
         '04-split-below': '03-other-window', '05-delete-other-windows': '04-split-below'}
w, h = records[0]['client']['size']
tmp = d / 'mutant.tmp.png'
rows, ok = [], True
for r in records:
    glyphs = json.loads((d / r['scene']).read_text())
    bands, png = r['bands'], r['screenshot']
    livecheck.CURSORS = r['cursors']   # recorded by run.py; a missing key is an error, not "no cursor"
    own = livecheck.check_step(png, bands, glyphs)
    GENUINE_MASK = livecheck.notifications_present(png)   # for the mutators below
    results = {}
    subprocess.check_call(['magick', '-size', f'{w}x{h}', 'xc:black', str(tmp)])
    results['blank'] = livecheck.check_step(tmp, bands, glyphs)
    other = stale[r['step']]
    results['stale:' + other] = livecheck.check_step(by[other]['screenshot'], bands, glyphs)
    results['shifted-300px'] = livecheck.check_step(png, bands, glyphs, dy=300)
    draw = []
    for b in bands:
        x, y, bw, bh = b['rect']
        draw += ['-fill', '#' + b['backgrounds'][0], '-draw', f'rectangle {x},{y} {x + bw - 1},{y + bh - 1}']
    subprocess.check_call(['magick', png, *draw, str(tmp)])
    results['empty-bands'] = livecheck.check_step(tmp, bands, glyphs)
    draw = []
    for b in bands:
        x, y, bw, _ = b['rect']
        top = livecheck.window_top(bands, b)
        draw += ['-fill', 'black', '-draw', f'rectangle {x},{top} {x + bw - 1},{y - 1}']
    subprocess.check_call(['magick', png, *draw, str(tmp)])
    results['empty-text'] = livecheck.check_step(tmp, bands, glyphs)
    right = [b for b in bands if b['rect'][0] > 0]
    if right:
        draw = []
        for b in right:
            x, y, bw, _ = b['rect']
            draw += ['-fill', 'black', '-draw', f'rectangle {x},{livecheck.window_top(bands, b)} {x + bw - 1},{y - 1}']
        subprocess.check_call(['magick', png, *draw, str(tmp)])
        results['right-blank'] = livecheck.check_step(tmp, bands, glyphs)
    left = next(b for b in bands if b['rect'][0] == 0)
    half = int((left['rect'][1] - livecheck.window_top(bands, left)) // 50 * 25)
    subprocess.check_call(['magick', png, '-fill', 'black', '-draw', f'rectangle 40,0 {min(left["rect"][2], 700) - 1},{half - 1}', str(tmp)])
    results['digits-erased'] = livecheck.check_step(tmp, bands, glyphs)
    subprocess.check_call(['magick', png, '(', png, '-crop', f'700x{half}+0+0', '+repage', '-roll', '+0+25', ')',
                           '-geometry', '+0+0', '-compose', 'Over', '-composite', str(tmp)])
    results['rows-rolled'] = livecheck.check_step(tmp, bands, glyphs)
    draw = []
    for b in bands:
        x, y, bw, bh = b['rect']
        draw += ['-fill', '#' + b['backgrounds'][0], '-draw', f'rectangle {x},{y} {x + bw - 1},{y + bh - 1}',
                 '-fill', 'white', '-draw', f'rectangle {x + 10},{y + 10} {x + 209},{y + 11}']
    subprocess.check_call(['magick', png, *draw, str(tmp)])
    results['band-dashes'] = livecheck.check_step(tmp, bands, glyphs)
    for name, roles, chars in [('swap-12', {'Text'}, {'1': '2', '2': '1'}),
                               ('swap-sa-band', {'ModeLine'}, {'s': 'a', 'a': 's'}),
                               ('r-to-o', {'Text'}, {'r': 'o'}),
                               ('stars-dashes', {'ModeLine'}, {'*': '-'}),
                               ('dashes-stars', {'ModeLine'}, {'-': '*'}),
                               ('text-swap-08', {'Text'}, {'0': '8', '8': '0'})]:
        if swap_cells(png, glyphs, roles, chars, tmp):
            results[name] = livecheck.check_step(tmp, bands, glyphs)
    for name, char, lit in [('dash-dot', '-', lambda dx, dy: 4 <= dx <= 5 and 11 <= dy <= 12),
                            ('star-plus', '*', lambda dx, dy: (4 <= dx <= 5 and 5 <= dy <= 12) or (2 <= dx <= 7 and 8 <= dy <= 9))]:
        changed = draw_cells(png, glyphs, char, lit, tmp)
        assert changed > 0, f'{name} did not change {png}'
        results[name] = livecheck.check_step(tmp, bands, glyphs)
    for name, char, substitute in [('en-dash', '-', '\u2013'), ('l-as-I', 'l', 'I')]:
        if any(g['role'] == 'ModeLine' and g['char'] == char for g in glyphs):
            changed = draw_glyph(png, glyphs, char, substitute, tmp)
            assert changed > 0, f'{name} did not change {png}'
            results[name] = livecheck.check_step(tmp, bands, glyphs)
    subprocess.check_call(['magick', png, '-fill', 'black', '-draw', f'rectangle 730,0 {w - 1},329', str(tmp)])
    results['corner-blank'] = livecheck.check_step(tmp, bands, glyphs)
    passed = {k: not v for k, v in results.items()}
    livecheck.check_step(png, bands, glyphs)
    masked = livecheck.MASK_ON
    if masked:
        passed.pop('corner-blank', None)   # notifications were drawn there
    good = not own and not any(passed.values())
    ok &= good
    rows.append({'step': r['step'], 'own_failures': own, 'notification_masked': masked, 'mutant_failures': results})
    print(r['step'], 'own:', 'pass' if not own else own, '| mutants passing (must be none):',
          [k for k, v in passed.items() if v])
tmp.unlink(missing_ok=True)
(d / 'mutate_check.json').write_text(json.dumps(rows, indent=1))
print('RESULT', 'PASS' if ok else 'FAIL')
sys.exit(0 if ok else 1)
