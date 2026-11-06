#!/usr/bin/env python3
"""C-n and C-p scroll past invisible text as pinned GNU does.

Pinned GNU pgtk and `romacs-vm-gui --loadup', each in its own isolated
session (private HOME/XDG/TMPDIR, headless labwc, keys typed with wtype
into that compositor only) on the same grid and font, show two buffers
whose text is partly invisible: a notmuch-show thread over the
synthetic store (notmuch_fixture.py) with every other message
collapsed (a thread with an image part), and an Org buffer with every
other subtree folded, its faces one height.  Under
`scroll-conservatively' 101, with `scroll-margin' 0 and 3, each types
C-n from the top to the end and C-p back, one key at a time; a
`pre-command-hook' records point and the window start before each key,
and Romacs's records must be GNU's, but for the KNOWN differences, and
the start must only move with the motion, never back as a recentre
does.  Then each holds C-n and C-p (the client's key repeat, so the keys
differ in number), with the same condition on the start.  Romacs's usage
log (ROMACS_USAGE_LOG) must hold no error.

usage: scroll_invisible_live.py --binary PATH --output DIR [--gnu PATH] [--tools DIR] [--timeout S]
"""

import argparse
import json
from pathlib import Path
import os
import sys
import time

import parity_live
from parity_live import C, gnu_engine, lisp_string, private_tree, romacs_engine
import notmuch_fixture

ANCHOR = int(time.time()) // 60 * 60

# Records known to differ from GNU's, by scene and margin.  Keys must
# still never scroll back there.
END_MARGIN = ('near the end of a buffer GNU lets point into the first bottom-margin row and scrolls with it there; '
              'plain, continued and truncated text in point_visibility.rs scroll as Romacs does')
KNOWN = {('notmuch', 3): END_MARGIN, ('org', 3): END_MARGIN}

SCENES = r'''
(require 'cl-lib)
(defvar si-log nil)
(defun si-record ()
  (when (memq this-command '(next-line previous-line))
    (push (list this-command (point) (window-start) (cdr (posn-col-row (posn-at-point)))) si-log)))
(add-hook 'pre-command-hook #'si-record)
(defun si-invisible-lines ()
  "Lines of the current buffer some invisible text hides."
  (let ((pos (point-min)) (lines 0))
    (while (< pos (point-max))
      (let ((next (next-single-char-property-change pos 'invisible)))
        (when (invisible-p pos) (setq lines (+ lines (count-lines pos next))))
        (setq pos next)))
    lines))
(defun si-notmuch ()
  ;; The longest thread with an image part.
  (let* ((threads (notmuch-call-notmuch-sexp "search" "--format=sexp" "--format-version=5" "--output=summary" "mimetype:image/png"))
         (thread (car (sort threads (lambda (a b) (> (plist-get a :total) (plist-get b :total)))))))
    (notmuch-show (concat "thread:" (plist-get thread :thread)))
    (let ((n 0))
      (notmuch-show-mapc (lambda () (notmuch-show-message-visible (notmuch-show-get-message-properties) (cl-evenp (cl-incf n))))))
    (delete-other-windows) (goto-char (point-min))))
(defun si-org ()
  (switch-to-buffer (get-buffer-create "folds.org")) (erase-buffer)
  (dotimes (i 40)
    (insert (format "* Heading %d\n" i))
    (dotimes (j (+ 3 (% (* 7 i) 11))) (insert (format "Body line %d of heading %d %s\n" j i (if (= j 2) (make-string 180 ?w) "")))))
  (org-mode) (org-fold-show-all)
  ;; One row height in both engines' themes.
  (dolist (face (face-list))
    (when (string-prefix-p "org-" (symbol-name face)) (set-face-attribute face nil :height 'unspecified :box nil :inherit nil)))
  (goto-char (point-min))
  (let ((n 0)) (org-map-entries (lambda () (when (cl-oddp (cl-incf n)) (org-fold-hide-subtree)))))
  (delete-other-windows) (goto-char (point-min)))
(defun si-begin (scene margin)
  (funcall (intern (concat "si-" scene)))
  (setq-local scroll-margin margin)
  (setq scroll-conservatively 101 si-log nil)
  (redisplay)
  (list (symbol-name major-mode) (si-invisible-lines) (count-lines (point-min) (point-max)) (window-body-height)))
'''

NOTMUCH = r'''
(add-to-list 'load-path "/usr/share/emacs/site-lisp")
(autoload 'notmuch-show "notmuch" nil t)
(autoload 'notmuch-call-notmuch-sexp "notmuch-lib")
'''


