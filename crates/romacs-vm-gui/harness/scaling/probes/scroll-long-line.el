;;; scroll-long-line.el --- scaling probe  -*- lexical-binding: t -*-

;; `scroll-up' and `scroll-down' a screen inside one logical line N
;; characters long, each step painted.  The new start comes from laying
;; the line out from its start, as GNU's iterator walks it from its
;; start: linear in the line, never worse.
(scaling-probe scroll-long-line :gui t :n 20000 :k 8 :steps 4
  :setup (let ((buffer (get-buffer-create "scaling-scroll-long")))
           (switch-to-buffer buffer)
           (erase-buffer)
           (insert "head\n" (mapconcat #'identity (make-list (/ n 5) "word") " ") "\ntail\n")
           (goto-char (point-min))
           (forward-line 1)
           (forward-char (/ n 8))
           (recenter 0)
           buffer)
  :body (with-current-buffer it
          (if (cl-evenp i) (scroll-up) (scroll-down))))
