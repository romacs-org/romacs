#!/usr/bin/env python3
"""`debug-on-event' in live sessions, pinned GNU and Romacs side by side:
SIGUSR2 enters the debugger at once, whether the session waits for input or
runs a command that reads none (keyboard.c handle_user_signal).

Each workflow starts a fresh session in its own private headless labwc,
HOME, XDG and TMPDIR (parity_live.py).  Its init sets `debug-on-event' to
`sigusr2' and a `debugger' that records its arguments and the time; it
loads no probe, and a second after the window, or a minibuffer, is up it
cancels every timer, so an idle session sleeps until something wakes it.

  idle    the session waits for input; SIGUSR2 arrives with no key and
          no timer.
  prompt  M-x waits in the minibuffer; SIGUSR2 arrives with no key and
          no timer.
  busy    F9 runs a command that loops without reading input; SIGUSR2
          breaks it, as C-g breaks it with `quit'.

PASS needs, in each engine: the debugger entered within 250 ms of the
signal, no timer pending when it was sent, and the session alive after.
The debugger's arguments, and `quit-flag', `inhibit-quit', `debug-on-quit'
and `debug-on-next-call' as it starts, are recorded: `(error (quit))' when the quit
entered it; GNU, idle, enters it first at the next function call
(`debug-on-next-call'), as `(lambda)', which Romacs does not implement.
Romacs renders on the real GPU when ROMACS_HARNESS_WLR_ENV is set.

usage: debug_event_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import sys
import time

import parity_live
from parity_live import K, M, gnu_engine, lisp_string, private_tree, read_sexp, romacs_engine, wait

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(setq debug-on-event 'sigusr2)
(defun debug-event-live-debugger (&rest args)
  (write-region (prin1-to-string (list (float-time) args
                                       (list quit-flag inhibit-quit debug-on-quit debug-on-next-call)))
                nil {entered} nil 'silent)
  (setq debug-on-quit nil)
  (top-level))
(setq debugger #'debug-event-live-debugger)
(defun debug-event-live-busy () (interactive)
  (write-region "busy" nil {busy} nil 'silent)
  (let ((n 0)) (while t (setq n (1+ n)))))
(global-set-key [f9] #'debug-event-live-busy)
(defun debug-event-live-quiet ()
  "In a second, cancel every timer and record how many are left."
  (run-at-time 1 nil
               (lambda ()
                 (mapc #'cancel-timer (append timer-list timer-idle-list))
                 (write-region (number-to-string (length (append timer-list timer-idle-list)))
                               nil {quiet} nil 'silent))))
(add-hook 'minibuffer-setup-hook #'debug-event-live-quiet)
(add-hook 'window-setup-hook
          (lambda ()
            (blink-cursor-mode -1)
            (debug-event-live-quiet)
            (with-temp-file (expand-file-name "alive" (getenv "PARITY_PROBE_DIR"))
              (insert (number-to-string (float-time))))))
'''

WORKFLOWS = ('idle', 'prompt', 'busy')
DEADLINE = .25


def printed(value):
    """VALUE, a list or atom read_sexp returned, printed as Lisp."""
    return '(' + ' '.join(map(printed, value)) + ')' if isinstance(value, list) else str(value)


def launch(engine, base, args, name):
    root, home, env = private_tree(engine, base, args.tools, prefix=f'debug-event-{engine}-{name}-')
    files = {key: root / key for key in ('entered', 'busy', 'quiet')}
    init = INIT.format(**{key: lisp_string(str(path)) for key, path in files.items()})
    if engine == 'gnu':
        init += '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools), files
    wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
    if wlr:
        env.update(wlr)
        env.pop('WGPU_BACKEND', None)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools), files


def quiet(engine, files):
    """The timers left once the session cancelled them all."""
    timers = int(wait(lambda: files['quiet'].exists() and files['quiet'].read_text(), 15, 'quiet timers', engine.process))
    files['quiet'].unlink()
    return timers


def run(engine, files, name):
    timers = quiet(engine, files)
    if name == 'busy':
        engine.keys(K('F9'))
        wait(files['busy'].exists, 15, f'{engine.name} busy command', engine.process)
        time.sleep(.5)
    else:
        if name == 'prompt':
            engine.keys(M('x'))
            timers = quiet(engine, files)
        time.sleep(1)
    sent = time.time()
    engine.process.send_signal(signal.SIGUSR2)
    try:
        wait(files['entered'].exists, 5, f'{engine.name} debugger', engine.process, .005)
        time.sleep(.05)
        entered, arguments, variables = read_sexp(files['entered'].read_text())
        latency, arguments, variables = round((entered - sent) * 1000, 1), printed(arguments), printed(variables)
    except TimeoutError:
        latency, arguments, variables = None, None, None
    time.sleep(.5)
    result = {'latency_ms': latency, 'debugger_args': arguments, 'timers_pending': timers,
              'quit_inhibit_debug_next': variables,
              'alive': engine.process.poll() is None}
    result['pass'] = latency is not None and latency < DEADLINE * 1000 and timers == 0 and result['alive']
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=WORKFLOWS)
    parser.add_argument('--engine', action='append', choices=('gnu', 'romacs'))
    parser.add_argument('--timeout', type=float, default=300)
    parser.add_argument('--keep', action='store_true', help="keep each session's private tree")
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'gnu': args.gnu, 'workflows': {}}
    status = 0
    for engine_name in args.engine or ('gnu', 'romacs'):
        for name in args.workflow or WORKFLOWS:
            try:
                engine, files = launch(engine_name, Path(args.work), args, name)
                try:
                    result = run(engine, files, name)
                finally:
                    engine.stop()
                    # Each tree holds a world image: a run's would fill a
                    # runtime-directory tmpfs.
                    if not args.keep:
                        shutil.rmtree(engine.root, ignore_errors=True)
            except Exception as error:  # noqa: BLE001 - every failure is the workflow's result
                result = {'pass': False, 'error': f'{type(error).__name__}: {error}'}
            report['workflows'].setdefault(name, {})[engine_name] = result
            print(engine_name, name, 'PASS' if result['pass'] else 'FAIL', json.dumps(result), flush=True)
            status |= not result['pass']
            (output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    return status


if __name__ == '__main__':
    sys.exit(main())
