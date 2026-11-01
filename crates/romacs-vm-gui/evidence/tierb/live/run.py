#!/usr/bin/env python3
"""Tier B real-compositor walk: GNU's mode-line engine, point-follows-scroll,
and active/inactive mode lines across C-x o / C-x 2 / C-x 1.

Same harness discipline as evidence/usable/live/run.py: launches the
command (scripts/romacs) on $ROMACS_LIVE_WORKSPACE of $ROMACS_LIVE_MONITOR without stealing
the workspace, drives it with real Wayland key events (wtype), captures
compositor pixels (grim) plus the GUI's own painted-glyph dump (--trace-scene),
returns focus to the user's window and terminates the GUI.

Every step ASSERTS against both sources: the dump says what was painted
(mode-line text and face background per window), the screenshot proves the
compositor showed it (the band's pixels have the dumped background colour at
the dumped geometry). A blank or wrong window fails either check.

    run.py OUT_DIR
"""
import json, os, re, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import livecheck

ROOT = Path(__file__).resolve().parents[5]  # worktree root
out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
(out / "home").mkdir(exist_ok=True)
env = os.environ.copy()
env.pop('LD_LIBRARY_PATH', None)
env['ROMACS_DIAGNOSTICS'] = '1'  # the per-paint lines read here
for required in ('XDG_RUNTIME_DIR', 'WAYLAND_DISPLAY', 'HYPRLAND_INSTANCE_SIGNATURE'):
    if not env.get(required):
        raise RuntimeError(f'{required} must select the intended live session')
MONITOR = os.environ['ROMACS_LIVE_MONITOR']
WORKSPACE = int(os.environ['ROMACS_LIVE_WORKSPACE'])
ACTIVE, INACTIVE = '50269a', '141414'  # startup.el mode-line / mode-line-inactive


def run(*a):
    return subprocess.check_output(a, text=True, env=env)


def hy(*a):
    return run('hyprctl', *a)


def active():
    return json.loads(hy('-j', 'activewindow'))


def clients():
    return json.loads(hy('-j', 'clients'))


original = active()['address']
before = {c['pid'] for c in clients()}
# A lane target dir, so the walk never replaces an installed binary.
target = os.environ.get('ROMACS_TARGET_DIR', str(ROOT / 'target'))
cmd = __import__('shlex').join(['env', f'HOME={out}/home', f'ROMACS_TARGET_DIR={target}', f'XDG_STATE_HOME={out}/state', str(ROOT / 'scripts/romacs'), '--trace-scene'])
hy('eval', f'hl.exec_cmd({json.dumps(cmd)}, {{workspace="{WORKSPACE} silent"}})')
records, failures, pid, addr = [], [], None, None


def safe():
    c = active()
    m = next(m for m in json.loads(hy('-j', 'monitors')) if m['id'] == c['monitor'])
    assert c['address'] == addr and m['name'] == MONITOR and c['workspace']['id'] == m['activeWorkspace']['id'], c
    return c


def keys(*chord, mod=None):
    safe()
    a = ['wtype']
    if mod:
        a += ['-M', mod]
    for k in chord:
        a += ['-k', k]
    if mod:
        a += ['-m', mod]
    run(*a)
    time.sleep(0.3)


def cx(key):
    keys('x', mod='ctrl')
    keys(key)


def text(s):
    safe()
    run('wtype', '-d', '12', s)
    time.sleep(0.3)


def last_scene():
    """The last submitted scene's glyphs and the cursors published for that
    presentation (the cursor-publication line logged just before it)."""
    state = out / 'state' / 'romacs' / 'romacs.log'
    last, cursors, pending = None, None, None
    for line in state.read_text(errors='replace').splitlines():
        if '"cursor-publication"' in line:
            pending = json.loads(line)['cursors']
        elif '"submitted-scene"' in line:
            last, cursors = line, pending
    return (json.loads(last)['glyphs'] if last else []), cursors


