#!/usr/bin/env python3
"""The *Romacs* landing's animation in a native --loadup session, forced on.

The landing's logo is a text animation the render thread draws over the
evaluator's frames (rlisp/romacs-animate.el, render_thread/text_animations.rs).
A software rasterizer draws none, so every other harness sees a still frame;
this one sets ROMACS_LAYER_ANIMATION=1 to run the path a GPU runs.

quick: typing works right after the landing appears: C-x b *scratch* RET
and "quick" reach the scene within seconds, while the landing opens; C-x b
*Romacs* RET goes back to it.

open: shots at moments of the landing's first seconds (open-*.png) differ
from first to last.  On a real GPU (ROMACS_HARNESS_WLR_ENV) the mark is a
page scene: braille dots rain into a lit, turning R, so every shot differs
from the one before and the last shows the mark as a solid.

split: C-x 2 shows the landing in two windows, both of which must turn,
and keep turning after C-x o and after moving the cursor; C-x 1 again.

pause: SPC holds the logo still, SPC again sets it moving.

scroll: M-> then M-<, C-v then M-v, and C-u 0 C-l on the logo's fifth line
then M-<, each with the logo turning.  Under ROMACS_DIAGNOSTICS=1 the
render thread says how each window draws its animation (a scene and its
placement, cells, or hidden) whenever that changes.  Back at the top the
window must draw it as it did before: on a GPU as the scene, at the same
placement (never braille cells, never a placement left a screen above);
elsewhere as cells.  Recentred on its fifth line it is drawn the same way,
four rows up.  Then the same trip under reduced motion, where the still
frame must look as it did before scrolling: no braille, no wordmark
stranded at the top edge.  In a C-x 2 split, scrolling only the selected
window leaves the other's placement where it was, and M-< brings the
scrolled one's back.

reduced: setting `romacs-animation' to reduced while the landing turns
shows its still frame: nothing moves, and on a GPU the mark stands lit and
posed (the still frame, not the frame it reached); t again.

alpha: in the still frame, `alpha-background' 100, 90 and 70 over a
coloured backdrop (swaybg from --tools or PATH, when there is one).  The
landing has no backdrop of its own: on a GPU a row and a column just below
the wordmark, across the edges of its placement, show one page from end to
end at every opacity, and the mark stays lit; where the surface composites
alpha, the page lets the backdrop through below 100.

away: from the landing, C-x b *scratch* RET and typing "hello".  The scene the evaluator
submitted must show it, and so must the pixels: an animation left over from
the landing would paint its block over *scratch*, as it once did.

beside: C-x b *Romacs* RET, C-x 3 splits it, C-x o and C-x b *scratch* RET
in the right window, then typing "abc": the left window's logo keeps
turning while the right window's text draws.

resize: a smaller output resizes the maximized frame; the landing beside
*scratch* keeps turning at its new size.

No pointer hit may be rejected as incoherent with the presented frame, and
no presented hit may be stale (StalePresentation).

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype, grim and
magick come from --tools or PATH.  ROMACS_HARNESS_WLR_ENV (KEY=VALUE ...)
is passed to the compositor alone, to run it on a real GPU.

usage: landing_anim_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import re
import live_oracle as live

WIDTH, HEIGHT = 1600, 1000
# The backdrop behind a translucent frame: far from any theme's background.
BACKDROP = '#3a9d6e'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tools', type=Path)
    parser.add_argument('--timeout', type=float, default=120)
    a = parser.parse_args()
    a.output.mkdir(parents=True)
    report = {'binary': str(a.binary.resolve()), 'checks': [], 'failures': []}

    def check(name, ok, detail=None):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    tool = lambda name: str(a.tools / name) if a.tools else name
    with tempfile.TemporaryDirectory(prefix='landing-anim-') as temp:
        runtime = Path(temp)
        runtime.chmod(0o700)
        config = runtime / 'config'
        config.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        home = runtime / 'home'
        for d in ('config', 'cache', 'data', 'state'):
            (home / d).mkdir(parents=True)
        # *scratch*'s rows are read as text: no line-number column.
        (home / 'config' / 'romacs').mkdir()
        (home / 'config' / 'romacs' / 'init.el').write_text(live.NO_LINE_NUMBERS)
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime), 'WLR_BACKENDS': 'headless',
               'WLR_HEADLESS_OUTPUTS': '1', 'WLR_RENDERER': 'pixman', 'WLR_LIBINPUT_NO_DEVICES': '1',
               'LIBSEAT_BACKEND': 'noop', 'ROMACS_LAYER_ANIMATION': '1', 'ROMACS_DIAGNOSTICS': '1'}
        if a.tools:
            env['PATH'] = f"{a.tools}:{env['PATH']}"
        # The compositor's own renderer on a real GPU (gles2), when asked:
        # Romacs then draws with the host's GPU instead of a software one.
        wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
        stderr_path = a.output / 'binary.stderr'
        comp = keyboard = proc = backdrop = None
        try:
            with (a.output / 'compositor.log').open('w') as clog, stderr_path.open('w') as blog:
                comp = subprocess.Popen([tool('labwc'), '-C', str(config)], env=env | wlr, stdout=clog, stderr=clog)
                fio.wait(lambda: any(s.is_socket() for s in runtime.glob('wayland-*')), a.timeout, 'compositor', comp)
                env['WAYLAND_DISPLAY'] = next(s for s in runtime.glob('wayland-*') if s.is_socket()).name
                subprocess.run([tool('wlr-randr'), '--output', 'HEADLESS-1', '--custom-mode', f'{WIDTH}x{HEIGHT}'], env=env, check=True)
                if shutil.which(tool('swaybg'), path=env['PATH']):
                    backdrop = subprocess.Popen([tool('swaybg'), '-c', BACKDROP], env=env, stdout=clog, stderr=clog)
                # A seat keyboard before the window maps, so it gets focus.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=blog, stderr=blog)
                time.sleep(0.3)
                proc = subprocess.Popen([str(a.binary.resolve()), '--loadup', '--trace-scene'], env=env,
                                        stdin=subprocess.DEVNULL, stdout=blog, stderr=blog)
                # *scratch* is a programming buffer, numbered by default: rows
                # are compared without their line-number column.
                rows = lambda: [re.sub(r'^\s*\d+ ', '', r) for r in fio.rows(stderr_path.read_text(errors='replace'))]
                fio.wait(lambda: any('SPC pause' in r for r in rows()), a.timeout * 3, 'the landing', proc)

                def keys(*parts):
                    subprocess.run(['wtype', *parts], env=env, check=True, timeout=20, stdin=subprocess.DEVNULL)
                    time.sleep(0.3)

                def shot(name):
                    path = a.output / f'{name}.png'
                    subprocess.run([tool('grim'), str(path)], env=env, check=True, timeout=10)
                    return path

                def differ(x, y, region):
                    out = subprocess.run([tool('magick'), 'compare', '-metric', 'AE', f'{x}[{region}]', f'{y}[{region}]', 'null:'],
                                         capture_output=True, text=True)
                    return float(out.stderr.split()[0] or 0)

                def ink(path, region):
                    """The share of REGION's pixels brighter than the background."""
                    out = subprocess.run([tool('magick'), str(path), '-crop', region, '-colorspace', 'Gray',
                                          '-threshold', '20%', '-format', '%[fx:mean]', 'info:'],
                                         capture_output=True, text=True, check=True)
                    return float(out.stdout)

                def turning(name, *regions):
                    """The most each REGION changed between three shots 2 s apart:
                    a third of the logo's sweep, so the light crosses the
                    letters in one of them."""
                    shots = []
                    for step in 'abc':
                        shots.append(shot(f'{name}-{step}'))
                        if step != 'c':
                            time.sleep(2.0)
                    most = [max(differ(x, y, region) for x, y in zip(shots, shots[1:])) for region in regions]
                    return most[0] if len(most) == 1 else tuple(most)

                def red(path, region):
                    """The share of REGION's pixels in the mark's lit red."""
                    out = subprocess.run([tool('magick'), str(path), '-crop', region, '-fx',
                                          'r > 0.35 && r > g + 0.16 ? 1 : 0', '-format', '%[fx:mean]', 'info:'],
                                         capture_output=True, text=True, check=True)
                    return float(out.stdout)

                whole = f'{WIDTH}x{HEIGHT - 80}+0+0'

                # quick
                started = time.monotonic()
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('b')
                keys('*scratch*')
                keys('-k', 'Return')
                keys('quick')
                keys('-k', 'Return')
                fio.wait(lambda: any(r.startswith('quick') for r in rows()), 10, 'quick in the scene', proc)
                latency = time.monotonic() - started
                check('typing right after the landing reaches the scene', latency < 5, round(latency, 2))
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('b')
                keys('*Romacs*')
                keys('-k', 'Return')

                # open
                back = time.monotonic()
                opening = []
                for at in (0.2, 0.7, 1.2, 1.8, 2.6, 4.0):
                    time.sleep(max(0.0, back + at - time.monotonic()))
                    opening.append(shot(f'open-{at:.1f}s'))
                steps = [differ(x, y, whole) for x, y in zip(opening, opening[1:])]
                check('the landing opens in motion', differ(opening[0], opening[-1], whole) > 1000, steps)
                if wlr:
                    check('on a GPU every moment of the open differs', min(steps) > 1000, steps)
                    solid = red(opening[-1], whole)
                    check('on a GPU the mark is a lit solid', solid > 0.04, solid)

                check('the landing turns', turning('landing', whole) > 1000)
                keys(' ')
                time.sleep(1.0)  # the pause reaches the display with the next frame
                held = turning('paused', whole)
                check('SPC holds the landing still', held < 600, held)
                keys(' ')
                check('SPC again sets it moving', turning('resumed', whole) > 1000)

                # scroll
                def drawn(since=0):
                    """Each window's animation as the render thread last drew it
                    after stderr offset SINCE: {window: ('scene', (x, y, w, h))},
                    ('cells', None) or ('hidden', None)."""
                    found = {}
                    for line in stderr_path.read_text(errors='replace')[since:].splitlines():
                        if not line.startswith('romacs-text-animation: '):
                            continue
                        fields = dict(f.split('=', 1) for f in line.split()[1:])
                        rect = tuple(round(float(fields[k])) for k in ('x', 'y', 'width', 'height')) \
                            if fields['drawn'] == 'scene' else None
                        found[fields['window']] = (fields['drawn'], rect)
                    return found

                def mark():
                    return len(stderr_path.read_text(errors='replace'))

                def until(done, timeout=30.0):
                    """Whether DONE holds within TIMEOUT seconds: a loaded host
                    may take seconds to paint a command."""
                    end = time.monotonic() + timeout
                    while not done():
                        if time.monotonic() > end:
                            return False
                        time.sleep(0.2)
                    return True

                way = 'scene' if wlr else 'cells'

                def ctrl(*names):
                    for name in names:
                        keys('-M', 'ctrl', name, '-m', 'ctrl')

                def alt(*names):
                    for name in names:
                        keys('-M', 'alt', name, '-m', 'alt')

                def trip(name, away, back):
                    """Keys AWAY then BACK from the top; how the window draws the
                    landing before, away and after."""
                    before = drawn()
                    since = mark()
                    away()
                    until(lambda: drawn(since))
                    there = drawn(since)
                    back()
                    until(lambda: drawn() == before)
                    after = drawn()
                    report.setdefault('scroll', {})[name] = {'before': before, 'away': there, 'after': after}
                    return before, there, after

                time.sleep(1.0)
                start = drawn()
                check(f'the landing draws as {way}', len(start) == 1 and next(iter(start.values()))[0] == way, start)
                for name, tag, away, back in (('M-> M-<', 'end', lambda: alt('>'), lambda: alt('<')),
                                              ('C-v M-v', 'page', lambda: ctrl('v'), lambda: alt('v')),
                                              ('M-> C-l M-<', 'recenter', lambda: (alt('>'), ctrl('l')), lambda: alt('<'))):
                    before, there, after = trip(name, away, back)
                    check(f'{name}: the window scrolled the landing away', any(v[0] != way or v != before.get(k)
                                                                              for k, v in there.items()), there)
                    check(f'{name}: back at the top it draws as before', after == before, {'before': before, 'after': after})
                    check(f'{name}: the landing still turns', turning(f'scroll-{tag}', whole) > 1000)
                window = next(iter(start), None)
                at = start.get(window, (None, None))[1]
                since = mark()
                ctrl('n', 'n', 'n', 'n')
                ctrl('u')
                keys('0')
                ctrl('l')
                until(lambda: drawn(since).get(window), 10 if not wlr else 30)
                moved = drawn(since).get(window)
                if wlr:
                    check('C-u 0 C-l on its fifth line: the scene moves four rows up',
                          at is not None and moved is not None and moved[0] == 'scene' and moved[1][0] == at[0]
                          and moved[1][2:] == at[2:] and at[1] - moved[1][1] > 40, {'before': at, 'after': moved})
                else:
                    check('C-u 0 C-l on its fifth line: still cells', moved is None or moved[0] == 'cells', moved)
                alt('<')
                until(lambda: drawn() == start)
                check('M-< after the recentre: as before', drawn() == start, drawn())

                # split: every window showing the landing turns, whichever is
                # selected and wherever its cursor goes.  A half window's logo is
                # a quarter the size; a still one changes 0, a cursor blink ~175.
                top, bottom = f'{WIDTH}x{HEIGHT // 2 - 60}+0+0', f'{WIDTH}x{HEIGHT // 2 - 60}+0+{HEIGHT // 2}'
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('2')
                time.sleep(1.0)
                turned = turning('split', top, bottom)
                check('both landing windows turn after C-x 2', min(turned) > 400, turned)
                # Scroll only the selected window, the upper one C-x 2 kept,
                # away and back.
                # Only this window has shown the landing before the split.
                pair = drawn()
                upper = window
                since = mark()
                alt('>')
                left = until(lambda: upper in drawn(since))
                away = drawn(since)
                alt('<')
                returned = left and until(lambda: drawn() == pair)
                back = drawn()
                lower = [k for k in pair if k != upper]
                report.setdefault('scroll', {})['split'] = {'before': pair, 'away': away, 'after': back}
                check('split: both windows draw the landing', len(pair) == 2 and all(v[0] == way for v in pair.values()), pair)
                check('split: M-> moves only the selected window', left and set(away) == {upper}, away)
                check('split: M-< brings it back, the other never moved', returned and back == pair and len(lower) == 1, back)
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('o')
                turned = turning('other', top, bottom)
                check('both landing windows turn after C-x o', min(turned) > 400, turned)
                keys('-M', 'ctrl', 'n', '-m', 'ctrl')
                keys('-M', 'ctrl', 'n', '-m', 'ctrl')
                turned = turning('moved', top, bottom)
                check('both landing windows turn after moving the cursor', min(turned) > 400, turned)
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('1')
                time.sleep(1.0)
                check('the landing turns again in one window', turning('one', whole) > 1000)

                # reduced
                def setq(value):
                    keys('-M', 'alt', ':', '-m', 'alt')
                    keys(f'(setq romacs-animation {value})')
                    keys('-k', 'Return')
                setq("'reduced")
                time.sleep(1.0)
                # The landing, short of the corner a message pops up in.
                held = turning('reduced', f'{WIDTH * 3 // 4}x{HEIGHT - 80}+0+0')
                check('under reduced motion the landing stands still', held < 600, held)
                if wlr:
                    posed = red(a.output / 'reduced-c.png', whole)
                    check('on a GPU the still frame is the lit mark', posed > 0.04, posed)
                # The still frame after a scroll away and back looks as before.
                landing = f'{WIDTH * 3 // 4}x{HEIGHT - 80}+0+0'
                for name, tag, away, back in (('M-> M-<', 'end', lambda: alt('>'), lambda: alt('<')),
                                              ('C-v M-v', 'page', lambda: ctrl('v'), lambda: alt('v'))):
                    before_drawn = drawn()
                    before = shot(f'still-{tag}-before')
                    since = mark()
                    away()
                    left = until(lambda: drawn(since))
                    time.sleep(0.5)
                    shot(f'still-{tag}-away')
                    back()
                    returned = left and until(lambda: drawn() == before_drawn)
                    time.sleep(0.5)
                    after = shot(f'still-{tag}-after')
                    changed = differ(before, after, landing)
                    check(f'reduced, {name}: the window scrolled away and back', returned,
                          {'away': drawn(since), 'before': before_drawn})
                    check(f'reduced, {name}: the still frame looks as before', changed < 600, changed)

                # alpha, in the still frame
                def pixels(path, region):
                    """REGION's pixels in PATH, as (r, g, b) bytes."""
                    out = subprocess.run([tool('magick'), str(path), '-crop', region, '+repage', '-depth', '8', 'txt:-'],
                                         capture_output=True, text=True, check=True)
                    found = []
                    for line in out.stdout.splitlines()[1:]:
                        rgb = line.split('(', 1)[1].split(')', 1)[0].split(',')[:3]
                        found.append(tuple(int(v) for v in rgb))
                    return found

                def bounds(path, region, fx):
                    """The bounding box (x0, y0, x1, y1) of REGION's pixels in PATH
                    for which FX holds; empty when none do."""
                    x, y = (int(v) for v in region.split('+')[1:])
                    out = subprocess.run([tool('magick'), str(path), '-crop', region, '+repage', '-fx',
                                          f'{fx} ? 1 : 0', '-format', '%@', 'info:'],
                                         capture_output=True, text=True, check=True)
                    size, bx, by = out.stdout.strip().split('+')
                    w, h = (int(v) for v in size.split('x'))
                    return x + int(bx), y + int(by), x + int(bx) + w, y + int(by) + h

                def frame_alpha(percent):
                    keys('-M', 'alt', ':', '-m', 'alt')
                    keys(f"(set-frame-parameter nil 'alpha-background {percent})")
                    keys('-k', 'Return')
                    time.sleep(1.0)
                    return shot(f'alpha-{percent}')

                # These shots compare pixel rows round the landing: a cursor
                # trail still animating under load would cross them.
                keys('-M', 'alt', ':', '-m', 'alt')
                keys("(setopt romacs-cursor-motion-trail-size 0)")
                keys('-k', 'Return')
                opaque = frame_alpha(100)
                # The lit mark and its glow inside the frame's border, short of
                # the message corner; then the wordmark below them.
                _, _, _, lit_y1 = bounds(opaque, f'{WIDTH - 200}x{HEIGHT - 140}+20+20', 'r > 0.35 && r > g + 0.16')
                wx0, _, wx1, wy1 = bounds(opaque, f'{WIDTH - 40}x{HEIGHT - 120 - lit_y1}+20+{lit_y1}',
                                          'r > 0.75 && g > 0.75 && b > 0.75')
                # The placement ends a little below the wordmark, past the glow:
                # a row there crosses its left and right edges and a column its
                # bottom edge, and each must show one page from end to end.
                lines = {'row': f'{WIDTH - 40}x1+20+{wy1 + 6}', 'column': f'1x40+{(wx0 + wx1) // 2}+{wy1 + 3}'}
                drawn = wx1 > wx0
                if wlr:
                    check('on a GPU the wordmark is drawn below the mark', drawn and wy1 + 43 < HEIGHT - 100,
                          (wx0, wx1, wy1))
                opaque_alpha = 'frame transparency renders opaque' in stderr_path.read_text(errors='replace')
                pages = {}
                for percent in (100, 90, 70):
                    path = opaque if percent == 100 else frame_alpha(percent)
                    if drawn:
                        spread = {}
                        for name, region in lines.items():
                            found = pixels(path, region)
                            spread[name] = max(max(p[c] for p in found) - min(p[c] for p in found) for c in range(3))
                        check(f'alpha-background {percent}: no edge round the landing', max(spread.values()) <= 2,
                              {'spread': spread, 'lines': lines})
                    if wlr:
                        lit = red(path, whole)
                        check(f'alpha-background {percent}: the mark is lit', lit > 0.04, lit)
                    pages[percent] = pixels(path, f'1x1+20+{HEIGHT // 2}')[0]
                if backdrop is None or opaque_alpha:
                    report['skipped'] = report.get('skipped', []) + [
                        'backdrop: ' + ('no swaybg' if backdrop is None else 'the surface composites opaque')]
                else:
                    check('below 100 the backdrop shows through the page', pages[70] != pages[100], pages)
                frame_alpha(100)
                setq('t')

                # away
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('b')
                keys('*scratch*')
                keys('-k', 'Return')
                keys('hello')
                fio.wait(lambda: any(r.startswith('hello') for r in rows()), a.timeout, 'hello in the scene', proc)
                check('the scene shows hello in *scratch*', any(r.startswith('hello') for r in rows()), rows()[:4])
                time.sleep(1.0)
                away = shot('away')
                # *scratch*'s first rows: its message and "hello", not a block of
                # animation cells.
                drawn = ink(away, f'{WIDTH // 2}x100+0+0')
                check('*scratch* text is drawn, no animation over it', drawn > 0.01, drawn)
                time.sleep(1.0)
                check('nothing turns over *scratch*', differ(away, shot('away-later'), whole) < 1000)

                # beside
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('b')
                keys('*Romacs*')
                keys('-k', 'Return')
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('3')
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('o')
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('b')
                keys('*scratch*')
                keys('-k', 'Return')
                keys('abc')
                fio.wait(lambda: any('helloabc' in r for r in rows()), a.timeout, 'abc in the scene', proc)
                check('the scene shows the typed text beside the landing', any('helloabc' in r for r in rows()), rows()[:6])
                time.sleep(1.0)
                left = f'{WIDTH // 2 - 20}x{HEIGHT - 80}+0+0'
                check('the landing beside *scratch* turns', turning('beside', left) > 1000)
                beside = a.output / 'beside-c.png'
                drawn = ink(beside, f'{WIDTH // 2 - 40}x100+{WIDTH // 2 + 20}+0')
                check('*scratch* beside the landing is drawn', drawn > 0.01, drawn)

                # resize
                small = (1200, 760)
                subprocess.run([tool('wlr-randr'), '--output', 'HEADLESS-1', '--custom-mode', f'{small[0]}x{small[1]}'],
                               env=env, check=True)
                time.sleep(1.5)
                left = f'{small[0] // 2 - 20}x{small[1] - 80}+0+0'
                check('the landing turns after a resize', turning('resized', left) > 1000)
        except Exception as e:  # noqa: BLE001
            report['error'] = repr(e)
        finally:
            for p in (proc, keyboard, backdrop, comp):
                if p is not None and p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(5)
                    except subprocess.TimeoutExpired:
                        p.kill()
        log = stderr_path.read_text(errors='replace') if stderr_path.exists() else None
        stale = log.count('rejecting incoherent presented pointer hit') if log is not None else -1
        check('no pointer hit is rejected as incoherent', stale == 0, stale)
        stale = log.count('StalePresentation') if log is not None else -1
        check('no presentation is stale', stale == 0, stale)
    report['status'] = 'PASS' if not report['failures'] and 'error' not in report else 'FAIL'
    (a.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(report['status'], json.dumps({k: report[k] for k in ('failures',)} | {'error': report.get('error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
