#!/usr/bin/env python3
"""Motion and editing keys never fail for a layout gap.

One isolated `romacs-vm-gui --loadup' session (private HOME/XDG/TMPDIR,
headless labwc, keys typed with wtype into that compositor only) visits
the kinds of buffer daily use moves through: wrapped prose under
`visual-line-mode', Org with display properties and invisible text, with
`org-indent-mode' (line prefixes), Emacs Lisp under
`prettify-symbols-mode' (compositions),
*Messages*, a jabber chat, the agent chat, dired, notmuch over the
synthetic store (notmuch_fixture.py) and a vc log.  In each it types
C-n, C-p, M->, M-<, an isearch (C-s, a word, C-s, C-g) and C-SPC C-n
M-w.  The session's usage log (ROMACS_USAGE_LOG) must hold no error;
the layout questions answered by approximation, its `refusal' records,
are reported by class and major mode.

usage: layout_journey_live.py --binary PATH --output DIR [--tools DIR] [--timeout S]
       (--romacs is an alias of --binary)
"""

import argparse
import json
from pathlib import Path
import os
import shutil
import subprocess
import sys
import time

import parity_live
from parity_live import C, M, T, lisp_string, private_tree, romacs_engine
import notmuch_fixture

HARNESS = Path(__file__).resolve().parent
ROOT = HARNESS.parents[2]
ANCHOR = int(time.time()) // 60 * 60
PROSE = ('A paragraph of prose long enough to wrap several times in the window, '
         'with words, punctuation and a few longer tokens such as configuration-files. ') * 6

# Each scene's setup selects one buffer kind in the selected window.
SCENES = r'''
(defun journey-prose ()
  (switch-to-buffer (get-buffer-create "prose")) (erase-buffer)
  (dotimes (i 40) (insert (format "%d. %s\n\n" i (getenv "JOURNEY_PROSE"))))
  (visual-line-mode 1) (goto-char (point-min)))
(defun journey-org ()
  (switch-to-buffer (get-buffer-create "notes.org")) (erase-buffer)
  (dotimes (i 30)
    (insert (format "* Heading %d :tag:\n:PROPERTIES:\n:ID: %d\n:END:\nBody with [[https://example.org/%d][a link]], *bold*, ~code~ and an [fn:%d] footnote.\n%s\n** Sub heading\n- item one\n- item two\n" i i i i (getenv "JOURNEY_PROSE"))))
  (org-mode) (org-overview) (org-show-all) (goto-char (point-min))
  (org-cycle-hide-drawers 'all))
(defun journey-messages ()
  (with-current-buffer (messages-buffer)
    (let ((inhibit-read-only t))
      (dotimes (i 120) (insert (format "Message %d: %s\n" i (substring (getenv "JOURNEY_PROSE") 0 (+ 20 (* 3 (% i 40)))))))))
  (switch-to-buffer (messages-buffer)) (goto-char (point-min)))
(defun journey-jabber ()
  (switch-to-buffer (get-buffer-create "*-jabber-chat-friend@example.org-*")) (erase-buffer)
  (condition-case nil (progn (require 'jabber-chatbuffer) (jabber-chat-mode)) (error nil))
  (let ((inhibit-read-only t))
    (dotimes (i 40)
      (insert (propertize (format "[%02d:%02d] " (/ i 6) (% i 60)) 'face 'shadow)
              (propertize (if (cl-evenp i) "friend> " "me> ") 'face (if (cl-evenp i) 'font-lock-keyword-face 'font-lock-string-face))
              (substring (getenv "JOURNEY_PROSE") 0 (+ 30 (* 7 (% i 20)))) "\n")))
  (setq-local truncate-lines nil word-wrap t) (goto-char (point-min)))
(defun journey-agent ()
  (switch-to-buffer (get-buffer-create "*agent journey*")) (erase-buffer)
  (condition-case nil (progn (require 'romacs-agent-ui) (romacs-agent-chat-mode)) (error (fundamental-mode)))
  (let ((inhibit-read-only t))
    (dotimes (i 25)
      (insert (propertize "You\n" 'face 'bold) (getenv "JOURNEY_PROSE") "\n\n"
              (propertize "Agent\n" 'face 'bold) (getenv "JOURNEY_PROSE") "\n    (code block line " (number-to-string i) ")\n\n")))
  (setq-local truncate-lines nil word-wrap t) (goto-char (point-min)))
(defun journey-org-indent ()
  (journey-org) (rename-buffer "indented.org" t) (org-indent-mode 1) (goto-char (point-min)))
(defun journey-prettify ()
  (switch-to-buffer (get-buffer-create "pretty.el")) (erase-buffer)
  (dotimes (i 60) (insert (format "(defun journey-f%d (x) \"Doc %d.\" (lambda (y) (and (>= x y) (<= y %d) (not (null x)))))\n" i i i)))
  (emacs-lisp-mode) (prettify-symbols-mode 1) (font-lock-ensure) (goto-char (point-min)))
(defun journey-dired () (dired (getenv "JOURNEY_TREE")) (goto-char (point-min)))
(defun journey-notmuch-hello () (notmuch-hello))
(defun journey-notmuch-search ()
  (notmuch-search "tag:inbox")
  (let ((tries 0))
    (while (and (get-buffer-process (current-buffer)) (< tries 300)) (accept-process-output nil 0.1) (setq tries (1+ tries))))
  (goto-char (point-min)))
(defun journey-vc-log ()
  (let ((default-directory (getenv "JOURNEY_REPO"))) (vc-print-log))
  (let ((tries 0))
    (while (and (get-buffer "*vc-change-log*") (get-buffer-process "*vc-change-log*") (< tries 300))
      (accept-process-output nil 0.1) (setq tries (1+ tries))))
  (switch-to-buffer "*vc-change-log*") (delete-other-windows) (goto-char (point-min)))
'''