def bands(glyphs):
    """Painted mode lines, one per window: clip rect, text, backgrounds."""
    out = {}
    for g in glyphs:
        if g['role'] != 'ModeLine':
            continue
        c = g['clip']
        key = (c['x'], c['y'], c['width'], c['height']) if isinstance(c, dict) else tuple(c)
        b = out.setdefault(key, {'rect': key, 'chars': [], 'backgrounds': set()})
        b['chars'].append((g['x'], g['char']))
        b['backgrounds'].add(g['background'].lstrip('#').lower())
    res = []
    for b in sorted(out.values(), key=lambda b: (b['rect'][1], b['rect'][0])):
        res.append({'rect': list(b['rect']), 'text': ''.join(ch for _, ch in sorted(b['chars'])),
                    'backgrounds': sorted(b['backgrounds'])})
    return res


def text_rows(glyphs, rect):
    """Painted text rows inside the window above band `rect`, top to bottom."""
    x0, y0, w, h = rect
    rows = {}
    for g in glyphs:
        if g['role'] == 'Text' and x0 <= g['x'] < x0 + w and g['y'] < y0:
            rows.setdefault(g['y'], []).append((g['x'], g['char']))
    return [''.join(c for _, c in sorted(cs)) for _, cs in sorted(rows.items())]


def pixel(png, x, y, w, h):
    """Mean colour of a region of the compositor capture, as hex."""
    s = run('magick', str(png), '-crop', f'{int(w)}x{int(h)}+{int(x)}+{int(y)}', '+repage',
            '-scale', '1x1!', '-format', '%[hex:p{0,0}]', 'info:')
    return s.strip().lower()[:6]


def close(a, b, tol=6):
    return all(abs(int(a[i:i + 2], 16) - int(b[i:i + 2], 16)) <= tol for i in (0, 2, 4))


def check(cond, what):
    if not cond:
        failures.append(what)
    return cond


def shot(name, note, expect):
    time.sleep(0.6)
    c = safe()
    x, y = c['at']
    w, h = c['size']
    p = out / f'{name}.png'
    run('grim', '-g', f'{x},{y} {w}x{h}', str(p))
    glyphs, cursors = last_scene()
    livecheck.CURSORS = cursors
    bs = bands(glyphs)
    for b in bs:
        rx, ry, rw, rh = b['rect']
        # A strip of the band to the right of its text: band background only.
        b['compositor'] = pixel(p, rx + rw - 60, ry + 2, 40, max(rh - 4, 1))
        b['rows'] = text_rows(glyphs, b['rect'])
    (out / f'{name}.scene.json').write_text(json.dumps(glyphs, ensure_ascii=False))
    rec = {'step': name, 'note': note, 'screenshot': str(p), 'scene': f'{name}.scene.json',
           'client': {k: c[k] for k in ('at', 'size', 'title', 'pid')}, 'bands': bs, 'cursors': cursors}
    before_n = len(failures)
    for f in livecheck.check_step(p, bs, glyphs):
        check(False, 'compositor: ' + f)
    expect(bs)
    rec['failures'] = failures[before_n:]
    records.append(rec)
    (out / 'results.json').write_text(json.dumps(records, indent=2, ensure_ascii=False))
    return rec


def common(bs, n, active_index):
    check(len(bs) == n, f'{n} mode lines, got {len(bs)}')
    for i, b in enumerate(bs):
        if active_index is None:
            want = b['backgrounds'][0] if b['backgrounds'] in ([ACTIVE], [INACTIVE]) else INACTIVE
        else:
            want = ACTIVE if i == active_index else INACTIVE
        check(b['backgrounds'] == [want], f'band {i} painted {b["backgrounds"]}, want {want}')
        check(close(b['compositor'], want), f'band {i} compositor pixels {b["compositor"]}, want {want}')
        check('[stand-in: no bindings.el]' in b['text'], f'band {i} label: {b["text"]!r}')


def position(b):
    m = re.search(r'(All|Top|Bottom|\s?\d+%) L(\d+)', b['text'])
    return (m.group(1).strip(), int(m.group(2))) if m else None