def init_source():
    return f''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
{NOTMUCH}
{SCENES}
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def run(engine, key, held=0, limit=400):
    """Hold KEY for HELD seconds, if any, then type it until point stops
    moving (at most LIMIT times); the records of its commands."""
    if held:
        engine.keys(['-M', 'ctrl', '-P', key, '-s', str(int(held * 1000)), '-p', key, '-m', 'ctrl'])
        time.sleep(.5)
    last, same = None, 0
    for _ in range(limit):
        engine.keys(C(key))
        time.sleep(.06)
        point = engine.ask('(point)')
        same = same + 1 if point == last else 0
        if same >= 4:
            break
        last = point
    return engine.ask('(prog1 (reverse si-log) (setq si-log nil))')


def backwards(log, sign):
    """The steps of LOG (command point start row) whose start moved
    against SIGN, the motion's direction."""
    return [[a, b] for a, b in zip(log, log[1:]) if (b[2] - a[2]) * sign < 0]


def session(name, args, base):
    """Each scene's records in engine NAME: per key down and up, then held."""
    root, home, env = private_tree(f'scroll-invisible-{name}', base, args.tools)
    env.update(notmuch_fixture.build(home, ANCHOR))
    usage = root / 'usage.jsonl'
    env['ROMACS_USAGE_LOG'] = str(usage)
    engine = (gnu_engine(root, home, env, init_source(), None, args.timeout, args.tools) if name == 'gnu'
              else romacs_engine(str(args.romacs.resolve()), root, home, env, init_source(), None, args.timeout, args.tools))
    runs = []
    try:
        engine.fit_grid()
        for scene in ('notmuch', 'org'):
            for margin in (0, 3):
                setup = engine.ask(f'(si-begin {lisp_string(scene)} {margin})', timeout=120)
                time.sleep(.3)
                down, up = run(engine, 'n'), run(engine, 'p')
                held_down, held_up = run(engine, 'n', 2), run(engine, 'p', 2)
                runs.append({'scene': scene, 'margin': margin, 'setup': setup, 'down': down, 'up': up,
                             'held-keys': [len(held_down), len(held_up)],
                             'held-backwards': backwards(held_down, 1) + backwards(held_up, -1)})
        if name == 'romacs':
            engine.ask('(progn (romacs-usage-log-flush) t)')
    finally:
        engine.stop()
    errors = []
    for line in usage.read_text().splitlines() if usage.exists() else []:
        record = json.loads(line)
        if record.get('kind') == 'error' and not any(s in record.get('error', '') for s in ('quit', 'Beginning of buffer', 'End of buffer')):
            errors.append(record)
    return runs, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--binary', '--romacs', dest='romacs', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', default=os.path.dirname(os.environ.get('ROMACS_TEST_LABWC', '/usr/bin/labwc')))
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    args.output.mkdir(parents=True, exist_ok=True)
    base = Path(os.environ.get('TMPDIR', '/tmp'))
    gnu, _ = session('gnu', args, base)
    romacs, errors = session('romacs', args, base)
    report = {'romacs': str(args.romacs), 'errors': errors, 'runs': []}
    for g, r in zip(gnu, romacs):
        # Records as (command point start): rows count the same lines.
        same = lambda log: [entry[:3] for entry in log]
        differ = [{'key': i, 'gnu': a, 'romacs': b} for i, (a, b) in enumerate(zip(same(g['down'] + g['up']), same(r['down'] + r['up']))) if a != b]
        report['runs'].append({'scene': r['scene'], 'margin': r['margin'], 'setup': [g['setup'], r['setup']],
                               'keys': [len(g['down']) + len(g['up']), len(r['down']) + len(r['up'])],
                               'scrolls': len({entry[2] for entry in r['down'] + r['up']}),
                               'differ': differ, 'held-keys': r['held-keys'], 'held-backwards': r['held-backwards'],
                               'gnu-held-backwards': g['held-backwards'], 'gnu': g, 'romacs': r})
    # Each scene hid lines in both, scrolled many times, and Romacs's
    # records are GNU's, but for KNOWN differences; keys never scroll back.
    for run in report['runs']:
        run['known'] = KNOWN.get((run['scene'], run['margin'])) if run['differ'] else None
        run['backwards'] = backwards(run['romacs']['down'], 1) + backwards(run['romacs']['up'], -1)
    ok = len(report['runs']) == 4 and all(
        run['setup'][0][1] > 0 and run['setup'][0] == run['setup'][1] and run['scrolls'] > 10
        and run['keys'][0] == run['keys'][1] and (not run['differ'] or run['known'])
        and not run['held-backwards'] and not run['backwards'] for run in report['runs'])
    report['status'] = 'PASS' if ok and not errors else 'FAIL'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(report['status'], json.dumps([{k: run[k] for k in ('scene', 'margin', 'setup', 'keys', 'scrolls', 'held-keys')}
                                        | {'differ': len(run['differ']), 'known': bool(run['known']), 'backwards': len(run['backwards']),
                                           'held-backwards': len(run['held-backwards']),
                                           'gnu-held-backwards': len(run['gnu-held-backwards'])} for run in report['runs']]),
          json.dumps(errors)[:2000])
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
