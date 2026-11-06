#!/usr/bin/env python3
"""Generate pre_redisplay.el: when `pre-redisplay-function' and
`pre-redisplay-functions' run, and on which windows.

A global hook and buffer-local hooks log the window and buffer they run in,
and a function of our own in `pre-redisplay-function' logs the windows it
gets: nil for the selected window, t for all, or a list.  Each step is a
redisplay after a change that touches one window, another window, a buffer
two windows show, or the window layout.  GNU may call the hooks more than
once per redisplay, so a log is the set of calls.  An error in a hook must
not stop redisplay.

It reuses point_visibility.el's frame and step functions, so that
tests/point_visibility.rs replays it the same way against pinned GNU X11's
answers (child_frames_oracle.py --probe pre_redisplay.el).

usage: pre_redisplay_gen.py
"""
from point_visibility_gen import HERE, PRELUDE

DEFS = r'''(defvar pr-log nil)
(defun pv-text () (mapconcat (lambda (i) (format "%03d line" i)) (number-sequence 1 40) "\n"))
(defun pr-name (w) (if (windowp w) (format "%s%s" (buffer-name (window-buffer w)) (if (eq w (selected-window)) "*" "")) w))
(defun pr-note (&rest item) (unless (member item pr-log) (push item pr-log)))
(defun pr-hook (w) (pr-note 'hook (pr-name w) (buffer-name) (eq w (selected-window)) inhibit-redisplay inhibit-quit))
(defun pr-local (w) (pr-note 'local (pr-name w) (buffer-name)))
(defun pr-function (windows) (pr-note 'function (if (listp windows) (sort (mapcar #'pr-name windows) #'string<) windows)))
(defun pr-failing (_w) (pr-note 'failed) (error "pr failing"))
(defun pv-state () (prog1 (sort (mapcar #'prin1-to-string pr-log) #'string<) (setq pr-log nil)))
(defun pr-reset () (setq pr-log nil) (setq pre-redisplay-functions (list #'pr-hook)) (setq pre-redisplay-function #'redisplay--pre-redisplay-functions))
(defun pr-two () (let ((ignore-window-parameters t)) (delete-other-windows)) (let ((w (split-window))) (set-window-buffer w (get-buffer-create "other")) (with-current-buffer "other" (erase-buffer) (insert "other text\n")) w))
(defun pr-edit-other () (with-current-buffer "other" (goto-char (point-max)) (insert "more\n")))
(defun pr-local-minibuffer () (with-current-buffer (window-buffer (minibuffer-window)) (add-hook 'pre-redisplay-functions #'pr-local nil t)))
(defun pr-clear-minibuffer () (with-current-buffer (window-buffer (minibuffer-window)) (remove-hook 'pre-redisplay-functions #'pr-local t)))
(defun pr-three () (let ((other (pr-two))) (select-window (split-window)) (set-window-buffer (selected-window) "pv") (select-window (frame-first-window)) other))
(defun pr-pick-other () (select-window (get-buffer-window "other")))
(defun pr-pick-first () (select-window (frame-first-window)))
(defun pr-shared () (let ((w (next-window))) (set-window-buffer w "pv") w))
'''

