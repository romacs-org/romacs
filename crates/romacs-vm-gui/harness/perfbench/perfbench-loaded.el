;;; perfbench-loaded.el --- what a perfbench run loaded  -*- lexical-binding: t -*-

;;; Commentary:
;; Loaded first in every perfbench session, batch or GUI, of either
;; engine.  perfbench.py checks what `perfbench-loaded' returns against
;; the compiled store's use log, so a load that read source shows.

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

;;; perfbench-loaded.el ends here
