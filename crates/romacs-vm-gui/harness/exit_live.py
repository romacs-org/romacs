#!/usr/bin/env python3
"""How a live `romacs-vm-gui --loadup' session ends, and what it logged.

Each workflow starts a fresh session in its own private headless labwc,
HOME, XDG and TMPDIR (parity_live.py), with ROMACS_USAGE_LOG naming a file
in its tree, as the Omarchy launcher sets it; keys go through the command
loop, typed by wtype.  With --launcher, the session runs through that
launcher (romacs-preview) instead, from a runtime directory its private
HOME names as ~/.local/state/romacs/current; the launcher then owns the
stderr log and the usage log, and --sessions (romacs-sessions) summarises
them.

  kill-emacs  self-insert keys, a command of 300 ms (F5), a command that
              signals with a file name in its message (F6), a command of
              300 ms after reading a name in the minibuffer (F7, counted
              once in its own mode, timed from the read's end), then
              M-: (kill-emacs 3) RET: exit status 3, stderr's last line
              `romacs-exit: {"reason": "kill-emacs", ..., "status": 3}',
              the usage log's session start, init-done and kill, the counts,
              the slow command, the error with <file> for its file name,
              and none of the typed text.
  compositor  the session's compositor killed (SIGKILL): the session ends
              with status 1, stderr saying `romacs-exit' with the reason
              display-lost.

usage: exit_live.py --romacs BIN --output DIR [--launcher romacs-preview --sessions romacs-sessions] [--workflow NAME]...
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

import parity_live
from parity_live import C, K, M, T, Engine, lisp_string, plist, prepare_world, private_tree, wait

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(defun ul-slow () (interactive) (let ((end (+ (float-time) 0.3))) (while (< (float-time) end))))
(defun ul-error () (interactive) (error "Cannot read %s" "/home/someone/private-notes.org"))
(defun ul-ask (name) (interactive "sName: ") (ul-slow) (message "Hello %s" name))
(global-set-key [f5] #'ul-slow)
(global-set-key [f7] #'ul-ask)
(global-set-key [f6] #'ul-error)
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

SECRET_TEXT = 'zqxjv'


def ready(engine, predicate, what):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         engine.timeout, f'{engine.name} {what}', engine.process, .05)


def runtime(root, romacs):
    """A launcher's runtime directory: the binary and this checkout's Lisp."""
    directory = root / 'runtime'
    directory.mkdir()
    (directory / 'romacs-vm-gui').symlink_to(romacs)
    for name in ('gnu-lisp', 'rlisp', 'etc'):
        (directory / name).symlink_to(parity_live.ROOT / name)
    return directory


def start(args, base, name):
    """A session for workflow NAME: (ENGINE, its stderr log, its usage log)."""
    root, home, env = private_tree('romacs', base, args.tools, prefix=f'exit-{name}-')
    (home / '.config/romacs').mkdir()
    (home / '.config/romacs/init.el').write_text(INIT.format(probe=lisp_string(str(parity_live.PROBE))))
    env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    if args.launcher:
        directory = runtime(root, args.romacs)
        (home / '.local/state/romacs').mkdir(parents=True)
        (home / '.local/state/romacs/current').symlink_to(directory)
        env.update(ROMACS_RUNTIME_ROOT=str(directory), ROMACS_LISP_DIR=str(directory / 'gnu-lisp'))
        argv = [args.launcher]
    else:
        env['ROMACS_USAGE_LOG'] = str(root / 'usage.jsonl')
        argv = [args.romacs, '--loadup']
    prepare_world(args.romacs, env, root / 'work')
    engine = Engine('romacs', argv, root, None, env, args.timeout).start(args.tools)
    # Keys typed before the window has the keyboard are lost.
    ready(engine, '(eq (frame-focus-state) t)', 'focus')
    if args.launcher:
        session = next((home / '.local/state/romacs/sessions').glob('*/'))
        return engine, session / 'stderr.log', session / 'usage.jsonl'
    return engine, root / 'engine.log', root / 'usage.jsonl'


def exit_line(log):
    lines = [line for line in log.read_text(errors='replace').splitlines() if line.startswith('romacs-exit: ')]
    return json.loads(lines[-1].removeprefix('romacs-exit: ')) if lines else None


def ended(engine):
    wait(lambda: engine.process.poll() is not None, engine.timeout, 'session exit')
    return engine.process.returncode


def run_kill_emacs(engine, stderr, usage):
    engine.keys(T(SECRET_TEXT), C('a'))
    ready(engine, f'(string-prefix-p {lisp_string(SECRET_TEXT)} (buffer-string))', 'typed')
    engine.keys(K('F5'))
    engine.keys(K('F6'))
    ready(engine, '(equal (current-message) "Cannot read /home/someone/private-notes.org")', 'the error shown')
    engine.keys(K('F7'))
    ready(engine, '(equal (minibuffer-prompt) "Name: ")', 'the prompt')
    engine.keys(T('ab'))
    ready(engine, '(equal (minibuffer-contents-no-properties) "ab")', 'the name typed')
    time.sleep(1.5)
    engine.keys(K('Return'))
    ready(engine, '(equal (current-message) "Hello ab")', 'the prompt command done')
    engine.keys(M(':'), T('(kill-emacs 3)'))
    ready(engine, f'(equal (minibuffer-contents-no-properties) {lisp_string("(kill-emacs 3)")})', 'kill-emacs typed')
    engine.keys(K('Return'))
    status = ended(engine)
    text = usage.read_text()
    lines = [json.loads(line) for line in text.splitlines()]
    counts = {(r['command'], r['mode']): r['n'] for r in lines if r['kind'] == 'command' and 'n' in r}
    slow = [r for r in lines if r['kind'] == 'command' and 'ms' in r]
    errors = [r for r in lines if r['kind'] == 'error']
    events = [r['event'] for r in lines if r['kind'] == 'session']
    checks = {
        'status 3': status == 3,
        'romacs-exit kill-emacs 3': exit_line(stderr) == {'reason': 'kill-emacs', 'detail': '', 'status': 3},
        'session start, init-done, kill': events == ['start', 'init-done', 'kill'],
        'self-insert counted': counts.get(('self-insert-command', 'lisp-interaction-mode'), 0) >= len(SECRET_TEXT),
        'slow command': any(r['command'] == 'ul-slow' and r['ms'] >= 300 for r in slow),
        'prompt command once, in its mode': counts.get(('ul-ask', 'lisp-interaction-mode')) == 1
            and not any(c == 'ul-ask' and m != 'lisp-interaction-mode' for c, m in counts),
        'prompt command timed from the read': [r['mode'] for r in slow if r['command'] == 'ul-ask'] == ['lisp-interaction-mode']
            and all(300 <= r['ms'] < 1300 for r in slow if r['command'] == 'ul-ask'),
        'error with <file>': {'kind': 'error', 'command': 'ul-error', 'error': '(error . "<file>")'} in errors,
        'no typed text or file name': SECRET_TEXT not in text and 'private-notes' not in text,
    }
    return checks, {'status': status, 'exit': exit_line(stderr), 'usage': lines}


def run_compositor(engine, stderr, usage):
    engine.keys(T('x'))
    ready(engine, '(string-prefix-p "x" (buffer-string))', 'typed')
    engine.processes[0].send_signal(signal.SIGKILL)
    status = ended(engine)
    line = exit_line(stderr)
    checks = {'status 1': status == 1, 'romacs-exit display-lost': bool(line) and line['reason'] == 'display-lost'
              and line['status'] == 1}
    return checks, {'status': status, 'exit': line}


WORKFLOWS = {'kill-emacs': run_kill_emacs, 'compositor': run_compositor}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--launcher', help='run each session through this launcher (romacs-preview)')
    parser.add_argument('--sessions', help='with --launcher: the session summariser (romacs-sessions)')
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'launcher': args.launcher, 'workflows': []}
    for name in args.workflow or list(WORKFLOWS):
        entry = {'workflow': name}
        report['workflows'].append(entry)
        engine = None
        try:
            engine, stderr, usage = start(args, Path(args.work), name)
            checks, entry['observed'] = WORKFLOWS[name](engine, stderr, usage)
            entry['checks'] = checks
            if args.launcher and args.sessions:
                home = engine.env['HOME']
                entry['sessions'] = subprocess.run([sys.executable, args.sessions], env={**os.environ, 'HOME': home},
                                                   capture_output=True, text=True, timeout=60).stdout
            entry['status'] = 'PASS' if all(checks.values()) else 'FAIL'
        except Exception as error:  # noqa: BLE001 - every failure is the workflow's result
            entry.update(status='ERROR', error=f'{type(error).__name__}: {error}')
        finally:
            if engine:
                engine.stop()
        print(name, entry['status'], entry.get('error', ''), {k: v for k, v in entry.get('checks', {}).items() if not v},
              flush=True)
        if entry.get('sessions'):
            print(entry['sessions'], flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
