#!/usr/bin/env python3
"""Display measurements in pinned GNU and in Romacs GUI --loadup, compared.

Each engine runs in its own private headless labwc with a private
HOME/XDG/TMPDIR (parity_live.py), both in Iosevka 14 on the same grid, GNU
without fringes, menu, tool or scroll bars, as Romacs draws its frame.  A
case inserts text into a buffer shown in the frame's one window, lets the
engine redisplay, and asks the same `window-text-pixel-size',
`buffer-text-pixel-size', `string-pixel-width', `pos-visible-in-window-p',
`posn-at-point' and `vertical-motion' forms of both.  The cases are the
shapes emacs-hermes's chat and the sessions browser measure (VM2-WTPS1):
its composer rule (a `(space :width 2)' stretch, faced text, an
`:align-to right' fill) measured in a work buffer against a chat window,
the full-width `(space :width text)' separator, `:align-to' columns,
invisible detail text, overlay strings, an isearch match's overlay face,
and the MODE-LINES, X-LIMIT and Y-LIMIT arguments.

Every answer must be GNU's exactly; a Romacs signal is a failure.

usage: wtps_live.py --romacs BIN [--gnu EMACS] --output DIR [--case NAME]...
"""
import argparse
import json
import os
from pathlib import Path
import sys
import tempfile
import time

from parity_live import LispError, lisp_string, private_tree, gnu_engine, romacs_engine
import parity_live

FACES = '''
(defface wtps-separator '((t :strike-through "gray30" :foreground "gray30")) "The chat rule's face.")
(defface wtps-big '((t :height 1.5)) "A taller face.")
'''

# The text hermes-chat--status-rule-decorate builds around the rule's
# status, and the fill hermes-chat--status-rule-line appends.
RULE = ('(concat (propertize " " (quote display) (quote (space :width 2)) (quote face) (quote wtps-separator)) " "'
        ' (propertize "● Running" (quote face) (quote warning)) " " (propertize "0:03" (quote face) (quote shadow))'
        ' (propertize " · " (quote face) (quote shadow)) (propertize "3 queued" (quote face) (quote font-lock-type-face)) " ")')
FILL = '(propertize " " (quote display) (quote (space :align-to right)) (quote face) (quote wtps-separator))'
# hermes-chat--footer-measure: the string in a work buffer, measured as
# the chat WINDOW shows it.
MEASURE = ('(with-current-buffer (get-buffer-create " *wtps-measure*") (erase-buffer) (insert {})'
           ' (car (buffer-text-pixel-size nil (selected-window) t)))').format

