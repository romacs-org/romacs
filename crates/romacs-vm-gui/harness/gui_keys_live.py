#!/usr/bin/env python3
"""The GUI's own keys, pressed for real in a native --loadup session in a
private headless compositor (ledger/divergences/VM2-GUI-KEYS).

buffer: in *scratch* (fundamental-mode), Return and an unbound Ctrl+m run
RET's `newline'; Tab and an unbound Ctrl+i TAB's binding; Ctrl+[ f is M-f;
Ctrl+Space sets the mark; Meta+Tab and Ctrl+Meta+i run M-TAB's binding.
Then F8 binds C-<m> and C-<i> (`keymap-set'): Ctrl+m and Ctrl+i run their
own command, Return and Tab stay RET and TAB; F6 removes the bindings.

minibuffer: M-x reads a command name, and Ctrl+m ends the read as RET.

terminal: in romacs-term a raw-mode program records the bytes it reads:
Ctrl+m, Ctrl+i, Ctrl+[, Ctrl+Space, Tab, Return and Ctrl+Meta+m send CR,
TAB, ESC, NUL, TAB, CR and ESC CR, as a terminal sends them.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY.  labwc, wtype and grim come from --tools or PATH.

usage: gui_keys_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live

INIT = '''(setq inhibit-startup-screen t initial-major-mode 'fundamental-mode)
(defun gkl-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(add-hook 'post-command-hook (lambda () (gkl-record
  (list 'post this-command last-command-event (this-command-keys-vector) (minibufferp)))))
(defun gkl-own () (interactive) (gkl-record (list 'own last-command-event)))
(defun gkl-mb () (interactive) (gkl-record '(mb-ran)))
(global-set-key [f8] (lambda () (interactive)
  (keymap-set global-map "C-<m>" #'gkl-own) (keymap-set global-map "C-<i>" #'gkl-own)))
(global-set-key [f6] (lambda () (interactive)
  (keymap-unset global-map "C-<m>" t) (keymap-unset global-map "C-<i>" t)))
(global-set-key [f7] (lambda () (interactive)
  (gkl-record (list 'facts (key-binding [?\\M-\\t]) (key-description (where-is-internal 'gkl-own nil t))))))
(run-with-timer 1 nil (lambda () (gkl-record '(ready))))
'''

# A raw-mode program writing the bytes it reads to `terminal-keys', until a Q.
RECORDER = '''import os, pathlib, termios, tty
saved = termios.tcgetattr(0)
data = bytearray()
try:
    tty.setraw(0)
    pathlib.Path('recorder-ready').write_text('ready')
    while not data.endswith(b'Q'):
        data.extend(os.read(0, 4096))
        pathlib.Path('terminal-keys').write_bytes(bytes(data))
finally:
    termios.tcsetattr(0, termios.TCSANOW, saved)
'''

# Keybindings of its own, so labwc's defaults (Alt+Tab among them) stay off.
RC_XML = live.RC_XML.replace('</labwc_config>',
                             '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', help='directory with labwc, wtype and grim')
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'checks': [], 'failures': []}

    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    state = None

    def prepare(s):
        nonlocal state
        state = s.work / 'gui-keys-state.el'
        (s.home / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))
        (s.work / 'record-keys.py').write_text(RECORDER)

    flags = ('--loadup', '--eval', '(setq romacs-keeper-enabled nil)', '--trace-scene')
    with live.session(binary, output, prefix='gui-keys-live-', args=flags, rc_xml=RC_XML, tools=args.tools,
                      env={'SHELL': '/bin/sh'}, prepare=prepare, report=report) as s:
        env, process, work, log_path = s.env, s.process, s.work, s.log

        def records():
            return state.read_text().splitlines() if state.exists() else []

        def posts():
            return [r for r in records() if r.startswith('(post ')]

        def press(*parts, commands=1):
            """Press PARTS (wtype's arguments); wait for COMMANDS more commands."""
            before = len(posts())
            subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
            fio.wait(lambda: len(posts()) >= before + commands, args.timeout, f'{parts} ran', process)
            return posts()[before:]

        def chord(modifiers, key):
            downs = [p for m in modifiers for p in ('-M', m)]
            ups = [p for m in reversed(modifiers) for p in ('-m', m)]
            return [*downs, *key, *ups]

        fio.wait(lambda: '(ready)' in records(), args.timeout, 'the init', process)
        steps = [
            ('Return', ['-k', 'Return'], 1), ('Ctrl+m', chord(['ctrl'], ['m']), 1),
            ('Tab', ['-k', 'Tab'], 1), ('Ctrl+i', chord(['ctrl'], ['i']), 1),
            ('Ctrl+[ f', chord(['ctrl'], ['[']) + ['f'], 1), ('Ctrl+Space', chord(['ctrl'], ['-k', 'space']), 1),
            ('Meta+Tab', chord(['alt'], ['-k', 'Tab']), 1), ('Ctrl+Meta+i', chord(['ctrl', 'alt'], ['i']), 1),
            ('F8', ['-k', 'F8'], 1),
            ('Ctrl+m bound', chord(['ctrl'], ['m']), 1), ('Return after', ['-k', 'Return'], 1),
            ('Ctrl+i bound', chord(['ctrl'], ['i']), 1), ('Tab after', ['-k', 'Tab'], 1),
            ('F7', ['-k', 'F7'], 1), ('F6', ['-k', 'F6'], 1),
        ]
        ran = {name: press(*keys, commands=n) for name, keys, n in steps}
        report['buffer'] = ran
        facts = next((r for r in records() if r.startswith('(facts')), '')
        report['facts'] = facts
        fio.shot(output / 'buffer.png', env, report)

        def command(name):
            return ran[name][-1].split(' ')[1]

        check('Return and unbound Ctrl+m run newline, event 13',
              [r.split(' ')[1:3] for r in ran['Return'] + ran['Ctrl+m']] == [['newline', '13']] * 2, ran)
        check('Tab and unbound Ctrl+i run TAB\'s binding, event 9',
              command('Tab') == command('Ctrl+i') and ran['Tab'][-1].split(' ')[2] == ran['Ctrl+i'][-1].split(' ')[2] == '9', ran)
        check('Ctrl+[ f is M-f', command('Ctrl+[ f') == 'forward-word', ran['Ctrl+[ f'])
        check('Ctrl+Space sets the mark', command('Ctrl+Space') == 'set-mark-command', ran['Ctrl+Space'])
        m_tab = facts.split(' ')[1] if facts else None
        check(f'Meta+Tab and Ctrl+Meta+i run M-TAB\'s binding ({m_tab})',
              m_tab not in (None, 'nil') and command('Meta+Tab') == command('Ctrl+Meta+i') == m_tab, [ran['Meta+Tab'], ran['Ctrl+Meta+i'], facts])
        check('bound, Ctrl+m and Ctrl+i run their own command with their own events',
              ran['Ctrl+m bound'][-1].startswith('(post gkl-own C-m [C-m]') and ran['Ctrl+i bound'][-1].startswith('(post gkl-own C-i [C-i]'),
              [ran['Ctrl+m bound'], ran['Ctrl+i bound']])
        check('Return and Tab stay RET and TAB once Ctrl+m and Ctrl+i are bound',
              command('Return after') == 'newline' and command('Tab after') == command('Tab'), [ran['Return after'], ran['Tab after']])
        check('where-is shows a binding in the syntax keymap-set takes', '"C-<' in facts, facts)

        # The minibuffer: M-x gkl-mb, ended by Ctrl+m (F6 removed the global
        # binding, which would come before the minibuffer's RET as a global
        # [return] does in GNU).
        subprocess.run(['wtype', *chord(['alt'], ['x'])], env=env, check=True, timeout=20)
        fio.wait(lambda: any('M-x ' in row for row in ff.current_rows(log_path.read_text(errors='replace'))), args.timeout, 'M-x', process)
        subprocess.run(['wtype', '-d', '40', 'gkl-mb'], env=env, check=True, timeout=20)
        time.sleep(.5)
        subprocess.run(['wtype', *chord(['ctrl'], ['m'])], env=env, check=True, timeout=20)
        try:
            fio.wait(lambda: '(mb-ran)' in records(), args.timeout, 'gkl-mb', process)
        except AssertionError:
            pass
        check('in the minibuffer Ctrl+m ends the read as RET', '(mb-ran)' in records(), records()[-6:])

        # The terminal: the program reads the bytes a terminal sends.
        subprocess.run(['wtype', *chord(['alt'], ['x'])], env=env, check=True, timeout=20)
        fio.wait(lambda: any('M-x ' in row for row in ff.current_rows(log_path.read_text(errors='replace'))), args.timeout, 'M-x', process)
        subprocess.run(['wtype', '-d', '40', 'romacs-term', '-k', 'Return'], env=env, check=True, timeout=20)
        time.sleep(2)
        subprocess.run(['wtype', '-d', '40', 'python3 record-keys.py', '-k', 'Return'], env=env, check=True, timeout=20)
        recorded = work / 'terminal-keys'
        try:
            fio.wait(lambda: (work / 'recorder-ready').exists(), args.timeout, 'the recorder', process)
            for keys in [chord(['ctrl'], ['m']), chord(['ctrl'], ['i']), chord(['ctrl'], ['[']), chord(['ctrl'], ['-k', 'space']),
                         ['-k', 'Tab'], ['-k', 'Return'], chord(['ctrl', 'alt'], ['m']), ['Q']]:
                subprocess.run(['wtype', *keys], env=env, check=True, timeout=20)
                time.sleep(.3)
            fio.wait(lambda: recorded.exists() and recorded.read_bytes().endswith(b'Q'), args.timeout, 'the bytes', process)
        except AssertionError as error:
            report['terminal_error'] = str(error)
        fio.shot(output / 'terminal.png', env, report)
        got = recorded.read_bytes() if recorded.exists() else b''
        report['terminal_bytes'] = list(got)
        check('in romacs-term the keys send CR TAB ESC NUL TAB CR, ESC CR', got == b'\r\t\x1b\x00\t\r\x1b\rQ', list(got))
        report['status'] = 'FAIL' if report['failures'] else 'PASS'
    print(report['status'], json.dumps({k: report.get(k) for k in ('failures', 'error', 'terminal_error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
