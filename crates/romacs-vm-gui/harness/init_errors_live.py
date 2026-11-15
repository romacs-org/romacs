#!/usr/bin/env python3
"""Broken user inits in a real GUI session (parity_live.py's private
labwc, HOME, XDG): each starts, says in *Warnings* what stopped its init
(the file, line, form and error, the backtrace a button away, as
lisp/romacs/romacs-init-load.el words it), ends the launch screen's init
stage `failed', and stops at the failing form.  A loop, one that
allocates, and a network call that never answers give way to C-g typed
during the init.

  ok               an init that loads: no warning.
  syntax           an unfinished form.
  void-function    a call to a function nobody defined, from a defun.
  missing-package  (require 'a-package-not-installed).
  loop             (while t).
  loop-alloc       (while t (make-string 10 ?a)).
  network          url-retrieve-synchronously from a port that never answers.
  loop-command     after startup: M-: (while t ...) RET, then C-g.

usage: init_errors_live.py --romacs BIN --output DIR [--case NAME]...
"""
import argparse
import json
import socket
import sys
import threading
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--romacs', required=True)
p.add_argument('--output', required=True, type=Path)
p.add_argument('--case', action='append')
args = p.parse_args()
sys.path.insert(0, str(Path(__file__).resolve().parent))
import parity_live  # noqa: E402
from parity_live import C, K, M, T, lisp_string, plist, private_tree, romacs_engine, wait  # noqa: E402

# A port that accepts and never answers: a slow network call.
silent = socket.socket()
silent.bind(('127.0.0.1', 0))
silent.listen(8)
PORT = silent.getsockname()[1]
held = []
threading.Thread(target=lambda: [held.append(silent.accept()) for _ in iter(int, 1)], daemon=True).start()

CASES = {
    'ok': ('(setq ie-after 1)\n', None),
    'syntax': ('(setq ie-after (\n', None),
    'void-function': ('(defun ie-broken () (ie-no-such-function 1))\n(ie-broken)\n(setq ie-after 1)\n', None),
    'missing-package': ("(require 'ie-no-such-package)\n(setq ie-after 1)\n", None),
    'loop': ('(while t)\n(setq ie-after 1)\n', 'C-g'),
    'loop-alloc': ('(while t (make-string 10 ?a))\n(setq ie-after 1)\n', 'C-g'),
    'loop-command': ('(setq ie-after 1)\n', 'command'),
    'network': (f'(require \'url)\n(url-retrieve-synchronously "http://127.0.0.1:{PORT}/" t nil 600)\n(setq ie-after 1)\n', 'C-g'),
}

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
{broken}'''

STATE = '''(list :warnings (let ((b (get-buffer "*Warnings*"))) (and b (with-current-buffer b (buffer-substring-no-properties (point-min) (point-max)))))
       :button (let ((b (get-buffer "*Warnings*"))) (and b (with-current-buffer b (save-excursion (goto-char (point-min)) (and (search-forward "Show the backtrace" nil t) (button-at (1- (point))) t)))))
       :init (alist-get 'init (bound-and-true-p romacs-progress-state))
       :after-init (and after-init-time t)
       :after (bound-and-true-p ie-after)
       :shown (mapcar (lambda (w) (buffer-name (window-buffer w))) (window-list nil 'nomini))
       :frame (and (frame-visible-p (selected-frame)) t))'''

base = args.output
base.mkdir(parents=True, exist_ok=True)
results = {}
for name in args.case or CASES:
    text, key = CASES[name]
    root, home, env = private_tree('romacs', base, parity_live.TOOLS, prefix=f'ie-{name}-')
    init = INIT.format(probe=lisp_string(str(parity_live.PROBE)), broken=text)
    started = time.time()
    engine = None
    result = {}
    try:
        engine = romacs_engine(args.romacs, root, home, env, init, None, 60, parity_live.TOOLS)
        if key == 'command':
            wait(lambda: engine.ask('(and after-init-time t)', 10), 60, f'{name} startup')
            engine.keys(M(':'), T('(while t (make-string 10 ?a))'), K('Return'))
            key = 'C-g'
        if key:
            time.sleep(3)
            result['before_key'] = engine.process.poll() is None
            engine.keys(C('g'))
        wait(lambda: engine.ask('(and after-init-time t)', 10), 60, f'{name} startup')
        result.update(json.loads(json.dumps(plist(engine.ask(STATE, 30)), default=str)))
        result['seconds'] = round(time.time() - started, 1)
        result['alive'] = engine.process.poll() is None
    except Exception as error:  # noqa: BLE001
        result['error'] = f'{type(error).__name__}: {error}'
    finally:
        if engine:
            engine.stop()
    warned = result.get('warnings') or ''
    if name in ('ok', 'loop-command'):
        result['ok'] = (result.get('alive') is True and not warned and result.get('after') == 1)
    else:
        said = {'syntax': 'reader: end-of-file', 'void-function': '(ie-broken): (void-function ie-no-such-function)',
                'missing-package': "(require 'ie-no-such-package): (file-missing", 'loop': '(while t): (quit)',
                'loop-alloc': '(make-string 10 ?a)): (quit)', 'network': '(url-retrieve-synchronously'}[name]
        result['ok'] = (result.get('alive') is True and result.get('frame') == 't' and result.get('after') is None
                        and 'Error (initialization)' in warned and said in warned and result.get('button') == 't'
                        and 'failed' in (result.get('init') or []))
    results[name] = result
    print(name, 'ok' if result['ok'] else 'FAIL', json.dumps(result)[:1500], flush=True)
(base / 'results.json').write_text(json.dumps(results, indent=1, default=str))
sys.exit(0 if all(r['ok'] for r in results.values()) else 1)
