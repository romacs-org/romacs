#!/usr/bin/env python3
"""Generate hidden_tail.el: `window-end', with and without UPDATE, and M->
and `recenter' over text hidden to the end of the buffer.

Each case hides a tail its own way: from a row's start, inside a line, from
a line's end, at a continuation point, with and without an ellipsis, by a
text property, an overlay or a spec, under narrowing, and beside controls
with nothing hidden at the end.  Each runs with `scroll-conservatively' 0
and 101, and goes on to `force-window-update' and an edit, which leave the
recorded end stale, and a `redisplay' with no paint, which makes it fresh.

It reuses point_visibility.el's frame and step functions, so that
tests/point_visibility.rs replays it the same way against pinned GNU X11's
answers (child_frames_oracle.py --probe hidden_tail.el).

usage: hidden_tail_gen.py
"""
from point_visibility_gen import HERE, PRELUDE

DEFS = r'''(defvar ht-lines 12)
(defun pv-text () (mapconcat (lambda (i) (format "%03d line" i)) (number-sequence 1 ht-lines) "\n"))
(defun pv-state () (mapcar (lambda (w) (list (window-start w) (window-point w) (window-end w) (window-end w t) (cdr (posn-col-row (posn-at-point (window-point w) w))))) (window-list nil 'never (frame-first-window))))
(defun ht-bol (line) (save-excursion (goto-char (point-min)) (forward-line (1- line)) (point)))
(defun ht-eol (line) (save-excursion (goto-char (ht-bol line)) (line-end-position)))
(defun ht-hide (kind from to) (pcase kind ('property (put-text-property from to 'invisible t)) ('overlay (overlay-put (make-overlay from to) 'invisible t)) ('spec (put-text-property from to 'invisible 'ht-fold) (add-to-invisibility-spec 'ht-fold)) ('ellipsis (put-text-property from to 'invisible 'ht-dots) (add-to-invisibility-spec '(ht-dots . t))) ('overlay-ellipsis (overlay-put (make-overlay from to) 'invisible 'ht-dots) (add-to-invisibility-spec '(ht-dots . t)))))
(defun ht-begin (config lines form) (setq ht-lines lines) (pv-begin config) (with-current-buffer "pv" (setq buffer-invisibility-spec '(t)) (eval form t) (goto-char (point-min))) (setq last-command nil))
(defun ht-motion () (with-current-buffer "pv" (save-excursion (goto-char (point-max)) (list (vertical-motion -19) (point)))))
(defun ht-force (object) (force-window-update object) (list (window-end) (window-end nil t)))
(defun ht-redisplay () (with-current-buffer "pv" (save-excursion (goto-char (point-min)) (insert "y"))) (redisplay t) (list (window-end) (window-end nil t) (window-start)))
(defun ht-config () (let ((c (current-window-configuration))) (switch-to-buffer (get-buffer-create "ht-other")) (redisplay t) (set-window-configuration c) (list (window-end) (window-end nil t))))
(defun ht-redisplay-end () (call-interactively #'end-of-buffer) (redisplay t) (list (window-start) (point) (window-end) (window-end nil t)))
(defun ht-edit () (with-current-buffer "pv" (save-excursion (goto-char (point-min)) (insert "x")) (list (window-end) (window-end nil t))))
'''

