;;; display-long-line-window.el --- scaling probe  -*- lexical-binding: t -*-

;; A window that starts in the middle of one long word-wrapped line, a
;; character typed at point just below its start each step.  The rows
;; above the window's start are wrapped once and kept: a step lays out
;; the rows the window shows from the edit on, whatever the line's length
;; above it; what grows is the text published and hashed, a few
;; instructions a byte.
(scaling-probe display-long-line-window :gui t :n 1000 :k 8 :steps 20 :budget 0.4
  :setup (let ((buffer (scaling--wrapped-line n t)))
           (goto-char (/ (point-max) 2))
           (set-window-start nil (save-excursion (beginning-of-visual-line) (point)))
           (vertical-motion 2)
           buffer)
  :body (with-current-buffer it (insert "x")))
