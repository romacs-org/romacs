#!/usr/bin/env python3
"""Emoji sequences in Romacs and in pinned GNU, compared on screen.

Both show a buffer whose first line is a family ZWJ sequence (man ZWJ
woman ZWJ girl) and, after spaces, a red heart with VS-16:
GNU's composition draws each as one colour glyph.  Romacs runs `--loadup'
in a private headless labwc; GNU, the pinned X11 build
($ROMACS_GNU_GUI_BINARY, `-Q') on a private Xvfb ($ROMACS_DISPLAY_TOOLS).
Each capture is cropped to that line and read the same way: the runs of
columns with ink.  Each must show exactly two runs, the first one glyph
wide (under twice the line's height) and the second red.

Every process gets a private HOME/XDG tree; Romacs gets only the private
compositor's WAYLAND_DISPLAY.  labwc, wlr-randr, wtype, grim and magick
come from --tools or PATH.

usage: emoji_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time

import fileio_live as fio
import timers_live as tl

FAMILY = '\U0001F468‍\U0001F469‍\U0001F467'
HEART = '❤️'
LINE = FAMILY + '    ' + HEART
# A buffer of its own: nothing but the line in colour above the mode line.
SETUP = ('(progn (setq inhibit-startup-screen t) (menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1) (switch-to-buffer (get-buffer-create "emoji")) '
         '(insert {line} "\\n\\n") (goto-char (point-max)) (write-region "ready\\n" nil {state} t \'silent))')


def pixels(path, magick):
    """PATH's width, height and RGB bytes."""
    width, height = map(int, subprocess.check_output([magick, 'identify', '-format', '%w %h', str(path)], timeout=10).split())
    return width, height, subprocess.check_output([magick, str(path), '-alpha', 'off', '-depth', '8', 'RGB:-'], timeout=10)