# (NAME, INSERT, FORMS): INSERT runs in the shown buffer *wtps*, emptied
# first, then each of FORMS is asked; a number among them waits that many
# seconds, for the engines to redisplay.
CASES = [
    ('rule-work-buffer', '(insert "chat transcript\\n")', [
        MEASURE(RULE),
        MEASURE(f'(concat {RULE} {FILL})'),
        f'(string-pixel-width {RULE})',
    ]),
    ('rule-in-window', f'(insert "transcript\\n" {RULE} {FILL} "\\n> draft")', [
        '(window-text-pixel-size nil 12 (save-excursion (goto-char 12) (line-end-position)))',
        '(window-text-pixel-size nil 12 (save-excursion (goto-char 12) (line-end-position)) t)',
        '(window-text-pixel-size)', '(window-text-pixel-size nil nil nil nil nil t)',
        '(window-text-pixel-size nil nil nil nil nil (quote mode-line))',
        '(window-text-pixel-size nil nil nil 200 30)',
        '(pos-visible-in-window-p 12 nil t)', '(pos-visible-in-window-p 14 nil t)', '(pos-visible-in-window-p (point-max) nil t)',
        '(let ((p (posn-at-point 14))) (list (nth 1 p) (nth 2 p) (nth 4 p) (nth 9 p)))',
    ]),
    ('separator', '(insert "above\\n" (propertize " " (quote display) (quote (space :width text)) (quote face) (quote wtps-separator)) "\\nbelow")', [
        '(window-text-pixel-size nil 7 8)', '(window-text-pixel-size nil 1 13)', '(window-text-pixel-size)',
        '(pos-visible-in-window-p 7 nil t)', '(pos-visible-in-window-p 9 nil t)',
        '(let ((p (posn-at-point 7))) (list (nth 1 p) (nth 2 p) (nth 9 p)))',
        '(progn (goto-char 1) (list (vertical-motion 1) (point) (vertical-motion 1) (point)))',
    ]),
    ('align-columns', '(dotimes (i 3) (insert (format "row%d" i) (propertize " " (quote display) (list (quote space) :align-to (+ 12 i))) "col" (propertize " " (quote display) (quote (space :align-to 30))) "end\\n"))', [
        '(window-text-pixel-size nil 1 (save-excursion (goto-char 1) (line-end-position)))',
        '(window-text-pixel-size)', '(window-text-pixel-size nil nil nil 150)',
        '(pos-visible-in-window-p 5 nil t)', '(pos-visible-in-window-p 6 nil t)', '(pos-visible-in-window-p 10 nil t)',
        '(progn (goto-char 6) (list (vertical-motion 1) (point) (vertical-motion (cons 14 1)) (point)))',
        '(let ((p (posn-at-point 9))) (list (nth 1 p) (nth 2 p) (nth 6 p) (nth 9 p)))',
    ]),
    ('invisible-detail', '(setq buffer-invisibility-spec (list t)) (insert "tool call\\n" (propertize "hidden detail\\nmore\\n" (quote invisible) (quote detail)) "after\\n") (add-to-invisibility-spec (quote detail))', [
        '(window-text-pixel-size)', '(window-text-pixel-size nil 1 30)', '(pos-visible-in-window-p 30 nil t)',
        '(progn (goto-char 1) (list (vertical-motion 1) (point)))',
    ]),
    ('overlay-strings', '(insert "first\\nsecond\\nthird") (overlay-put (make-overlay 7 8) (quote before-string) (propertize "» " (quote face) (quote shadow))) (overlay-put (make-overlay 13 13) (quote after-string) "  (edited)")', [
        '(window-text-pixel-size nil 7 13)', '(window-text-pixel-size)', '(pos-visible-in-window-p 7 nil t)', '(pos-visible-in-window-p 14 nil t)',
        '(let ((p (posn-at-point 8))) (list (nth 1 p) (nth 2 p) (nth 9 p)))',
    ]),
    ('own-row-before-string', '(insert "first line\\nwhich fuse") (overlay-put (make-overlay 1 11) (quote before-string) (propertize " " (quote display) (quote (space :align-to 130))))', [
        '(window-text-pixel-size nil 1 11)', '(window-text-pixel-size nil 12 22 1000)', '(window-text-pixel-size)',
        '(pos-visible-in-window-p 1 nil t)',
    ]),
    ('isearch-match', '(insert "a session about steam and nginx\\nsteam again\\n") (let ((o (make-overlay 17 22))) (overlay-put o (quote face) (quote isearch)) (overlay-put o (quote priority) 1001))', [
        '(pos-visible-in-window-p 22 nil t)', '(pos-visible-in-window-p 17 nil t)', '(window-text-pixel-size nil 1 33)',
        '(let ((p (posn-at-point 17))) (list (nth 1 p) (nth 2 p) (nth 9 p)))',
    ]),
    ('display-string', '(insert "a" (propertize "b" (quote display) "DISPLAYED") "c\\n" (propertize "x" (quote display) (propertize "BIG" (quote face) (quote wtps-big))) "y")', [
        '(window-text-pixel-size nil 1 4)', '(window-text-pixel-size)', '(pos-visible-in-window-p 2 nil t)', '(pos-visible-in-window-p 3 nil t)',
        '(let ((p (posn-at-point 2))) (list (nth 1 p) (nth 2 p) (nth 4 p) (nth 9 p)))',
    ]),
    ('ellipsis', '(setq buffer-invisibility-spec (list (cons (quote hide) t))) (insert "ab" (propertize "xyz" (quote invisible) (quote hide)) "cd\nnext")', [
        '(window-text-pixel-size)', '(window-text-pixel-size nil 1 8)',
        '(pos-visible-in-window-p 2 nil t)', '(pos-visible-in-window-p 3 nil t)', '(pos-visible-in-window-p 4 nil t)',
        '(pos-visible-in-window-p 6 nil t)', '(pos-visible-in-window-p 7 nil t)',
    ]),
    ('empty-display', '(insert "a" (propertize "b" (quote display) "") "\nc" (propertize "de" (quote invisible) t))', [
        '(window-text-pixel-size)', '(pos-visible-in-window-p 2 nil t)', '(pos-visible-in-window-p 3 nil t)',
        '(pos-visible-in-window-p 5 nil t)', '(pos-visible-in-window-p 6 nil t)', '(pos-visible-in-window-p 7 nil t)',
    ]),
    # isearch splits off a one-line window on a slow terminal (isearch-update).
    ('terminal', '(insert "x")', ['baud-rate', '(<= baud-rate search-slow-speed)']),
    # The session browser's C-v: a header line over rows padded with spaces.
    ('scroll-header', '(setq header-line-format " Session Title") (dotimes (i 200) (insert (format "id%03d" i) (propertize " " (quote display) (quote (space :align-to 12))) "title" (propertize " " (quote display) (quote (space :align-to 40))) "9\n"))', [
        '(list (window-body-height) (window-start) (window-end nil t))',
        '(progn (scroll-up) (list (window-start) (point)))',
        '(progn (scroll-up) (list (window-start) (point)))',
        '(progn (goto-char (point-max)) (recenter -1) (list (window-start) (point)))',
    ]),
    # The session browser itself: a tabulated list, its header line, its
    # padded and ellipsized columns.
    ('tabulated-list', '(tabulated-list-mode) (setq tabulated-list-format [("Session" 24 t) ("Title" 36 t) ("Msgs" 6 t) ("Source" 10 t)]) (setq tabulated-list-entries (let (es) (dotimes (i 200) (push (list i (vector (format "20261022_%06d_b0ad48db%d" i i) (if (= (% i 3) 0) "A title long enough to be cut short by its column" "Short") (number-to-string i) "xmpp")) es)) (nreverse es))) (tabulated-list-init-header) (tabulated-list-print)', [
        # Its C-v and recenter move by lines, GNU's by pixels, which differ
        # under its taller header line (VM2-SCROLL1), so they are not asked.
        '(list (window-body-height) (window-start) (window-end nil t) (window-header-line-height))',
        '(list (window-body-height nil t) (count-lines 1 (window-end nil t)) (window-text-pixel-size nil 1 (window-end nil t)) (pos-visible-in-window-p (save-excursion (goto-char 1) (forward-line 30) (point)) nil t))',
        '(list (line-pixel-height) (save-excursion (goto-char 1) (forward-line 1) (line-pixel-height)))',
        '(progn (goto-char (point-min)) (forward-line 3) (list (vertical-motion 1) (point) (pos-visible-in-window-p (point) nil t)))',
        '(progn (goto-char (point-min)) (forward-line 2) (list (vertical-motion (cons 30 1)) (point) (cdr (posn-at-point))))',
        '(window-text-pixel-size nil (point-min) (save-excursion (goto-char (point-min)) (line-end-position)))',
    ]),
    # Lines a window hides show no row: what it shows is further on.
    ('hidden-lines', '(insert (propertize (make-string 100 ?\\n) (quote invisible) t) (propertize " " (quote display) (quote (space :width 3))) "VISIBLE\nend")', [
        '(window-end nil t)', '(window-end)', '(pos-visible-in-window-p 102 nil t)', '(window-text-pixel-size)',
    ]),
    # An overlay ending where the window starts shows its after-string there.
    ('after-string-at-start', '(insert "old\nnew\nend") (overlay-put (make-overlay 1 5) (quote after-string) "AFTER") (set-window-start nil 5)', [
        '(progn (set-window-start nil 5) (redisplay t) t)', 0.5,
        '(list (window-start) (pos-visible-in-window-p 5 nil t) (pos-visible-in-window-p 6 nil t))',
    ]),
    # Recentring chooses a start from the rows above point.
    ('recenter-above', '(dotimes (i 120) (insert (format "line %d" i) (propertize " " (quote display) (quote (space :width 2))) "x\n")) (setq-local scroll-conservatively 0)', [
        '(progn (goto-char (point-min)) (forward-line 60) (set-window-start nil (point)) (forward-line 10) t)', 0.5,
        '(window-start)', '(progn (forward-line -20) t)', 0.5, '(list (window-start) (point))',
    ]),
    ('multibyte-lines', '(dotimes (i 100) (insert "Ω\n"))', ['(window-text-pixel-size nil 1 3)', '(window-end nil t)']),
    # An overlay's string shows over the text's face, not the overlay's.
    ('overlay-string-face', '(insert "abc") (let ((o (make-overlay 1 2))) (overlay-put o (quote before-string) "BEFORE") (overlay-put o (quote face) (quote wtps-big)))', [
        '(window-text-pixel-size)', '(pos-visible-in-window-p 2 nil t)',
    ]),
    ('empty-with-string', '(overlay-put (make-overlay 1 1) (quote before-string) "BEFORE")', [
        '(window-text-pixel-size)', '(pos-visible-in-window-p 1 nil t)',
    ]),
    # What the iterator refuses further on refuses only once reached.
    ('refusal-ahead', '(insert "abc\n" (propertize " " (quote display) (quote (space :width 2 :height 2))) "x")', [
        '(window-text-pixel-size nil 1 4)', '(pos-visible-in-window-p 2 nil t)', '(progn (goto-char 2) (vertical-motion 0))',
    ]),
    # The owner's global-display-line-numbers-mode: numbers before each
    # row, which C-n, isearch, help and log windows measure past.
    ('line-numbers', '(setq display-line-numbers t) (dotimes (i 30) (insert (format "line %d" i) (if (= i 3) "\tTAB" "") (propertize " " (quote display) (quote (space :align-to 20))) "x\n"))', [
        '(line-number-display-width)', '(line-number-display-width t)', '(line-number-display-width (quote columns))',
        '(window-text-pixel-size)', '(window-text-pixel-size nil 1 (save-excursion (goto-char 1) (line-end-position)))',
        '(window-text-pixel-size nil nil nil (window-body-width nil t) (window-body-height nil t))',
        '(pos-visible-in-window-p 1 nil t)', '(pos-visible-in-window-p 8 nil t)', '(pos-visible-in-window-p (point-max) nil t)',
        '(let ((p (posn-at-point 8))) (list (nth 1 p) (nth 2 p) (nth 6 p) (nth 9 p)))',
        '(progn (goto-char 1) (next-line) (point))', '(progn (goto-char 5) (next-line 2) (list (point) (current-column)))',
        '(progn (goto-char 1) (list (vertical-motion (cons 12 1)) (point)))',
        '(progn (goto-char 1) (forward-line 3) (list (pos-visible-in-window-p (+ (point) 7) nil t) (pos-visible-in-window-p (+ (point) 12) nil t)))',
        # Alignments in (NUM . UNIT) count the number once, after the product.
        '(progn (goto-char (point-max)) (insert "a" (propertize " " (quote display) (quote (space :align-to (0.5 . text)))) "b" (propertize " " (quote display) (quote (space :align-to (2 . 30)))) "c") (list (pos-visible-in-window-p (- (point) 2) nil t) (pos-visible-in-window-p (1- (point)) nil t) (window-text-pixel-size nil (line-beginning-position) (point))))',
        # A window too narrow for its numbers shows none (maybe_produce_line_number).
        '(let ((w (split-window nil -9 t))) (prog1 (with-selected-window w (list (window-body-width) (line-number-display-width) (line-number-display-width t) (window-text-pixel-size w 1 4))) (delete-window w)))',
    ]),
    ('lnum-align', '(setq display-line-numbers t) (dolist (spec (quote ((space :align-to text) (space :align-to (0.5 . text)) (space :align-to (0.25 . text)) (space :width (0.5 . text)) (space :align-to (+ 10 (0.5 . text))) (space :align-to (2 . 30)) (space :align-to 20)))) (insert "a" (propertize " " (quote display) spec) "b\n"))', [
        '(let (xs) (goto-char 1) (while (search-forward "b" nil t) (push (car (pos-visible-in-window-p (1- (point)) nil t)) xs)) (nreverse xs))',
    ]),
    # A label longer than the width estimated shifts its row's text.
    ('lnum-overflow', '(setq display-line-numbers t) (insert "first\n" (propertize (make-string 1000 ?\\n) (quote invisible) t) "NEXT\nlast")', [
        '(list (line-number-display-width) (line-number-display-width t))',
        '(pos-visible-in-window-p 1008 nil t)', '(pos-visible-in-window-p 1013 nil t)',
        '(let ((p (posn-at-point 1009))) (list (nth 1 p) (nth 2 p) (nth 6 p)))',
    ]),
    # A display over hidden lines at the window start still shows there.
    ('lnum-display-hidden', '(setq display-line-numbers t) (insert (propertize (make-string 1000 ?\\n) (quote invisible) t (quote display) "SHOWN") "\nnext")', [
        '(list (line-number-display-width) (line-number-display-width t))', '(pos-visible-in-window-p (point-max) nil t)',
    ]),
    # An empty display over hidden lines shows nothing there.
    ('lnum-display-empty', '(setq display-line-numbers t) (insert (propertize (make-string 1000 ?\\n) (quote invisible) t (quote display) "") "next\nlast")', [
        '(list (line-number-display-width) (line-number-display-width t))', '(pos-visible-in-window-p (point-max) nil t)',
    ]),
    ('lnum-display-empty-list', '(setq display-line-numbers t) (insert (propertize (make-string 1000 ?\\n) (quote invisible) t (quote display) (list "")) "next\nlast")', [
        '(list (line-number-display-width) (line-number-display-width t))', '(pos-visible-in-window-p (point-max) nil t)',
    ]),
    ('lnum-display-empty-vector', '(setq display-line-numbers t) (insert (propertize (make-string 1000 ?\\n) (quote invisible) t (quote display) (vector "")) "next\nlast")', [
        '(list (line-number-display-width) (line-number-display-width t))', '(pos-visible-in-window-p (point-max) nil t)',
    ]),
    # Composed text at the window start still has its line's number.
    ('lnum-composed', '(setq display-line-numbers t) (insert "ab\ncd") (compose-region 1 2 ?a) (goto-char 1)', [
        '(list (line-number-display-width) (line-number-display-width t))',
    ]),
    # A non-replacing display over hidden lines hides with them.
    ('lnum-display-raise', '(setq display-line-numbers t) (insert (propertize (make-string 1000 ?\\n) (quote invisible) t (quote display) (quote (raise 0.5))) "next\nlast")', [
        '(list (line-number-display-width) (line-number-display-width t))', '(pos-visible-in-window-p (point-max) nil t)',
    ]),
    # An overlay string's relative space before point: point shows after it.
    ('relative-width-overlay', '(insert "ab") (overlay-put (make-overlay 1 1) (quote before-string) (propertize " " (quote display) (quote (space :relative-width 2)))) (goto-char 1)', [
        '(pos-visible-in-window-p 1 nil t)', '(posn-x-y (posn-at-point 1))', '(window-text-pixel-size)',
    ]),
    # Org's table separators: spaces as wide as the glyph they cover.
    ('relative-width', '(insert "| a" (propertize " " (quote display) (quote (space :relative-width 1))) "| bb" (propertize "W" (quote display) (quote (space :relative-width 2.5))) "|")', [
        '(window-text-pixel-size)', '(pos-visible-in-window-p 4 nil t)', '(pos-visible-in-window-p 5 nil t)', '(pos-visible-in-window-p 9 nil t)', '(pos-visible-in-window-p 10 nil t)',
    ]),
    # The label's digits reach the matrix's last row (required_matrix_height).
    ('line-numbers-start', '(setq display-line-numbers t) (dotimes (i 150) (insert "x\n"))', [
        '(progn (goto-char 135) (set-window-start nil 129) t)', 0.5, '(list (window-start) (line-number-display-width) (line-number-display-width t))',
        '(progn (goto-char 105) (set-window-start nil 101) t)', 0.5, '(list (window-start) (line-number-display-width) (line-number-display-width t))',
    ]),
    ('line-numbers-relative', '(setq display-line-numbers (quote relative)) (dotimes (i 200) (insert (format "row %d\n" i)))', [
        '(progn (set-window-start nil 1) (goto-char 1) (list (line-number-display-width) (line-number-display-width t)))', 0.5,
        '(progn (goto-char (point-min)) (forward-line 80) (set-window-start nil (point)) (forward-line 5) t)', 0.5,
        '(list (window-start) (line-number-display-width) (line-number-display-width t) (pos-visible-in-window-p (point) nil t))',
        '(progn (next-line) (next-line) (point))', '(window-text-pixel-size nil (window-start) (+ (window-start) 7))',
    ]),
    # Org's ellipsis: a display table that sets only its ellipsis.
    ('org-ellipsis', '(setq buffer-display-table (make-display-table)) (set-display-table-slot buffer-display-table 4 (vconcat (mapcar (lambda (c) (make-glyph-code c (quote wtps-big))) " ▼"))) (setq buffer-invisibility-spec (list (cons (quote outline) t))) (insert "* Heading" (propertize "\nbody\nmore" (quote invisible) (quote outline)) "\n* Next\n")', [
        '(window-text-pixel-size)', '(window-text-pixel-size nil 1 10)', '(pos-visible-in-window-p 1 nil t)', '(pos-visible-in-window-p 23 nil t)',
        '(progn (goto-char 1) (list (vertical-motion 1) (point)))',
    ]),
    # tab-bar's new tab reads the vscroll in canonical lines (C-x t 2).
    ('vscroll', '(insert "x")', [
        '(list (window-vscroll) (window-vscroll nil t))',
        '(list (set-window-vscroll nil 1.5) (window-vscroll nil t) (window-vscroll))',
        '(list (set-window-vscroll nil 50 t) (window-vscroll) (set-window-vscroll nil -3) (window-vscroll nil t))',
        '(progn (set-window-vscroll nil 0 t) (tab-bar-new-tab) (prog1 (length (tab-bar-tabs)) (tab-bar-close-tab)))',
    ]),
    # The calendar: a face-propertized grid in a window fitted to it.
    ('calendar', '(calendar)', [
        '(with-current-buffer "*Calendar*" (list (window-text-pixel-size (get-buffer-window)) (window-body-height (get-buffer-window))))',
        '(progn (calendar-exit) t)',
    ]),
    # Eshell's banner and prompt (em-prompt.el's `field', `read-only' and
    # faced prompt), disp-table loaded, `recenter' at its end.
    ('eshell-prompt', '(require (quote disp-table)) (insert "Welcome to the Emacs shell\n\n") (let ((prompt (propertize "~ $ " (quote field) (quote prompt) (quote read-only) t (quote face) (quote wtps-big) (quote rear-nonsticky) t))) (insert prompt))', [
        '(list (and standard-display-table t) (progn (goto-char (point-max)) (vertical-motion -32 (selected-window))) (point))',
        '(progn (goto-char (point-max)) (recenter) (list (window-start) (pos-visible-in-window-p (point) nil t)))',
        '(progn (goto-char (point-max)) (list (vertical-motion -1) (point) (window-text-pixel-size nil (line-beginning-position) (point-max))))',
    ]),
    ('overlay-face-size', '(insert "plain " (propertize "big" (quote face) (quote wtps-big)) " text") (overlay-put (make-overlay 1 6) (quote face) (quote wtps-big))', [
        # A glyph's Y is its row's top, not its baseline's (VM2-POSVIS1).
        '(window-text-pixel-size)', '(car (pos-visible-in-window-p 7 nil t))',
    ]),
]

