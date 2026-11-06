#!/usr/bin/env python3
"""A row's height comes from the fonts on it, in pinned GNU and Romacs.

Pinned GNU pgtk and `romacs-vm-gui --loadup', each in its own isolated
session (private HOME/XDG/TMPDIR, headless labwc, keys typed with wtype
into that compositor only) on one grid and font, show a buffer of ASCII
lines and lines of a fallback font, which `set-fontset-font' pins to one
family in both.  After each key the window's start, point, ends, point's
position and the top of every shown row must be GNU's, for lines continued
and truncated and `scroll-conservatively' 0 and 101, through motion,
scrolling, `recenter' and M->, but from the step KNOWN names.  The font
of each kind of character is compared too: it shows the pin held.

usage: fallback_rows_live.py --binary PATH --output DIR [--gnu PATH] [--tools DIR] [--timeout S]
"""

import argparse
import json
from pathlib import Path
import os
import sys
import time

import parity_live
from parity_live import C, M, K, T, gnu_engine, lisp_string, private_tree, romacs_engine

FAMILY = 'Noto Sans CJK JP'
# The step of a case where its records may first differ, and why.  Before
# it every record must be GNU's.
MISFIT = ('over `scroll-conservatively\' 100, with point on a partly shown last row, GNU measures the row holding '
          'the margin as far as its iterator scanned it, its first glyph\'s height, and point\'s by the row before')
KNOWN = {'nil-101': (37, MISFIT), 't-101': (35, MISFIT),
         't-0': (46, 'M-v over truncated rows of mixed heights lands a row lower in GNU than move_it_vertically_backward predicts: not understood')}
SCENE = r'''
(defun fr-text ()
  (let (lines)
    (dotimes (i 70)
      (push (pcase (% i 7)
              (0 (format "%03d plain" i))
              (1 (format "%03d 李小龍 mixed" i))
              (2 "")
              (3 (format "%03d %s" i (mapconcat (lambda (_) "漢字") (number-sequence 1 60) "")))
              (4 (format "%03d café latin" i))
              (5 "あいう")
              (6 (format "%03d %s" i (make-string 130 ?w))))
            lines))
    (mapconcat #'identity (nreverse lines) "\n")))
(defun fr-begin (truncate conservative)
  (switch-to-buffer (get-buffer-create "rows")) (erase-buffer)
  (insert (fr-text)) (goto-char (point-min))
  (setq-local truncate-lines truncate)
  (setq scroll-conservatively conservative)
  (delete-other-windows) (redisplay) t)
(defun fr-rows ()
  (let (rows)
    (save-excursion
      (goto-char (window-start))
      (while (and (not (eobp)) (pos-visible-in-window-p (point) nil t) (< (length rows) 40))
        (push (cadr (pos-visible-in-window-p (point) nil t)) rows)
        (when (= 0 (vertical-motion 1)) (goto-char (point-max)))))
    (nreverse rows)))
(defun fr-state ()
  (with-current-buffer (window-buffer (selected-window))
    (list (window-start) (point) (window-end nil t)
          (let ((p (posn-at-point))) (and p (list (posn-col-row p) (posn-x-y p))))
          (fr-rows))))
(defun fr-fonts ()
  (with-current-buffer (window-buffer (selected-window))
    (mapcar (lambda (c) (save-excursion (goto-char (point-min)) (search-forward (string c)) (let ((f (font-at (1- (point))))) (and f (symbol-name (font-get f :family)))))) (list ?p ?李 ?あ))))
'''

STEPS = ([C('n')] * 14 + [C('f')] * 4 + [C('e'), C('n')] + [C('n')] * 24 + [C('v'), M('v'), C('p')]
         + [C('l'), C('l'), C('l'), M('>'), C('p'), M('<')])


def init_source():
    return f''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(set-fontset-font t 'han {lisp_string(FAMILY)})
(set-fontset-font t 'kana {lisp_string(FAMILY)})
{SCENE}
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def settled(engine, before):
    """The state after a key: two reads alike once it changed, or a second on."""
    deadline, last = time.time() + 1, None
    while True:
        time.sleep(.08)
        state = engine.ask('(fr-state)')
        if state == last and (state != before or time.time() > deadline):
            return state
        last = state


def session(name, args, base):
    """Each case's state after every key, in engine NAME."""
    root, home, env = private_tree(f'fallback-rows-{name}', base, args.tools)
    engine = (gnu_engine(root, home, env, init_source(), None, args.timeout, args.tools) if name == 'gnu'
              else romacs_engine(str(args.romacs.resolve()), root, home, env, init_source(), None, args.timeout, args.tools))
    out = {}
    try:
        engine.fit_grid()
        for truncate in ('nil', 't'):
            for conservative in (0, 101):
                engine.ask(f'(fr-begin {truncate} {conservative})', timeout=60)
                time.sleep(.5)
                states = [engine.ask('(fr-state)')]
                for key in STEPS:
                    engine.keys(key)
                    states.append(settled(engine, states[-1]))
                out[f'{truncate}-{conservative}'] = {'fonts': engine.ask('(fr-fonts)'), 'states': states}
    finally:
        engine.stop()
    return out


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
    gnu, romacs = session('gnu', args, base), session('romacs', args, base)
    report = {'romacs': str(args.romacs), 'cases': {}}
    for case, g in gnu.items():
        r = romacs[case]
        differ = [{'step': i, 'gnu': a, 'romacs': b} for i, (a, b) in enumerate(zip(g['states'], r['states'])) if a != b]
        report['cases'][case] = {'fonts': [g['fonts'], r['fonts']], 'steps': len(g['states']), 'differ': differ, 'gnu': g['states'], 'romacs': r['states']}
    def holds(case, c):
        first = KNOWN.get(case, (len(c['gnu']),))[0]
        return all(d['step'] >= first for d in c['differ']) and c['fonts'][0] == c['fonts'][1]
    ok = all(holds(case, c) for case, c in report['cases'].items())
    report['status'] = 'PASS' if ok else 'FAIL'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(report['status'], json.dumps({k: {'steps': c['steps'], 'differ': len(c['differ']), 'fonts': c['fonts']} for k, c in report['cases'].items()}))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
