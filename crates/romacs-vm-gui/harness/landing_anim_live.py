#!/usr/bin/env python3
"""The *Romacs* landing's animation in a native --loadup session, forced on.

The landing's sphere is a text animation the render thread draws over the
evaluator's frames (rlisp/romacs-animate.el, render_thread/text_animations.rs).
A software rasterizer draws none, so every other harness sees a still frame;
this one sets ROMACS_LAYER_ANIMATION=1 to run the path a GPU runs.

split: C-x 2 shows the landing in two windows, both of which must turn,
and keep turning after C-x o and after moving the cursor; C-x 1 again.

away: on the landing, + + and - zoom the sphere, which must keep turning;
then C-x b *scratch* RET and typing "hello".  The scene the evaluator
submitted must show it, and so must the pixels: an animation left over from
the landing would paint its block over *scratch*, as it once did.

beside: C-x b *Romacs* RET, C-x 3 splits it, C-x o and C-x b *scratch* RET
in the right window, then typing "abc": the left window's sphere keeps
turning while the right window's text draws.

No pointer hit may be rejected as incoherent with the presented frame.

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
import subprocess
import tempfile
import time

import fileio_live as fio
import live_oracle as live

WIDTH, HEIGHT = 1600, 1000


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
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime), 'WLR_BACKENDS': 'headless',
               'WLR_HEADLESS_OUTPUTS': '1', 'WLR_RENDERER': 'pixman', 'WLR_LIBINPUT_NO_DEVICES': '1',
               'LIBSEAT_BACKEND': 'noop', 'ROMACS_LAYER_ANIMATION': '1'}
        if a.tools:
            env['PATH'] = f"{a.tools}:{env['PATH']}"
        # The compositor's own renderer on a real GPU (gles2), when asked:
        # Romacs then draws with the host's GPU instead of a software one.
        wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
        stderr_path = a.output / 'binary.stderr'
        comp = keyboard = proc = None
        try:
            with (a.output / 'compositor.log').open('w') as clog, stderr_path.open('w') as blog:
                comp = subprocess.Popen([tool('labwc'), '-C', str(config)], env=env | wlr, stdout=clog, stderr=clog)
                fio.wait(lambda: any(s.is_socket() for s in runtime.glob('wayland-*')), a.timeout, 'compositor', comp)
                env['WAYLAND_DISPLAY'] = next(s for s in runtime.glob('wayland-*') if s.is_socket()).name
                subprocess.run([tool('wlr-randr'), '--output', 'HEADLESS-1', '--custom-mode', f'{WIDTH}x{HEIGHT}'], env=env, check=True)
                # A seat keyboard before the window maps, so it gets focus.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=blog, stderr=blog)
                time.sleep(0.3)
                proc = subprocess.Popen([str(a.binary.resolve()), '--loadup', '--trace-scene'], env=env,
                                        stdin=subprocess.DEVNULL, stdout=blog, stderr=blog)
                rows = lambda: fio.rows(stderr_path.read_text(errors='replace'))
                fio.wait(lambda: any('SPC pause' in r for r in rows()), a.timeout * 3, 'the landing', proc)
                time.sleep(2)

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

                def turning(name, region):
                    first = shot(f'{name}-a')
                    time.sleep(1.0)
                    second = shot(f'{name}-b')
                    return differ(first, second, region)

                whole = f'{WIDTH}x{HEIGHT - 80}+0+0'
                check('the landing turns', turning('landing', whole) > 1000)
                keys('++')
                keys('-k', 'minus')
                check('the landing turns after zooming', turning('zoomed', whole) > 1000)

                # split: every window showing the landing turns, whichever is
                # selected and wherever its cursor goes.
                top, bottom = f'{WIDTH}x{HEIGHT // 2 - 60}+0+0', f'{WIDTH}x{HEIGHT // 2 - 60}+0+{HEIGHT // 2}'
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('2')
                time.sleep(1.0)
                turned = (turning('split', top), differ(a.output / 'split-a.png', a.output / 'split-b.png', bottom))
                check('both landing windows turn after C-x 2', min(turned) > 1000, turned)
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('o')
                turned = (turning('other', top), differ(a.output / 'other-a.png', a.output / 'other-b.png', bottom))
                check('both landing windows turn after C-x o', min(turned) > 1000, turned)
                keys('-M', 'ctrl', 'n', '-m', 'ctrl')
                keys('-M', 'ctrl', 'n', '-m', 'ctrl')
                turned = (turning('moved', top), differ(a.output / 'moved-a.png', a.output / 'moved-b.png', bottom))
                check('both landing windows turn after moving the cursor', min(turned) > 1000, turned)
                keys('-M', 'ctrl', 'x', '-m', 'ctrl')
                keys('1')
                time.sleep(1.0)
                check('the landing turns again in one window', turning('one', whole) > 1000)

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
                check('the landing beside *scratch* turns', turning('beside', left) > 300)
                beside = a.output / 'beside-b.png'
                drawn = ink(beside, f'{WIDTH // 2 - 40}x100+{WIDTH // 2 + 20}+0')
                check('*scratch* beside the landing is drawn', drawn > 0.01, drawn)
        except Exception as e:  # noqa: BLE001
            report['error'] = repr(e)
        finally:
            for p in (proc, keyboard, comp):
                if p is not None and p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(5)
                    except subprocess.TimeoutExpired:
                        p.kill()
        stale = stderr_path.read_text(errors='replace').count('rejecting incoherent presented pointer hit') if stderr_path.exists() else -1
        check('no pointer hit is rejected as incoherent', stale == 0, stale)
    report['status'] = 'PASS' if not report['failures'] and 'error' not in report else 'FAIL'
    (a.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(report['status'], json.dumps({k: report[k] for k in ('failures',)} | {'error': report.get('error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