SCENE_NAMES = ('prose', 'org', 'org-indent', 'prettify', 'messages', 'jabber', 'agent', 'dired',
               'notmuch-hello', 'notmuch-search', 'vc-log')

NOTMUCH = r'''
(add-to-list 'load-path "/usr/share/emacs/site-lisp")
(autoload 'notmuch-hello "notmuch" nil t)
(autoload 'notmuch-search "notmuch" nil t)
'''

DOWN = [C('n')] * 12
KEYS = [C('p')] * 6 + [M('>'), M('<'), C('s'), T('the'), C('s'), C('s'), C('g'), C('g'),
        C('space'), C('n'), C('n'), M('w'), C('g')] + [C('n')] * 6 + [M('>')] + [C('p')] * 6 + [M('<')]


def init_source():
    return f''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
{NOTMUCH}
{SCENES}
(random "layout-journey")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def fixture_tree(home):
    """A directory of files for dired and a git repository for the vc log."""
    tree = home / 'tree'
    tree.mkdir()
    for i in range(60):
        (tree / f'file-{i:02d}-{"long-name-" * (i % 4)}.txt').write_text(f'{i}\n')
    repo = home / 'repo'
    repo.mkdir()
    git = ['git', '-C', str(repo), '-c', 'user.name=Journey', '-c', 'user.email=journey@example.org',
           '-c', 'commit.gpgsign=false']
    subprocess.run(git + ['init', '-q'], check=True)
    for i in range(40):
        (repo / 'notes.txt').write_text(f'{i}\n')
        subprocess.run(git + ['add', 'notes.txt'], check=True)
        subprocess.run(git + ['commit', '-q', '-m', f'Commit {i}: ' + PROSE[:40 + 5 * (i % 12)]], check=True)
    return tree, repo


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--binary', '--romacs', dest='romacs', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', default=os.path.dirname(os.environ.get('ROMACS_TEST_LABWC', '/usr/bin/labwc')))
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    base = Path(os.environ.get('TMPDIR', '/tmp'))
    root, home, env = private_tree('journey', base, args.tools)
    env.update(notmuch_fixture.build(home, ANCHOR))
    tree, repo = fixture_tree(home)
    usage = root / 'usage.jsonl'
    env.update(ROMACS_USAGE_LOG=str(usage), JOURNEY_PROSE=PROSE, JOURNEY_TREE=str(tree), JOURNEY_REPO=str(repo))
    report = {'romacs': str(args.romacs), 'scenes': [], 'errors': [], 'refusals': []}
    engine = romacs_engine(str(args.romacs.resolve()), root, home, env, init_source(), None, args.timeout, args.tools)
    try:
        for scene in SCENE_NAMES:
            mode = engine.ask(f'(progn (journey-{scene}) (redisplay) (symbol-name major-mode))', timeout=120)
            time.sleep(.5)
            for group in DOWN:
                engine.keys(group)
                time.sleep(.05)
            time.sleep(.3)
            moved = engine.ask('(point)')
            for group in KEYS:
                engine.keys(group)
                time.sleep(.05)
            time.sleep(.5)
            report['scenes'].append({'scene': scene, 'mode': mode, 'after-C-n': moved,
                                     'end': engine.ask('(list (point) (window-start) (buffer-size))')})
        engine.ask('(progn (romacs-usage-log-flush) t)')
    finally:
        engine.stop()
        # What a run leaves is its evidence in OUTPUT, and no home in TMPDIR.
        for name in ('engine.log', 'compositor.log', 'stacks.txt', usage.name):
            if (root / name).exists():
                shutil.copy(root / name, args.output / name)
        shutil.rmtree(root, ignore_errors=True)
    usage = args.output / usage.name
    commands = 0
    for line in usage.read_text().splitlines() if usage.exists() else []:
        record = json.loads(line)
        commands += record.get('n', 0) if record.get('kind') == 'command' else 0
        # The usage log tags the user's own: a C-g, the buffer's end, a refusal.
        if record.get('kind') == 'error' and not record.get('user'):
            report['errors'].append(record)
        if record.get('kind') == 'refusal':
            report['refusals'].append(record)
    report['commands'] = commands
    # Keys reached the session: it logged their commands and C-n moved point.
    moved = all(s['after-C-n'] > 1 for s in report['scenes'])
    report['status'] = 'PASS' if not report['errors'] and len(report['scenes']) == len(SCENE_NAMES) and moved and commands > 100 else 'FAIL'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(report['status'], json.dumps({'commands': commands, 'errors': report['errors'], 'refusals': report['refusals'],
                                        'moved': [(s['scene'], s['mode'], s['after-C-n']) for s in report['scenes']]}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
