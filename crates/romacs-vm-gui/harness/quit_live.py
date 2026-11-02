#!/usr/bin/env python3
"""C-g during a long evaluation, in pinned GNU and Romacs, side by side.

GNU never queues a C-g typed while Lisp runs: it sets `quit-flag' at once
and the evaluator quits at its next safepoint.  Each workflow starts a fresh
pair of sessions (parity_live.py), runs a command that evaluates for 10 s,
types C-g 200 ms into it (wtype), and measures when the evaluation ended,
from the time its `unwind-protect' records.

  spin         a key bound to a 10 s Lisp loop.
  eval-buffer  `eval-buffer' over a buffer of 2000 forms of 5 ms each,
               as one evaluates a large init file.

PASS needs, in each engine: the evaluation ended by `quit' within 100 ms
of the C-g, `keyboard-quit' never ran (the C-g was the quit, not also a
queued command), the echo area says "Quit", and for eval-buffer, some forms
but not all ran.  Romacs's busy indicator is captured (grim) 1 s in, and once it has gone.
Romacs renders on the real GPU when ROMACS_HARNESS_WLR_ENV is set.

usage: quit_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from parity_live import C, K, gnu_engine, lisp_string, plist, private_tree, romacs_engine, wait
import parity_live

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(defvar quit-live-keyboard-quits 0)
(add-hook 'pre-command-hook
          (lambda () (when (eq this-command 'keyboard-quit)
                       (setq quit-live-keyboard-quits (1+ quit-live-keyboard-quits)))))
(defvar quit-live-forms 0)
(defun quit-live-record (outcome)
  (write-region (prin1-to-string (list outcome (float-time) quit-live-forms)) nil
                (expand-file-name "end.eld" parity-probe-dir) nil 'silent))
(defun quit-live-spin ()
  (interactive)
  (let ((end (+ (float-time) 10)) (outcome 'quit))
    (unwind-protect (progn (while (< (float-time) end)) (setq outcome 'finished))
      (quit-live-record outcome))))
(defun quit-live-eval-buffer ()
  (interactive)
  (let ((outcome 'quit))
    (unwind-protect (progn (eval-buffer) (setq outcome 'finished))
      (quit-live-record outcome))))
(global-set-key [f9] #'quit-live-spin)
(global-set-key [f10] #'quit-live-eval-buffer)
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

FORMS = 2000
FORM = '(let ((end (+ (float-time) 0.005))) (while (< (float-time) end)) (setq quit-live-forms (1+ quit-live-forms)))\n'

WORKFLOWS = {'spin': [K('F9')], 'eval-buffer': [K('F10')]}


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    init = INIT.format(probe=lisp_string(str(parity_live.PROBE)))
    if engine == 'gnu':
        init += '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    # On the real GPU when ROMACS_HARNESS_WLR_ENV gives the compositor its
    # renderer (KEY=VALUE ...): Romacs then draws with the host's Vulkan.
    wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
    if wlr:
        env.update(wlr)
        env.pop('WGPU_BACKEND', None)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


def prepare(engine, name):
    if name == 'eval-buffer':
        source = engine.root / 'work' / 'forms.el'
        source.write_text(';; -*- lexical-binding: t -*-\n' + FORM * FORMS)
        engine.ask(f'(progn (find-file {lisp_string(str(source))}) (setq quit-live-forms 0) t)')
    engine.ask('(progn (message nil) (setq quit-live-keyboard-quits 0) t)')
    end = engine.probe_dir / 'end.eld'
    if end.exists():
        end.unlink()


def run(engine, name, output):
    prepare(engine, name)
    engine.keys(*WORKFLOWS[name])
    time.sleep(.2)
    pressed = time.time()
    engine.keys(C('g'))
    end = engine.probe_dir / 'end.eld'
    wait(end.exists, 15, f'{engine.name} {name} end', engine.process, .005)
    outcome, ended, forms = parity_live.read_sexp(end.read_text())
    time.sleep(.5)
    observed = plist(engine.ask('(list :keyboard-quits quit-live-keyboard-quits'
                                ' :echo (let ((m (current-message))) (and m (substring-no-properties m)))'
                                ' :pending (input-pending-p) :quit-flag quit-flag)'))
    result = {'outcome': str(outcome), 'latency_ms': round((ended - pressed) * 1000, 1), 'forms': forms,
              **observed}
    checks = [str(outcome) == 'quit', result['latency_ms'] < 100, observed['keyboard-quits'] == 0,
              observed['echo'] == 'Quit', not observed['quit-flag']]
    if name == 'eval-buffer':
        checks.append(0 < forms < FORMS)
    result['pass'] = all(checks)
    return result


def busy_indicator(engine, output):
    """Romacs's indicator 1 s into a 10 s evaluation, then a C-g ends it."""
    prepare(engine, 'spin')
    engine.keys(*WORKFLOWS['spin'])
    time.sleep(1)
    shot = output / f'{engine.name}-busy.png'
    subprocess.run([f'{engine.tools}/grim', str(shot)], env=engine.env, check=True, timeout=30)
    engine.keys(C('g'))
    wait((engine.probe_dir / 'end.eld').exists, 15, f'{engine.name} busy end', engine.process, .005)
    time.sleep(.5)
    idle = output / f'{engine.name}-idle.png'
    subprocess.run([f'{engine.tools}/grim', str(idle)], env=engine.env, check=True, timeout=30)
    return {'busy': str(shot), 'idle': str(idle)}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append')
    parser.add_argument('--engine', action='append', choices=('gnu', 'romacs'))
    parser.add_argument('--timeout', type=float, default=300)
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'gnu': args.gnu, 'workflows': {}}
    status = 0
    for engine_name in args.engine or ('gnu', 'romacs'):
        engine = launch(engine_name, Path(args.work), args)
        try:
            for name in args.workflow or WORKFLOWS:
                result = run(engine, name, output)
                report['workflows'].setdefault(name, {})[engine_name] = result
                print(engine_name, name, 'PASS' if result['pass'] else 'FAIL', json.dumps(result), flush=True)
                status |= not result['pass']
            if engine_name == 'romacs':
                report['busy_indicator'] = busy_indicator(engine, output)
        finally:
            engine.stop()
            (output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    return status


if __name__ == '__main__':
    sys.exit(main())