FRAME_SETUP = {
    'gnu': '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1) (fringe-mode 0)\n',
    'romacs': '',
}


def init_source(engine):
    return f''';; -*- lexical-binding: t -*-
{FRAME_SETUP[engine]}(setq inhibit-startup-screen t initial-scratch-message nil)
{FACES}
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    lisp = Path(tempfile.mkdtemp(prefix=f'lisp-{engine}-', dir=base))
    if engine == 'gnu':
        return gnu_engine(root, home, env, init_source(engine), lisp, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init_source(engine), lisp, args.timeout, args.tools)


def answer(engine, form):
    try:
        return engine.ask(f'(with-current-buffer "*wtps*" {form})')
    except LispError as error:
        return {'signal': error.condition}


def run_case(engines, name, insert, forms):
    setup = ('(progn (switch-to-buffer (get-buffer-create "*wtps*")) (delete-other-windows)'
             ' (let ((inhibit-read-only t)) (erase-buffer) (delete-all-overlays) (kill-all-local-variables)'
             # GNU's kill-all-local-variables resets the spec (buffer.c:1163);
             # Romacs's keeps a case's, so each case starts from GNU's.
             ' (setq buffer-invisibility-spec t)'
             f' {insert}) (goto-char (point-min)) (set-window-start nil (point-min)) t)')
    for engine in engines:
        engine.ask(setup)
    # Let each engine redisplay the case before measuring it.
    time.sleep(.5)
    for engine in engines:
        engine.ask('(progn (redisplay t) t)')
    time.sleep(.3)
    results = []
    for form in forms:
        if isinstance(form, float):
            time.sleep(form)
            continue
        got = {e.name: answer(e, form) for e in engines}
        results.append({'form': form, **got, 'same': got['gnu'] == got['romacs']})
    return {'case': name, 'status': 'PASS' if all(r['same'] for r in results) else 'FAIL', 'results': results}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--case', action='append')
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'gnu': args.gnu, 'cases': []}
    engines = []
    try:
        for engine in ('gnu', 'romacs'):
            engines.append(launch(engine, Path(args.work), args))
        for engine in engines:
            engine.fit_grid()
        report['frame'] = {e.name: e.ask('(list (frame-char-width) (frame-char-height) (window-body-width nil t) (window-body-height nil t)'
                                        ' (window-mode-line-height))') for e in engines}
        for name, insert, forms in CASES:
            if args.case and name not in args.case:
                continue
            entry = run_case(engines, name, insert, forms)
            report['cases'].append(entry)
            print(name, entry['status'], flush=True)
        report['messages'] = {e.name: e.ask('(with-current-buffer (messages-buffer) (buffer-substring-no-properties (point-min) (point-max)))') for e in engines}
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        for engine in engines:
            for log in ('engine.log',):
                if (engine.root / log).exists():
                    (output / f'{engine.name}-{log}').write_bytes((engine.root / log).read_bytes())
            engine.stop()
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    ok = 'error' not in report and report['cases'] and all(c['status'] == 'PASS' for c in report['cases'])
    print('PASS' if ok else 'FAIL', output / 'report.json')
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
