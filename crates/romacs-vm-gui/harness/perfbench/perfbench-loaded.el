;;; perfbench-loaded.el --- what a perfbench run loaded, and where it works  -*- lexical-binding: t -*-

;;; Commentary:
;; Loaded first in every perfbench session, batch or GUI, of either
;; engine.  perfbench.py checks what `perfbench-loaded' returns against
;; the compiled store's use log, so a load that read source shows.
;; `perfbench-middle' is the place in a file where both the batch syntax
;; rows and the GUI typing rows type.

;;; Code:

(defvar perfbench-loaded--start (length load-history)
  "How much of `load-history' there was before this file.")

(defun perfbench-loaded ()
  "The files loaded since this file began."
  (let ((new (- (length load-history) perfbench-loaded--start))
        (entries load-history)
        (files nil))
    (while (> new 0)
      (when (stringp (caar entries)) (push (caar entries) files))
      (setq entries (cdr entries) new (1- new)))
    files))

(defun perfbench-middle ()
  "Put point at the end of a code line in the middle of the buffer, before
its last character.  Found by text alone, so no syntax cache is warmed, and
the same place in every engine."
  (goto-char (/ (point-max) 2))
  (forward-line 0)
  (re-search-forward (if (derived-mode-p 'emacs-lisp-mode) "^ +([a-z].*)$" "^[ \t]+[a-z_]+ .*;$"))
  (backward-char 1))

;;; perfbench-loaded.el ends here
