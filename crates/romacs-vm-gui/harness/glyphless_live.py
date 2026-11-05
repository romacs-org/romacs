#!/usr/bin/env python3
"""Characters no font has, typed into *scratch* of a native --loadup
session in a private headless compositor.

Real keys run `M-: (insert #xCODE) RET` for each character of CODES: one
in GNU's ideographic description block, a Private Use Area character in
the Basic Multilingual Plane and one in plane 16, and an unassigned code
point; the host's fontconfig must have no font for any of them (checked
first with `fc-list').  After each the session must still run, the
character must be in the buffer, and the frame must show it as GNU does:
an assigned or private-use character as the no-font glyph for
`glyphless-char-display''s default `hex-code', the code in two rows of
small digits (`2F' over `FF') inside a one-pixel box; the unassigned code
point as its escape (`\\1570'), GNU's for a character that is not
printable (CHAR_PRINTABLE_P), which no font is asked for.  The capture
is checked at the digits' place: the box's four bars in the text's
foreground, its inside in the background.  *Messages* must hold no error
and stderr no evaluator stop.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype, grim and
magick come from --tools (the gate's headless tools) or PATH.

usage: glyphless_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

import fileio_live as fio
import find_file_live as ff
import timers_live as tl

CODES = [0x2FFF, 0xF8FE, 0x10FFFD, 0x0378]
BOXED, ESCAPED = CODES[:3], {0x0378: '\\1570'}

INIT = '''(setq inhibit-startup-screen t)
(defun glyphless-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(add-hook 'post-command-hook
          (lambda () (when (eq this-command 'eval-expression)
                       (glyphless-live-record (list 'inserted (mapcar (lambda (c) (format "%X" c)) (buffer-string)))))))
(glyphless-live-record (list 'init (char-table-extra-slot glyphless-char-display 0)))
'''


def covering_fonts(code, env):
    """The fonts fontconfig lists with a glyph for CODE."""
    result = subprocess.run(['fc-list', f':charset={code:x}', 'family'], env=env, capture_output=True, text=True, timeout=30)
    return [line for line in result.stdout.splitlines() if line.strip()]


def pixels(path, magick):
    """PATH's width, height and RGB bytes."""
    width, height = map(int, subprocess.check_output([magick, 'identify', '-format', '%w %h', str(path)], timeout=10).split())
    return width, height, subprocess.check_output([magick, str(path), '-alpha', 'off', '-depth', '8', 'RGB:-'], timeout=10)


def boxes(scene, codes):
    """Where SCENE's text rows show each of CODES as a code in two rows:
    the x of its first column of digits and the y of the cells inside
    its box.  A box's two rows share their columns' x."""
    columns = {}
    for g in scene['glyphs']:
        if g['role'] == 'Text':
            columns.setdefault((g['y'], g['x']), []).append(g['char'])
    found = []
    for code in codes:
        digits = f'{code:04X}' if code < 0x10000 else f'{code:06X}'
        half = len(digits) // 2
        want = [sorted(pair) for pair in zip(digits[:half], digits[half:])]
        keys = sorted(columns)
        for i in range(len(keys) - half + 1):
            run = keys[i:i + half]
            if len({y for y, _ in run}) == 1 and [sorted(columns[k]) for k in run] == want:
                found.append({'code': f'{code:X}', 'x': run[0][1], 'y': run[0][0]})
                break
    return found


