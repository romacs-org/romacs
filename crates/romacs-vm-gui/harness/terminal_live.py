#!/usr/bin/env python3
"""Native terminal/editor key and child-lifecycle proof in a private compositor."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live


def cursor_state(trace):
    latest = None
    for line in trace.split('\n'):
        try:
            record = json.loads(line)
        except (ValueError, TypeError):
            continue
        if record.get('diagnostic') == 'cursor-publication':
            latest = record
    assert latest is not None, 'TERMINAL_CURSOR_PUBLICATION'
    cursor = next(c for c in latest['cursors'] if c['active'])
    point = next(p['point_byte'] for p in latest['points'] if p['id'] == cursor['window'])
    return {'point_byte': point, 'xy': [cursor['x'], cursor['y']]}


def process_running(pid):
    try:
        status = Path(f'/proc/{pid}/stat').read_text()
        return status.rsplit(')', 1)[1].strip().split()[0] != 'Z'
    except FileNotFoundError:
        return False


def exercise_history_buffers(keys, control, extended, shell, visible, check, trace, report):
    """Real bash: M-p recalls and re-runs its last line; C-2 M-x romacs-term
    makes *romacs-term*<2>, and each name switches back to its own terminal."""
    shell('n=0')
    shell('echo HISTORY_$((++n))')
    visible('HISTORY_1')
    keys('-M', 'alt', 'p', '-m', 'alt')
    keys('-k', 'Return')
    visible('HISTORY_2')
    control('2')
    extended('romacs-term')
    check('TERMINAL_C_2_OPENS_ANOTHER', lambda: not any('HISTORY_' in row for row in ff.current_rows(trace())))
    shell('echo SECOND_TERMINAL')
    visible('SECOND_TERMINAL')
    extended('romacs-term')
    visible('HISTORY_2')
    control('2')
    extended('romacs-term')
    visible('SECOND_TERMINAL')
    report['checks'].append('TERMINAL_HISTORY_AND_NUMBERED_BUFFERS')


def exercise_exit_resize(process, keys, control, extended, shell, visible, capture,
                         trace, original, env, args, report):
    control('x')
    keys('1')
    keys('DIRTY')
    visible('DIRTYTerminal safety sentinel')
    extended('romacs-term')
    shell('stty -echo; printf "EXIT_READY\\n"')
    visible('EXIT_READY')
    shell('printf "FINAL_EXIT_OUTPUT\\n"; exit')
    visible('FINAL_EXIT_OUTPUT')
    fio.wait(lambda: any(g['role'] == 'ModeLine' for g in (fio.last_scene(trace()) or {}).get('glyphs', []))
             and 'exited' in ''.join(g['char'] for g in fio.last_scene(trace())['glyphs'] if g['role'] == 'ModeLine'),
             args.timeout, 'final output and exited mode line', process)
    capture('exited-transcript')
    subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', '1800x1000'],
                   env=env, check=True, timeout=15)
    extended('romacs-gui-split-below')
    visible('FINAL_EXIT_OUTPUT')
    capture('exited-after-resize-and-split')
    assert process.poll() is None, 'EXITED_TERMINAL_GEOMETRY_MUST_NOT_EXIT_EDITOR'
    extended('switch-to-buffer')
    visible('Switch to buffer')
    keys('original.txt', '-k', 'Return')
    visible('DIRTYTerminal safety sentinel')
    keys('!')
    visible('DIRTY!Terminal safety sentinel')
    assert original.read_text() == 'Terminal safety sentinel\n', 'EXIT_RESIZE_MUST_PRESERVE_UNSAVED_FILE'
    control('x')
    control('s')
    fio.wait(lambda: original.read_text() == 'DIRTY!Terminal safety sentinel\n', args.timeout,
             'dirty file remains editable and saves after terminal exit and resize', process)
    capture('dirty-file-saved-after-terminal-exit')
    keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
    process.wait(timeout=args.timeout)
    assert process.returncode == 0
    report.update(status='PASS', exit=process.returncode,
                  checks=['EXITED_TERMINAL_RESIZE_AND_SPLIT', 'DIRTY_TEXT_POINT_AND_SAVE_PRESERVED'],
                  saved_text=original.read_text())


def check_editor_prefix(process, keys, control, shell, visible, capture, trace, work, original, report, timeout):
    received = work / 'terminal-keys'
    shell('/usr/bin/python3 record-keys.py')
    visible('RAW_KEYS_READY')

    def windows():
        records = []
        for line in trace().split('\n'):
            try:
                record = json.loads(line)
            except ValueError:
                continue
            if record.get('diagnostic') == 'cursor-publication':
                records = record['points']
        return len(records)

    control('x')
    keys('2')
    fio.wait(lambda: windows() >= 3 or received.exists(), timeout, 'C-x split or leaked child bytes', process)
    assert windows() >= 3, 'NATIVE_C_X_PREFIX_MUST_STAY_IN_EDITOR'
    control('x')
    keys('b')
    visible('Switch to buffer')
    keys('original.txt', '-k', 'Return')
    visible('Terminal safety sentinel')
    capture('terminal-cx-buffer-switch')
    control('x')
    keys('o')
    control('x')
    keys('b')
    visible('Switch to buffer')
    control('g')
    control('x')
    keys('z')
    control('x')
    control('g')
    keys('TERMINAL_ONLY', '-k', 'Return')
    visible('RAW_KEYS_DONE')
    assert received.read_bytes() == b'TERMINAL_ONLY\r', ('EDITOR_PREFIX_MUST_NOT_SEND_CHILD_BYTES', received.read_bytes())
    capture('terminal-cx-input-resumed')
    assert original.read_text() == 'Terminal safety sentinel\n'
    keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
    visible('Running terminals exist;')
    keys('y')
    process.wait(timeout=timeout)
    assert process.returncode == 0
    report.update(status='PASS', exit=0, original_file_unchanged=True,
                  received_hex=received.read_bytes().hex(),
                  checks=['C_X_SPLIT_IN_TERMINAL_INPUT', 'C_X_B_SWITCHES_BUFFER', 'C_X_O_RETURNS_TO_TERMINAL_INPUT',
                          'C_X_PROMPT_CANCEL_UNKNOWN_AND_QUIT_NO_PTY_BYTES'])


def term_windows(trace, marker):
    """(clip, rows) of each window whose text shows MARKER, top first, and the line height."""
    scene = fio.last_scene(trace) or {'glyphs': []}
    clips = {json.dumps(g['clip'], sort_keys=True) for g in scene['glyphs'] if g['role'] == 'Text'}
    found = [(json.loads(c), fio.scene_rows(scene, json.loads(c))) for c in clips]
    found = sorted(((c, r) for c, r in found if any(marker in row for row in r)), key=lambda cr: cr[0]['y'])
    lines = [g['clip']['height'] for g in scene['glyphs'] if g['role'] == 'ModeLine']
    return found, (min(lines) if lines else None)


def check_prompt_grow(process, keys, control, extended, shell, visible, capture, trace, work, report, timeout):
    """A grown minibuffer covers the grid's bottom rows; the pty never changes size for it."""
    sizes = work / 'pty-sizes'

    def recorded():
        return sizes.read_text().split('\n')[:-1] if sizes.exists() else []

    shell(f"PS1='$ '; trap 'stty size >> {sizes}' WINCH; stty size >> {sizes}; seq 1 200; printf 'SIZE_READY\\n'")
    visible('SIZE_READY')
    fio.wait(lambda: recorded(), timeout, 'pty size recorded', process)
    time.sleep(.5)
    (clip, before), line = term_windows(trace(), 'SIZE_READY')[0][0], term_windows(trace(), 'SIZE_READY')[1]
    created = recorded()[0]
    body = int(clip['height'] // line)
    assert int(created.split()[0]) == body, ('PTY_ROWS_ARE_THE_WINDOW_BODY', created, body)
    capture('prompt-grow-before', cursor=False)
    control('x')
    control('f')
    keys('ls-entry-')
    fio.wait(lambda: (lambda w: w[0] and w[0][0][0]['height'] < clip['height'])(term_windows(trace(), '$ ')),
             timeout, 'the minibuffer grows over the terminal window', process)
    time.sleep(.5)
    (shrunk, during), _ = term_windows(trace(), '$ ')[0][0], None
    capture('prompt-grow-during', cursor=False)
    assert len(during) == int(shrunk['height'] // line) and during == before[-len(during):], \
        ('THE_SHORTER_WINDOW_SHOWS_THE_GRID_TAIL_WITH_THE_CURSOR_ROW', before[-3:], during[-3:])
    control('g')
    fio.wait(lambda: (lambda w: w[0] and w[0][0][0]['height'] == clip['height'])(term_windows(trace(), '$ ')),
             timeout, 'the terminal window restored', process)
    time.sleep(.5)
    after = term_windows(trace(), '$ ')[0][0][1]
    assert after == before, ('ROWS_AFTER_THE_PROMPT_ARE_THE_ROWS_BEFORE', before[-3:], after[-3:])
    assert recorded() == [created], ('NO_WINCH_FOR_A_GROWN_MINIBUFFER', recorded())
    control('g')  # an editor command in the terminal buffer
    shell(f"stty size >> {sizes}; printf 'STTY_DONE\\n'")
    visible('STTY_DONE')
    assert recorded() == [created, created], ('PTY_SIZE_UNCHANGED_AFTER_PROMPT_AND_EDITOR_COMMAND', recorded())
    capture('prompt-grow-after', cursor=False)
    extended('romacs-gui-split-below')
    fio.wait(lambda: len(recorded()) == 3, timeout, 'one WINCH for the split', process)
    time.sleep(1)
    windows, _ = term_windows(trace(), 'STTY_DONE')
    split = recorded()
    top = int(windows[0][0]['height'] // line)
    assert len(split) == 3 and int(split[2].split()[0]) == top, ('SPLIT_RESIZES_ONCE_TO_THE_CONTROLLING_BODY', split, top)
    capture('prompt-grow-split', cursor=False)
    # A prompt over the split restores both panes exactly, without a WINCH.
    panes = [w[0]['height'] for w in windows]
    control('x')
    control('f')
    keys('ls-entry-')
    fio.wait(lambda: [w[0]['height'] for w in term_windows(trace(), 'STTY_DONE')[0]] != panes,
             timeout, 'the minibuffer grows over the split', process)
    control('g')
    fio.wait(lambda: [w[0]['height'] for w in term_windows(trace(), 'STTY_DONE')[0]] == panes,
             timeout, 'the split panes restored', process)
    time.sleep(.5)
    assert recorded() == split, ('NO_WINCH_FOR_A_PROMPT_OVER_A_SPLIT', recorded())
    report.update(status='PASS', exit=0, pty_sizes=split, body_rows=body, shrunk_rows=len(during), split_rows=top,
                  checks=['PTY_ROWS_ARE_THE_WINDOW_BODY', 'THE_SHORTER_WINDOW_SHOWS_THE_GRID_TAIL_WITH_THE_CURSOR_ROW',
                          'ROWS_AFTER_THE_PROMPT_ARE_THE_ROWS_BEFORE', 'NO_WINCH_FOR_A_GROWN_MINIBUFFER',
                          'PTY_SIZE_UNCHANGED_AFTER_PROMPT_AND_EDITOR_COMMAND', 'SPLIT_RESIZES_ONCE_TO_THE_CONTROLLING_BODY',
                          'SPLIT_PANES_RESTORED_AFTER_A_PROMPT', 'NO_WINCH_FOR_A_PROMPT_OVER_A_SPLIT'])
    keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
    visible('Running terminals exist;')
    keys('y')
    process.wait(timeout=timeout)


def check_leave(keys, control, shell, visible, capture, trace, report):
    """C-c C-j leaves the cursor: C-p, M-<, an isearch and M-w work in the
    scrollback as in any buffer; typing returns to the prompt and sends."""
    # The sentinel is in the output alone, not the command line.
    shell('stty -echo; printf "TOP_%s\\n" SENTINEL; seq 1 300; printf "LEAVE_READY\\n"')
    visible('LEAVE_READY')
    control('c')
    control('j')
    visible('Terminal left at its cursor')
    for _ in range(3):
        control('p')
    keys('-M', 'alt', '<', '-m', 'alt')
    visible('TOP_SENTINEL')
    capture('leave-scrolled-back', cursor=False)
    report['checks'] += ['C_C_C_J_LEAVES_THE_CURSOR', 'M_LESS_MOVES_TO_THE_SCROLLBACK_TOP']
    control('s')
    keys('TOP_SENTI')
    keys('-k', 'Return')
    control('a')
    keys('-M', 'ctrl', '-k', 'space', '-m', 'ctrl')
    control('e')
    keys('-M', 'alt', 'w', '-m', 'alt')
    # Typing goes back to the prompt and sends; C-y sends what M-w copied.
    keys('echo COPIED:')
    control('y')
    keys('-k', 'Return')
    visible('COPIED:TOP_SENTINEL')
    capture('leave-returned', cursor=False)
    report['checks'] += ['ISEARCH_AND_M_W_COPY_FROM_THE_SCROLLBACK', 'TYPING_RETURNS_TO_THE_PROMPT']
    # Motion keys stay with Emacs at the prompt, as in shell-mode: C-p moves
    # into the scrollback, C-n back; typing then runs at the prompt.
    control('p')
    control('p')
    control('n')
    shell('printf "TYPED_%s\\n" AWAY')
    visible('TYPED_AWAY')
    report['checks'] += ['C_P_C_N_MOVE_AT_THE_PROMPT', 'TYPING_AFTER_MOVING_RUNS_AT_THE_PROMPT']
    # Up stays bash's history: the last command runs again.
    keys('-k', 'Up')
    keys('-k', 'Return')
    fio.wait(lambda: sum('TYPED_AWAY' in row for row in ff.current_rows(trace())) >= 2, 30, 'Up recalls the last command', None)
    report['checks'].append('UP_RECALLS_BASH_HISTORY')
    # A program the shell runs in the foreground gets C-p: Python's REPL
    # recalls its own history with it.
    shell('python3 -q')
    visible('>>>')
    shell('print("PY_" + "RECALLED")')
    visible('PY_RECALLED')
    control('p')
    keys('-k', 'Return')
    fio.wait(lambda: sum('PY_RECALLED' in row for row in ff.current_rows(trace())) >= 2, 30, 'C-p recalls Python history', None)
    shell('exit()')
    shell('printf "BACK_%s\\n" AT_BASH')
    visible('BACK_AT_BASH')
    report['checks'].append('A_FOREGROUND_REPL_GETS_C_P')
    # Out of the terminal and back, by C-x b and by C-x o: the window is on
    # the prompt again, following the output it shows.
    shell('(sleep 3; printf "BG_%s\\n" BUFFER) &')
    control('x')
    keys('b')
    keys('*scratch*')
    keys('-k', 'Return')
    control('x')
    keys('b')
    keys('*romacs-term*')
    keys('-k', 'Return')
    visible('BG_BUFFER')
    shell('(sleep 3; printf "BG_%s\\n" WINDOW) &')
    control('x')
    keys('2')
    control('x')
    keys('o')
    control('x')
    keys('o')
    visible('BG_WINDOW')
    control('x')
    keys('1')
    capture('leave-switched-back', cursor=False)
    report['checks'] += ['C_X_B_OUT_AND_BACK_LANDS_ON_THE_PROMPT', 'C_X_O_OUT_AND_BACK_LANDS_ON_THE_PROMPT']
    report.update(status='PASS', exit=0)


def check_colors(shell, visible, capture, trace, output, report):
    import struct
    shell("printf '\\033[31mANSI_RED\\033[0m \\033[91mBRIGHT_RED\\033[0m \\033[38;5;196mINDEXED\\033[0m \\033[38;2;12;34;56mTRUE\\033[48;2;65;43;21mBG_SAMPLE\\033[0m RESET\\nCOLOR_DONE\\n'")
    visible('COLOR_DONE')
    capture('ansi-colors')
    scene = fio.last_scene(trace())
    rows = {}
    for glyph in scene['glyphs']:
        if glyph['role'] == 'Text':
            key = (json.dumps(glyph['clip'], sort_keys=True), glyph['y'])
            rows.setdefault(key, []).append(glyph)
    row = next(sorted(row, key=lambda g: g['x']) for row in rows.values()
               if ''.join(g['char'] for g in sorted(row, key=lambda g: g['x'])).startswith('ANSI_RED BRIGHT_RED INDEXED TRUEBG_SAMPLE RESET'))
    text = ''.join(g['char'] for g in row)
    expected = {'ANSI_RED': '#ac4242', 'BRIGHT_RED': '#c55555', 'INDEXED': '#ff0000', 'TRUE': '#0c2238'}
    for label, color in expected.items():
        assert row[text.index(label)]['foreground'] == color, ('NATIVE_TERMINAL_COLOR', label, row[text.index(label)], color)
    assert row[text.index('BG_SAMPLE')]['background'] == '#412b15', 'NATIVE_TERMINAL_BACKGROUND_COLOR'
    assert row[text.index('RESET')]['foreground'] == '#d4c5a0', 'NATIVE_TERMINAL_SGR_RESET'
    pixels = subprocess.run(['magick', str(output / 'ansi-colors.png'), '-alpha', 'off', '-depth', '8', 'rgb:-'],
                            capture_output=True, check=True, timeout=15).stdout
    background_pixels = sum(rgb == (65, 43, 21) for rgb in struct.iter_unpack('BBB', pixels))
    assert background_pixels > 100, 'TERMINAL_BACKGROUND_MUST_REACH_COMPOSITOR_PIXELS'
    report['colors'] = {'foregrounds': expected, 'background': '#412b15', 'background_pixels': background_pixels}
    report['checks'].append('ANSI_16_256_TRUE_COLOR_RESET_AND_BACKGROUND_PIXELS')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--exit-resize', action='store_true')
    parser.add_argument('--colors', action='store_true')
    parser.add_argument('--editor-prefix', action='store_true')
    parser.add_argument('--leave', action='store_true',
                        help='C-c C-j leaves the cursor for the scrollback; typing returns')
    parser.add_argument('--prompt-grow', action='store_true',
                        help='under --loadup (fido-vertical), a growing minibuffer must not resize the pty')
    parser.add_argument('--history-buffers', action='store_true',
                        help='M-p recalls the shell history; numeric prefixes choose the terminal')
    parser.add_argument('--labwc', default=live.LABWC)
    parser.add_argument('--wlr-randr', default=live.RANDR)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'captures': [], 'checks': [], 'scope': 'private software-rendered native keyboard; shell text projection'}
    compositor = keyboard = process = None
    background_pid = None
    try:
        with tempfile.TemporaryDirectory(prefix='term-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home = root / 'home'
            work = root / 'work'
            config = root / 'config'
            for directory in (home, work, config):
                directory.mkdir()
            original = work / 'original.txt'
            original.write_text('Terminal safety sentinel\n')
            if args.editor_prefix:
                (work / 'record-keys.py').write_text(
                    "import os, pathlib, termios, tty\n"
                    "saved = termios.tcgetattr(0)\n"
                    "data = bytearray()\n"
                    "try:\n"
                    "    tty.setraw(0)\n"
                    "    os.write(1, b'\\r\\nRAW_KEYS_READY\\r\\n')\n"
                    "    while not data.endswith(b'TERMINAL_ONLY\\r'):\n"
                    "        data.extend(os.read(0, 4096))\n"
                    "        pathlib.Path('terminal-keys').write_bytes(data)\n"
                    "finally:\n"
                    "    termios.tcsetattr(0, termios.TCSANOW, saved)\n"
                    "print('\\nRAW_KEYS_DONE', flush=True)\n")
            for number in range(24):
                (work / f'ls-entry-{number:02}').write_text('listing fixture\n')
            (work / 'interrupt.py').write_text(
                "import pathlib, signal, time\n"
                "def interrupted(*_):\n"
                "    pathlib.Path('interrupt').write_text('interrupt')\n"
                "    raise SystemExit(0)\n"
                "signal.signal(signal.SIGINT, interrupted)\n"
                "pathlib.Path('sleep-ready').write_text('ready')\n"
                "while True: time.sleep(1)\n")
            (config / 'rc.xml').write_text(live.RC_XML.replace('</labwc_config>',
                '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>'))
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20',
                   'SHELL': os.environ.get('SHELL') or '/bin/sh'}
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([args.labwc, '-C', str(config)], env=env,
                                              stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), args.timeout,
                         'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', '2532x1347'],
                               env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '120000', '-k', 'Shift_L'], env=env,
                                            stdout=log, stderr=log)
                time.sleep(.3)
                # The leave journey needs GNU's Lisp (isearch, M-w), as a
                # daily session has it.
                flags = ['--loadup'] if args.prompt_grow or args.leave or args.history_buffers else []
                if args.prompt_grow:
                    # The minibuffer must grow over the terminal: fido-vertical's
                    # display, not the default layer (VM2-COMPLETE-DEFAULT).
                    (home / 'config' / 'romacs').mkdir(parents=True, exist_ok=True)
                    (home / 'config' / 'romacs' / 'init.el').write_text(
                        "(romacs-complete-mode -1)\n(fido-vertical-mode 1)\n")
                # The terminal in this process, as these journeys always measured it; a
                # kept terminal is term_keep_live.py's.  (A host copied alone has no
                # keeper beside it, and a session meant to use one would refuse.)
                flags += ['--eval', '(setq romacs-keeper-enabled nil)']
                process = subprocess.Popen([str(binary), *flags, '--trace-scene', str(original)], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def trace():
                    return log_path.read_text(errors='replace')

                def keys(*parts):
                    subprocess.run(['/usr/bin/wtype', '-d', '50', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def control(key):
                    keys('-M', 'ctrl', key, '-m', 'ctrl')

                def visible(text):
                    fio.wait(lambda: any(text in row for row in ff.current_rows(trace())), args.timeout,
                             text, process)

                def capture(name, cursor=True):
                    fio.shot(output / (name + '.png'), env, report)
                    assert (output / (name + '.png')).exists(), 'TERMINAL_CAPTURE_REQUIRED'
                    # A blinking cursor (--loadup) may have no active publication.
                    report['captures'].append({'name': name, 'rows': ff.current_rows(trace()),
                                               'cursor': cursor_state(trace()) if cursor else None})

                def extended(command):
                    keys('-M', 'alt', 'x', '-m', 'alt')
                    visible('M-x ')
                    keys(command, '-k', 'Return')

                def shell(command):
                    keys(command, '-k', 'Return')

                def check(name, predicate):
                    fio.wait(predicate, args.timeout, name, process)
                    report['checks'].append(name)

                visible('Terminal safety sentinel')
                if args.exit_resize:
                    exercise_exit_resize(process, keys, control, extended, shell, visible, capture,
                                         trace, original, env, args, report)
                    return 0
                control('x')
                keys('1')
                extended('romacs-term')
                check('TERMINAL_OPEN_NATIVE', lambda: 'romacs-term' in trace() and process.poll() is None)
                if args.history_buffers:
                    exercise_history_buffers(keys, control, extended, shell, visible, check, trace, report)
                    return 0
                if args.prompt_grow:
                    check_prompt_grow(process, keys, control, extended, shell, visible, capture, trace, work, report, args.timeout)
                    return 0
                if args.editor_prefix:
                    check_editor_prefix(process, keys, control, shell, visible, capture, trace, work, original, report, args.timeout)
                    return 0
                if args.leave:
                    check_leave(keys, control, shell, visible, capture, trace, report)
                    return 0
                shell('stty -echo; printf "ROMACS_SHELL_READY\\n"')
                visible('ROMACS_SHELL_READY')
                capture('shell-opened')
                shell('ls -C; ls -C --color=always; printf \"LS_COMPLETED\\n\"')
                visible('LS_COMPLETED')
                capture('ls-columns-and-colors')
                report['checks'].append('NATIVE_LS_COLUMNS_DO_NOT_CRASH_LAYOUT')
                if args.colors:
                    check_colors(shell, visible, capture, trace, output, report)
                chosen_shell = work / 'chosen-shell'
                shell('printf \"%s\" \"$0\" > ' + shlex.quote(str(chosen_shell)))
                check('NATIVE_DEFAULT_IS_USER_SHELL', chosen_shell.exists)
                assert chosen_shell.read_text() == env['SHELL'], (chosen_shell.read_text(), env['SHELL'])
                report['selected_shell'] = chosen_shell.read_text()
                result = work / 'result'
                shell('printf native-child > ' + shlex.quote(str(result)) + '; printf "CHILD_EXECUTED\\n"')
                check('TERMINAL_CHILD_RECEIVED_INPUT', lambda: result.exists() and result.read_text() == 'native-child')
                visible('CHILD_EXECUTED')
                interrupt = work / 'interrupt'
                keys('printf cg-kept > ' + shlex.quote(str(result)))
                control('g')
                keys('-k', 'Return')
                check('C_G_EDITOR_NOT_ETX', lambda: result.read_text() == 'cg-kept')
                assert not interrupt.exists(), 'C_G_MUST_NOT_INTERRUPT_CHILD'
                keys('-M', 'alt', 'x', '-m', 'alt')
                visible('M-x ')
                capture('mx-preserved')
                control('g')
                shell('printf "AFTER_MX_QUIT\\n"')
                visible('AFTER_MX_QUIT')
                ready = work / 'sleep-ready'
                shell('/usr/bin/python3 interrupt.py')
                check('INTERRUPT_CHILD_WAITING', ready.exists)
                control('c')
                control('k')
                check('C_C_C_K_ACTUAL_SIGINT', interrupt.exists)
                shell('printf "INTERRUPT_RECOVERED\\n"')
                visible('INTERRUPT_RECOVERED')
                capture('interrupt-recovered')
                # Moving point off the cursor (M-x is an exception) makes the
                # window a plain Emacs buffer: output does not move it, and
                # typing puts it back on the cursor and sends the keys.
                gate = work / 'output-gate'
                tail = work / 'tail-written'
                shell('seq 1 200')
                shell('while [ ! -f ' + shlex.quote(str(gate)) + ' ]; do sleep .05; done; printf "NAVIGATION_%s\\n" TAIL; : > '
                      + shlex.quote(str(tail)))
                extended('beginning-of-buffer')
                time.sleep(.5)
                before = cursor_state(trace())
                gate.write_text('go')
                check('NAVIGATION_TAIL_WRITTEN', tail.exists)
                time.sleep(.5)
                after = cursor_state(trace())
                assert after == before, f'TERMINAL_AWAY_POINT_STABLE: {before} -> {after}'
                assert not any('NAVIGATION_TAIL' in row for row in ff.current_rows(trace())), 'TERMINAL_AWAY_VIEW_STABLE'
                capture('scrollback-during-output')
                shell('printf "SNAPPED_%s\\n" BACK')
                visible('SNAPPED_BACK')
                visible('NAVIGATION_TAIL')
                report['checks'].append('TERMINAL_SCROLLBACK_STABLE_AND_TYPING_SNAPS_BACK')
                gate2 = work / 'prompt-gate'
                shell('while [ ! -f ' + shlex.quote(str(gate2)) + ' ]; do sleep .05; done; printf "OUTPUT_DURING_MX\\n"')
                keys('-M', 'alt', 'x', '-m', 'alt')
                visible('M-x ')
                gate2.write_text('go')
                check('TERMINAL_OUTPUT_WHILE_PROMPT_SUSPENDED', lambda: any('OUTPUT_DURING_MX' in row for row in fio.rows(trace())))
                capture('output-during-mx')
                control('g')
                shell('printf "CLOSE_READY\\n"')
                visible('CLOSE_READY')
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('Running terminals exist;')
                capture('running-close-consent')
                keys('n')
                visible('Cancelled')
                shell('printf "CANCELLED_CLOSE_USABLE\\n"')
                visible('CANCELLED_CLOSE_USABLE')
                keys('-M', 'alt', 'x', '-m', 'alt')
                visible('M-x ')
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('Running terminals exist;')
                control('g')
                visible('Cancelled')
                shell('printf "PROMPT_CLOSE_RESUMED\\n"')
                visible('PROMPT_CLOSE_RESUMED')
                capture('prompt-close-cancelled')
                extended('switch-to-buffer')
                visible('Switch to buffer')
                keys('original.txt', '-k', 'Return')
                visible('Terminal safety sentinel')
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('Running terminals exist;')
                capture('hidden-terminal-close-consent')
                keys('n')
                visible('Cancelled')
                extended('switch-to-buffer')
                visible('Switch to buffer')
                keys('*romacs-term*', '-k', 'Return')
                visible('PROMPT_CLOSE_RESUMED')
                shell('printf "HIDDEN_TERMINAL_RESUMED\\n"')
                visible('HIDDEN_TERMINAL_RESUMED')
                report['checks'].append('HIDDEN_RUNNING_TERMINAL_CLOSE_PROTECTED')
                assert original.read_text() == 'Terminal safety sentinel\n', 'TERMINAL_UNRELATED_FILE_UNCHANGED'
                pidfile = work / 'background-pid'
                shell('sleep 7777 & printf \"%s\" \"$!\" > ' + shlex.quote(str(pidfile)) + '; printf \"BACKGROUND_READY\\n\"')
                visible('BACKGROUND_READY')
                background_pid = int(pidfile.read_text())
                assert process_running(background_pid), 'NATIVE_BACKGROUND_JOB_STARTED'
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('Running terminals exist;')
                keys('y')
                process.wait(timeout=args.timeout)
                assert process.returncode == 0, 'TERMINAL_CONFIRMED_CLOSE_EXIT'
                fio.wait(lambda: not process_running(background_pid), args.timeout,
                         'confirmed close terminates normal shell background jobs', None)
                report['checks'].append('CONFIRMED_CLOSE_RETIRES_BACKGROUND_JOB')
                report.update(status='PASS', exit=process.returncode, original_file_unchanged=True,
                              child_result=result.read_text(), actual_interrupt=interrupt.read_text())
    except Exception as error:
        report.update(status='FAIL', error=repr(error))
    finally:
        if background_pid is not None and process_running(background_pid):
            command_line = Path(f'/proc/{background_pid}/cmdline').read_bytes()
            if command_line.startswith(b'sleep\x007777\x00'):
                os.kill(background_pid, 9)
        for child in (process, keyboard, compositor):
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=8)
        (output / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
        (output / 'INDEX.txt').write_text('\n'.join(c['name'] + '.png' for c in report['captures']) + '\n')
        print(json.dumps(report, ensure_ascii=False))
    return 0 if report.get('status') == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