NL = '(progn (goto-char (point-max)) (insert "\\n"))'
TO_ZV = '(point-max)'
# Name, lines, and the form that hides the tail.
CASES = [
    ('plain', 12, 't'),
    ('plain-newline', 12, NL),
    ('row-property', 12, f"(ht-hide 'property (ht-bol 8) {TO_ZV})"),
    ('row-property-newline', 12, f"(progn {NL} (ht-hide 'property (ht-bol 8) {TO_ZV}))"),
    ('row-overlay', 12, f"(progn {NL} (ht-hide 'overlay (ht-bol 8) {TO_ZV}))"),
    ('row-spec', 12, f"(progn {NL} (ht-hide 'spec (ht-bol 8) {TO_ZV}))"),
    ('row-ellipsis', 12, f"(progn {NL} (ht-hide 'ellipsis (ht-bol 8) {TO_ZV}))"),
    ('row-overlay-ellipsis', 12, f"(progn {NL} (ht-hide 'overlay-ellipsis (ht-bol 8) {TO_ZV}))"),
    ('mid-line', 12, f"(progn {NL} (ht-hide 'property (+ (ht-bol 8) 3) {TO_ZV}))"),
    ('mid-line-ellipsis', 12, f"(progn {NL} (ht-hide 'ellipsis (+ (ht-bol 8) 3) {TO_ZV}))"),
    ('line-end', 12, f"(progn {NL} (ht-hide 'property (ht-eol 8) {TO_ZV}))"),
    ('line-end-ellipsis', 12, f"(progn {NL} (ht-hide 'ellipsis (ht-eol 8) {TO_ZV}))"),
    ('continuation', 12, f"(progn (goto-char (ht-eol 8)) (insert (make-string 250 ?c)) (ht-hide 'property (+ (ht-bol 8) 99) {TO_ZV}))"),
    ('after-string', 12, f"(progn {NL} (ht-hide 'property (ht-bol 8) {TO_ZV}) (overlay-put (make-overlay (ht-bol 8) (ht-bol 8)) 'after-string \"shown\"))"),
    ('narrowed', 30, f"(progn (ht-hide 'property (ht-bol 8) (ht-bol 14)) (narrow-to-region 1 (ht-bol 14)))"),
    ('narrowed-text-after', 30, f"(progn (ht-hide 'property (ht-bol 8) (ht-bol 10)) (narrow-to-region 1 (ht-bol 14)))"),
    ('all-hidden', 12, f"(ht-hide 'property (point-min) {TO_ZV})"),
    ('block-then-text', 12, f"(progn {NL} (ht-hide 'property (ht-bol 5) (ht-bol 9)))"),
    ('below-window', 80, f"(ht-hide 'property (ht-bol 50) {TO_ZV})"),
    ('at-window-end', 60, f"(ht-hide 'property (ht-bol 23) {TO_ZV})"),
    ('two-blocks', 80, f"(progn {NL} (ht-hide 'property (ht-bol 20) (ht-bol 40)) (ht-hide 'property (ht-bol 70) {TO_ZV}))"),
    ('long-block-above', 120, f"(progn {NL} (ht-hide 'property (ht-bol 30) (ht-bol 100)) (ht-hide 'property (ht-bol 115) {TO_ZV}))"),
]
STEPS = ['nil', "'(end-of-buffer)", "'(end-of-buffer)", "'(previous-line)", "'(end-of-buffer)",
         "'((call recenter -3))", "'(beginning-of-buffer)", "'((call goto-char (point-max)))", "'((call recenter -3))"]


# After `force-window-update' the end is stale; after a `redisplay' with no
# paint between, it is fresh again.
FRESHNESS = ['(pv-step nil)\n', '(ht-force nil)\n', '(pv-step nil)\n', '(ht-force (selected-window))\n',
             "(pv-step '(beginning-of-buffer))\n", '(ht-redisplay)\n', '(ht-redisplay-end)\n', '(pv-step nil)\n', '(ht-config)\n', '(pv-step nil)\n']


def main():
    header = (';;; hidden_tail.el --- window-end and M-> over a hidden tail  -*- lexical-binding: t -*-\n'
              ';; Generated by hidden_tail_gen.py; answered by pinned GNU X11\n'
              ';; (child_frames_oracle.py --probe), replayed by tests/point_visibility.rs.\n')
    out = [header, PRELUDE[PRELUDE.index('(progn'):], DEFS]
    for name, lines, form in CASES:
        for conservative in (0, 101):
            config = f'(set scroll-conservatively {conservative}) (set scroll-margin 0) (setq-local truncate-lines nil)'
            out.append(f';; {name}-{conservative}\n')
            out.append(f"(progn (ht-begin '({config}) {lines} '{form}) (pv-step nil))\n")
            out.extend(f'(pv-step {step})\n' for step in STEPS)
            out.append('(ht-motion)\n')
            out.extend(FRESHNESS)
            out.append('(ht-edit)\n')
    (HERE / 'hidden_tail.el').write_text(''.join(out))


if __name__ == '__main__':
    main()
