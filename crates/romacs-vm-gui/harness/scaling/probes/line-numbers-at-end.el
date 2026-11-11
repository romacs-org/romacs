;;; line-numbers-at-end.el --- scaling probe  -*- lexical-binding: t -*-

;; Typing at the end of N numbered lines: a label comes from the text's line
;; index, not from counting the lines before it.
(scaling-probe line-numbers-at-end :gui t :n 20000 :steps 20 :budget 0.5
  :setup (let ((buffer (switch-to-buffer (get-buffer-create "scaling-line-numbers"))))
           (dotimes (_ n) (insert "a line of the buffer\n"))
           (setq display-line-numbers t)
           (goto-char (point-max))
           buffer)
  :body (with-current-buffer it (insert "x")))