CONFIG = "'((set scroll-conservatively 0) (set scroll-margin 0) (setq-local truncate-lines nil))"
SCENES = {
    'global': ['(pr-reset)', "(pv-step nil)", "(pv-step '(next-line))", "(pv-step '((call insert \"x\")))", "(pv-step '(forward-char))",
               '(progn (pv-state) (forward-line 1) (redisplay t) (pv-state))'],
    'two-windows': ['(progn (pr-two) (pr-reset))', "(pv-step nil)", "(pv-step '(next-line))", "(pv-step '((call insert \"x\")))",
                    "(pv-step '((call pr-edit-other)))", "(pv-step '((call pr-shared)))", "(pv-step '((call insert \"y\")))", "(pv-step '(other-window))", "(pv-step '(next-line))"],
    'local': ['(progn (pr-two) (pr-reset) (with-current-buffer "other" (add-hook (quote pre-redisplay-functions) (function pr-local) nil t)))',
              "(pv-step nil)", "(pv-step '((call pr-edit-other)))", "(pv-step '(other-window))", "(pv-step '(next-line))"],
    'local-no-global': ['(progn (pr-reset) (setq pre-redisplay-functions nil) (add-hook (quote pre-redisplay-functions) (function pr-local) nil t))',
                        "(pv-step nil)", "(pv-step '(next-line))"],
    'function': ['(pr-two)', "(pv-step '((call pr-reset) (set pre-redisplay-function pr-function)))", "(pv-step nil)", "(pv-step '(next-line))",
                 "(pv-step '((call insert \"x\")))", "(pv-step '((call pr-edit-other)))", "(pv-step '(other-window))", "(pv-step '(delete-other-windows))", "(pv-step nil)"],
    'nil-function': ['(progn (pr-reset) (setq pre-redisplay-function nil))', "(pv-step nil)", "(pv-step '(next-line))"],
    'error': ['(progn (pr-reset) (add-hook (quote pre-redisplay-functions) (function pr-failing)))', "(pv-step nil)", "(pv-step '(next-line))"],
    'cursor-sensor': ['(progn (pr-reset) (setq pre-redisplay-functions (list (function redisplay--update-region-highlight))) (with-current-buffer "pv" (put-text-property 10 20 (quote cursor-sensor-functions) (list (lambda (_w _old dir) (pr-note (quote sensor) dir (point))))) (cursor-sensor-mode 1)))',
                      "(pv-step nil)", "(pv-step '(next-line))", "(pv-step '(forward-char))", "(pv-step '(next-line))", "(pv-step '(next-line))", "(pv-step '(previous-line))"],
    'point-other': ['(progn (pr-two) (pr-reset))', "(pv-step nil)", "(pv-step '((call set-window-point (next-window) 3)))", "(pv-step '((call pr-edit-other) next-line))", "(pv-step nil)"],
    'minibuffer-local': ['(progn (pr-reset) (setq pre-redisplay-functions nil))',
                         "(pv-step '((call pr-local-minibuffer)))", "(pv-step nil)", "(pv-step '((call select-window (minibuffer-window))))", "(pv-step nil)", "(pv-step '((call select-window (next-window))))",
                         "(pv-step '((call pr-clear-minibuffer)))"],
    'region': ['(progn (pr-reset) (setq pre-redisplay-functions (list (function redisplay--update-region-highlight) (function pr-hook))))',
               "(pv-step '(set-mark-command))", "(pv-step '(next-line))", "(pv-step '((call deactivate-mark)))"],
    'three-windows': ['(progn (pr-three) (pr-reset))', "(pv-step nil)", "(pv-step '((call pr-pick-other)))", "(pv-step nil)", "(pv-step '((call pr-pick-first)))", "(pv-step nil)"],
}


def main():
    header = (';;; pre_redisplay.el --- pre-redisplay-function and its hook  -*- lexical-binding: t -*-\n'
              ';; Generated by pre_redisplay_gen.py; answered by pinned GNU X11\n'
              ';; (child_frames_oracle.py --probe), replayed by tests/point_visibility.rs.\n')
    out = [header, PRELUDE[PRELUDE.index('(progn'):], DEFS]
    for name, steps in SCENES.items():
        out.append(f';; {name}\n')
        out.append(f"(progn (pv-begin {CONFIG}) (setq pre-redisplay-functions nil) {steps[0]} (pv-step nil))\n")
        out.extend(f'{step}\n' for step in steps[1:])
    out.append('(progn (setq pre-redisplay-functions (list (function redisplay--update-region-highlight)) pre-redisplay-function (function redisplay--pre-redisplay-functions)) t)\n')
    (HERE / 'pre_redisplay.el').write_text(''.join(out))


if __name__ == '__main__':
    main()
