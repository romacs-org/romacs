#!/usr/bin/env python3
"""Real-compositor walk of the romacs GUI on the owner's Hyprland session.

Launches the owner's command (scripts/romacs) on workspace 2 (HDMI-A-1, never
DP-1) without stealing the workspace, drives it with real Wayland key events
(wtype), and captures compositor pixels (grim) plus the GUI's own painted-glyph
dump (--trace-scene) for every step. Focus is returned to the owner's window
at the end, and the GUI is terminated.

    run.py OUT_DIR [--gnu-lisp]
"""
import json, os, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]  # worktree root
out = Path(sys.argv[1]).resolve()
gnu = '--gnu-lisp' in sys.argv
out.mkdir(parents=True, exist_ok=True)
(out / "home").mkdir(exist_ok=True)
env = os.environ.copy()
env.pop('LD_LIBRARY_PATH', None)
for required in ('XDG_RUNTIME_DIR', 'WAYLAND_DISPLAY', 'HYPRLAND_INSTANCE_SIGNATURE'):
    if not env.get(required):
        raise RuntimeError(f'{required} must select the intended live session')
MONITOR = os.environ['ROMACS_LIVE_MONITOR']
WORKSPACE = int(os.environ['ROMACS_LIVE_WORKSPACE'])
log = out / 'romacs.log'


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
args = ' --gnu-lisp' if gnu else ''
# The owner's command, exactly; the state dir only moves the log here.
cmd = __import__('shlex').join(['env', f'HOME={out}/home', f'XDG_STATE_HOME={out}/state', str(ROOT / 'scripts/romacs'), '--trace-scene', *(['--gnu-lisp'] if gnu else [])])
hy('eval', f'hl.exec_cmd({json.dumps(cmd)}, {{workspace="{WORKSPACE} silent"}})')
records, pid = [], None
addr = None


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
    time.sleep(0.25)


def text(s):
    safe()
    run('wtype', '-d', '30', s)
    time.sleep(0.3)


def scene():
    """Last painted scene: rows of (role, y) -> text, from the GUI's own dump."""
    state = out / 'state' / 'romacs' / 'romacs.log'
    last = None
    for line in state.read_text(errors='replace').splitlines():
        if '"submitted-scene"' in line:
            last = line
    if not last:
        return {}
    rows = {}
    for g in json.loads(last)['glyphs']:
        rows.setdefault((g['role'], g['y']), []).append((g['x'], g['char']))
    return {f'{r}@{y}': ''.join(c for _, c in sorted(cs)) for (r, y), cs in sorted(rows.items(), key=lambda kv: kv[0][1])}


def cursor_pixels(p):
    """Pixels in the Rum cursor colour #9099d9: the cursor blinks, so a single
    capture can land in its off phase."""
    return float(run('magick', str(p), '-fx',
                     'abs(r-0.565)<0.08 && abs(g-0.6)<0.08 && abs(b-0.851)<0.08 ? 1 : 0',
                     '-format', '%[fx:mean*w*h]', 'info:'))


def shot(name, note):
    time.sleep(0.4)
    c = safe()
    x, y = c['at']
    w, h = c['size']
    # Five captures across more than one blink period; keep them all, and
    # name the one showing the most cursor pixels as the step's screenshot.
    frames = []
    for i in range(5):
        q = out / f'{name}.{i}.png'
        run('grim', '-g', f'{x},{y} {w}x{h}', str(q))
        frames.append((cursor_pixels(q), str(q)))
        time.sleep(0.25)
    best = max(frames)[1]
    p = out / f'{name}.png'
    p.write_bytes(Path(best).read_bytes())
    rec = {'step': name, 'note': note, 'screenshot': str(p), 'burst': frames,
           'client': {k: c[k] for k in ('at', 'size', 'title', 'pid')}, 'scene': scene()}
    records.append(rec)
    (out / 'results.json').write_text(json.dumps(records, indent=2, ensure_ascii=False))
    return rec


try:
    deadline = time.monotonic() + 300  # includes a no-op or real build
    own = []
    while time.monotonic() < deadline:
        own = [c for c in clients() if c['class'] == 'romacs' and c['pid'] not in before]
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
    shot('00-window', 'scripts/romacs opened a window')
    if not gnu:
        text('hello κόσμε λ')
        shot('01-typed-multibyte', 'self-insert of ASCII, Greek')
        keys('a', mod='ctrl'); keys('f', mod='ctrl'); keys('f', mod='ctrl'); keys('f', mod='ctrl'); text('X')
        keys('e', mod='ctrl'); text('Y'); keys('b', mod='ctrl'); keys('b', mod='ctrl'); text('Z')
        shot('02-motion', 'C-a C-f C-f C-f X, C-e Y, C-b C-b Z: expect helXlo κόσμε ZλY')
        keys('e', mod='ctrl'); keys('Return'); text('(car 1)'); keys('x', 'e', mod='ctrl')
        shot('03-error-echo', 'C-x C-e on (car 1): echo shows GNU text')
        text(' '); keys('f', mod='ctrl')
        shot('04-end-of-buffer', 'next key cleared the message; C-f at end signals end-of-buffer')
        keys('Return'); text('(+ 20 22)'); keys('x', 'e', mod='ctrl')
        shot('05-still-alive', 'session survives: next evaluation works')
        # fcitx5's unicode picker commits through text-input-v3 (IME), not keysyms.
        keys('Return')
        safe(); run('wtype', '-M', 'ctrl', '-M', 'alt', '-M', 'shift', '-k', 'u', '-m', 'shift', '-m', 'alt', '-m', 'ctrl')
        time.sleep(0.5); text('3bb'); time.sleep(0.8)
        shot('06-fcitx5-unicode-picker', 'fcitx5 unicode addon C-M-S-u 3bb: the IME popup at point')
        keys('1'); time.sleep(0.5)
        shot('07-fcitx5-committed', 'select candidate 1: expect λ committed through text-input-v3')
    else:
        text('abc λ')
        keys('a', mod='ctrl'); keys('f', mod='ctrl'); text('X'); keys('e', mod='ctrl'); keys('b', mod='ctrl'); text('Y')
        shot('01-gnu-keymap-editing', "GNU subr.el global-map: expect aXbc Yλ")
        keys('x', mod='alt')
        shot('02-gnu-mx', 'M-x through esc-map: echo reports the missing simple.el command')
        keys('Escape'); keys('x')
        shot('03-gnu-esc-x', 'ESC x, the same binding via the ESC prefix')
        text('!')
        shot('04-gnu-continues', 'next command clears the echo area and inserts')
finally:
    (out / 'results.json').write_text(json.dumps(records, indent=2, ensure_ascii=False))
    try:
        hy('dispatch', f'hl.dsp.focus({{window="address:{original}"}})')
    except Exception as e:  # never leave the owner without focus silently
        print('focus restore failed', e)
    if pid:
        os.kill(pid, 15)
for r in records:
    print(r['step'], '|', r['note'])
    for k, v in r['scene'].items():
        if v.strip():
            print('   ', k, repr(v))