def glyph_runs(path, magick, left=10, right=640, bottom=240):
    """The glyphs PATH shows on its first line of text in the region from
    LEFT to RIGHT above BOTTOM: the runs of columns with ink (pixels far
    from the region's commonest colour, its background) in the first band
    of rows with ink, [(x0, x1, red pixels)], and that band's height.
    Rows inked most of the way across (a pane's border) are not read."""
    width, height, rgb = pixels(path, magick)
    right, bottom = min(right, width), min(bottom, height)
    at = lambda x, y: tuple(rgb[3 * (y * width + x):3 * (y * width + x) + 3])
    counts = {}
    for y in range(bottom):
        for x in range(left, right):
            counts[at(x, y)] = counts.get(at(x, y), 0) + 1
    background = max(counts, key=counts.get)
    ink = lambda px: sum(abs(a - b) for a, b in zip(px, background)) > 90
    # A row inked most of the way across is a border or a bar, not text.
    inked = [(y, sum(ink(at(x, y)) for x in range(left, right))) for y in range(bottom)]
    rows = [y for y, n in inked if 2 <= n < (right - left) // 2]
    if not rows:
        return [], 0
    band = [rows[0]]
    for y in rows[1:]:
        if y - band[-1] > 2:
            break
        band.append(y)
    band = range(band[0], band[-1] + 1)
    runs = []
    lines = {y for y, n in inked if n >= (right - left) // 2}
    for x in (x for x in range(left, right) if any(ink(at(x, y)) for y in band if y not in lines)):
        if runs and x - runs[-1][1] <= 3:
            runs[-1][1] = x
        else:
            runs.append([x, x])
    red = lambda x0, x1: sum(1 for x in range(x0, x1 + 1) for y in band
                             if (px := at(x, y))[0] > 150 and px[1] < 90 and px[2] < 90)
    return [(x0, x1, red(x0, x1)) for x0, x1 in runs], len(band)


def judge(name, runs, band, check):
    check(f'{name}: the line shows two glyphs', len(runs) == 2, runs)
    if len(runs) == 2:
        (f0, f1, _), (_, _, red) = runs
        check(f'{name}: the family is one glyph', f1 - f0 + 1 < 2 * band, {'width': f1 - f0 + 1, 'band': band})
        check(f'{name}: the VS-16 heart is red', red >= 10, red)


def free_display():
    for number in range(140, 200):
        with socket.socket() as s:
            if s.connect_ex(('127.0.0.1', 6000 + number)) != 0:
                return number
    raise SystemExit('no free X display')


def run_gnu(output, magick, timeout):
    """Capture pinned GNU X11 showing LINE: the capture's path."""
    tools = Path(os.environ['ROMACS_DISPLAY_TOOLS'])
    with tempfile.TemporaryDirectory(prefix='emoji-gnu-') as temporary:
        root = Path(temporary)
        number = free_display()
        display = f'127.0.0.1:{number}'
        auth = root / 'Xauthority'
        subprocess.run([tools / 'xauth', '-f', auth, 'add', display, 'MIT-MAGIC-COOKIE-1', secrets.token_hex(16)], check=True)
        server = subprocess.Popen([tools / 'Xvfb', f':{number}', '-screen', '0', '1280x800x24', '-nolisten', 'unix',
                                   '-listen', 'tcp', '-nolock', '-noreset', '-auth', auth],
                                  cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        emacs = None
        try:
            fio.wait(lambda: socket.socket().connect_ex(('127.0.0.1', 6000 + number)) == 0, 10, 'Xvfb', server)
            home = root / 'home'
            home.mkdir()
            state = root / 'state'
            env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'DISPLAY': display, 'XAUTHORITY': str(auth),
                   'GDK_BACKEND': 'x11', 'LANG': 'C.UTF-8', 'TMPDIR': str(root)}
            if 'ROMACS_GATE_FONTCONFIG' in os.environ:
                env['FONTCONFIG_FILE'] = os.environ['ROMACS_GATE_FONTCONFIG']
            setup = SETUP.format(line=json.dumps(LINE, ensure_ascii=False), state=json.dumps(str(state)))
            emacs = subprocess.Popen([os.environ['ROMACS_GNU_GUI_BINARY'], '-Q', '--eval', setup], env=env, cwd=root,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            fio.wait(state.exists, timeout, 'GNU ready', emacs)
            time.sleep(2)
            capture = output / 'gnu.png'
            subprocess.run([magick, 'import', '-window', 'root', str(capture)], env=env, check=True, timeout=30)
            return capture
        finally:
            if emacs is not None:
                emacs.kill()
                emacs.wait()
            server.terminate()
            server.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype, grim and magick')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'checks': [], 'failures': []}

    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    tl.INIT = SETUP.format(line=json.dumps(LINE, ensure_ascii=False), state='{state}') + '\n'
    try:
        with tl.session(binary, args, output, 'emoji', report, True) as (env, state, process, log_path):
            magick = shutil.which('magick', path=env['PATH']) or 'magick'
            fio.wait(lambda: tl.records(state) == ['ready'], args.timeout, 'Romacs ready', process)
            tl.quiet(process, 2, args.timeout)
            fio.shot(output / 'romacs.png', env, report)
            scene = fio.last_scene(log_path.read_text(errors='replace'))
            report['romacs_glyphs'] = [[g.get('composed') or g['char'], g['x']] for g in (scene or {}).get('glyphs', [])
                                       if g['role'] == 'Text' and not g['char'].isascii()]
        runs, band = glyph_runs(output / 'romacs.png', magick)
        report['romacs'] = {'runs': runs, 'band': band}
        judge('Romacs', runs, band, check)
        if os.environ.get('ROMACS_GNU_GUI_BINARY') and os.environ.get('ROMACS_DISPLAY_TOOLS'):
            runs, band = glyph_runs(run_gnu(output, magick, args.timeout), magick)
            report['gnu'] = {'runs': runs, 'band': band}
            judge('GNU', runs, band, check)
        else:
            check('pinned GNU X11 is available', False, 'ROMACS_GNU_GUI_BINARY / ROMACS_DISPLAY_TOOLS unset')
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('romacs', 'gnu', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
