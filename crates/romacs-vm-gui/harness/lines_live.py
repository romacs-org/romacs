#!/usr/bin/env python3
"""Lines in a native --loadup session, in a private headless compositor.

tabs: GNU's own Lisp is visited from the command line (a copy of a
gnu-lisp/ file with TABs).  Every glyph the binary submits on the file's
rows must sit at the column GNU gives it on a terminal frame: TABs stop at
multiples of `tab-width' from the line start, a control character shows as
its escape.  The window's font is monospace, so a column is x from the
text area's left edge over the frame's character width.

motion: C-f, C-n and C-p run GNU's next-line and previous-line over a
mixed buffer (long continued lines, short and empty lines, non-ASCII
text).  The point after each command must equal pinned GNU's on a
terminal frame (`emacs -Q -nw' over a pty, private HOME) whose window
continues lines after as many columns as the native window does.

keys: a timer runs `sit-for' (as simple.el's M-x suggestion does) and C-b
typed during it, which sit-for reads and pushes back onto
`unread-command-events', runs as the next command with no other key.
Then Return in *scratch* runs what RET is bound to there (GNU's
`newline'), as the function key `return' that `local-function-key-map'
translates to RET; C-j runs its own binding.  Both insert a newline.
Last, F8 runs a command that fills read_char's three unread queues: their
events run next, in GNU's order (post-input-method, command, input-method),
each between `pre-command-hook' and `post-command-hook', and the
(no-record . EVENT) one stays out of `recent-keys'.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set and no ROMACS_GUI_FONT is passed.
labwc, wlr-randr, wtype and grim come from --tools or PATH.

usage: lines_live.py --binary PATH --output DIR [--tools DIR] [--gnu EMACS]
"""
import argparse
import contextlib
import fcntl
import pty
import select
import struct
import termios
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

# The journeys start where a GNU init that inhibits the startup screen
# starts: in *scratch*, or in the visited file alone.  Without it GNU, and
# --loadup, show the read-only screen, or its concise form beside the file.
INIT = '''(setq inhibit-startup-screen t)
(defun lines-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(run-with-timer 1 nil (lambda () (lines-live-record
  (list 'geometry (frame-char-width) (frame-char-height) (window-body-width nil t) tab-width
        (car (window-inside-pixel-edges)) (nth 1 (window-fringes))
        (line-number-at-pos (window-start)) (buffer-name)))))
'''

MIXED_FILE = Path(__file__).resolve().parent / 'fixtures/lines-mixed.txt'
TALL_FILE = Path(__file__).resolve().parent / 'fixtures/lines-tall.txt'
# (key name for wtype, byte for a terminal): C-f six times, then C-n down
# the buffer and C-p back up.
KEYS = [('f', b'\x06')] * 6 + [('n', b'\x0e')] * 16 + [('p', b'\x10')] * 16
# Down past several windows of continued lines and back: the window must
# scroll so the cursor stays on screen.
TALL_KEYS = [('f', b'\x06')] * 6 + [('n', b'\x0e')] * 90 + [('p', b'\x10')] * 90

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
               'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
               'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
        for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GATE_FONTCONFIG'):
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
        _, cw, ch, body, tab_width, left, right_fringe, start, name = records(state)[0].strip('()').split(' ')
        cw, ch, body, tab_width, left, start, name = int(cw), int(ch), int(body), int(tab_width), int(left), int(start), json.loads(name)
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
        # The glyphs a row holds: without a right fringe, one column less
        # for the continuation glyph.
        columns = body // cw - (int(right_fringe) == 0)
        for index, line in enumerate(lines[:len(lines) and max(painted) + 1]):
            row = painted.get(index, [])
            expected = gnu_columns(line, tab_width)
            if expected and expected[-1][0] >= columns:
                break  # a continued line: rows no longer pair with lines
            got = [(round((x - left) / cw), c) for x, c in row]
            compared += 1
            tabbed += '\t' in line
            if got != expected:
                mismatches.append({'line': line, 'gnu': expected[:12], 'painted': got[:12]})
        check(f'{name}: every glyph of lines {start} to {start + compared - 1} ({tabbed} with TABs) at its GNU column',
              compared > 10 and tabbed > 8 and not mismatches, mismatches[:3])
        check('the window shows the visited file', name == TAB_FILE.name, name)


