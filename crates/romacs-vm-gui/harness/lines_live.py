#!/usr/bin/env python3
"""Lines in a native --loadup session, in a private headless compositor.

tabs: GNU's own Lisp is visited from the command line (a copy of a
gnu-lisp/ file with TABs).  Every glyph the binary submits on the file's
rows must sit at the column GNU gives it on a terminal frame: TABs stop at
multiples of `tab-width' from the line start, a control character shows as
its escape.  The window's font is monospace, so a column is x over the
frame's character width.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set and no ROMACS_GUI_FONT is passed.
labwc, wlr-randr, wtype and grim come from --tools or PATH.

usage: lines_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import live_oracle as live

ROOT = Path(__file__).resolve().parents[3]
TAB_FILE = ROOT / 'gnu-lisp/help-macro.el'

INIT = '''(defun lines-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(run-with-timer 1 nil (lambda () (lines-live-record
  (list 'geometry (frame-char-width) (frame-char-height) (window-body-width nil t) tab-width
        (line-number-at-pos (window-start)) (buffer-name)))))
'''


@contextlib.contextmanager
def session(binary, args, output, tools, init, files=()):
    """A native --loadup session on ARGS in a private compositor: yields
    (process, env, work directory, state file, report display record)."""
    with tempfile.TemporaryDirectory(prefix='lines-live-') as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        home, work, config = [root / name for name in ('home', 'work', 'config')]
        for directory in (home, work, config):
            directory.mkdir()
        for file in files:
            shutil.copyfile(file, work / file.name)
        state = work / 'lines-state.el'
        (home / 'config/romacs').mkdir(parents=True)
        (home / 'config/romacs/init.el').write_text(init.format(state=json.dumps(str(state))))
        (config / 'rc.xml').write_text(live.RC_XML)
        path = os.environ.get('PATH', '/usr/bin:/bin')
        env = {'PATH': f'{tools}:{path}' if tools else path,
               'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
        for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_GATE_FONTCONFIG'):
            if key in os.environ:
                env[key] = os.environ[key]
        if 'ROMACS_GATE_FONTCONFIG' in env:
            env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
        compositor = keyboard = process = None
        log_path = output / 'binary.stderr'
        try:
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                socket = root / env['WAYLAND_DISPLAY']
                assert 'DISPLAY' not in env and socket.is_socket() and socket.parent == root
                display = {'wayland_display': env['WAYLAND_DISPLAY'], 'wayland_socket': str(socket)}
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene', *args], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                child_env = Path(f'/proc/{process.pid}/environ').read_bytes().split(b'\0')
                display['client_wayland_display'] = next((e.decode() for e in child_env if e.startswith(b'WAYLAND_DISPLAY=')), None)
                display['client_has_display'] = any(e.startswith(b'DISPLAY=') for e in child_env)
                yield process, env, work, state, display
        finally:
            live.stop(process)
            live.stop(keyboard)
            live.stop(compositor)


def records(state):
    return state.read_text().splitlines() if state.exists() else []


def scene_glyphs(log_path):
    """The last submitted scene's Text glyphs: (clip, y, x, char)."""
    scene = fio.last_scene(log_path.read_text(errors='replace'))
    if scene is None:
        return []
    return [(json.dumps(g['clip'], sort_keys=True), g['y'], g['x'], g['char']) for g in scene['glyphs'] if g['role'] == 'Text']


def gnu_columns(line, tab_width):
    """Each shown glyph of LINE and its column on a terminal frame: TAB
    stops every TAB-WIDTH columns, a control character is ^X."""
    shown, column = [], 0
    for c in line:
        if c == '\t':
            column = (column // tab_width + 1) * tab_width
        elif ord(c) < 32 or ord(c) == 127:
            shown += [(column, '^'), (column + 1, chr(ord(c) ^ 0x40))]
            column += 2
        else:
            if c != ' ':
                shown.append((column, c))
            column += 1
    return shown


def check_tabs(args, report, check):
    output = args.output / 'tabs'
    output.mkdir(parents=True, exist_ok=True)
    # Lines 41 to 58 are TAB-indented; the window ends at 58.
    with session(args.binary, ['+58', TAB_FILE.name], output, args.tools, INIT, files=[TAB_FILE]) as (process, env, work, state, display):
        report['display'] = display
        fio.wait(lambda: any(r.startswith('(geometry') for r in records(state)), args.timeout, 'geometry record', process)
        _, cw, ch, body, tab_width, start, name = records(state)[0].strip('()').split(' ')
        cw, ch, body, tab_width, start, name = int(cw), int(ch), int(body), int(tab_width), int(start), json.loads(name)
        lines = TAB_FILE.read_text().split('\n')
        log = output / 'binary.stderr'
        time.sleep(1)
        fio.shot(args.output / 'tabs.png', env, report)
        glyphs = scene_glyphs(log)
        # The file's window is the one whose top row shows its window-start line.
        lines = lines[start - 1:]
        clips = {}
        for clip, y, x, c in glyphs:
            if c != ' ':
                clips.setdefault(clip, {}).setdefault(y, []).append((x, c))
        top_text = ''.join(c for c in lines[0] if not c.isspace())
        window = next(rows_ for rows_ in clips.values() if ''.join(c for _, c in sorted(rows_[min(rows_)])) == top_text)
        top = min(window)
        painted = {round((y - top) / ch): sorted(row) for y, row in window.items()}
        compared = tabbed = 0
        mismatches = []
        columns = body // cw - 1
        for index, line in enumerate(lines[:len(lines) and max(painted) + 1]):
            row = painted.get(index, [])
            expected = gnu_columns(line, tab_width)
            if expected and expected[-1][0] >= columns:
                break  # a continued line: rows no longer pair with lines
            got = [(round(x / cw), c) for x, c in row]
            compared += 1
            tabbed += '\t' in line
            if got != expected:
                mismatches.append({'line': line, 'gnu': expected[:12], 'painted': got[:12]})
        check(f'{name}: every glyph of lines {start} to {start + compared - 1} ({tabbed} with TABs) at its GNU column',
              compared > 10 and tabbed > 8 and not mismatches, mismatches[:3])
        check('the window shows the visited file', name == TAB_FILE.name, name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'checks': [], 'failures': []}

    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)
    try:
        check_tabs(args, report, check)
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