def box_at(rgb, width, x, y):
    """The box drawn round the upper half starting at X on the row from Y:
    its left bar two pixels left of X and its top bar a pixel above Y,
    its inside corner the background;
    how far its top bar and its left bar run in pixels far from that."""
    at = lambda px, py: tuple(rgb[3 * (py * width + px):3 * (py * width + px) + 3])
    left, top = int(round(x)) - 2, int(round(y)) - 1
    background = at(left + 1, top + 1)
    ink = lambda px: sum(abs(a - b) for a, b in zip(px, background)) > 180
    run = 0
    while left + run < width and ink(at(left + run, top)):
        run += 1
    down = 0
    while ink(at(left, top + down)):
        down += 1
    return {'left': left, 'top': top, 'top_bar': run, 'left_bar': down, 'background': list(background), 'bar': list(at(left, top))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype, grim and magick')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--settle', type=float, default=2)
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

    tl.INIT = INIT
    try:
        with tl.session(binary, args, output, 'glyphless', report, True) as (env, state, process, log_path):
            magick = shutil.which('magick', path=env['PATH']) or 'magick'
            covered = {f'{code:X}': covering_fonts(code, env) for code in CODES}
            check('no installed font has these characters', not any(covered.values()), covered)
            records = lambda: tl.records(state)
            fio.wait(lambda: any(r.startswith('(init') for r in records()), args.timeout, 'init.el after --loadup', process)
            fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')), args.timeout, 'first frame', process)
            init = next(r for r in records() if r.startswith('(init'))
            check("glyphless-char-display's no-font method is hex-code", init == '(init hex-code)', init)
            tl.quiet(process, args.settle, args.timeout)
            for n, code in enumerate(CODES, 1):
                tl.keys(env, '-M', 'alt', ':', '-m', 'alt')
                tl.keys(env, f'(insert #x{code:x})')
                tl.keys(env, '-k', 'Return')
                want = [f'{c:X}' for c in CODES[:n]]
                try:
                    fio.wait(lambda: any(r.startswith('(inserted') and all(f'"{w}"' in r for w in want) for r in records()),
                             20, f'insert U+{code:04X}', process)
                except AssertionError as error:
                    check(f'U+{code:04X} inserted', False, repr(error))
                    break
                try:
                    tl.quiet(process, args.settle, args.timeout)
                except (RuntimeError, OSError):
                    pass
                check(f'the session survives U+{code:04X}', process.poll() is None, process.poll())
                if process.poll() is not None:
                    break
            if process.poll() is not None:
                raise AssertionError(f'the session ended with status {process.poll()}')
            fio.shot(output / 'glyphless.png', env, report)
            # *Messages*, read through the same keys a user would: M-: shows it.
            tl.keys(env, '-M', 'alt', ':', '-m', 'alt')
            tl.keys(env, '(glyphless-live-record (list (quote messages) (with-current-buffer "*Messages*" (buffer-string))))')
            tl.keys(env, '-k', 'Return')
            fio.wait(lambda: any(r.startswith('(messages') for r in records()), 20, '*Messages*', process)
            messages = next(r for r in records() if r.startswith('(messages'))
            check('*Messages* holds no error', 'rror' not in messages and 'unresolved' not in messages, messages)
            check('the session is still running', process.poll() is None, process.poll())
            scene = fio.last_scene(log_path.read_text(errors='replace'))
            found = boxes(scene, BOXED) if scene else []
            check('each assigned character shows its code in the frame', len(found) == len(BOXED), found)
            rows = fio.scene_rows(scene) if scene else []
            check('the unassigned code point shows its escape', all(any(e in row for row in rows) for e in ESCAPED.values()), rows[:6])
            width, height, rgb = pixels(output / 'glyphless.png', magick)
            measured = [dict(code, **box_at(rgb, width, code['x'], code['y'])) for code in found]
            report['boxes'] = measured
            for box in measured:
                wide = 6 if int(box['code'], 16) >= 0x10000 else 4
                check(f"U+{box['code']}'s box is painted", box['top_bar'] >= wide * 3 and box['left_bar'] >= 8, box)
        stderr = log_path.read_text(errors='replace')
        check('no evaluator stop', 'evaluator stopped' not in stderr and 'unresolved face text' not in stderr,
              [line for line in stderr.splitlines() if 'evaluator stopped' in line or 'romacs-exit' in line][:3])
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('boxes', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