def gnu_points(emacs, file, columns, rows, keys):
    """Pinned GNU -nw on FILE in a COLUMNS x ROWS terminal: the window's
    continued-line width probe, then (COMMAND POINT) after each of KEYS."""
    with tempfile.TemporaryDirectory(prefix='lines-gnu-') as temporary:
        home = Path(temporary)
        log = home / 'points.el'
        shutil.copyfile(file, home / file.name)
        record = f'(lambda (form) (write-region (concat (prin1-to-string form) "\\n") nil {json.dumps(str(log))} t 0))'
        probe = '(with-temp-buffer (insert (make-string 1000 ?x)) (goto-char 1) (vertical-motion 1) (1- (point)))'
        program = (f'(progn (funcall {record} (list (quote geometry) (window-body-width) {probe}))'
                   " (add-hook 'post-command-hook (lambda () (when (memq this-command '(forward-char next-line previous-line))"
                   f" (funcall {record} (list this-command (point)))))))")
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TERM': 'xterm-256color', 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
               'XDG_CONFIG_HOME': str(home / 'c'), 'XDG_CACHE_HOME': str(home / 'k'), 'XDG_DATA_HOME': str(home / 'd')}
        pid, fd = pty.fork()
        if pid == 0:
            os.execve(emacs, [emacs, '-Q', '-nw', str(home / file.name), '--eval', program], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))

        def drain(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([fd], [], [], 0.02)[0]:
                    try:
                        os.read(fd, 65536)
                    except OSError:
                        return
        try:
            drain(3)
            for _, byte in keys:
                os.write(fd, byte)
                drain(0.2)
            drain(1)
        finally:
            os.kill(pid, 9)
            os.waitpid(pid, 0)
        return log.read_text().splitlines() if log.exists() else []


def command_signals(stderr_text):
    """The condition each command the binary replied to signalled, or None."""
    out = []
    for line in stderr_text.split('\n'):
        if line.startswith('{') and '"native_command"' in line:
            try:
                signal = json.loads(line).get('signal')
            except json.JSONDecodeError:
                continue
            out.append(signal and f"{signal['condition']} {signal['data']}")
    return out


def command_points(stderr_text, text):
    """(COMMAND POINT) after each command the binary replied to: the
    command's reply, then the selected window's point in the next cursor
    publication, as a character position of TEXT."""
    out, command, selected = [], None, None
    for line in stderr_text.split('\n'):
        if not line.startswith('{'):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if 'native_command' in record:
            command = record['native_command']
        elif record.get('diagnostic') == 'cursor-publication':
            active = [c['window'] for c in record['cursors'] if c['active']]
            selected = active[0] if active else selected
            if command is not None:
                byte = next(p['point_byte'] for p in record['points'] if p['id'] == selected)
                out.append(f'({command} {len(text.encode()[:byte].decode()) + 1})' + ('' if active else ' hidden'))
                command = None
    return out


def check_motion(args, report, check, file=MIXED_FILE, keys=KEYS, name='motion'):
    output = args.output / name
    output.mkdir(parents=True, exist_ok=True)
    text = file.read_text()
    log = output / 'binary.stderr'
    with session(args.binary, [file.name], output, args.tools, INIT, files=[file]) as (process, env, work, state, display):
        report[f'{name}_display'] = display
        fio.wait(lambda: any(r.startswith('(geometry') for r in records(state)), args.timeout, 'geometry record', process)
        _, cw, _, body, _, _, right_fringe, *_ = records(state)[0].strip('()').split(' ')
        # A native row holds the body's columns, less the continuation
        # glyph's without a right fringe; GNU -nw always shows the glyph,
        # so its terminal is that many columns and one more.
        glyphs = int(body) // int(cw) - (int(right_fringe) == 0)
        columns = glyphs + 1
        time.sleep(.5)
        fio.shot(args.output / f'{name}-before.png', env, report)
        for index, (key, _) in enumerate(keys):
            subprocess.run(['wtype', '-M', 'ctrl', key, '-m', 'ctrl'], env=env, check=True, timeout=20)
            try:
                fio.wait(lambda: len(command_points(log.read_text(errors='replace'), text)) > index, 10, f'command {index + 1} ({key})', process)
            except AssertionError:
                break
            if index in (6 + 3, 6 + 60):
                time.sleep(.5)
                fio.shot(args.output / f'{name}-{index + 1}.png', env, report)
        time.sleep(.5)
        fio.shot(args.output / f'{name}-after.png', env, report)
        romacs = command_points(log.read_text(errors='replace'), text)
        signals = command_signals(log.read_text(errors='replace'))
        report[f'{name}_echo'] = fio.rows((output / 'binary.stderr').read_text(errors='replace'))[-2:]
    gnu = gnu_points(args.gnu, file, columns, 40, keys)
    report[name] = {'columns': columns, 'gnu_geometry': gnu[:1], 'gnu': gnu[1:], 'romacs': romacs}
    check(f'GNU -nw at {columns} columns continues lines after {glyphs} glyphs, as the native window does',
          bool(gnu) and gnu[0].endswith(f' {glyphs})'), gnu[:1])
    # GNU signals only where a motion runs out of buffer: point stays put.
    edges = [i for i in range(1, len(gnu) - 1) if i > 6 and gnu[i + 1] == gnu[i]]
    expected = [('end-of-buffer nil' if keys[i][0] == 'n' else 'beginning-of-buffer nil') if i in edges else None
                for i in range(len(keys))]
    report[name]['signals'] = signals
    check(f'{file.name}: commands signal only at the ends of the buffer, as GNU does', signals == expected,
          [{'command': i + 1, 'expected': e, 'signal': g} for i, (e, g) in enumerate(zip(expected, signals)) if e != g][:6])
    check(f'{file.name}: the cursor shows after every command', not any(r.endswith(' hidden') for r in romacs),
          [r for r in romacs if r.endswith(' hidden')][:6])
    shown = [r.removesuffix(' hidden') for r in romacs]
    check(f'{file.name}: {len(keys)} C-f/C-n/C-p commands move point as pinned GNU does', shown == gnu[1:] and len(shown) == len(keys),
          [{'command': i + 1, 'gnu': g, 'romacs': r} for i, (g, r) in enumerate(zip(gnu[1:], shown + [None] * len(keys))) if g != r][:6])


# *scratch* in fundamental-mode, so C-j has its global binding, which
# inserts a newline; initial-major-mode's lisp-interaction-mode binds it to
# eval-print-last-sexp, as in GNU.
RETURN_INIT = INIT + '''(setq initial-major-mode 'fundamental-mode)
(run-with-timer 1 nil (lambda () (lines-live-record
  (list 'bindings (with-current-buffer (window-buffer) (list (key-binding [13]) (key-binding [10])))
        (lookup-key local-function-key-map [return])))))
(add-hook 'pre-command-hook (lambda () (lines-live-record (list 'pre this-command))))
(add-hook 'post-command-hook (lambda () (lines-live-record
  (list 'post this-command last-command-event (and (memq ?b (append (recent-keys) nil)) t)))))
(global-set-key [f8] (lambda () (interactive)
  (setq unread-input-method-events (list ?c) unread-command-events (list '(no-record . ?b))
        unread-post-input-method-events (list ?a))))
(run-with-timer 3 nil (lambda ()
  (lines-live-record '(sit-start))
  (lines-live-record (list 'sit-end (sit-for 6)))))
'''


def check_return(args, report, check):
    output = args.output / 'keys'
    output.mkdir(parents=True, exist_ok=True)
    log = output / 'binary.stderr'
    with session(args.binary, [], output, args.tools, RETURN_INIT, files=[]) as (process, env, work, state, display):
        report['keys_display'] = display
        fio.wait(lambda: any(r.startswith('(bindings') for r in records(state)), args.timeout, 'bindings record', process)
        bindings = next(r for r in records(state) if r.startswith('(bindings'))
        ret, cj = bindings.split('(', 2)[2].split(')')[0].split()
        replies = lambda: [json.loads(line) for line in log.read_text(errors='replace').split('\n') if line.startswith('{') and '"native_command"' in line]
        fio.wait(lambda: '(sit-start)' in records(state), args.timeout, 'sit-for in a timer', process)
        time.sleep(.5)
        typed = time.monotonic()
        subprocess.run(['wtype', '-M', 'ctrl', 'b', '-m', 'ctrl'], env=env, check=True, timeout=20)
        fio.wait(lambda: len(replies()) >= 1, 5, 'the key typed during sit-for as a command', process)
        unread = {'records': records(state), 'reply': replies()[0], 'seconds': round(time.monotonic() - typed, 2)}
        report['unread'] = unread
        check('a key sit-for read in a timer runs as the next command, with no other key',
              '(sit-end nil)' in unread['records'] and unread['reply']['native_command'] == 'backward-char'
              and unread['reply']['signal'] is None, unread)
        time.sleep(.5)
        for count, key in enumerate([['-k', 'Return'], ['-M', 'ctrl', 'j', '-m', 'ctrl']], 2):
            subprocess.run(['wtype', *key], env=env, check=True, timeout=20)
            fio.wait(lambda: len(replies()) >= count, 10, f'command {count}', process)
        time.sleep(.5)
        shown = replies()[1:]
        start = len(records(state))
        subprocess.run(['wtype', '-k', 'F8'], env=env, check=True, timeout=20)
        try:
            fio.wait(lambda: any(r.startswith('(post self-insert-command 99') for r in records(state)[start:]), 5, 'the queued events', process)
        except AssertionError:
            pass
        later = records(state)[start:]
        posts = [r for r in later if r.startswith('(post self-insert-command')]
        report['keys_queued_records'] = later
        # F8's command and the three it queued, each between the hooks.
        hooked = len(later) == 8 and all(later[i].startswith('(pre') and later[i + 1].startswith('(post') for i in range(0, 8, 2))
        queued = {'posts': posts, 'hooked': bool(later) and hooked}
        time.sleep(.5)
        fio.shot(args.output / 'keys.png', env, report)
        # The selected window's point before the keys and after each.
        points, selected = [], None
        for line in log.read_text(errors='replace').split('\n'):
            if line.startswith('{') and '"cursor-publication"' in line:
                record = json.loads(line)
                selected = next((c['window'] for c in record['cursors'] if c['active']), selected)
                points.append((record['presentation'], next(p['point_byte'] for p in record['points'] if p['id'] == selected)))
        # A reply names the last presentation submitted when its command
        # ended: the paint of the state before that command.
        before = [b for presentation, b in points if presentation <= shown[0]['presentation']][-1]
        after = [next(b for presentation, b in points if presentation > r['presentation']) for r in shown]
    report['keys'] = {'bindings': bindings, 'replies': shown, 'point_bytes': [before, *after], 'queued': queued}
    check('each inserted one newline at point', [after[0] - before, after[1] - after[0]] == [1, 1], [before, *after])
    queued = report['keys'].get('queued')
    check('F8\'s unread events run next, a b c, between the command hooks, b unrecorded', queued == {
        'posts': ['(post self-insert-command 97 nil)', '(post self-insert-command 98 nil)', '(post self-insert-command 99 nil)'],
        'hooked': True}, queued)
    check(f'RET is bound to newline and C-j elsewhere in *scratch* ({bindings})', ret == 'newline' and cj not in ('newline', 'nil')
          and bindings.endswith('[13])'), bindings)
    check('Return runs RET\'s binding, C-j its own, neither signalling',
          [(r['native_command'], r['signal']) for r in shown] == [(ret, None), (cj, None)], shown)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'), help='pinned GNU emacs')
    parser.add_argument('--only', choices=['tabs', 'motion', 'keys'])
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
        if args.only in (None, 'keys'):
            check_return(args, report, check)
        if args.only in (None, 'tabs'):
            check_tabs(args, report, check)
        if args.only in (None, 'motion'):
            check_motion(args, report, check)
            check_motion(args, report, check, TALL_FILE, TALL_KEYS, 'scroll')
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