LOOP = '(let ((i 1)) (while (<= i 120) (insert (format "row %d\\n" i)) (setq i (1+ i))))'
try:
    deadline = time.monotonic() + 600  # includes a no-op or real build
    own = []
    while time.monotonic() < deadline:
        # Only a window whose executable is OUR build: other lanes run their
        # own walks on this workspace, and the walk terminates what it picks.
        own = [c for c in clients() if c['class'] == 'romacs' and c['pid'] not in before
               and os.path.realpath(f"/proc/{c['pid']}/exe").startswith(os.path.realpath(target) + '/')]
        if own:
            break
        time.sleep(0.2)
    assert own, 'no romacs window appeared'
    c = own[0]
    pid, addr = c['pid'], c['address']
    assert c['workspace']['id'] == WORKSPACE, c
    time.sleep(2.0)
    hy('dispatch', f'hl.dsp.window.fullscreen({{window="address:{addr}",mode="fullscreen",state="on"}})')
    hy('dispatch', f'hl.dsp.focus({{window="address:{addr}"}})')
    time.sleep(0.8)

    def s0(bs):
        common(bs, 2, 0)
        check('*scratch*' in bs[0]['text'] and '*VM evaluations*' in bs[1]['text'], 'each band names its buffer')
        check(bs[0]['rect'][2] + bs[1]['rect'][2] <= bs[0]['rect'][2] * 2 + 2, 'side by side halves')
    shot('00-startup', 'two windows; selected band mode-line, other mode-line-inactive', s0)

    text(LOOP); keys('x', mod='ctrl'); keys('e', mod='ctrl')
    def s1(bs):
        common(bs, 2, 0)
        pos = position(bs[0])
        check(pos is not None and pos[1] >= 130, f'point line in band after 120 rows: {bs[0]["text"]!r}')
        rows = bs[0]['rows']
        check(any(r.startswith('row 120') for r in rows), f'row 120 painted after scrolling: {rows[-3:]}')
        check(not any(r.startswith('romacs / VM') for r in rows), 'top of buffer scrolled away')
    shot('01-scrolled-to-point', 'C-x C-e inserts 120 rows: window recentres on point, band shows L and %p', s1)

    text('(goto-char 1)'); keys('x', mod='ctrl'); keys('e', mod='ctrl')
    def s2(bs):
        common(bs, 2, 0)
        check(position(bs[0]) == ('Top', 1), f'band after (goto-char 1): {bs[0]["text"]!r}')
        check(bs[0]['rows'] and bs[0]['rows'][0].startswith('romacs / VM'), f'first row: {bs[0]["rows"][:1]}')
    shot('02-scrolled-back-to-top', '(goto-char 1): recentre brings line 1 back, band shows Top L1', s2)

    cx('o')
    shot('03-other-window', 'C-x o: the evaluations window is selected, the faces swap', lambda bs: common(bs, 2, 1))

    cx('2')
    def s4(bs):
        common(bs, 3, None)
        check(sum(1 for b in bs if b['backgrounds'] == [ACTIVE]) == 1, 'exactly one active band')
        check(sum('*VM evaluations*' in b['text'] for b in bs) == 2, 'split window shows the same buffer')
    shot('04-split-below', 'C-x 2 on the evaluations window: three mode lines, one active', s4)

    cx('1')
    def s5(bs):
        common(bs, 1, 0)
        check('*VM evaluations*' in bs[0]['text'], 'kept window')
    shot('05-delete-other-windows', 'C-x 1: one full-width mode line', s5)
finally:
    (out / 'results.json').write_text(json.dumps(records, indent=2, ensure_ascii=False))
    try:
        hy('dispatch', f'hl.dsp.focus({{window="address:{original}"}})')
    except Exception as e:  # never leave the user without focus silently
        print('focus restore failed', e)
    if pid:
        os.kill(pid, 15)
for r in records:
    print(r['step'], '|', r['note'], '| FAIL' if r['failures'] else '| ok')
    for b in r['bands']:
        print('   ', b['rect'], b['backgrounds'], b['compositor'], repr(b['text']))
    for f in r['failures']:
        print('    FAILED:', f)
print('RESULT', 'FAIL' if failures else 'PASS', len(failures))
sys.exit(1 if failures else 0)
