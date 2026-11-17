;;; display-long-line-append.el --- scaling probe  -*- lexical-binding: t -*-

;; Text streamed onto the end of one long word-wrapped line, the window on
;; its last rows: each step appends 20 characters there, as a streamed
;; reply does.  The layout engine keeps the line's rows and wraps again
;; from the last row or two, so its work is flat in the line's length.
;; What still grows is the evaluator's: it publishes the whole logical
;; line for the window's text and again for each row question (0.56 from
;; 1,000 to 8,000 words, from 0.95 before the engine kept rows).
(scaling-probe display-long-line-append :gui t :n 1000 :k 8 :steps 20 :budget 0.4 :known "romacs#145"
  :setup (let ((buffer (scaling--wrapped-line n t)))
           (goto-char (1- (point-max)))
           buffer)
  :body (with-current-buffer it
          (goto-char (1- (point-max)))
          (insert "streamed words more ")))
